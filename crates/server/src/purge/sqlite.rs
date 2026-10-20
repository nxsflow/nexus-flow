//! The SQLite purge store (design §3 SQLite).
//!
//! - **Opened without create**: a wrong path never creates an empty database. Each file must hold
//!   its tables and the `relay_store_identity` row the server's own open writes.
//! - **Dry runs and confirm cannot write**: `PRAGMA query_only = ON` refuses every statement that
//!   writes. It does NOT refuse `PRAGMA wal_checkpoint`, so the read-only store never issues one and
//!   its `delete` refuses outright; and so that being the last connection to a stopped relay's files
//!   neither checkpoints nor deletes the WAL (MF-3), it sets `wal_autocheckpoint = 0`,
//!   `SQLITE_FCNTL_PERSIST_WAL` and `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE` — the last is what actually
//!   stops the checkpoint SQLite runs when it closes the last connection; the first two alone do
//!   not (a test that leaves it out goes red).
//! - **The delete is one step under the write locks** (S1): `secure_delete`, `BEGIN IMMEDIATE` on
//!   the ops file and then, when it is a second file, on the registry file; the unknown-table
//!   guard; the plan read again and compared with `--plan` inside the transaction — any difference
//!   rolls back and is exit 4 with nothing deleted; one `DELETE` per table; COMMIT the ops file,
//!   then the registry file. A push lands wholly before (the re-read differs: 4) or wholly after
//!   (residue: 3).
//! - **The checkpoint** (S3): `wal_checkpoint(TRUNCATE)` per file, retried 10 times 500 ms apart
//!   while a reader holds an old snapshot. All done: `physical: "file_level"`; still busy: the
//!   logical delete stands, `physical: "pending"`, exit 5, and the next bound `--yes` (an empty
//!   plan) checkpoints again.
//!
//! The write lock is held for the re-read and one `DELETE` per table. The runbook gives the
//! measured lock times and the rule to stop the relay above 100,000 ops or 64 MB of values.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use nxs_sync::protocol::StreamId;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use super::{
    name_unknown_table, parse_rfc3339, plan_digest, Deleted, OpsFacts, Physical, Plan, PlanDigest,
    PlanKey, PlannedKind, PurgeError, PurgeStore, RunCtx, StoreIdentity,
};
use crate::store::StoreError;

/// How often the checkpoint is tried while a reader holds an old snapshot, and how far apart.
const CHECKPOINT_ATTEMPTS: u32 = 10;
const CHECKPOINT_PAUSE: Duration = Duration::from_millis(500);

/// The tables each file must hold.
const OPS_TABLES: &[&str] = &["stream_ops"];
const REGISTRY_TABLES: &[&str] = &["prefix_claims", "machine_presence"];

fn sql_err(e: rusqlite::Error) -> PurgeError {
    PurgeError::Store(StoreError(format!("sqlite: {e}")))
}

/// One relay store on SQLite: the op log's file and the registry's file, which may be one file.
pub struct SqlitePurgeStore {
    ops: Mutex<Connection>,
    /// `None` when the registry lives in the op log's file.
    registry: Option<Mutex<Connection>>,
    ops_path: PathBuf,
    registry_path: PathBuf,
    /// Opened for a dry run or a confirm: `delete` refuses, and nothing here checkpoints.
    read_only: bool,
}

impl SqlitePurgeStore {
    /// Open the two files named by `NXF_RELAY_DB` and `NXF_RELAY_REGISTRY_DB` (they may be the
    /// same). `read_only` for dry runs and confirm.
    pub fn open(
        ops_path: &str,
        registry_path: &str,
        read_only: bool,
    ) -> Result<SqlitePurgeStore, PurgeError> {
        let ops_path = existing("NXF_RELAY_DB", ops_path)?;
        let registry_path = existing("NXF_RELAY_REGISTRY_DB", registry_path)?;
        let ops = open_connection(&ops_path, read_only)?;
        let registry = if registry_path == ops_path {
            None
        } else {
            Some(Mutex::new(open_connection(&registry_path, read_only)?))
        };
        Ok(SqlitePurgeStore {
            ops: Mutex::new(ops),
            registry,
            ops_path,
            registry_path,
            read_only,
        })
    }

