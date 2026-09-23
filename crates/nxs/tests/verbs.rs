//! End-to-end acceptance for the `nxs` foundation-only verbs (nexus-flow-aye.26): `migrate`,
//! `doctor`/`status` run against a real `.nxs/` workspace over the shared substrate, with NO
//! product binary involved (foundation-only). The workspace is materialized through the foundation
//! library — exactly the seam every `<mod> init` uses — so this proves the verbs operate on the
//! one shared log without any module being active.

use assert_cmd::Command;
use nxs_foundation::workspace::{self, WorkspaceConfig};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tempfile::TempDir;

/// A `.nxs/` workspace with the given active modules, materialized via the foundation seam.
fn workspace_with(modules: &[&str]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let config = WorkspaceConfig {
        active_modules: modules.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    workspace::setup(tmp.path(), &config).unwrap();
    tmp
}

fn nxs(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("nxs").unwrap();
    cmd.current_dir(dir);
    cmd
}

#[test]
fn doctor_json_reports_the_shared_workspace_diagnosis() {
    let tmp = workspace_with(&["flow", "memory"]);
    nxs(tmp.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        // declared field order is the contract: active_modules in append order, schema standing.
        .stdout(contains("\"active_modules\":[\"flow\",\"memory\"]"))
        .stdout(contains("\"schema_status\":\"current\""))
        .stdout(contains("\"sync_bound\":false"))
        .stdout(contains("\"integrity_ok\":true"));
}

#[test]
fn status_is_the_human_alias_of_doctor() {
    let tmp = workspace_with(&["flow"]);
    nxs(tmp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(contains("nxs workspace at"))
        .stdout(contains("modules:    flow"))
        .stdout(contains("schema:     v"))
        .stdout(contains("integrity:  ok"));
}

#[test]
fn migrate_on_a_current_workspace_is_a_clean_no_op() {
    let tmp = workspace_with(&["flow"]);
    nxs(tmp.path())
        .args(["migrate", "--json"])
        .assert()
        .success()
        .stdout(contains("\"changed\":false"));
    nxs(tmp.path())
        .arg("migrate")
        .assert()
        .success()
        .stdout(contains("already current"));
}

#[test]
fn verbs_outside_a_workspace_fail_loudly() {
    let tmp = TempDir::new().unwrap(); // no `.nxs/` here
    nxs(tmp.path())
        .args(["doctor", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"kind\":\"no_workspace\""));
}

// ---- prime fan-out (no-shell-out paths; the real fan-out to nxf/nxm is the slice-close E2E) ----

#[test]
fn prime_with_no_active_modules_yields_an_empty_fan_out() {
    // A workspace with no active module fans out to nothing — no shell-out, byte-stable empty set.
    let tmp = workspace_with(&[]);
    nxs(tmp.path())
        .args(["prime", "--json"])
        .assert()
        .success()
        .stdout(contains("\"modules\":[]"))
        .stdout(contains("\"now\":"));
    nxs(tmp.path())
        .arg("prime")
        .assert()
        .success()
        .stdout(contains("No active modules"));
}

// ---- v0.6.0 umbrella migration of a simulated v0.5.x workspace (aye.32) ----

#[test]
fn migrate_raises_a_legacy_v0_5_x_workspace_onto_the_umbrella_model() {
    // A simulated v0.5.x flow workspace: a legacy top-level `plugin` scalar (flow's), NOT yet
    // registered in active_modules, and a direct `nxf prime` SessionStart hook beside an unrelated
    // one. `nxs migrate` must register flow active and leave the wiring correct.
    //
    // **Superseded 2026-08-29 (nxf 6j6v.1k6y): the hook needs no rewriting here any more.** This
    // used to require "one entry per active module, so flow's own `nxf prime` WITH the fallback
    // tail (it leads)". The tail moved to memory's entry and memory is not active in a v0.5.x
    // workspace, so the legacy `nxf prime` and the wanted `nxf prime` are the SAME STRING — there
    // is nothing to rewrite, and saying so is the honest report. What this workspace really needs
    // from `migrate` still happens and is asserted: flow registered active, and the `Bash(nxs:*)`
    // allowlist entry. The rewrite line itself is covered end-to-end by the intermediate-form case
    // below, which is the shape workspaces in the field actually carried.
    // *(It rewrote to a single `nxs prime` hook until nxf n2m6 + a2a1.)*
    let tmp = workspace_with(&[]);
    std::fs::write(
        tmp.path().join(".nxs/config.toml"),
        "plugin = \"issue-tracker\"\n",
    )
    .unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{ "hooks": { "SessionStart": [
            { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] },
            { "matcher": "", "hooks": [ { "type": "command", "command": "nxf prime" } ] }
        ] } }"#,
    )
    .unwrap();

    nxs(tmp.path())
        .arg("migrate")
        .assert()
        .success()
        .stdout(contains("registered active module"))
        .stdout(contains("SessionStart wiring rewritten").not());

    // The hook this workspace already carried is the one it wants, and the allowlist entry the
    // umbrella needs was still added — the two halves that make the run worth making.
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    let commands: Vec<String> = settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect();
    assert!(commands.contains(&"nxf prime".to_string()), "{commands:?}");
    assert!(commands.contains(&"bd prime".to_string()), "{commands:?}");
    assert!(
        !commands.iter().any(|c| c.contains("NEXUS_MEMORY.md")),
        "no memory module, so no `cat` tail anywhere: {commands:?}"
    );
    assert!(
        settings["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Bash(nxs:*)"),
        "the umbrella's allowlist entry is added even when the hook needed no change"
    );

    // flow is now active — proven via the foundation-only `doctor` (no shell-out), which is exactly
    // what the wiring reads to decide which modules get a hook.
    nxs(tmp.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stdout(contains("\"active_modules\":[\"flow\"]"));

    // **Superseded 2026-08-29 (nxf 6j6v.1k6y): the legacy entry and the wanted one are the same
    // string, so there is no "the legacy BARE entry is gone" to assert.** This block used to check
    // exactly that, and its own history is worth keeping because it has now turned over twice:
    // before nxf n2m6 + a2a1 it asserted the single umbrella `nxs prime` was wired and the string
    // "nxf prime" was ABSENT; that inverted when the wiring became one hook per module, and it was
    // rewritten to require `nxf prime` WITH the fallback tail and the bare form absent. The tail
    // then moved to memory (which is not active here), leaving the bare form as the correct answer
    // — the very thing the previous version asserted must not appear.
    //
    // What survives the churn is the part that never depended on where the tail sits: `nxs prime`
    // must not be wired as a host hook. The rest is asserted above, against the settings this run
    // actually produced.
    assert!(
        !commands.iter().any(|c| c.starts_with("nxs prime")),
        "the umbrella hook this used to migrate TO is not wired either — `nxs prime` is the \
         by-hand verb now, not a host hook: {commands:?}"
    );

    // Idempotent: a second migrate rewrites nothing and registers nothing.
    nxs(tmp.path())
        .arg("migrate")
        .assert()
        .success()
        .stdout(contains("already current").or(contains("migrated workspace db")))
        .stdout(contains("rewrote legacy").not())
        .stdout(contains("registered active module").not());
}

/// **The intermediate form, end to end through the real binary** (nxf 6j6v.sp5v; the review of
/// PR #389 named its absence, Test Quality #3).
///
/// `sp5v` is covered at the `migrate_with` seam, which is the right engine seam — but the shape it
/// fixes is the one 19 of 19 workspaces in the field carried, and the line a person reads when it
/// happens is printed by `cli.rs`. That line had exactly one end-to-end assertion, on the v0.5.x
/// case, which since nxf 6j6v.1k6y no longer triggers a rewrite at all. So it moves here, onto the
/// case that does.
#[test]
fn migrate_converges_the_intermediate_umbrella_hook_through_the_cli() {
    let tmp = workspace_with(&["flow", "memory"]);
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{ "hooks": { "SessionStart": [
            { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] },
            { "matcher": "", "hooks": [ { "type": "command", "command": "nxs prime || cat NEXUS_MEMORY.md 2>/dev/null" } ] }
        ] } }"#,
    )
    .unwrap();

    nxs(tmp.path())
        .arg("migrate")
        .assert()
        .success()
        .stdout(contains("outdated SessionStart wiring rewritten"))
        // The half the lead-in alone cannot catch: WHAT it rewrote TO. That line named a single
        // `nxs prime` until nxf n2m6 + a2a1 and then stayed on the old wording after the wiring
        // had already moved, so the target is asserted explicitly.
        .stdout(contains("wired SessionStart → `nxf prime`"))
        .stdout(contains("`nxm prime || cat NEXUS_MEMORY.md 2>/dev/null`"));

    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(claude.join("settings.json")).unwrap())
            .unwrap();
    let commands: Vec<String> = settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect();
    assert!(
        !commands.iter().any(|c| c.starts_with("nxs prime")),
        "the umbrella entry is gone, not topped up: {commands:?}"
    );
    assert!(commands.contains(&"bd prime".to_string()), "{commands:?}");
    assert_eq!(
        commands
            .iter()
            .filter(|c| c.contains("NEXUS_MEMORY.md"))
            .collect::<Vec<_>>(),
        vec!["nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"],
        "and the tail sits on memory's entry, once: {commands:?}"
    );
}

#[test]
fn prime_with_an_unknown_active_module_fails_with_the_roster_error() {
    // An active module the roster does not know is surfaced loudly BEFORE any shell-out.
    let tmp = workspace_with(&["ghost"]);
    nxs(tmp.path())
        .args(["prime", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"kind\":\"validation\""))
        .stdout(contains("ghost"));
}
