//! #yod — a workspace from before the plugin type system holds legacy core-enum item types
//! (`project`/`task`) the active plugin doesn't declare. `nxf prime` surfaces a non-silent hint at
//! session start, and the retype primitive (`nxf update --set type=`) repairs them.

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
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

/// Seed an item carrying a legacy core-enum type directly into the on-disk op-log — the CLI's
/// `create` would reject an undeclared type, so this reproduces a pre-plugin workspace.
fn seed_legacy_item(dir: &Path, id: &str, legacy_type: &str) {
    let ws = workspace::discover(dir).expect("workspace");
    let mut store = ws.open_store().expect("store");
    store.create_item(id, legacy_type, "Legacy item", "tester");
}

fn prime_stdout(dir: &Path) -> String {
    let out = nxf()
        .arg("prime")
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf8")
}

#[test]
fn prime_warns_about_legacy_types_and_retype_clears_it() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    seed_legacy_item(tmp.path(), "0ppa.1", "project");

    // Non-silent hint at session start: names the undeclared type and the supported repair.
    let before = prime_stdout(tmp.path());
    assert!(
        before.contains("Legacy item types") && before.contains("project"),
        "prime surfaces the legacy-type hint: {before}"
    );
    assert!(
        before.contains("--set type="),
        "the hint points at the retype primitive: {before}"
    );

    // The supported repair: retype the legacy item onto a declared type.
    nxf()
        .args(["update", "0ppa.1", "--set", "type=epic"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Once repaired, the hint is gone.
    let after = prime_stdout(tmp.path());
    assert!(
        !after.contains("Legacy item types"),
        "the hint clears after the legacy item is retyped: {after}"
    );
}

/// Seed a fully-legacy `project → task` hierarchy (the real 0ppa shape) directly on disk.
fn seed_legacy_hierarchy(dir: &Path) {
    let ws = workspace::discover(dir).expect("workspace");
    let mut store = ws.open_store().expect("store");
    store.create_item("0ppa.p", "project", "Legacy parent", "tester");
    store.create_item("0ppa.c", "task", "Legacy child", "tester");
    store.set_parent("0ppa.c", "0ppa.p", "tester").unwrap();
}

#[test]
fn legacy_hierarchy_migrates_via_retype_without_deadlock() {
    // The real #yod deadlock (verified on 0ppa): a whole project→task hierarchy, neither type
    // declared. Each end's retype was blocked by the other's still-legacy type — nothing migrable.
    // The retype now skips the not-yet-migrated neighbor edge, so the board migrates item by item.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    seed_legacy_hierarchy(tmp.path());

    // child-first: task→chore succeeds despite the still-`project` parent.
    nxf()
        .args(["update", "0ppa.c", "--set", "type=chore"])
        .current_dir(tmp.path())
        .assert()
        .success();
    // then the parent: project→epic succeeds (the child is now the declared `chore`).
    nxf()
        .args(["update", "0ppa.p", "--set", "type=epic"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // both migrated → the legacy-type hint is gone.
    let after = prime_stdout(tmp.path());
    assert!(
        !after.contains("Legacy item types"),
        "the whole hierarchy migrated to declared types: {after}"
    );
}
