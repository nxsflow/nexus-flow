//! C5 (#916.5): the `archive`/`unarchive` mutations through the CLI seam — cascade directions,
//! preconditions, the partial-batch result shape, and the round-trip with the `archived`/`closed`
//! lanes (C3). Deterministic ids + a pinned clock keep ids and the archive instant byte-stable.

use assert_cmd::Command;
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

/// Create an item of `ty` (optionally under `parent`); returns its id.
fn create(dir: &Path, ty: &str, title: &str, parent: Option<&str>) -> String {
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
    ];
    if let Some(p) = parent {
        args.push("--parent");
        args.push(p);
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
}

fn close(dir: &Path, id: &str) {
    nxf()
        .args(["close", id, "--reason", "done"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Run a command, returning its parsed `--json` object.
fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
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

fn ids(dir: &Path, args: &[&str]) -> Vec<String> {
    json(dir, args)
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn is_archived_via_show(dir: &Path, id: &str) -> bool {
    json(dir, &["show", id])["item"]["archived"]
        .as_str()
        .is_some()
}

#[test]
fn archive_cascades_down_and_round_trips_through_the_lanes() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let epic = create(p, "epic", "E", None);
    let child = create(p, "bug", "C", Some(&epic));
    close(p, &child);
    close(p, &epic);

    // Both are in the `closed` lane before archiving.
    assert!(ids(p, &["closed"]).contains(&epic));

    let res = json(p, &["archive", &epic]);
    assert_eq!(res["results"][0]["status"], "archived");
    let mut affected: Vec<String> = res["archived"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    affected.sort();
    let mut want = vec![epic.clone(), child.clone()];
    want.sort();
    assert_eq!(affected, want, "cascade archives the epic AND its child");

    // Now both are in the `archived` lane and out of `closed`.
    let archived = ids(p, &["archived"]);
    assert!(archived.contains(&epic) && archived.contains(&child));
    assert!(
        !ids(p, &["closed"]).contains(&epic),
        "archived leaves the closed lane"
    );

    // Unarchive the child: it + its ancestor (epic) resurface; nothing else.
    let un = json(p, &["unarchive", &child]);
    assert_eq!(un["results"][0]["status"], "unarchived");
    assert!(!is_archived_via_show(p, &child) && !is_archived_via_show(p, &epic));
}

#[test]
fn archive_rejects_an_open_root_with_a_reason() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let task = create(p, "bug", "open one", None); // never closed

    let res = json(p, &["archive", &task]);
    assert_eq!(res["results"][0]["status"], "failed");
    assert_eq!(res["results"][0]["reason"], "not-closed");
    assert!(res["archived"].as_array().unwrap().is_empty());
    assert!(
        !is_archived_via_show(p, &task),
        "a rejected archive writes nothing"
    );
}

#[test]
fn archive_rejects_a_subtree_with_an_open_descendant() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let epic = create(p, "epic", "E", None);
    let child = create(p, "bug", "C", Some(&epic)); // stays open
    close(p, &epic);

    let res = json(p, &["archive", &epic]);
    assert_eq!(res["results"][0]["status"], "failed");
    assert_eq!(res["results"][0]["reason"], "has-open-descendants");
    assert!(
        !is_archived_via_show(p, &epic),
        "atomic per root: the closed epic is untouched"
    );
    let _ = child;
}

#[test]
fn archive_batch_reports_per_root_and_is_independent() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let ok = create(p, "bug", "ok", None);
    close(p, &ok);
    let bad = create(p, "bug", "bad", None); // open → fails

    let res = json(p, &["archive", &ok, &bad, "nope"]);
    let results = res["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "one result per input id, in order");
    assert_eq!(results[0]["status"], "archived");
    assert_eq!(results[1]["reason"], "not-closed");
    assert_eq!(results[2]["reason"], "not-found");
    assert_eq!(res["archived"], serde_json::json!([ok]));
}

#[test]
fn unarchive_rejects_a_not_archived_item() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let task = create(p, "bug", "live", None);

    let res = json(p, &["unarchive", &task]);
    assert_eq!(res["results"][0]["status"], "failed");
    assert_eq!(res["results"][0]["reason"], "not-archived");
}
