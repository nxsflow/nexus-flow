//! `nxf mention` — the reference edge (bm0/§8). A pure reference from one item to a
//! short-id it cites in free text; it must NOT affect ready/blocked (that is a `dep`).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir);
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
