//! `nxf update` — generic field setting with validation, and the retype primitive (#yod / #5qp.7):
//! `type` is settable, but only to a type the active plugin declares.

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

fn new_task(dir: &Path) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "T",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn update(dir: &Path, id: &str, sets: &[&str]) -> assert_cmd::assert::Assert {
    let mut args = vec!["update", id];
    for s in sets {
        args.push("--set");
        args.push(s);
    }
    args.push("--json");
    nxf().args(args).current_dir(dir).assert()
}

#[test]
fn update_sets_whitelisted_fields() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(
        tmp.path(),
        &id,
        &["title=Renamed", "status=in_progress", "priority=P1"],
    )
    .success()
    .get_output()
    .stdout
    .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["title"], serde_json::json!("Renamed"));
    assert_eq!(v["status"], serde_json::json!("in_progress"));
    // Label P1 is stored as its canonical ordinal "1".
    assert_eq!(v["priority"], serde_json::json!("1"));
}

#[test]
fn invalid_status_is_validation_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(tmp.path(), &id, &["status=bogus"])
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
}

#[test]
fn unknown_priority_label_is_validation_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    update(tmp.path(), &id, &["priority=P9"]).failure();
}

#[test]
fn retype_to_a_declared_type_succeeds() {
    // #yod / #5qp.7: `type` is settable via the retype primitive — but only to a type the active
    // plugin declares. (`new_task` creates a `bug`; retype it to `feature`.)
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(tmp.path(), &id, &["type=feature"])
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["type"], serde_json::json!("feature"));
}

#[test]
fn retype_to_an_undeclared_type_is_validation_error() {
    // Retyping can only land on a declared type — the legacy enum value `project` is not in
    // issue-tracker's set, so it is rejected (the repair never re-introduces an undeclared type).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(tmp.path(), &id, &["type=project"])
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
}

#[test]
fn update_unknown_id_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = update(tmp.path(), "nope.MISSING", &["title=x"])
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}

/// Create a project and return its id (for the parent-alias test).
fn new_project(dir: &Path) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "epic",
            "--title",
            "P",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn set_parent_is_an_alias_for_belongs_to() {
    // create uses `--parent`; update must accept the same word (8qv.4) — today only
    // `defer→defer_until` is aliased, so `--set parent=…` silently failed.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let pid = new_project(tmp.path());
    let id = new_task(tmp.path());
    let out = update(tmp.path(), &id, &[&format!("parent={pid}")])
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["belongs_to"], serde_json::json!(pid));
}

#[test]
fn unknown_field_error_lists_the_settable_fields() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(tmp.path(), &id, &["bogus=x"])
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    // The message names the settable set so an agent self-corrects without a schema round-trip.
    assert!(
        msg.contains("title") && msg.contains("status") && msg.contains("description"),
        "unknown-field error must list the settable fields, got: {msg}"
    );
}

#[test]
fn description_and_design_are_settable_via_set() {
    // The new longtext fields (8qv.1) must be reachable through the ordinary --set path (8qv.4).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = update(
        tmp.path(),
        &id,
        &["description=why and goal", "design=the approach"],
    )
    .success()
    .get_output()
    .stdout
    .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["description"], serde_json::json!("why and goal"));
    assert_eq!(v["design"], serde_json::json!("the approach"));
}
