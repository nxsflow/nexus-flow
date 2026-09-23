//! `nxf ready` / `nxf blocked` over the core derivation (E2.10).

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

fn create(dir: &Path, args: &[&str]) -> String {
    let mut full = vec![
        "create",
        "--type",
        "bug",
        "--description",
        "d",
        "--priority",
        "P1",
    ];
    full.extend_from_slice(args);
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn ids(dir: &Path, cmd: &[&str]) -> Vec<String> {
    let mut full: Vec<&str> = cmd.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
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
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn open_task_is_ready_and_drops_out_when_closed() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), &["--title", "A"]);
    assert!(ids(tmp.path(), &["next"]).contains(&id));

    nxf()
        .args(["close", &id, "--reason", "done"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(!ids(tmp.path(), &["next"]).contains(&id));
}

#[test]
fn deferred_task_respects_now() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), &["--title", "later", "--defer", "2030-01-01"]);

    // Before the defer date: not ready.
    assert!(!ids(tmp.path(), &["next", "--now", "2026-01-01"]).contains(&id));
    // After it: ready.
    assert!(ids(tmp.path(), &["next", "--now", "2031-01-01"]).contains(&id));
}

#[test]
fn nothing_is_blocked_without_dependencies() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), &["--title", "lonely"]);
    assert!(ids(tmp.path(), &["blocked"]).is_empty());
}
