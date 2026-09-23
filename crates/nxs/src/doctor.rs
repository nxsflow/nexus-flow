//! `nxs doctor` / `nxs status` (spec §6.3) — the cross-module diagnosis over the ONE shared
//! workspace: which modules are active, the foundation schema version (and whether this binary is
//! current/degraded/incompatible with the on-disk db), the replica identity, the sync-bound state,
//! the op count, and a db integrity check. Foundation-only: it reads the substrate via the schema
//! spine and registers NO product reducer, so the diagnosis is identical whatever modules are
//! active. `doctor` and `status` render the SAME report (status is the terse alias).

use crate::error::{NxfError, Result};
use nxs_foundation::schema::{self, SchemaCompat};
use nxs_foundation::workspace::Workspace;
use rusqlite::Connection;
use serde::Serialize;

/// This binary's schema standing relative to the on-disk db (spec §4.3), flattened for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaStatus {
    /// The db is at or below this binary — fully current after an (auto-)migrate.
    Current,
    /// The db is newer but within the compatibility floor — usable, degraded.
    Degraded,
    /// The db's floor is above this binary — refuse to touch it; upgrade nxs.
    Incompatible,
}

impl From<&SchemaCompat> for SchemaStatus {
    fn from(c: &SchemaCompat) -> SchemaStatus {
        match c {
            SchemaCompat::Current => SchemaStatus::Current,
            SchemaCompat::Degraded { .. } => SchemaStatus::Degraded,
            SchemaCompat::Incompatible { .. } => SchemaStatus::Incompatible,
        }
    }
}

/// The cross-module workspace diagnosis. Declared field order IS the `--json` order (serde
/// preserves it), so the machine output is byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnosis {
    /// The resolved `.nxs/` workspace directory.
    pub workspace: String,
    /// The replica's CRDT site id.
    pub site_id: i64,
    /// The replica's id prefix.
    pub prefix: String,
    /// The active modules in registry (append) order — the fan-out order `nxs prime` uses.
    pub active_modules: Vec<String>,
    /// The foundation schema version stamped in the db.
    pub schema_version: i64,
    /// The foundation schema version THIS binary speaks ([`schema::SCHEMA_VERSION`]).
    pub schema_supported: i64,
    /// This binary's standing relative to the db (current / degraded / incompatible).
    pub schema_status: SchemaStatus,
    /// Whether the workspace is bound to a sync stream (`.nxs/sync.toml` present).
    pub sync_bound: bool,
    /// The number of ops in the shared log.
    pub op_count: i64,
    /// How many `(lamport, site)` coordinates more than one op holds (6j6v.hehx #4). Zero on every
    /// log written since 6j6v.fc5p; a log written before it can carry a few, minted by two
    /// processes that shared a stale clock. On such a pair the CRDT order does not decide a
    /// concurrent edit — arrival order does — and nothing but this count makes that visible.
    pub duplicate_coordinates: i64,
    /// Whether `PRAGMA integrity_check` reports the db sound.
    pub integrity_ok: bool,
    /// beads config is still WIRED (a managed block in AGENTS.md/CLAUDE.md, or a `bd prime` hook)
    /// even though this nxs workspace exists — the signature of a half-finished beads → nxs
    /// migration (nexus-flow-6ef): e.g. a crash after the import but before the config rollback.
    /// The backup makes it recoverable; re-run `nxs init` once the `.nxs/` is removed, or remove the
    /// beads block/hook by hand. False on a clean workspace.
    pub beads_residue: bool,
}

/// Diagnose the resolved workspace. Opens a raw connection (no product store), so a db a newer
/// nxs marked incompatible is still *describable* — `doctor` reports the standing instead of
/// failing the way a store-open would (its job is to explain the workspace, not to use it).
pub fn diagnose(ws: &Workspace) -> Result<Diagnosis> {
    let path = ws.db_path_str()?;
    let conn = Connection::open(&path).map_err(|e| db_err(&path, e))?;

    let compat = schema::assess_open(&conn).map_err(|e| db_err(&path, e))?;
    let schema_version = schema::schema_version(&conn).map_err(|e| db_err(&path, e))?;
    let op_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
        .map_err(|e| db_err(&path, e))?;
    // The same count `ensure_op_coordinate_index` takes to decide whether the index can be UNIQUE,
    // served by that index either way.
    let duplicate_coordinates: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM ops GROUP BY lamport, site HAVING COUNT(*) > 1)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| db_err(&path, e))?;
    // `integrity_check(1)` stops at the first problem (fast); "ok" means sound.
    let integrity: String = conn
        .query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))
        .map_err(|e| db_err(&path, e))?;

    // beads residue (nexus-flow-6ef): the project root is the parent of the `.nxs/` dir. A still-
    // wired beads block/hook here (an `issues.jsonl` export is data, not config) means the migration
    // did not finish its rollback — surface it. Filesystem-only, no product reducer (doctor stays
    // foundation-only in spirit).
    let root = ws.dir.parent().unwrap_or(ws.dir.as_path());
    let beads = nxs_init::beads::detect(root);
    let beads_residue = beads.managed_block || beads.session_hook;

    Ok(Diagnosis {
        workspace: ws.dir.display().to_string(),
        site_id: ws.replica.site_id,
        prefix: ws.replica.prefix.clone(),
        active_modules: ws.config.active_modules.clone(),
        schema_version,
        schema_supported: schema::SCHEMA_VERSION,
        schema_status: SchemaStatus::from(&compat),
        sync_bound: ws.dir.join("sync.toml").is_file(),
        op_count,
        duplicate_coordinates,
        integrity_ok: integrity == "ok",
        beads_residue,
    })
}

