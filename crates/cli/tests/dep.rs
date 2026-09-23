//! `nxf dep add/remove` with write-time cycle rejection (E2.12).

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

fn blocked_ids(dir: &Path) -> Vec<String> {
    let out = nxf()
        .args(["blocked", "--json"])
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

fn close(dir: &Path, id: &str) {
    nxf()
        .args(["close", id, "--reason", "done"])
        .current_dir(dir)
        .assert()
        .success();
}

fn dep_add(dir: &Path, from: &str, to: &str) {
    nxf()
        .args(["dep", "add", from, to])
        .current_dir(dir)
        .assert()
        .success();
}

#[test]
fn show_annotates_each_dep_with_its_status() {
    // nexus-flow-97b: `show` must explain the dependency, not just list ids — each dep is
    // annotated with the live status of its target so the open blocker is obvious without a
    // follow-up `show`.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let x = new_task(tmp.path(), "X");
    let open_blocker = new_task(tmp.path(), "still open");
    let done_blocker = new_task(tmp.path(), "already done");
    dep_add(tmp.path(), &x, &open_blocker);
    dep_add(tmp.path(), &x, &done_blocker);
    close(tmp.path(), &done_blocker);

    // --json: deps are structured {id, status} entries.
    let shown = run_json(tmp.path(), &["show", &x]);
    let deps = shown["deps"].as_array().unwrap();
    let by_id = |id: &str| {
        deps.iter()
            .find(|d| d["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("dep {id} present in {deps:?}"))
            .clone()
    };
    assert_eq!(by_id(&open_blocker)["status"], serde_json::json!("open"));
    assert_eq!(by_id(&done_blocker)["status"], serde_json::json!("closed"));

    // human (8qv.8): `BLOCKED BY` lists the OPEN blockers only — a closed (satisfied) dep is no
    // longer a blocker, so it does not appear in the human layout (it remains in --json above).
    let human = run_human(tmp.path(), &["show", &x]);
    // ykv: a local blocker is named by its bare suffix in human text (full id only under --json).
    let bare = |id: &str| id.split_once('.').unwrap().1.to_string();
    assert!(
        human.contains(&format!("BLOCKED BY: {}", bare(&open_blocker))),
        "open blocker listed under BLOCKED BY: {human}"
    );
    assert!(
        !human.contains(&done_blocker),
        "closed dep must not appear as a blocker: {human}"
    );
}

#[test]
fn blocked_surfaces_the_open_blockers_per_item() {
    // nexus-flow-97b: `blocked` must show WHICH open blocker holds each item, inline.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let dependent = new_task(tmp.path(), "dependent");
    let blocker = new_task(tmp.path(), "the blocker");
    dep_add(tmp.path(), &dependent, &blocker);

    // --json: each blocked entry keeps its id and gains structured `blockers` (id + status).
    let arr = run_json(tmp.path(), &["blocked"]);
    let entry = arr
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"].as_str() == Some(dependent.as_str()))
        .expect("dependent is listed as blocked");
    let blockers = entry["blockers"].as_array().unwrap();
    assert_eq!(blockers.len(), 1, "one open blocker: {blockers:?}");
    assert_eq!(blockers[0]["id"], serde_json::json!(blocker));
    assert_eq!(blockers[0]["status"], serde_json::json!("open"));

    // human: the blocker id is named inline (bare local suffix, ykv), no follow-up call needed.
    let human = run_human(tmp.path(), &["blocked"]);
    assert!(
        human.contains(blocker.split_once('.').unwrap().1),
        "human blocked names the blocker: {human}"
    );
}

#[test]
fn dep_add_blocks_the_dependent_then_remove_unblocks() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = new_task(tmp.path(), "A");
    let b = new_task(tmp.path(), "B");

    // A depends on B; with B open, A is blocked.
    nxf()
        .args(["dep", "add", &a, &b])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(blocked_ids(tmp.path()).contains(&a));

    nxf()
        .args(["dep", "remove", &a, &b])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(!blocked_ids(tmp.path()).contains(&a));
}

#[test]
fn cycle_creating_edge_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = new_task(tmp.path(), "A");
    let b = new_task(tmp.path(), "B");

    nxf()
        .args(["dep", "add", &a, &b])
        .current_dir(tmp.path())
        .assert()
        .success();
    // Adding B->A would close a cycle; reject and write nothing.
    assert_eq!(err_kind(tmp.path(), &["dep", "add", &b, &a]), "cycle");
    // B is therefore not blocked.
    assert!(!blocked_ids(tmp.path()).contains(&b));
}

#[test]
fn self_dependency_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = new_task(tmp.path(), "A");
    assert_eq!(err_kind(tmp.path(), &["dep", "add", &a, &a]), "cycle");
}

#[test]
fn dep_on_unknown_item_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = new_task(tmp.path(), "A");
    assert_eq!(
        err_kind(tmp.path(), &["dep", "add", &a, "nope.MISSING"]),
        "not_found"
    );
}

#[test]
fn dep_add_help_spells_out_the_direction() {
    // 452: the `dep` help must make the dependency direction unmistakable — `dep add A B`
    // means A depends on B, so B blocks A.
    let out = nxf()
        .args(["dep", "add", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8_lossy(&out).to_lowercase();
    assert!(
        help.contains("depend"),
        "names the depends-on relation: {help}"
    );
    assert!(help.contains("block"), "names which side blocks: {help}");
}
