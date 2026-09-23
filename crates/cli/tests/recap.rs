//! `nxf recap` (r4kb): the recency recall verb — most-recently closed first, archived closes
//! INCLUDED (distinct from `nxf closed`, the lane, which drops archived). Full records under
//! `--json`, a compact capped line in text; `--limit` (default 10) and `--since` bound the view.

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", "2026-06-23T00:00:00Z");
    c
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
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Close `id`, stamping `closed_at` from `when` so recency order is deterministic.
fn close_at(dir: &Path, id: &str, reason: &str, when: &str) {
    nxf()
        .args(["close", id, "--reason", reason])
        .env("NXF_NOW", when)
        .current_dir(dir)
        .assert()
        .success();
}

/// Set the `archived` cell directly (recap must still INCLUDE an archived CLOSED item).
fn archive_cell(dir: &Path, id: &str, when: &str) {
    let mut store = workspace::discover(dir).unwrap().open_store().unwrap();
    store.set_field(id, "archived", Some(when.to_string()), "test");
}

fn recap_json(dir: &Path, args: &[&str]) -> Vec<serde_json::Value> {
    let mut full = vec!["recap"];
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
        .clone()
}

fn recap_ids(dir: &Path, args: &[&str]) -> Vec<String> {
    recap_json(dir, args)
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn recap_text(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["recap"];
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

#[test]
fn recap_lists_closed_newest_first_and_includes_archived_closed() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "older");
    let b = create(tmp.path(), "newer, then archived");
    close_at(tmp.path(), &a, "did a", "2026-06-01T00:00:00Z");
    close_at(tmp.path(), &b, "did b", "2026-06-05T00:00:00Z");
    archive_cell(tmp.path(), &b, "2026-06-20T00:00:00Z");

    // recap: newest close first, the archived-closed item INCLUDED.
    assert_eq!(recap_ids(tmp.path(), &[]), vec![b.clone(), a.clone()]);

    // Contrast — the `closed` LANE drops the archived one (archived precedence). recap is distinct.
    let closed_out = nxf()
        .args(["closed", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed_ids: Vec<String> = serde_json::from_slice::<serde_json::Value>(&closed_out)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        closed_ids,
        vec![a],
        "closed lane excludes the archived item; recap keeps it"
    );
}

#[test]
fn recap_json_returns_full_records_in_recency_order() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "a");
    let b = create(tmp.path(), "b");
    close_at(tmp.path(), &a, "ra", "2026-06-01T00:00:00Z");
    close_at(tmp.path(), &b, "rb", "2026-06-02T00:00:00Z");

    let recs = recap_json(tmp.path(), &[]);
    assert_eq!(
        recs.iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![b.as_str(), a.as_str()],
        "recency order preserved"
    );
    // Full canonical record (uncapped): status/type/full closing_comment, not a ginned-down summary.
    assert_eq!(recs[0]["status"].as_str().unwrap(), "closed");
    assert!(recs[0]["type"].is_string());
    assert_eq!(
        recs[0]["closing_comment"].as_str().unwrap(),
        "rb",
        "the full, uncapped closing_comment rides the JSON record"
    );
}

#[test]
fn recap_limit_defaults_to_10_and_respects_the_flag() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    for i in 0..3 {
        let id = create(tmp.path(), &format!("t{i}"));
        close_at(
            tmp.path(),
            &id,
            "r",
            &format!("2026-06-0{}T00:00:00Z", i + 1),
        );
    }
    assert_eq!(
        recap_ids(tmp.path(), &["--limit", "2"]).len(),
        2,
        "limit caps"
    );
    assert_eq!(
        recap_ids(tmp.path(), &[]).len(),
        3,
        "default (10) shows all 3"
    );
}

#[test]
fn recap_default_limit_caps_at_ten() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // 11 closed items at distinct instants; the default window (no --limit) keeps the 10 newest.
    let mut ids = Vec::new();
    for i in 1..=11 {
        let id = create(tmp.path(), &format!("t{i:02}"));
        close_at(tmp.path(), &id, "r", &format!("2026-06-{i:02}T00:00:00Z"));
        ids.push(id);
    }
    let shown = recap_ids(tmp.path(), &[]);
    assert_eq!(shown.len(), 10, "the default caps at 10");
    assert!(
        !shown.contains(&ids[0]),
        "the oldest close (the 11th) is the one dropped"
    );
    // An explicit larger --limit lifts the cap and shows all 11.
    assert_eq!(
        recap_ids(tmp.path(), &["--limit", "20"]).len(),
        11,
        "--limit 20 shows all 11"
    );
}

#[test]
fn recap_json_returns_the_full_uncapped_closing_comment() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "big");
    let long = "y".repeat(500);
    close_at(tmp.path(), &id, &long, "2026-06-10T00:00:00Z");

    // Unlike prime's capped mirror, `recap --json` carries the WHOLE closing_comment (500 chars).
    let recs = recap_json(tmp.path(), &[]);
    assert_eq!(
        recs[0]["closing_comment"].as_str().unwrap(),
        long,
        "the full, uncapped 500-char comment rides --json"
    );
}

#[test]
fn recap_since_filters_by_close_date_inclusive() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "before");
    let b = create(tmp.path(), "on the floor");
    let c = create(tmp.path(), "after");
    close_at(tmp.path(), &a, "r", "2026-06-01T00:00:00Z");
    close_at(tmp.path(), &b, "r", "2026-06-10T00:00:00Z");
    close_at(tmp.path(), &c, "r", "2026-06-15T00:00:00Z");

    // Inclusive floor: the close ON 2026-06-10 stays, the earlier one drops. Plain-date floor vs
    // full-timestamp closed_at — the ISO-8601 string order makes it correct.
    let ids = recap_ids(tmp.path(), &["--since", "2026-06-10"]);
    assert_eq!(ids, vec![c, b], "closed_at >= floor, newest first");
    assert!(!ids.contains(&a), "the earlier close is excluded");
}

#[test]
fn recap_rejects_a_bad_since_date() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxf()
        .args(["recap", "--since", "not-a-date"])
        .current_dir(tmp.path())
        .assert()
        .failure();
}

#[test]
fn recap_text_is_a_compact_capped_line() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "Ship <v1>");
    let long = "x".repeat(400);
    close_at(tmp.path(), &id, &long, "2026-06-10T00:00:00Z");

    let out = recap_text(tmp.path(), &[]);
    // `- id · title · note`: the title's markdown metacharacters are escaped, the note collapsed
    // and capped to 280 chars + `…`.
    assert!(
        out.contains("· Ship \\<v1\\> ·"),
        "compact escaped line:\n{out}"
    );
    assert!(out.contains('…'), "the long note is ellipsized");
    assert!(
        !out.contains(&"x".repeat(281)),
        "the note is capped below its full length"
    );
}

#[test]
fn recap_help_documents_limit_and_since() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args(["recap", "--help"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(out).unwrap();
    assert!(help.contains("--limit"), "help documents --limit");
    assert!(help.contains("--since"), "help documents --since");
}
