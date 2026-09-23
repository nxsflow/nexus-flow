//! Failure-path robustness: handled error envelopes instead of panics (PR review).

use assert_cmd::Command;
use std::fs;
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

#[test]
fn corrupt_db_yields_io_error_not_panic() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // Clobber the sqlite file with garbage (simulates a truncated/foreign file).
    fs::write(
        tmp.path().join(".nxs").join("db.sqlite"),
        b"this is definitely not a sqlite database",
    )
    .unwrap();

    let out = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    // A structured io envelope on stdout — not a panic/backtrace on stderr.
    let v: serde_json::Value = serde_json::from_slice(&out).expect("io error envelope on stdout");
    assert_eq!(v["error"]["kind"], serde_json::json!("io"));
}

#[test]
fn dep_remove_on_unknown_item_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args(["dep", "remove", "nope.aaaa", "nope.bbbb", "--json"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}
