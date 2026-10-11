//! `nxf mention` — the reference edge (bm0/§8). A pure reference from one item to a
//! short-id it cites in free text; it must NOT affect ready/blocked (that is a `dep`).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    // Sequential ids, so creation order is id order (the order `mention by` answers in).
    c.current_dir(dir).env("NXF_DETERMINISTIC_IDS", "1");
    c
}

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = nxf(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn create(dir: &Path, title: &str) -> String {
    run_json(
        dir,
        &[
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
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn mention_add_records_a_reference_that_does_not_block() {
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let a = create(tmp.path(), "cites b in its body");
    let b = create(tmp.path(), "the cited task");

    run_json(tmp.path(), &["mention", "add", &a, &b, "--json"]);

    // The reference is listed...
    let listed: Vec<String> = run_json(tmp.path(), &["mention", "list", &a, "--json"])
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed, vec![b.clone()], "mention is recorded and listed");

    // ...but it does NOT block: both tasks stay ready (a mention is not a dep).
    let ready: Vec<String> = run_json(tmp.path(), &["next", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ready.contains(&a) && ready.contains(&b),
        "mention does not block: {ready:?}"
    );
}

// ---- mention by: the reverse read (6j6v.q29p) -------------------------------

fn create_typed(dir: &Path, ty: &str, title: &str, extra: &[&str]) -> String {
    let mut args = vec![
        "create",
        "--type",
        ty,
        "--title",
        title,
        "--description",
        "d",
        "--priority",
        "P1",
        "--json",
    ];
    args.extend_from_slice(extra);
    run_json(dir, &args)["id"].as_str().unwrap().to_string()
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .expect("an array of records")
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

/// A target and four mentioners: a feature in the epic, two bugs (one contributing to the epic),
/// and a feature outside it. Returns `(target, epic, [mentioners in id order])`.
fn mentioned_board(dir: &Path) -> (String, String, Vec<String>) {
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let target = create_typed(dir, "feature", "the cited one", &[]);
    let epic = create_typed(dir, "epic", "the container", &[]);
    let m = vec![
        create_typed(dir, "feature", "in the epic", &["--parent", &epic]),
        create_typed(dir, "bug", "contributes", &[]),
        create_typed(dir, "bug", "loose bug", &[]),
        create_typed(dir, "feature", "loose feature", &[]),
    ];
    run_json(dir, &["contributes", "add", &m[1], &epic, "--json"]);
    for from in &m {
        run_json(dir, &["mention", "add", from, &target, "--json"]);
    }
    (target, epic, m)
}

#[test]
fn mention_by_lists_the_mentioners_as_list_records_and_filters_like_next() {
    let tmp = TempDir::new().unwrap();
    let (target, epic, m) = mentioned_board(tmp.path());

    let all = run_json(tmp.path(), &["mention", "by", &target, "--json"]);
    assert_eq!(
        ids(&all),
        m,
        "every mentioner, in id order, as a bare array"
    );
    assert_eq!(all[0]["type"], "feature", "a list record");
    assert!(
        all[0]["type_label"].is_string(),
        "with list's presentation labels"
    );

    let bugs = run_json(
        tmp.path(),
        &["mention", "by", &target, "--type", "bug", "--json"],
    );
    assert_eq!(ids(&bugs), m[1..3]);
    let in_epic = run_json(
        tmp.path(),
        &["mention", "by", &target, "--in", &epic, "--json"],
    );
    assert_eq!(
        ids(&in_epic),
        [m[0].clone(), m[1].clone()],
        "child or contributor"
    );
    let nothing = run_json(tmp.path(), &["mention", "by", &epic, "--json"]);
    assert_eq!(nothing, serde_json::json!([]));
}

#[test]
fn mention_by_pages_with_the_next_envelope() {
    let tmp = TempDir::new().unwrap();
    let (target, _, m) = mentioned_board(tmp.path());

    let cut = run_json(
        tmp.path(),
        &["mention", "by", &target, "--limit", "3", "--json"],
    );
    assert_eq!(ids(&cut["items"]), m[0..3]);
    assert_eq!(cut["total"], 4);
    assert!(
        cut.get("next_token").is_none(),
        "no token without --paginate"
    );

    let p1 = run_json(
        tmp.path(),
        &[
            "mention",
            "by",
            &target,
            "--limit",
            "3",
            "--paginate",
            "--json",
        ],
    );
    assert_eq!(ids(&p1["items"]), m[0..3]);
    assert_eq!(p1["restarted"], false);
    let token = p1["next_token"]
        .as_str()
        .expect("a row remains")
        .to_string();
    let p2 = run_json(
        tmp.path(),
        &[
            "mention",
            "by",
            &target,
            "--limit",
            "3",
            "--paginate",
            "--token",
            &token,
            "--json",
        ],
    );
    assert_eq!(ids(&p2["items"]), m[3..]);
    assert_eq!(p2["next_token"], serde_json::Value::Null, "the last page");

    // A stale token: a removed mention names a cached item, so the next answer restarts.
    run_json(tmp.path(), &["mention", "remove", &m[3], &target, "--json"]);
    let again = run_json(
        tmp.path(),
        &[
            "mention",
            "by",
            &target,
            "--limit",
            "3",
            "--paginate",
            "--token",
            &token,
            "--json",
        ],
    );
    assert_eq!(again["restarted"], true);
    assert_eq!(ids(&again["items"]), m[0..3]);
    assert_eq!(again["total"], 3);
}

#[test]
fn mention_by_human_view_discloses_the_cut_and_prints_the_token() {
    let tmp = TempDir::new().unwrap();
    let (target, _, _) = mentioned_board(tmp.path());
    let out = nxf(tmp.path())
        .args(["mention", "by", &target, "--limit", "2", "--paginate"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    // Trimmed: the CLI frames a human answer with blank lines (`output_framing.rs`).
    let out = String::from_utf8(out).unwrap().trim().to_string();
    assert!(out.starts_with("showing 2 of 4\n"), "{out}");
    assert_eq!(out.lines().count(), 4, "notice, two rows, token: {out}");
    assert!(
        out.lines()
            .last()
            .unwrap()
            .starts_with("next page: --paginate --token "),
        "{out}"
    );
}

#[test]
fn mention_by_a_missing_target_fails_and_paginate_needs_a_limit() {
    let tmp = TempDir::new().unwrap();
    let (target, _, _) = mentioned_board(tmp.path());
    nxf(tmp.path())
        .args(["mention", "by", "ab12.zzzz", "--json"])
        .assert()
        .failure();
    nxf(tmp.path())
        .args(["mention", "by", &target, "--paginate"])
        .assert()
        .failure();
}
