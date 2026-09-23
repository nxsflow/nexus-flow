//! `create --type` validates against the active plugin's declared type set — its `[types].list`
//! (sp6.4), not a hardcoded core enum. The declared type is stored verbatim in the canonical
//! `--json` record. Swapping the plugin swaps the valid set, with nothing hardcoded (nexus-flow-9se).

use assert_cmd::Command;
use std::fs;
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

fn set_plugin(dir: &Path, plugin: &str) {
    fs::write(
        dir.join(".nxs").join("config.toml"),
        format!("plugin = \"{plugin}\"\n"),
    )
    .unwrap();
}

/// create with `--type ty`, expect success, return the canonical `type` field.
/// `prio` must be a valid priority label for the active plugin.
fn created_core_type(dir: &Path, ty: &str, prio: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            ty,
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            prio,
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["type"].as_str().unwrap().to_string()
}

fn create_fails_kind(dir: &Path, ty: &str, prio: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            ty,
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            prio,
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["error"]["kind"].as_str().unwrap().to_string()
}

#[test]
fn issue_tracker_accepts_its_declared_types() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path()); // default plugin = issue-tracker

    // Every declared type resolves and is stored verbatim.
    for ty in ["epic", "bug", "feature", "chore", "decision"] {
        assert_eq!(created_core_type(tmp.path(), ty, "P1"), ty);
    }
    // The former core roles project/task are NOT issue-tracker types — the enum gate is gone.
    assert_eq!(create_fails_kind(tmp.path(), "task", "P1"), "validation");
    assert_eq!(create_fails_kind(tmp.path(), "project", "P1"), "validation");
}

#[test]
fn unknown_type_is_validation_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    assert_eq!(create_fails_kind(tmp.path(), "banana", "P1"), "validation");
}

#[test]
fn type_set_is_config_driven_not_hardcoded() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    set_plugin(tmp.path(), "personal-todo");

    // personal-todo's own declared types resolve, stored verbatim...
    assert_eq!(created_core_type(tmp.path(), "todo", "soon"), "todo");
    assert_eq!(created_core_type(tmp.path(), "termin", "soon"), "termin");
    assert_eq!(created_core_type(tmp.path(), "project", "soon"), "project");
    // ...and issue-tracker's types do NOT (proves no hardcoded epic/bug).
    assert_eq!(create_fails_kind(tmp.path(), "epic", "soon"), "validation");
    assert_eq!(create_fails_kind(tmp.path(), "bug", "soon"), "validation");
}
