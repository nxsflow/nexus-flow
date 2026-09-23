//! `nxf search` over the materialized projection: title, body, notes (E2.14).

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

fn create(dir: &Path, title: &str) -> String {
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

fn search(dir: &Path, args: &[&str]) -> Vec<String> {
    let mut full = vec!["search"];
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
fn matches_title_case_insensitively() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "Alpha Beta");
    create(tmp.path(), "Gamma");
    assert_eq!(search(tmp.path(), &["beta"]), vec![id]);
    assert!(search(tmp.path(), &["delta"]).is_empty());
}

#[test]
fn matches_description_and_notes() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let desc_hit = create(tmp.path(), "one");
    nxf()
        .args([
            "update",
            &desc_hit,
            "--set",
            "description=a secret ingredient",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();
    let note_hit = create(tmp.path(), "two");
    nxf()
        .args(["note", "add", &note_hit, "remember the secret"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let mut found = search(tmp.path(), &["secret"]);
    found.sort();
    let mut expected = vec![desc_hit, note_hit];
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn respects_type_filter() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "match me");
    nxf()
        .args([
            "create",
            "--type",
            "epic",
            "--title",
            "match me too",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert_eq!(search(tmp.path(), &["match", "--type", "bug"]).len(), 1);
}

// ---- lane-ranked search (C6 #916.6) ----------------------------------------

/// Create an open task whose description carries `needle`; returns its id.
fn create_match(dir: &std::path::Path, title: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            title,
            "--description",
            "findme here",
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

fn search_ids(dir: &std::path::Path, args: &[&str]) -> Vec<String> {
    let mut full = vec!["search", "findme"];
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
    serde_json::from_slice::<serde_json::Value>(&out)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn search_groups_by_lane_and_handles_the_archive_scope() {
    let tmp = tempfile::TempDir::new().unwrap();
    let p = tmp.path();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(p)
        .assert()
        .success();

    let ready = create_match(p, "ready");
    let blocked = create_match(p, "blocked");
    nxf()
        .args(["dep", "add", &blocked, &ready])
        .current_dir(p)
        .assert()
        .success();
    let closed = create_match(p, "closed");
    nxf()
        .args(["close", &closed, "--reason", "done"])
        .current_dir(p)
        .assert()
        .success();
    let archived = create_match(p, "archived");
    nxf()
        .args(["close", &archived, "--reason", "done"])
        .current_dir(p)
        .assert()
        .success();
    nxf()
        .args(["archive", &archived])
        .current_dir(p)
        .assert()
        .success();

    // Default: archived excluded, grouped next+ip → blocked → closed.
    assert_eq!(
        search_ids(p, &[]),
        vec![ready.clone(), blocked.clone(), closed.clone()],
        "matches grouped by lane priority; archive excluded"
    );
    // --include-archived appends the archived match last.
    assert_eq!(
        search_ids(p, &["--include-archived"]),
        vec![
            ready.clone(),
            blocked.clone(),
            closed.clone(),
            archived.clone()
        ]
    );
    // --archived-only searches only the archive.
    assert_eq!(search_ids(p, &["--archived-only"]), vec![archived]);
}

#[test]
fn search_surfaces_a_child_masked_by_its_parents_through_the_cli() {
    // 07a.3 §8 end-to-end: a child whose effective lane is decided by its parents must not vanish
    // from `search`. A still-open child of a closed parent is closed-masked → it groups under the
    // closed lane; an open child of a blocked parent is suppressed → it groups under blocked. Both
    // used to disappear (the gap shipped in #146); both must now appear.
    let tmp = tempfile::TempDir::new().unwrap();
    let p = tmp.path();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(p)
        .assert()
        .success();

    // The issue-tracker hierarchy lets a `bug` sit under an `epic`, so the gating parents are epics.
    let create = |dir: &std::path::Path, ty: &str, title: &str, parent: Option<&str>| -> String {
        let mut args = vec![
            "create",
            "--type",
            ty,
            "--title",
            title,
            "--description",
            "findme here",
            "--priority",
            "P1",
        ];
        if let Some(pid) = parent {
            args.push("--parent");
            args.push(pid);
        }
        args.push("--json");
        let out = nxf()
            .args(args)
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
    };

    // Closed parent → closed-masked open child.
    let closed_parent = create(p, "epic", "closed parent", None);
    let masked_child = create(p, "bug", "masked child", Some(&closed_parent));
    nxf()
        .args(["close", &closed_parent, "--reason", "done"])
        .current_dir(p)
        .assert()
        .success();
    // Blocked parent (own open dep) → suppressed open child.
    let blocked_parent = create(p, "epic", "blocked parent", None);
    let blocker = create_match(p, "the blocker");
    nxf()
        .args(["dep", "add", &blocked_parent, &blocker])
        .current_dir(p)
        .assert()
        .success();
    let suppressed_child = create(p, "bug", "suppressed child", Some(&blocked_parent));

    let ids = search_ids(p, &[]);
    for (id, what) in [
        (&masked_child, "closed-masked child"),
        (&suppressed_child, "suppressed child"),
    ] {
        assert!(
            ids.contains(id),
            "the {what} surfaces in search instead of vanishing: {ids:?}"
        );
    }
}
