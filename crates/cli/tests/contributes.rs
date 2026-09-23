//! `nxf contributes add/remove/list` — the n:m "contributes-to" relation exposed on the CLI
//! (nexus-flow-vf4). Modeled on `dep`/`mention`: both endpoints must exist, but unlike `dep` it
//! carries no blocking semantics (no cycle check), and unlike a dep it is shown separately.

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

fn run_human(dir: &Path, args: &[&str]) -> String {
    let out = nxf()
        .args(args)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
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

fn blocked_ids(dir: &Path) -> Vec<String> {
    let v = run_json(dir, &["blocked"]);
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn contributes_add_records_a_relation_that_does_not_block() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let issue = new_task(tmp.path(), "bug");
    let epic = new_task(tmp.path(), "epic");

    // issue contributes to epic — recorded, listed, but it does NOT block (it is not a dep).
    run_json(tmp.path(), &["contributes", "add", &issue, &epic]);
    let listed = run_json(tmp.path(), &["contributes", "list", &issue]);
    assert_eq!(listed, serde_json::json!([epic]));
    assert!(
        !blocked_ids(tmp.path()).contains(&issue),
        "contributes-to must not block the contributor"
    );
}

#[test]
fn contributes_remove_drops_the_edge() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let issue = new_task(tmp.path(), "bug");
    let epic = new_task(tmp.path(), "epic");

    run_json(tmp.path(), &["contributes", "add", &issue, &epic]);
    run_json(tmp.path(), &["contributes", "remove", &issue, &epic]);
    let listed = run_json(tmp.path(), &["contributes", "list", &issue]);
    assert_eq!(listed, serde_json::json!([]));
}

#[test]
fn show_lists_contributes_to_separately_from_deps() {
    // nexus-flow-vf4: `show` must surface the contributes-to edges separately from `deps` — the
    // two relations are distinct (one blocks, one does not) and must not be conflated.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let issue = new_task(tmp.path(), "bug");
    let epic = new_task(tmp.path(), "epic");
    let blocker = new_task(tmp.path(), "blocker");

    // issue depends on blocker (a dep) AND contributes to epic (not a dep).
    run_json(tmp.path(), &["dep", "add", &issue, &blocker]);
    run_json(tmp.path(), &["contributes", "add", &issue, &epic]);

    // --json: contributes_to and deps are distinct fields.
    let shown = run_json(tmp.path(), &["show", &issue]);
    let deps: Vec<&str> = shown["deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    let contributes: Vec<&str> = shown["contributes_to"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(deps, vec![blocker.as_str()], "the dep is a dep");
    assert_eq!(
        contributes,
        vec![epic.as_str()],
        "the contributes-to edge is separate"
    );

    // human: a CONTRIBUTES TO line names the epic, distinct from BLOCKED BY — each a bare local
    // suffix in human text (ykv; the full id stays under --json above).
    let human = run_human(tmp.path(), &["show", &issue]);
    let bare = |id: &str| id.split_once('.').unwrap().1.to_string();
    assert!(
        human.contains(&format!("CONTRIBUTES TO: {}", bare(&epic))),
        "human show names the contributes-to target: {human}"
    );
    assert!(
        human.contains(&format!("BLOCKED BY: {}", bare(&blocker))),
        "the dep still shows as a blocker: {human}"
    );
}

#[test]
fn contributes_on_unknown_item_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = new_task(tmp.path(), "A");
    assert_eq!(
        err_kind(tmp.path(), &["contributes", "add", &a, "nope.MISSING"]),
        "not_found"
    );
    assert_eq!(
        err_kind(tmp.path(), &["contributes", "add", "nope.MISSING", &a]),
        "not_found"
    );
}
