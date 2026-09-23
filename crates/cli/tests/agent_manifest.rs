//! Integration tests for `nxf agent-manifest` (nexus-flow-aye.10, spec §6.2).
//!
//! The command emits nxf's DECLARED contribution to the shared agent file as data, so the `nxs`
//! umbrella (P3) can assemble AGENTS.md + one SessionStart hook per active module without
//! hardcoding
//! any per-module knowledge. The contribution is static — it depends on neither a workspace nor the
//! active plugin — so the command runs anywhere. (`CLAUDE.md` was a second destination until nxf
//! 6j6v.q6e3; it belongs to the host that runs the hook and is only ever cleaned now.)

use assert_cmd::Command;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

#[test]
fn agent_manifest_json_declares_nxfs_contribution() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "agent-manifest"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("agent-manifest emits json");

    // Since nexus-flow-0lj.2 the manifest declares only the prime fan-out target + the hook to wire;
    // it carries NO per-module Markdown block (the umbrella writes a single nxs-owned AGENTS.md
    // discovery pointer instead).
    assert_eq!(v["prime_command"], "nxf prime", "prime command: {v}");
    assert_eq!(v["hook"]["event"], "SessionStart", "hook event: {v}");
    assert_eq!(v["hook"]["command"], "nxf prime", "hook command: {v}");
    assert!(
        v.get("agents_section").is_none() && v.get("claude_section").is_none(),
        "no per-module section fields remain on the manifest: {v}"
    );
}

#[test]
fn agent_manifest_runs_without_a_workspace() {
    // It declares a static contribution — no `.nxs/` is required (unlike every store command).
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["--json", "agent-manifest"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(
        !tmp.path().join(".nxs").exists(),
        "agent-manifest creates no workspace"
    );
}
