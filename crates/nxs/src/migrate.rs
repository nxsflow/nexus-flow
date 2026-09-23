//! `nxs migrate` (spec §6.3, TB-3) — the explicit CI/repair lever that raises the ONE shared
//! `.nxs/db.sqlite` to this binary's foundation schema version. Auto-migrate-on-open is the
//! default (every `<mod>` store-open already migrates); `migrate` makes it explicit and reports
//! the `from → to` transition. Foundation-only: it touches the substrate via the foundation's
//! schema spine and registers NO product reducer, so it works regardless of which modules are
//! active.

use crate::error::{ErrorKind, NxfError, Result};
use nxs_foundation::schema;
use nxs_foundation::workspace::Workspace;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// What `migrate` did to the shared substrate db.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrateReport {
    /// The schema version the db was stamped at before this run.
    pub from: i64,
    /// The schema version after this run (this binary's [`schema::SCHEMA_VERSION`], or higher if a
    /// newer peer had already migrated the shared file past us — never rolled back).
    pub to: i64,
    /// Whether the stamp actually advanced (false ⇒ already current, a clean no-op).
    pub changed: bool,
    /// The SessionStart wiring was out of date and has been rewritten. False ⇒ it was already the
    /// per-module set this binary wires, or the workspace has no hook of ours at all.
    ///
    /// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** the target used to be "the single `nxs prime`
    /// umbrella hook". It is now one entry per ACTIVE module — [`MigrateReport::hook_commands`]
    /// carries what this run actually wired, so neither a reader nor the success line has to guess.
    ///
    /// **Superseded 2026-08-29 (nxf 6j6v.sp5v): it used to say "a legacy `nxf prime` hook was
    /// found".** That was the whole of what [`converge_session_hook`] could see, and it was too
    /// narrow by exactly one shape — the intermediate single `nxs prime` hook, which is what 19 of
    /// 19 real workspaces were carrying when this was measured. The field now reports what its name
    /// always claimed: the wiring changed.
    pub hook_rewritten: bool,
    /// What the rewrite wired, in assembly order — empty when [`MigrateReport::hook_rewritten`] is
    /// false. Reported rather than re-derived, so the success line names the real set.
    pub hook_commands: Vec<String>,
    /// v0.6.0 umbrella upgrade (aye.32): active modules newly registered into `config.toml` — a
    /// pre-registry flow workspace gets `flow` registered here so `nxs prime` fans out to it.
    pub modules_registered: Vec<String>,
}

/// Raise the workspace's shared db to the current foundation schema version. Opens a raw
/// connection (no product store, no reducer), classifies the file against this binary's
/// version FIRST (a newer peer that raised the compatibility floor → loud refusal, zero writes),
/// then applies the forward-only, idempotent migration and reports `from → to`.
pub fn migrate(ws: &Workspace) -> Result<MigrateReport> {
    migrate_with(&crate::registry::roster(), ws)
}

