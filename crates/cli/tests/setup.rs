//! `nxf setup claude` — host-specific Claude Code integration end-to-end (nexus-flow-131).
//!
//! Since nexus-flow-5od the verb's implementation lives on the `nxs` umbrella; `nxf setup claude`
//! delegates by re-exec'ing `nxs setup claude`. These tests drive the flow persona and so also prove
//! the delegation preserves the observable contract (the same `--json` shape, workspace resolution,
//! and loud outside-a-workspace failure).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

fn setup_json(dir: &Path) -> serde_json::Value {
    let out = nxf()
        .args(["setup", "claude", "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn setup_claude_regenerates_a_deleted_agents_file() {
    // nexus-flow-6c4: `nxf setup claude` (delegating to the umbrella `nxs setup claude`) must bring
    // back a DELETED AGENTS.md — the canonical path assembles the agent files, not just the hook.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    assert!(
        tmp.path().join("AGENTS.md").is_file(),
        "init wrote AGENTS.md"
    );
    std::fs::remove_file(tmp.path().join("AGENTS.md")).unwrap();

    let out = setup_json(tmp.path());
    assert_eq!(out["ok"], true);
    // The agent file did not exist on this run, so the assemble reports it as freshly created.
    assert_eq!(
        out["agents"], "created",
        "regenerated a missing AGENTS.md: {out}"
    );

    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "AGENTS.md regenerated: {agents}"
    );
    assert!(
        agents.contains("## nexus-flow tools for agents"),
        "regenerated with the Markdown header (s1w): {agents}"
    );
}

#[test]
fn setup_claude_wires_hook_and_permission_then_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // `nxf init` already wires the `nxs prime` hook (via the assembler). Remove `.claude` to exercise
    // the delegated `nxs setup claude` re-wiring it from scratch — its remaining repair-tool role.
    std::fs::remove_dir_all(tmp.path().join(".claude")).ok();

    let first = setup_json(tmp.path());
    assert_eq!(first["ok"], true);
    assert_eq!(first["hook_added"], true);
    assert_eq!(first["permission_added"], true);

    // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** the single SessionStart hook used to run
    // `nxs prime` (the umbrella fan-out, "not `nxf prime`"). It is one entry per active module
    // now, so a flow-only workspace wires `nxf prime` — bare, since nxf 6j6v.1k6y put the
    // `|| cat NEXUS_MEMORY.md` tail on memory's entry and memory is not active here. The
    // nxs + nxf allowlist entries are unchanged.
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        serde_json::json!("nxf prime")
    );
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    assert!(allow.iter().any(|v| v == "Bash(nxs:*)"), "nxs allowed");
    assert!(allow.iter().any(|v| v == "Bash(nxf:*)"), "nxf allowed");

    // A second run reports no changes (idempotent).
    let second = setup_json(tmp.path());
    assert_eq!(second["hook_added"], false);
    assert_eq!(second["permission_added"], false);
}

#[test]
fn setup_claude_from_a_subdirectory_writes_at_the_project_root() {
    // nexus-flow-131 / review (Integrity Low): run from a nested directory, the hook must land
    // at the project root (where `.nxs/` and the agent's `.claude/` live), NOT the subdir.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // Remove the init-wired `.claude` so this exercises `setup claude` writing it at the root.
    std::fs::remove_dir_all(tmp.path().join(".claude")).ok();
    let deep = tmp.path().join("crates").join("cli");
    std::fs::create_dir_all(&deep).unwrap();

    setup_json(&deep);

    assert!(
        tmp.path().join(".claude/settings.json").is_file(),
        "settings written at the project root"
    );
    assert!(
        !deep.join(".claude").exists(),
        "nothing scattered in the subdirectory"
    );
}

#[test]
fn setup_claude_outside_a_workspace_fails_loudly() {
    // No `.nxs/` anywhere up-tree → a clear error, never a silent `.claude/` in a random dir.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["setup", "claude", "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("no_workspace"));
    assert!(!tmp.path().join(".claude").exists(), "no settings written");
}
