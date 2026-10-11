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

// ---- filters and paging (6j6v.n698) ---------------------------------------------------

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = nxf()
        .args(args)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn ids_of(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .expect("an array")
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// An epic with a child bug and a contributing feature, plus an unrelated chore. Returns
/// `(epic, bug, feature, chore)`.
fn contained_board(dir: &Path) -> (String, String, String, String) {
    init(dir);
    let epic = create(dir, "epic", "E");
    let bug = create(dir, "bug", "B");
    let feature = create(dir, "feature", "F");
    let chore = create_with_prio(dir, "chore", "C", "P0");
    run_json(
        dir,
        &["update", &bug, "--set", &format!("parent={epic}"), "--json"],
    );
    run_json(dir, &["contributes", "add", &feature, &epic, "--json"]);
    (epic, bug, feature, chore)
}

#[test]
fn list_filters_by_several_types_and_by_container_like_next() {
    let tmp = TempDir::new().unwrap();
    let (epic, bug, feature, chore) = contained_board(tmp.path());
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(
        list(tmp.path(), &["--in", &epic]),
        sorted(vec![bug.clone(), feature.clone()]),
        "a container holds its children and its contributors"
    );
    assert_eq!(
        list(tmp.path(), &["--type", "bug", "--type", "chore"]),
        sorted(vec![bug.clone(), chore]),
        "the types are alternatives"
    );
    assert_eq!(list(tmp.path(), &["--in", &epic, "--type", "bug"]), [bug]);
    // A short id resolves, as for `next --in`.
    let short = epic.rsplit('.').next().unwrap().to_string();
    assert_eq!(list(tmp.path(), &["--in", &short]).len(), 2);
}

#[test]
fn list_limit_and_paginate_answer_with_the_next_envelope() {
    let tmp = TempDir::new().unwrap();
    let (_, bug, _, _) = contained_board(tmp.path());
    let all = list(tmp.path(), &[]);

    let cut = run_json(tmp.path(), &["list", "--limit", "3", "--json"]);
    assert_eq!(ids_of(&cut["items"]), all[0..3]);
    assert_eq!(cut["total"], 4);
    assert!(
        cut.get("next_token").is_none(),
        "no token without --paginate"
    );

    let p1 = run_json(
        tmp.path(),
        &["list", "--limit", "3", "--paginate", "--json"],
    );
    assert_eq!(ids_of(&p1["items"]), all[0..3]);
    assert_eq!(p1["restarted"], false);
    let token = p1["next_token"]
        .as_str()
        .expect("a row remains")
        .to_string();
    let page2 = |dir: &Path| {
        run_json(
            dir,
            &[
                "list",
                "--limit",
                "3",
                "--paginate",
                "--token",
                &token,
                "--json",
            ],
        )
    };
    let p2 = page2(tmp.path());
    assert_eq!(ids_of(&p2["items"]), all[3..]);
    assert_eq!(p2["next_token"], serde_json::Value::Null, "the last page");

    // A change to a cached item makes the token stale: the answer is the first page again.
    run_json(tmp.path(), &["update", &bug, "--set", "title=B2", "--json"]);
    let again = page2(tmp.path());
    assert_eq!(again["restarted"], true);
    assert_eq!(ids_of(&again["items"]), all[0..3]);
}

#[test]
fn list_sort_rank_pages_through_the_ranked_order() {
    let tmp = TempDir::new().unwrap();
    contained_board(tmp.path());
    let ranked = list(tmp.path(), &["--sort", "rank"]);
    // The P0 chore ranks first under issue-tracker; by id it would not necessarily.
    assert_eq!(ranked.len(), 4);
    let p1 = run_json(
        tmp.path(),
        &[
            "list",
            "--sort",
            "rank",
            "--limit",
            "2",
            "--paginate",
            "--json",
        ],
    );
    assert_eq!(ids_of(&p1["items"]), ranked[0..2]);
    let token = p1["next_token"].as_str().unwrap().to_string();
    let p2 = run_json(
        tmp.path(),
        &[
            "list",
            "--sort",
            "rank",
            "--limit",
            "2",
            "--paginate",
            "--token",
            &token,
            "--json",
        ],
    );
    assert_eq!(ids_of(&p2["items"]), ranked[2..]);
    // The same token under the id order is another query: it restarts.
    let other = run_json(
        tmp.path(),
        &[
            "list",
            "--limit",
            "2",
            "--paginate",
            "--token",
            &token,
            "--json",
        ],
    );
    assert_eq!(other["restarted"], true);
}

#[test]
fn list_human_view_discloses_the_cut_and_prints_the_token() {
    let tmp = TempDir::new().unwrap();
    contained_board(tmp.path());
    let out = list_human(tmp.path(), &["--limit", "2", "--paginate"]);
    // Trimmed: the CLI frames a human answer with blank lines (`output_framing.rs`).
    let out = out.trim().to_string();
    assert!(out.starts_with("showing 2 of 4\n"), "{out}");
    assert_eq!(out.lines().count(), 4, "notice, two rows, token: {out}");
    assert!(
        out.lines()
            .last()
            .unwrap()
            .starts_with("next page: --paginate --token "),
        "{out}"
    );
    // Without --limit, the view is the plain list: no notice, no token.
    let plain = list_human(tmp.path(), &[]);
    assert!(
        !plain.contains("showing") && !plain.contains("next page"),
        "{plain}"
    );
}

#[test]
fn list_paginate_needs_a_limit_and_a_token_needs_paginate() {
    let tmp = TempDir::new().unwrap();
    contained_board(tmp.path());
    let cases: [&[&str]; 3] = [
        &["list", "--paginate"],
        &["list", "--limit", "2", "--token", "x.2"],
        &["list", "--limit", "0", "--paginate"],
    ];
    for args in cases {
        nxf().args(args).current_dir(tmp.path()).assert().failure();
    }
}