/// [`migrate`] with the suite roster injected — the testable core. The `nxs` lib's own unit tests
/// run WITHOUT linking the module crates, so the inventory roster is empty there; they pass a roster
/// explicitly. The binary (and [`migrate`]) pass the real self-registered roster.
pub fn migrate_with(
    roster: &[&crate::registry::ModuleInit],
    ws: &Workspace,
) -> Result<MigrateReport> {
    let path = ws.db_path_str()?;
    let conn = Connection::open(&path).map_err(|e| db_err(&path, e))?;

    // Fail-loud BEFORE any write if a newer nxs raised the floor above us (spec §4.3).
    if let Some(msg) = schema::assess_open(&conn)
        .map_err(|e| db_err(&path, e))?
        .refusal()
    {
        return Err(NxfError::new(ErrorKind::Conflict, msg));
    }

    let from = schema::schema_version(&conn).map_err(|e| db_err(&path, e))?;
    // Ensure the substrate table exists (a no-op on a real workspace db) before migrating, so a
    // repair run on a half-written `.nxs/` still converges rather than erroring on a missing table.
    schema::try_apply(&conn).map_err(|e| db_err(&path, e))?;
    schema::migrate(&conn).map_err(|e| db_err(&path, e))?;
    let to = schema::schema_version(&conn).map_err(|e| db_err(&path, e))?;
    drop(conn); // release the db before the (filesystem-only) umbrella upgrade touches the workspace

    // Raise a pre-umbrella (v0.5.x) workspace to the v0.6.0 model: register every initialized-but-
    // inactive module active + converge an outdated SessionStart wiring. The same back-fill
    // `nxs prime` self-heals with (nexus-flow-cur) — shared so the two paths can never diverge.
    let modules_registered = crate::selfheal::backfill_active_modules(roster, ws)?;
    let root = ws.dir.parent().unwrap_or(ws.dir.as_path());
    // The ACTIVE modules, resolved from the config as the back-fill above LEFT it — `ws.config` is
    // the copy read before this call and does not know about anything `backfill_active_modules`
    // just registered. A pre-umbrella flow workspace is precisely the case where the two differ,
    // and reading the stale one would wire no hook at all for the module it just registered.
    let active_keys: Vec<String> = ws
        .config
        .active_modules
        .iter()
        .cloned()
        .chain(modules_registered.iter().cloned())
        .collect();
    let binaries: Vec<&str> = roster
        .iter()
        .filter(|m| active_keys.iter().any(|k| k == m.key))
        .map(|m| m.binary)
        .collect();
    let hook_commands = converge_session_hook(root, &binaries)?;

    Ok(MigrateReport {
        from,
        to,
        changed: from != to,
        hook_rewritten: !hook_commands.is_empty(),
        hook_commands,
        modules_registered,
    })
}

/// Bring `<root>/.claude/settings.json`'s SessionStart wiring up to what this binary wires: one
/// entry per active module, via the canonical assembler seam — so `migrate` can never drift from
/// how the hooks are wired everywhere else. Returns what it wired, empty when nothing changed.
///
/// **The condition is "this workspace already has a hook of OURS"**, and the removal set decides
/// what counts as ours: [`assembler::managed_hook_commands`], every command the assembler writes or
/// has ever written, for this workspace's binaries. A workspace whose SessionStart hooks are all
/// somebody else's — or which has no `.claude/settings.json` at all — is left untouched, because
/// `migrate` repairs a wiring that exists and does not decide that a user wants one.
///
/// **Superseded 2026-08-29 (nxf 6j6v.sp5v): it used to look for a bare `nxf prime` and nothing
/// else.** That was written when the only upgrade in existence was the v0.5.x one, and it silently
/// stopped covering the field: a workspace on the INTERMEDIATE form — a single `nxs prime` hook,
/// with or without the `|| cat NEXUS_MEMORY.md` tail — carries no `nxf prime` at all, so the strip
/// found nothing, this returned empty, and `migrate` left the umbrella hook in place while claiming
/// to be the repair lever. Measured on 2026-08-29: of 19 workspaces with an nxs hook, **19** were
/// on the intermediate form and `nxs migrate` would have converged none of them.
///
/// The fix is not a longer list of strings here but a shorter path: `SUPERSEDED_HOOK_COMMANDS` and
/// the per-binary forms already live in the assembler, `wire_session_hook` already drops what is
/// stale and ensures what is wanted, and this function's own copy of that logic (a bespoke strip
/// plus a second atomic settings writer) is gone. One removal set, one merge seam, two callers.
fn converge_session_hook(root: &Path, binaries: &[&str]) -> Result<Vec<String>> {
    let commands = crate::assembler::hook_commands(binaries);
    let managed = crate::assembler::managed_hook_commands(binaries);
    if !has_managed_session_hook(root, &managed)? {
        return Ok(Vec::new());
    }
    let report = crate::assembler::wire_session_hook(root, &commands, &managed, &["Bash(nxs:*)"])?;
    // Report what was WIRED only when the wiring actually moved. `wire_session_hook` is idempotent,
    // so reaching it says nothing about whether anything changed — and a `migrate` that announced a
    // rewrite on every steady-state re-run would be the same false map from the other side.
    Ok(if report.hook_added {
        commands
    } else {
        Vec::new()
    })
}

