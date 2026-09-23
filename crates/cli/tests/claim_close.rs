//! `nxf claim` / `nxf close` sugar over the update write path (E2.9).

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

fn new_task(dir: &Path) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "T",
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

fn run(dir: &Path, args: &[&str]) -> serde_json::Value {
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

#[test]
fn claim_sets_in_progress_and_optional_assignee() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());

    let v = run(tmp.path(), &["claim", &id, "--assignee", "alice"]);
    assert_eq!(v["status"], serde_json::json!("in_progress"));
    assert_eq!(v["assignee"], serde_json::json!("alice"));
}

#[test]
fn close_sets_closed_and_reason() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());

    let v = run(tmp.path(), &["close", &id, "--reason", "done & dusted"]);
    assert_eq!(v["status"], serde_json::json!("closed"));
    assert_eq!(v["closing_comment"], serde_json::json!("done & dusted"));
}

#[test]
fn close_without_reason_is_a_validation_error() {
    // nexus-flow-ohh: close-only with a mandatory reason — closing without `--reason` is rejected
    // as a loud validation error in the standard envelope (also under `--json`), not silently
    // accepted with an empty closing comment.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());

    let out = nxf()
        .args(["close", &id, "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));

    // Rejected and wrote nothing: the item is still open and can still be closed with a reason.
    let still = run(tmp.path(), &["close", &id, "--reason", "now with a reason"]);
    assert_eq!(still["status"], serde_json::json!("closed"));
}

#[test]
fn claim_unknown_id_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args(["claim", "nope.MISSING", "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}

// ---- 07a.5: close(parent) sweep warning ------------------------------------

/// Create an item of `ty` (optionally under `parent`); returns its full id.
fn create_typed(dir: &Path, ty: &str, title: &str, parent: Option<&str>) -> String {
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
}

#[test]
fn closing_a_parent_lists_swept_children_in_the_json_receipt() {
    // 07a.5: closing the (only) open parent of a child sweeps it into the effective-closed lane
    // without writing to it — the receipt must say so via a `swept_children` join. (The multi-parent
    // "still has an open parent → not swept" case is pinned by the facade tests; issue-tracker is
    // single-parent.) A close that sweeps nothing carries no field.
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let epic = create_typed(p, "epic", "Container", None);
    let child = create_typed(p, "bug", "Nested work", Some(&epic));

    let v = run(p, &["close", &epic, "--reason", "container done"]);
    assert_eq!(
        v["id"].as_str().unwrap(),
        epic,
        "receipt is the closed epic"
    );
    let swept = v["swept_children"]
        .as_array()
        .expect("swept_children present");
    let ids: Vec<&str> = swept.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [child.as_str()],
        "the open child with no parent left is swept"
    );
    assert_eq!(swept[0]["title"], "Nested work");
    assert!(
        swept[0]["other_parents"].as_array().unwrap().is_empty(),
        "the swept child had only the closed epic as a parent"
    );

    // A childless close carries no swept_children field (sparse).
    let lonely = create_typed(p, "bug", "Solo", None);
    let v2 = run(p, &["close", &lonely, "--reason", "done"]);
    assert!(
        v2.get("swept_children").is_none(),
        "no sweep → no swept_children field"
    );
}

#[test]
fn closing_a_parent_warns_about_swept_children_in_human_output() {
    // The human receipt carries the same sweep as a warning line, naming the swept child by title.
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    init(p);
    let epic = create_typed(p, "epic", "Container", None);
    let _child = create_typed(p, "bug", "Nested work", Some(&epic));

    let out = nxf()
        .args(["close", &epic, "--reason", "done"])
        .current_dir(p)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("warning: you closed not only") && text.contains("Nested work"),
        "human close output carries the sweep warning naming the child: {text}"
    );
}
