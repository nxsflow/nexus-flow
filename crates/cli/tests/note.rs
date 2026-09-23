//! `nxf note add/list` over the core note stream (E2.15).

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

fn note_list(dir: &Path, id: &str) -> Vec<String> {
    let out = nxf()
        .args(["note", "list", id, "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n["body"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn note_add_then_list_preserves_order() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());

    nxf()
        .args(["note", "add", &id, "first note"])
        .current_dir(tmp.path())
        .assert()
        .success();
    nxf()
        .args(["note", "add", &id, "second note"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert_eq!(
        note_list(tmp.path(), &id),
        vec!["first note", "second note"]
    );
}

#[test]
fn note_on_unknown_item_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args(["note", "add", "nope.MISSING", "x", "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}