/// Whether any SessionStart hook in `<root>/.claude/settings.json` runs one of `managed` — this
/// workspace's opt-in signal, and the whole of what makes [`converge_session_hook`] a repair rather
/// than an installation.
///
/// An absent, empty or SessionStart-less settings file is `false`. A malformed one is a loud
/// `validation` error, never a clobber: the alternative is rewriting a file we could not read.
fn has_managed_session_hook(root: &Path, managed: &[String]) -> Result<bool> {
    let path = root.join(".claude").join("settings.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(NxfError::io(format!("reading {}: {e}", path.display()))),
    };
    if raw.trim().is_empty() {
        return Ok(false);
    }
    let settings: Value = serde_json::from_str(&raw)
        .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?;
    let Some(groups) = settings
        .get("hooks")
        .and_then(|h| h.get("SessionStart"))
        .and_then(Value::as_array)
    else {
        return Ok(false);
    };
    Ok(groups
        .iter()
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|h| h.get("command").and_then(Value::as_str))
        .any(|c| managed.iter().any(|m| m == c)))
}

fn db_err(path: &str, e: rusqlite::Error) -> NxfError {
    NxfError::io(format!("opening/migrating substrate db at {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::workspace::{self, WorkspaceConfig};
    use tempfile::TempDir;

    #[test]
    fn migrate_on_a_fresh_workspace_is_a_clean_no_op() {
        // `foundation::setup` already materializes the db at the current version, so an explicit
        // migrate right after is a no-op: from == to == SCHEMA_VERSION, changed == false.
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let report = migrate_with(&roster(), &ws).unwrap();
        assert_eq!(report.from, schema::SCHEMA_VERSION);
        assert_eq!(report.to, schema::SCHEMA_VERSION);
        assert!(!report.changed, "a current db needs no migration");
    }

    #[test]
    fn migrate_raises_a_legacy_v0_db_and_reports_the_transition() {
        // Simulate a pre-spine db (user_version 0, no `domain` column) under a real `.nxs/`, then
        // migrate: from 0 → SCHEMA_VERSION, changed.
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        // Roll the db back to a v0 shape behind the foundation's back.
        {
            let conn = Connection::open(ws.db_path()).unwrap();
            conn.execute_batch("ALTER TABLE ops DROP COLUMN domain;")
                .unwrap();
            conn.pragma_update(None, "user_version", 0).unwrap();
        }
        let report = migrate_with(&roster(), &ws).unwrap();
        assert_eq!(report.from, 0, "started at the legacy version");
        assert_eq!(report.to, schema::SCHEMA_VERSION);
        assert!(report.changed);
        // Idempotent: a second migrate is a clean no-op.
        let again = migrate_with(&roster(), &ws).unwrap();
        assert!(!again.changed);
    }

    #[test]
    fn migrate_refuses_a_db_written_by_a_newer_nxs() {
        // A db stamped past us with the compatibility floor raised above us → fail-loud, no write.
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let newer = schema::SCHEMA_VERSION + 1;
        {
            let conn = Connection::open(ws.db_path()).unwrap();
            conn.pragma_update(None, "user_version", newer).unwrap();
            conn.pragma_update(None, "application_id", newer).unwrap(); // floor above us
        }
        let err = migrate(&ws).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert!(
            err.msg.contains("upgrade"),
            "tells the user to upgrade: {}",
            err.msg
        );
    }

    // ---- v0.6.0 umbrella upgrade of a pre-umbrella (v0.5.x) workspace (aye.32) ----

    use crate::registry::{InitRequest, ModuleInit};
    use serde_json::Value;

    fn noop(_: &InitRequest) -> Result<()> {
        Ok(())
    }

    // The self-registered roster, as `static` descriptors so the umbrella-upgrade tests can inject a
    // populated roster (the `nxs` lib test binary links no module crate, so the real inventory roster
    // is empty here — the same reason prime/selfheal take the roster as a parameter).
    static FLOW: ModuleInit = ModuleInit {
        key: "flow",
        binary: "nxf",
        now_env: "NXF_NOW",
        blurb: "",
        recommended: true,
        order: 10,
        accepts_plugin: true,
        init_fn: noop,
        welcome: None,
        details: None,
        first_command: None,
    };
    static MEMORY: ModuleInit = ModuleInit {
        key: "memory",
        binary: "nxm",
        now_env: "NXM_NOW",
        blurb: "",
        recommended: false,
        order: 20,
        accepts_plugin: false,
        init_fn: noop,
        welcome: None,
        details: None,
        first_command: None,
    };
    fn roster() -> Vec<&'static ModuleInit> {
        vec![&FLOW, &MEMORY]
    }

    /// A simulated v0.5.x flow workspace: a `[flow]` product section with a chosen plugin, but NO
    /// `flow` in `active_modules` (the registry postdates v0.5.x), and a `.claude/settings.json`
    /// whose SessionStart hook runs the legacy `nxf prime` (plus an unrelated hook + nxf perm).
    fn legacy_flow_workspace(root: &std::path::Path) -> Workspace {
        let mut flow = toml::value::Table::new();
        flow.insert(
            "plugin".to_string(),
            toml::Value::String("issue-tracker".to_string()),
        );
        let mut products = toml::value::Table::new();
        products.insert("flow".to_string(), toml::Value::Table(flow));
        let cfg = WorkspaceConfig {
            active_modules: vec![], // v0.5.x: flow not yet registered
            products,
        };
        let ws = workspace::setup(root, &cfg).unwrap();
        let dir = root.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{
  "hooks": { "SessionStart": [
    { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] },
    { "matcher": "", "hooks": [ { "type": "command", "command": "nxf prime" } ] }
  ] },
  "permissions": { "allow": ["Bash(nxf:*)"] },
  "model": "opus"
}"#,
        )
        .unwrap();
        ws
    }

    fn settings(root: &std::path::Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(".claude/settings.json")).unwrap())
            .unwrap()
    }
    fn hook_commands(root: &std::path::Path) -> Vec<String> {
        let mut cmds: Vec<String> = settings(root)["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap().clone())
            .filter_map(|h| h["command"].as_str().map(String::from))
            .collect();
        cmds.sort();
        cmds
    }
    fn active_modules(ws_dir: &std::path::Path) -> Vec<String> {
        let raw = std::fs::read_to_string(ws_dir.join("config.toml")).unwrap();
        let cfg: WorkspaceConfig = toml::from_str(&raw).unwrap();
        cfg.active_modules
    }

    #[test]
    fn migrate_raises_a_legacy_workspace_onto_the_umbrella_model() {
        let tmp = TempDir::new().unwrap();
        let ws = legacy_flow_workspace(tmp.path());
        let report = migrate_with(&roster(), &ws).unwrap();

        // Flow's own per-module hook stands, and the unrelated `bd prime` hook is preserved.
        //
        // **Superseded 2026-08-29 (nxf 6j6v.1k6y): the legacy hook and the wanted hook are now the
        // SAME STRING here.** The two paragraphs this replaces read: the target "used to be the
        // single `nxs prime` umbrella hook. It is one entry per active module now — flow only,
        // here — and flow leads, so its entry carries the `|| cat NEXUS_MEMORY.md` tail. That tail
        // is also what distinguishes it from the LEGACY bare `nxf prime` this test starts from."
        //
        // The tail moved to memory's entry, and memory is not active in this v0.5.x fixture — so
        // `nxf prime` is exactly what this workspace should carry, and it already carries it.
        // Nothing about the hook needs rewriting, which is why `hook_rewritten` is now false. That
        // is the right answer, not a lost capability: the two things a v0.5.x workspace really
        // needs from `migrate` — the module registered active and the `Bash(nxs:*)` allowlist
        // entry — both still happen, and both are asserted below.
        assert_eq!(
            hook_commands(tmp.path()),
            crate::assembler::hook_commands(&["nxf"])
                .into_iter()
                .chain(["bd prime".to_string()])
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert!(
            !report.hook_rewritten,
            "nothing to rewrite: `nxf prime` is already what a flow-only workspace wants"
        );
        assert!(
            report.hook_commands.is_empty(),
            "and nothing is reported as wired"
        );
        // flow is now registered active, so `nxs prime` fans out to it as before.
        assert_eq!(active_modules(&ws.dir), vec!["flow".to_string()]);
        assert_eq!(report.modules_registered, vec!["flow".to_string()]);
        // The nxs hook now needs its allowlist entry; flow's is preserved, model untouched.
        let allow = settings(tmp.path())["permissions"]["allow"].clone();
        let allow = allow.as_array().unwrap();
        assert!(allow.iter().any(|v| v == "Bash(nxs:*)"), "nxs allowlisted");
        assert!(allow.iter().any(|v| v == "Bash(nxf:*)"), "nxf preserved");
        assert_eq!(settings(tmp.path())["model"], "opus", "unrelated keys kept");
    }

    #[test]
    fn the_umbrella_upgrade_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ws = legacy_flow_workspace(tmp.path());
        migrate_with(&roster(), &ws).unwrap();
        let settings_once =
            std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
        let config_once = std::fs::read_to_string(ws.dir.join("config.toml")).unwrap();

        // A second migrate (re-resolving the now-upgraded workspace) changes nothing.
        let ws2 = workspace::discover(tmp.path()).unwrap();
        let report = migrate_with(&roster(), &ws2).unwrap();
        assert!(!report.hook_rewritten, "no second rewrite");
        assert!(report.modules_registered.is_empty(), "flow already active");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap(),
            settings_once,
            "settings.json byte-stable across a re-run"
        );
        assert_eq!(
            std::fs::read_to_string(ws.dir.join("config.toml")).unwrap(),
            config_once,
            "config.toml byte-stable across a re-run"
        );
    }

    #[test]
    fn a_workspace_already_on_the_umbrella_model_is_a_clean_no_op() {
        // flow already active + the nxs prime hook already wired → migrate touches nothing.
        let tmp = TempDir::new().unwrap();
        let mut flow = toml::value::Table::new();
        flow.insert(
            "plugin".to_string(),
            toml::Value::String("issue-tracker".to_string()),
        );
        let mut products = toml::value::Table::new();
        products.insert("flow".to_string(), toml::Value::Table(flow));
        let ws = workspace::setup(
            tmp.path(),
            &WorkspaceConfig {
                active_modules: vec!["flow".to_string()],
                products,
            },
        )
        .unwrap();
        let report = migrate_with(&roster(), &ws).unwrap();
        assert!(!report.hook_rewritten);
        assert!(report.modules_registered.is_empty());
        assert!(
            !tmp.path().join(".claude").exists(),
            "migrate never conjures a hook where none existed"
        );
    }

    // ---- the INTERMEDIATE form: a single `nxs prime` umbrella hook (nxf 6j6v.sp5v) ----

    /// A workspace on the intermediate wiring: modules registered the modern way, but ONE
    /// `nxs prime` SessionStart hook — the shape `aye.32` wired and `nxf n2m6 + a2a1` replaced.
    /// `command` picks which of the two shipped forms it carries (bare, or with the `|| cat` tail).
    /// The unrelated `bd prime` hook and the `model` key are there to prove the rewrite is a merge.
    fn intermediate_form_workspace(
        root: &std::path::Path,
        command: &str,
        active: &[&str],
    ) -> Workspace {
        let ws = workspace::setup(
            root,
            &WorkspaceConfig {
                active_modules: active.iter().map(|s| s.to_string()).collect(),
                products: toml::value::Table::new(),
            },
        )
        .unwrap();
        let dir = root.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            format!(
                r#"{{
  "hooks": {{ "SessionStart": [
    {{ "matcher": "", "hooks": [ {{ "type": "command", "command": "bd prime" }} ] }},
    {{ "matcher": "", "hooks": [ {{ "type": "command", "command": "{command}" }} ] }}
  ] }},
  "permissions": {{ "allow": ["Bash(nxf:*)"] }},
  "model": "opus"
}}"#
            ),
        )
        .unwrap();
        ws
    }

    #[test]
    fn migrate_raises_the_intermediate_umbrella_hook_in_both_its_shipped_forms() {
        // The bug this closes: `nxs migrate` looked for a bare `nxf prime` and nothing else, so a
        // workspace on the intermediate form — which carries no `nxf prime` at all — came out
        // unchanged, with the umbrella hook still delivering the composed block that the per-hook
        // budget exists to split up. Both forms shipped, so both are tested.
        for command in ["nxs prime", "nxs prime || cat NEXUS_MEMORY.md 2>/dev/null"] {
            let tmp = TempDir::new().unwrap();
            let ws = intermediate_form_workspace(tmp.path(), command, &["flow", "memory"]);
            let report = migrate_with(&roster(), &ws).unwrap();

            assert!(
                report.hook_rewritten,
                "{command:?} is the intermediate form and has to be rewritten"
            );
            assert_eq!(
                report.hook_commands,
                crate::assembler::hook_commands(&["nxf", "nxm"]),
                "and rewritten to the per-module set, in assembly order"
            );
            assert_eq!(
                hook_commands(tmp.path()),
                crate::assembler::hook_commands(&["nxf", "nxm"])
                    .into_iter()
                    .chain(["bd prime".to_string()])
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
                "the umbrella entry is GONE (not topped up) and the foreign hook is preserved"
            );
            assert_eq!(settings(tmp.path())["model"], "opus", "unrelated keys kept");
        }
    }

    #[test]
    fn raising_the_intermediate_form_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ws = intermediate_form_workspace(tmp.path(), "nxs prime", &["flow"]);
        migrate_with(&roster(), &ws).unwrap();
        let once = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();

        let report = migrate_with(&roster(), &workspace::discover(tmp.path()).unwrap()).unwrap();
        assert!(!report.hook_rewritten, "no second rewrite");
        assert!(
            report.hook_commands.is_empty(),
            "and nothing to report as wired"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap(),
            once,
            "settings.json byte-stable across a re-run"
        );
    }

    #[test]
    fn a_module_that_joined_after_the_hooks_were_written_gets_its_entry_too() {
        // The other thing widening the condition bought (nxf 6j6v.sp5v), and it is worth pinning
        // rather than leaving as a side effect: a workspace whose per-module hooks were written
        // when only flow was active, and which has since registered memory, is INCOMPLETE, not
        // outdated — no entry there is stale, one is simply missing. The old strip could not see
        // that either, for the same reason. `migrate` is the repair lever, so it converges it.
        let tmp = TempDir::new().unwrap();
        let ws = intermediate_form_workspace(tmp.path(), "nxf prime", &["flow", "memory"]);

        let report = migrate_with(&roster(), &ws).unwrap();
        assert!(report.hook_rewritten);
        assert_eq!(
            hook_commands(tmp.path()),
            crate::assembler::hook_commands(&["nxf", "nxm"])
                .into_iter()
                .chain(["bd prime".to_string()])
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            "flow's bare entry gains the fallback tail it now leads with, and memory joins it"
        );
    }

    #[test]
    fn migrate_leaves_a_workspace_whose_session_hooks_are_all_foreign_alone() {
        // The other half of the contract, and the reason this is not simply "always wire": migrate
        // repairs a wiring somebody set up, it does not decide that they want one. A workspace with
        // only a neighbour's hook has never opted in, and a hook conjured here would print a block
        // nobody asked for at every session start.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        let untouched = r#"{
  "hooks": { "SessionStart": [
    { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] }
  ] }
}"#;
        std::fs::write(dir.join("settings.json"), untouched).unwrap();
        let ws = workspace::setup(
            tmp.path(),
            &WorkspaceConfig {
                active_modules: vec!["flow".to_string()],
                products: toml::value::Table::new(),
            },
        )
        .unwrap();

        let report = migrate_with(&roster(), &ws).unwrap();
        assert!(!report.hook_rewritten);
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            untouched,
            "not a byte moved"
        );
    }

    #[test]
    fn a_legacy_top_level_plugin_scalar_also_registers_flow() {
        // A pre-rework config (`plugin = "..."` at top level, no `[flow]` table) is still a flow
        // workspace — the upgrade registers flow from the legacy scalar too.
        let tmp = TempDir::new().unwrap();
        let cfg: WorkspaceConfig = toml::from_str("plugin = \"personal-todo\"\n").unwrap();
        let ws = workspace::setup(tmp.path(), &cfg).unwrap();
        let report = migrate_with(&roster(), &ws).unwrap();
        assert_eq!(report.modules_registered, vec!["flow".to_string()]);
        assert_eq!(active_modules(&ws.dir), vec!["flow".to_string()]);
    }
}