fn db_err(path: &str, e: rusqlite::Error) -> NxfError {
    NxfError::io(format!("reading substrate db at {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::workspace::{self, WorkspaceConfig};
    use tempfile::TempDir;

    fn config(modules: &[&str]) -> WorkspaceConfig {
        WorkspaceConfig {
            active_modules: modules.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diagnose_reports_a_healthy_fresh_workspace() {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &config(&["flow"])).unwrap();
        let d = diagnose(&ws).unwrap();
        assert_eq!(d.active_modules, vec!["flow".to_string()]);
        assert_eq!(d.schema_version, schema::SCHEMA_VERSION);
        assert_eq!(d.schema_supported, schema::SCHEMA_VERSION);
        assert_eq!(d.schema_status, SchemaStatus::Current);
        assert!(!d.sync_bound, "a fresh workspace is not bound to a stream");
        assert!(d.integrity_ok, "a fresh db is sound");
        assert_eq!(d.site_id, ws.replica.site_id);
    }

    /// A log written before 6j6v.fc5p, reproduced: two different ops on one `(lamport, site)`.
    /// The index is dropped first because a log that starts clean gets it UNIQUE — which is the
    /// point of it, and exactly why such a log cannot be produced through the engine any more.
    #[test]
    fn diagnose_counts_coordinates_more_than_one_op_holds() {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &config(&["flow"])).unwrap();
        assert_eq!(diagnose(&ws).unwrap().duplicate_coordinates, 0);

        let conn = Connection::open(ws.db_path_str().unwrap()).unwrap();
        conn.execute_batch("DROP INDEX IF EXISTS ops_lamport_site;")
            .unwrap();
        // Shaped so that counting by lamport alone (7 ×3, 8 ×2) or by site alone (1 ×3, 2 ×2)
        // gives 2 — only the PAIR gives 1 (review of PR #487, Test Quality #7).
        for (op_id, lamport, site) in [
            ("a", 7, 1),
            ("b", 7, 1),
            ("c", 7, 2),
            ("d", 8, 1),
            ("e", 8, 2),
        ] {
            conn.execute(
                "INSERT INTO ops(op_id, lamport, site, target_kind, target_id, field, op_type)
                 VALUES (?1, ?2, ?3, 'item', 'x.1', 'title', 'set')",
                rusqlite::params![op_id, lamport, site],
            )
            .unwrap();
        }
        drop(conn);
        let d = diagnose(&ws).unwrap();
        assert_eq!(d.op_count, 5);
        assert_eq!(
            d.duplicate_coordinates, 1,
            "(7, 1) twice is one ambiguous coordinate; (7, 2), (8, 1) and (8, 2) are not"
        );
    }

    #[test]
    fn diagnose_preserves_active_module_order() {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &config(&["flow", "memory"])).unwrap();
        assert_eq!(
            diagnose(&ws).unwrap().active_modules,
            vec!["flow".to_string(), "memory".to_string()]
        );
    }

    #[test]
    fn diagnose_sees_a_bound_stream() {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        std::fs::write(ws.dir.join("sync.toml"), "stream_id = \"abc\"\n").unwrap();
        assert!(diagnose(&ws).unwrap().sync_bound);
    }

    #[test]
    fn diagnose_flags_beads_residue_when_beads_config_lingers() {
        // nexus-flow-6ef: a wired beads block/hook still present alongside an nxs workspace is the
        // signature of a half-finished migration. A clean workspace reports no residue.
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &config(&["flow"])).unwrap();
        assert!(
            !diagnose(&ws).unwrap().beads_residue,
            "a clean workspace has no beads residue"
        );
        // A leftover beads managed block at the project root (the parent of `.nxs/`).
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "<!-- BEGIN BEADS INTEGRATION v:1 -->\nx\n<!-- END BEADS INTEGRATION -->\n",
        )
        .unwrap();
        assert!(
            diagnose(&ws).unwrap().beads_residue,
            "a lingering beads block is flagged as residue"
        );
    }

    #[test]
    fn diagnose_describes_an_incompatible_db_without_failing() {
        // A db a newer nxs marked incompatible must still be *describable* — doctor explains the
        // standing rather than refusing the way a store-open would.
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let newer = schema::SCHEMA_VERSION + 1;
        {
            let conn = Connection::open(ws.db_path()).unwrap();
            conn.pragma_update(None, "user_version", newer).unwrap();
            conn.pragma_update(None, "application_id", newer).unwrap();
        }
        let d = diagnose(&ws).unwrap();
        assert_eq!(d.schema_status, SchemaStatus::Incompatible);
        assert_eq!(d.schema_version, newer);
    }
}