    fn one_file(&self) -> bool {
        self.registry.is_none()
    }
}

/// The canonical path of a file that exists, or `NotARelayStore` naming the variable.
fn existing(variable: &str, path: &str) -> Result<PathBuf, PurgeError> {
    if !Path::new(path).is_file() {
        return Err(PurgeError::NotARelayStore(format!(
            "{variable} names no existing file; the purge never creates one"
        )));
    }
    std::fs::canonicalize(path)
        .map_err(|_| PurgeError::NotARelayStore(format!("{variable} cannot be resolved")))
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection, PurgeError> {
    // READ_WRITE without CREATE: SQLite refuses a missing file instead of making one. Not
    // READ_ONLY for the dry run: on a WAL file whose `-shm` is gone a read-only open fails, and
    // `query_only` forbids every write just as well.
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(sql_err)?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(sql_err)?;
    if read_only {
        conn.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .map_err(sql_err)?;
        persist_wal(&conn)?;
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0; PRAGMA query_only = ON;")
            .map_err(sql_err)?;
    }
    Ok(conn)
}

/// `SQLITE_FCNTL_PERSIST_WAL = 1`: the WAL file is not deleted when this connection closes.
fn persist_wal(conn: &Connection) -> Result<(), PurgeError> {
    let mut on: std::os::raw::c_int = 1;
    // SAFETY: `handle()` is the live connection's pointer for the duration of the call; the
    // file-control op reads and writes the one `c_int` passed by pointer, which outlives it.
    let rc = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            (&mut on as *mut std::os::raw::c_int).cast(),
        )
    };
    if rc == rusqlite::ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(PurgeError::Store(StoreError(format!(
            "sqlite: SQLITE_FCNTL_PERSIST_WAL failed ({rc})"
        ))))
    }
}

fn lock(m: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Every table in the file, and the ones that have a `stream_id` column.
fn tables(conn: &Connection) -> Result<(Vec<String>, Vec<String>), PurgeError> {
    let mut all = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .map_err(sql_err)?;
    let all: Vec<String> = all
        .query_map([], |r| r.get(0))
        .map_err(sql_err)?
        .collect::<Result<_, _>>()
        .map_err(sql_err)?;
    let mut with = conn
        .prepare(
            "SELECT m.name FROM sqlite_master m JOIN pragma_table_info(m.name) p
             WHERE m.type = 'table' AND p.name = 'stream_id' ORDER BY m.name",
        )
        .map_err(sql_err)?;
    let with: Vec<String> = with
        .query_map([], |r| r.get(0))
        .map_err(sql_err)?
        .collect::<Result<_, _>>()
        .map_err(sql_err)?;
    Ok((all, with))
}

fn require(conn: &Connection, variable: &str, needed: &[&str]) -> Result<String, PurgeError> {
    let (all, _) = tables(conn)?;
    for table in needed
        .iter()
        .chain(std::iter::once(&"relay_store_identity"))
    {
        if !all.iter().any(|t| t == table) {
            return Err(PurgeError::NotARelayStore(format!(
                "the file {variable} names has no {table} table (start the current relay once \
                 against it, then purge)"
            )));
        }
    }
    let id: Option<String> = conn
        .query_row(
            "SELECT store_id FROM relay_store_identity WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(sql_err(other)),
        })?;
    id.ok_or_else(|| {
        PurgeError::NotARelayStore(format!(
            "the file {variable} names has no relay_store_identity row (start the current relay \
             once against it, then purge)"
        ))
    })
}

