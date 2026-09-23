//! `nxf label add/remove/list` + the `--label` filter on `list`/`next` (h89s.2). Labels are a
//! user-vocabulary OR-set, distinct from the plugin's display type.

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

fn new_task(dir: &Path, title: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            title,
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

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn err_kind(dir: &Path, args: &[&str]) -> String {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["error"]["kind"].as_str().unwrap().to_string()
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn label_add_list_remove_roundtrip() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    let id = new_task(dir.path(), "labelled");

    run_json(dir.path(), &["label", "add", &id, "urgent"]);
    run_json(dir.path(), &["label", "add", &id, "backend"]);

    // `label list` returns the sorted set.
    let listed = run_json(dir.path(), &["label", "list", &id]);
    assert_eq!(listed, serde_json::json!(["backend", "urgent"]));

    // `show --json` carries them too.
    let shown = run_json(dir.path(), &["show", &id]);
    assert_eq!(shown["labels"], serde_json::json!(["backend", "urgent"]));

    // Remove one → only the other remains.
    run_json(dir.path(), &["label", "remove", &id, "urgent"]);
    let listed = run_json(dir.path(), &["label", "list", &id]);
    assert_eq!(listed, serde_json::json!(["backend"]));
}

#[test]
fn list_and_next_filter_by_label() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    let a = new_task(dir.path(), "A");
    let b = new_task(dir.path(), "B");
    run_json(dir.path(), &["label", "add", &a, "urgent"]);

    // `list --label urgent` returns only A, and each record carries its labels.
    let listed = run_json(dir.path(), &["list", "--label", "urgent"]);
    assert_eq!(ids(&listed), vec![a.clone()]);
    assert_eq!(listed[0]["labels"], serde_json::json!(["urgent"]));

    // `next --label urgent` likewise filters the ready set to A (b is unlabelled).
    let ready = run_json(dir.path(), &["next", "--label", "urgent"]);
    assert_eq!(ids(&ready), vec![a]);

    // A different label matches nothing.
    let none = run_json(dir.path(), &["list", "--label", "nope"]);
    assert!(none.as_array().unwrap().is_empty());
    let _ = b;
}

#[test]
fn label_add_on_a_missing_item_is_not_found() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    assert_eq!(
        err_kind(dir.path(), &["label", "add", "zz.9999", "x"]),
        "not_found"
    );
}

#[test]
fn label_add_rejects_a_blank_label() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    let id = new_task(dir.path(), "x");
    assert_eq!(
        err_kind(dir.path(), &["label", "add", &id, "   "]),
        "validation"
    );
}
