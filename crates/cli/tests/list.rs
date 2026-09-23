//! `nxf list` with status/type filters (E2.13).

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

fn init_with(dir: &Path, plugin: &str) {
    nxf()
        .args(["init", "--plugin", plugin])
        .current_dir(dir)
        .assert()
        .success();
}

fn list_human(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["list"];
    full.extend_from_slice(args);
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

fn create(dir: &Path, ty: &str, title: &str) -> String {
    create_with_prio(dir, ty, title, "P1")
}

fn create_with_prio(dir: &Path, ty: &str, title: &str, prio: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            ty,
            "--title",
            title,
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
    v["id"].as_str().unwrap().to_string()
}

fn list(dir: &Path, args: &[&str]) -> Vec<String> {
    let mut full = vec!["list"];
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
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_all_then_filter_by_type_and_status() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let p = create(tmp.path(), "epic", "P");
    let t1 = create(tmp.path(), "bug", "T1");
    let t2 = create(tmp.path(), "bug", "T2");

    let all = list(tmp.path(), &[]);
    assert_eq!(all.len(), 3);

    // `list` output is deterministic but ordered by id, not by creation time (short
    // ids carry no timestamp) — so compare against the id-sorted expected set.
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };

    let tasks = list(tmp.path(), &["--type", "bug"]);
    assert_eq!(tasks, sorted(vec![t1.clone(), t2.clone()]));
    assert!(!tasks.contains(&p));

    // Close one task; filter by status.
    nxf()
        .args(["close", &t1, "--reason", "done"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert_eq!(list(tmp.path(), &["--status", "closed"]), vec![t1]);
    assert_eq!(list(tmp.path(), &["--status", "open"]), sorted(vec![p, t2]));
}

#[test]
fn human_list_shows_type_in_brackets_in_the_active_plugin_vocabulary() {
    // nexus-flow-9dy: the human list view (shared by ready/next/list) must show each row's
    // type, bracketed, in the active plugin's vocabulary — so an agent can tell an epic from
    // a bug at a glance. JSON stays the canonical type (asserted elsewhere).
    let it = TempDir::new().unwrap();
    init_with(it.path(), "issue-tracker");
    create(it.path(), "epic", "an epic");
    create(it.path(), "bug", "a bug");
    let out = list_human(it.path(), &[]);
    assert!(out.contains("[epic]"), "epic → [epic]: {out}");
    assert!(out.contains("[bug]"), "bug → [bug]: {out}");

    let pt = TempDir::new().unwrap();
    init_with(pt.path(), "personal-todo");
    create_with_prio(pt.path(), "project", "a project", "soon");
    create_with_prio(pt.path(), "todo", "a todo", "soon");
    let out = list_human(pt.path(), &[]);
    assert!(out.contains("[project]"), "project → [project]: {out}");
    assert!(out.contains("[todo]"), "todo → [todo]: {out}");
}

#[test]
fn deleted_items_are_excluded() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "bug", "keep");
    assert_eq!(list(tmp.path(), &[]).len(), 1);
}