/// The unknown-table guard of one file: every table with a `stream_id` column that this file's
/// part of the purge does not delete from.
fn unknown_tables(conn: &Connection, covered: &[&str]) -> Result<Vec<String>, PurgeError> {
    let (_, with) = tables(conn)?;
    Ok(with
        .into_iter()
        .filter(|t| !covered.contains(&t.as_str()))
        .map(|t| name_unknown_table(&t))
        .collect())
}

/// What one plan read finds in the op log's file.
struct OpsRead {
    keys: Vec<PlanKey>,
    facts: OpsFacts,
}

fn read_ops(conn: &Connection, stream: &StreamId) -> Result<OpsRead, PurgeError> {
    // Bytes, not characters, and the value never leaves SQLite: it is measured where it lies.
    let mut stmt = conn
        .prepare(
            "SELECT seq, op_id, wall_clock, COALESCE(LENGTH(CAST(value AS BLOB)), 0)
             FROM stream_ops WHERE stream_id = ?1 ORDER BY seq",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([stream.as_str()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(sql_err)?;
    let mut keys = Vec::new();
    let mut facts = OpsFacts {
        value_bytes: Some(0),
        ..OpsFacts::default()
    };
    for row in rows {
        let (seq, op_id, wall_clock, bytes) = row.map_err(sql_err)?;
        keys.push(PlanKey::Op { seq, op_id });
        match parse_rfc3339(&wall_clock) {
            Some(t) => {
                facts.newest_wall_clock = Some(facts.newest_wall_clock.map_or(t, |n| n.max(t)))
            }
            None => facts.wall_clock_unparseable += 1,
        }
        facts.value_bytes = facts.value_bytes.map(|v| v + bytes.max(0) as u64);
    }
    Ok(OpsRead { keys, facts })
}

/// The registry rows: prefix claims and presence, and the newest `last_seen`.
fn read_registry(
    conn: &Connection,
    stream: &StreamId,
) -> Result<(PlannedKind, Option<i64>), PurgeError> {
    let mut kind = PlannedKind::new("registry");
    let mut claims = conn
        .prepare("SELECT prefix, replica_uuid FROM prefix_claims WHERE stream_id = ?1")
        .map_err(sql_err)?;
    for row in claims
        .query_map([stream.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql_err)?
    {
        let (prefix, uuid) = row.map_err(sql_err)?;
        kind.keys.push(PlanKey::Row {
            table: "prefix_claims",
            key: prefix,
            extra: uuid,
        });
        *kind.by_prefix.entry("prefix_claims".into()).or_insert(0) += 1;
    }
    let mut machines = conn
        .prepare("SELECT machine_id, last_seen FROM machine_presence WHERE stream_id = ?1")
        .map_err(sql_err)?;
    let mut newest: Option<i64> = None;
    for row in machines
        .query_map([stream.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(sql_err)?
    {
        let (machine, last_seen) = row.map_err(sql_err)?;
        kind.keys.push(PlanKey::Row {
            table: "machine_presence",
            key: machine,
            extra: String::new(),
        });
        *kind.by_prefix.entry("machine_presence".into()).or_insert(0) += 1;
        newest = Some(newest.map_or(last_seen, |n| n.max(last_seen)));
    }
    kind.keys.sort();
    Ok((kind, newest))
}

impl SqlitePurgeStore {
    /// The tables each connection's part of the purge deletes from.
    fn covered(&self) -> (&'static [&'static str], &'static [&'static str]) {
        if self.one_file() {
            (super::SQL_PURGE_TABLES, &[])
        } else {
            (OPS_TABLES, REGISTRY_TABLES)
        }
    }

    /// Read the plan from the open connections, each inside a transaction the caller holds (a
    /// read transaction for a plan, the write transaction for the re-read).
    fn read_plan(
        &self,
        ops: &Connection,
        registry: &Connection,
        stream: &StreamId,
    ) -> Result<Plan, PurgeError> {
        let (ops_covered, registry_covered) = self.covered();
        let mut unknown = unknown_tables(ops, ops_covered)?;
        if !self.one_file() {
            unknown.extend(unknown_tables(registry, registry_covered)?);
        }
        let read = read_ops(ops, stream)?;
        let (registry_kind, newest_last_seen) = read_registry(registry, stream)?;
        let mut ops_kind = PlannedKind::new("ops");
        ops_kind.keys = read.keys;
        Ok(Plan {
            kinds: vec![ops_kind, registry_kind],
            ops: read.facts,
            unknown,
            lease: None,
            newest_last_seen,
        })
    }
}

/// Run `f` inside `BEGIN` … `COMMIT` on each distinct connection (a consistent read per file).
fn in_read_transaction<T>(
    conns: &[&Connection],
    f: impl FnOnce() -> Result<T, PurgeError>,
) -> Result<T, PurgeError> {
    for c in conns {
        c.execute_batch("BEGIN").map_err(sql_err)?;
    }
    let result = f();
    for c in conns {
        let _ = c.execute_batch("COMMIT");
    }
    result
}

/// `wal_checkpoint(TRUNCATE)`, retried while a reader holds an old snapshot. `true` when it
/// completed (or the file is not in WAL mode, where there is nothing to checkpoint).
fn checkpoint(conn: &Connection) -> Result<bool, PurgeError> {
    for attempt in 0..CHECKPOINT_ATTEMPTS {
        let (busy, log, _done): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(sql_err)?;
        if busy == 0 || log == -1 {
            return Ok(true);
        }
        if attempt + 1 < CHECKPOINT_ATTEMPTS {
            std::thread::sleep(CHECKPOINT_PAUSE);
        }
    }
    Ok(false)
}

impl PurgeStore for SqlitePurgeStore {
    fn backend(&self) -> &'static str {
        "sqlite"
    }

    fn kinds(&self) -> &'static [&'static str] {
        &["ops", "registry"]
    }

    fn covers(&self) -> Vec<String> {
        vec![
            "ops".into(),
            "registry:prefix_claims".into(),
            "registry:machine_presence".into(),
        ]
    }

    fn identity(&self) -> Result<StoreIdentity, PurgeError> {
        let ops = lock(&self.ops);
        let (ops_needed, registry_needed): (&[&str], &[&str]) = if self.one_file() {
            (super::SQL_PURGE_TABLES, &[])
        } else {
            (OPS_TABLES, REGISTRY_TABLES)
        };
        let ops_id = require(&ops, "NXF_RELAY_DB", ops_needed)?;
        let registry_id = match &self.registry {
            Some(r) => require(&lock(r), "NXF_RELAY_REGISTRY_DB", registry_needed)?,
            None => ops_id.clone(),
        };
        let populated: bool = ops
            .query_row("SELECT EXISTS (SELECT 1 FROM stream_ops)", [], |r| r.get(0))
            .map_err(sql_err)?;
        let ops_path = self.ops_path.to_string_lossy().into_owned();
        let registry_path = self.registry_path.to_string_lossy().into_owned();
        let mut locator = serde_json::Map::new();
        locator.insert("ops_path".into(), json!(ops_path));
        locator.insert("registry_path".into(), json!(registry_path));
        Ok(StoreIdentity {
            backend: "sqlite",
            locator,
            identity: json!({
                "ops_path": ops_path,
                "ops_store_id": ops_id,
                "registry_path": registry_path,
                "registry_store_id": registry_id,
            }),
            populated,
        })
    }

    fn plan(&self, stream: &StreamId, ctx: &mut RunCtx<'_>) -> Result<Plan, PurgeError> {
        let ops = lock(&self.ops);
        let plan = match &self.registry {
            None => in_read_transaction(&[&ops], || self.read_plan(&ops, &ops, stream))?,
            Some(r) => {
                let registry = lock(r);
                in_read_transaction(&[&ops, &registry], || {
                    self.read_plan(&ops, &registry, stream)
                })?
            }
        };
        ctx.plan_page()?;
        Ok(plan)
    }

    fn delete(
        &self,
        stream: &StreamId,
        _plan: &Plan,
        expected: &PlanDigest,
        identity: &Value,
        ctx: &mut RunCtx<'_>,
        deleted: &mut Deleted,
    ) -> Result<Physical, PurgeError> {
        if self.read_only {
            return Err(PurgeError::Store(StoreError(
                "sqlite: the store was opened for a dry run; it does not delete".into(),
            )));
        }
        ctx.reached("S0")?;
        let ops = lock(&self.ops);
        let registry_guard = self.registry.as_ref().map(lock);
        let registry: &Connection = registry_guard.as_deref().unwrap_or(&ops);
        let conns: Vec<&Connection> = if self.one_file() {
            vec![&ops]
        } else {
            vec![&ops, registry]
        };
        for c in &conns {
            c.execute_batch("PRAGMA secure_delete = ON")
                .map_err(sql_err)?;
        }
        // Ops file first, registry second, always: the server never holds both, so this order
        // cannot deadlock against it.
        let locked = std::time::Instant::now();
        for (i, c) in conns.iter().enumerate() {
            if let Err(e) = c.execute_batch("BEGIN IMMEDIATE") {
                for held in &conns[..i] {
                    let _ = held.execute_batch("ROLLBACK");
                }
                return Err(sql_err(e));
            }
        }
        let rollback = |conns: &[&Connection]| {
            for c in conns {
                let _ = c.execute_batch("ROLLBACK");
            }
        };
        let fresh = match self.read_plan(&ops, registry, stream) {
            Ok(plan) => plan,
            Err(e) => {
                rollback(&conns);
                return Err(e);
            }
        };
        if !fresh.unknown.is_empty() {
            rollback(&conns);
            return Err(PurgeError::UnknownKind(fresh.unknown));
        }
        if plan_digest(identity, &fresh) != *expected {
            rollback(&conns);
            return Err(PurgeError::PlanChanged);
        }
        let deletes = (|| -> Result<(u64, u64), PurgeError> {
            let n_ops = ops
                .execute(
                    "DELETE FROM stream_ops WHERE stream_id = ?1",
                    [stream.as_str()],
                )
                .map_err(sql_err)?;
            let n_claims = registry
                .execute(
                    "DELETE FROM prefix_claims WHERE stream_id = ?1",
                    [stream.as_str()],
                )
                .map_err(sql_err)?;
            let n_machines = registry
                .execute(
                    "DELETE FROM machine_presence WHERE stream_id = ?1",
                    [stream.as_str()],
                )
                .map_err(sql_err)?;
            Ok((n_ops as u64, (n_claims + n_machines) as u64))
        })();
        let (n_ops, n_registry) = match deletes {
            Ok(counts) => counts,
            Err(e) => {
                rollback(&conns);
                return Err(e);
            }
        };
        if let Err(e) = ctx.reached("S1-tx") {
            rollback(&conns);
            return Err(e);
        }
        ops.execute_batch("COMMIT").map_err(|e| {
            rollback(&conns);
            sql_err(e)
        })?;
        deleted.add("ops", n_ops);
        if self.one_file() {
            deleted.add("registry", n_registry);
        } else {
            if let Err(e) = ctx.reached("S1") {
                let _ = registry.execute_batch("ROLLBACK");
                return Err(e);
            }
            registry.execute_batch("COMMIT").map_err(|e| {
                let _ = registry.execute_batch("ROLLBACK");
                sql_err(e)
            })?;
            deleted.add("registry", n_registry);
        }
        let lock_ms = locked.elapsed().as_millis();
        let checkpointing = std::time::Instant::now();
        let mut complete = true;
        for c in &conns {
            complete &= checkpoint(c)?;
        }
        // MF-11: the write lock's time, and the checkpoint's wait on top of it — the numbers the
        // runbook's "stop the relay above …" rule is measured with.
        ctx.progress(&format!(
            "deleted ops={n_ops} registry={n_registry} lock_ms={lock_ms} checkpoint_ms={}",
            checkpointing.elapsed().as_millis()
        ));
        ctx.reached("S3")?;
        Ok(if complete {
            Physical::FileLevel
        } else {
            Physical::Pending
        })
    }

    #[cfg(feature = "test-seams")]
    fn inject(&self, stream: &StreamId, write: super::InjectWrite) -> Result<(), StoreError> {
        use crate::presence::PresenceStore;
        use crate::registry::{PrefixRegistry, SqlitePrefixRegistry};
        use crate::store::{OpStore, SqliteOpStore};
        let ops_path = self.ops_path.to_string_lossy();
        let registry_path = self.registry_path.to_string_lossy();
        match write {
            super::InjectWrite::Op => {
                SqliteOpStore::open(&ops_path)?.append(stream, &super::injected_op())?;
            }
            super::InjectWrite::Machine => {
                let now = super::unix_secs(std::time::SystemTime::now());
                SqlitePrefixRegistry::open(&registry_path)?.announce(
                    stream,
                    &super::injected_hello(),
                    now,
                    i64::MIN,
                )?;
            }
            super::InjectWrite::Prefix => {
                let (prefix, uuid) = super::injected_claim();
                SqlitePrefixRegistry::open(&registry_path)?.register(stream, &prefix, &uuid)?;
            }
            super::InjectWrite::Steal => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay_file(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("relay.sqlite");
        let path = path.to_str().unwrap().to_string();
        crate::store::SqliteOpStore::open(&path).unwrap();
        crate::registry::SqlitePrefixRegistry::open(&path).unwrap();
        path
    }

    #[test]
    fn a_dry_run_connection_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = relay_file(&dir);
        let store = SqlitePurgeStore::open(&path, &path, true).unwrap();
        let conn = lock(&store.ops);
        let write = conn.execute("DELETE FROM stream_ops WHERE stream_id = 'x'", []);
        assert!(write.is_err(), "query_only must refuse a write");
    }

    #[test]
    fn a_dry_run_store_refuses_to_delete() {
        // query_only does not refuse `PRAGMA wal_checkpoint`; the store itself does not run one
        // when opened for a dry run, because its delete — the only caller — refuses.
        let dir = tempfile::tempdir().unwrap();
        let path = relay_file(&dir);
        let store = SqlitePurgeStore::open(&path, &path, true).unwrap();
        let seams = crate::purge::PurgeSeams::shipped();
        let clock = std::time::SystemTime::now;
        let mut sink = |_: &str| {};
        let mut ctx = crate::purge::RunCtx::for_test(&seams, &clock, &mut sink);
        let refused = store.delete(
            &StreamId("s".into()),
            &store.plan(&StreamId("s".into()), &mut ctx).unwrap(),
            &crate::purge::Sha256Hex([0; 32]),
            &serde_json::Value::Null,
            &mut ctx,
            &mut Deleted::default(),
        );
        assert!(matches!(refused, Err(PurgeError::Store(_))));
    }

    #[test]
    fn a_delete_connection_can_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = relay_file(&dir);
        let store = SqlitePurgeStore::open(&path, &path, false).unwrap();
        let conn = lock(&store.ops);
        conn.execute("DELETE FROM stream_ops WHERE stream_id = 'x'", [])
            .unwrap();
    }

    #[test]
    fn a_missing_file_is_refused_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sqlite");
        let path = path.to_str().unwrap();
        let refused = SqlitePurgeStore::open(path, path, true);
        assert!(matches!(refused, Err(PurgeError::NotARelayStore(_))));
        assert!(!Path::new(path).exists());
    }
}
