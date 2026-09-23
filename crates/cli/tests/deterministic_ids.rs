//! The golden-docs determinism switch (nexus-flow-4oa.2): with `NXF_DETERMINISTIC_IDS` set,
//! a fresh workspace takes a FIXED identity and `create` mints sequential ids, so the trycmd
//! golden examples produce byte-stable output. These tests pin that contract directly (the
//! trycmd cases consume it). Production never sets the var; `init.rs`/`dogfood.rs` cover the
//! real random path.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

/// An `nxf` invocation with the determinism switch on (set per-child, so it never leaks into
/// other tests' processes).
fn nxf_det() -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// `nxf create … --json` under the switch; return the minted id. `priority` is passed by the
/// caller so it stays a label the active plugin accepts (P1 for issue-tracker, now for
/// personal-todo); the value never affects the minted id.
fn create_id(dir: &Path, ty: &str, title: &str, priority: &str) -> String {
    let out = nxf_det()
        .args([
            "--json",
            "create",
            "--type",
            ty,
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            priority,
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("create json");
    v["id"].as_str().expect("id string").to_string()
}

#[test]
fn deterministic_mode_inits_fixed_identity() {
    let tmp = TempDir::new().unwrap();
    let out = nxf_det()
        .args(["--json", "init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("init json");
    assert_eq!(v["prefix"], "ab12", "fixed prefix under the switch");
    assert_eq!(v["site_id"], 1, "fixed site_id under the switch");
}

#[test]
fn deterministic_mode_mints_sequential_ids() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    nxf_det()
        .args(["--json", "init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();

    // Each create — a SEPARATE process — yields the next id in the sequence.
    assert_eq!(create_id(dir, "bug", "first", "P1"), "ab12.0001");
    assert_eq!(create_id(dir, "bug", "second", "P1"), "ab12.0002");
    assert_eq!(create_id(dir, "bug", "third", "P1"), "ab12.0003");
}

#[test]
fn deterministic_mode_is_byte_stable_across_fresh_workspaces() {
    // Two independent workspaces built with the same script produce identical ids — the
    // property the golden harness relies on (sandbox A and sandbox B agree).
    let run = || {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        nxf_det()
            .args(["--json", "init", "--plugin", "personal-todo"])
            .current_dir(&dir)
            .assert()
            .success();
        let a = create_id(&dir, "todo", "alpha", "now");
        let b = create_id(&dir, "todo", "beta", "now");
        (a, b)
    };
    assert_eq!(run(), run());
    assert_eq!(run(), ("ab12.0001".to_string(), "ab12.0002".to_string()));
}
