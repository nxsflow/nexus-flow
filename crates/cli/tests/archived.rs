//! C1 (#916.1): the `archived` tombstone end-to-end through the CLI seam. Archiving has no verb
//! yet (that is C5), so the test sets the `archived` cell directly on the workspace store, then
//! asserts the C1 contract: archived items drop out of the *enumerating* reads (`list`/`search`)
//! but an explicit `show` still resolves them with a visible marker (no `not_found`).

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
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
            "shared-needle",
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

/// Archive `id` by setting the `archived` timestamp cell directly on the workspace store — the C5
/// `archive` verb does not exist yet, so this is the only C1 write path to the new field.
fn archive(dir: &Path, id: &str, when: &str) {
    let mut store = workspace::discover(dir).unwrap().open_store().unwrap();
    store.set_field(id, "archived", Some(when.to_string()), "test");
}

fn ids(dir: &Path, args: &[&str]) -> Vec<String> {
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
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn archived_item_drops_out_of_list_and_search() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let live = create(tmp.path(), "live one");
    let gone = create(tmp.path(), "archived one");
    archive(tmp.path(), &gone, "2026-06-23T10:00:00Z");

    let listed = ids(tmp.path(), &["list"]);
    assert!(listed.contains(&live), "live item still listed: {listed:?}");
    assert!(
        !listed.contains(&gone),
        "archived item excluded from list: {listed:?}"
    );

    // Both items match the shared description needle; only the live one comes back.
    let found = ids(tmp.path(), &["search", "shared-needle"]);
    assert_eq!(
        found,
        vec![live],
        "search excludes the archived item by default"
    );
}

#[test]
fn show_still_displays_an_archived_item_with_a_marker() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let gone = create(tmp.path(), "archived one");
    archive(tmp.path(), &gone, "2026-06-23T10:00:00Z");

    // --json: the canonical record carries the archived timestamp (no not_found).
    let out = nxf()
        .args(["show", &gone, "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["item"]["archived"].as_str(),
        Some("2026-06-23T10:00:00Z"),
        "show --json carries the archived marker: {v}"
    );

    // Human view: a visible ARCHIVED marker line.
    nxf()
        .args(["show", &gone])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("ARCHIVED"));
}
