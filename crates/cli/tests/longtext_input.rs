//! Epic 95d — agent-native, escaping-free long-text input.
//!
//! 95d.1: the `-` STDIN sentinel for the long-text fields (description, design, dod, and
//! `close --reason`) plus `note add <id> -`, on both create and update. The shared rule:
//! exactly one source per field, and at most ONE field may read STDIN per call.

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

/// A deliberately nasty multi-line payload: backticks, single + double quotes, a bang, and
/// shell metacharacters including an embedded `=` (which would break a naive `field=value`
/// split). The whole point of the STDIN path is that none of this needs shell-escaping.
const NASTY: &str =
    "first `line` with backticks\nsecond \"quoted\" 'line' with a bang!\nx = y && echo $(boom)\n";

/// `nxf create … --json` with the given extra args and a piped STDIN body; returns the record.
fn create_stdin(dir: &Path, extra: &[&str], stdin: &str) -> serde_json::Value {
    let mut args = vec![
        "create",
        "--type",
        "bug",
        "--title",
        "T",
        "--priority",
        "P1",
    ];
    args.extend_from_slice(extra);
    args.push("--json");
    let out = nxf()
        .args(args)
        .current_dir(dir)
        .write_stdin(stdin)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("create json")
}

fn new_task(dir: &Path) -> String {
    let v = create_stdin(dir, &["--description", "seed"], "");
    v["id"].as_str().unwrap().to_string()
}

#[test]
fn create_description_from_stdin_is_verbatim() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = create_stdin(tmp.path(), &["--description", "-"], NASTY);
    assert_eq!(v["description"], serde_json::json!(NASTY));
}

#[test]
fn create_design_from_stdin_keeps_inline_description() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = create_stdin(
        tmp.path(),
        &["--description", "short", "--design", "-"],
        NASTY,
    );
    assert_eq!(v["design"], serde_json::json!(NASTY));
    assert_eq!(v["description"], serde_json::json!("short"));
}

#[test]
fn create_dod_from_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = create_stdin(tmp.path(), &["--description", "short", "--dod", "-"], NASTY);
    assert_eq!(v["completion_criterion"], serde_json::json!(NASTY));
}

#[test]
fn stdin_body_is_preserved_verbatim_including_trailing_newline() {
    // Conservative contract (95d.1): STDIN is stored byte-for-byte — a trailing newline is
    // kept, and a body without one stays without one. What you pipe is what you get.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let with_nl = create_stdin(tmp.path(), &["--description", "-"], "abc\n");
    assert_eq!(with_nl["description"], serde_json::json!("abc\n"));
    let without_nl = create_stdin(tmp.path(), &["--description", "-"], "abc");
    assert_eq!(without_nl["description"], serde_json::json!("abc"));
}

#[test]
fn update_set_value_from_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = nxf()
        .args(["update", &id, "--set", "description=-", "--json"])
        .current_dir(tmp.path())
        .write_stdin(NASTY)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["description"], serde_json::json!(NASTY));
}

#[test]
fn close_reason_from_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let out = nxf()
        .args(["close", &id, "--reason", "-", "--json"])
        .current_dir(tmp.path())
        .write_stdin(NASTY)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["closing_comment"], serde_json::json!(NASTY));
    assert_eq!(v["status"], serde_json::json!("closed"));
}

#[test]
fn note_add_text_from_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    nxf()
        .args(["note", "add", &id, "-"])
        .current_dir(tmp.path())
        .write_stdin(NASTY)
        .assert()
        .success();
    let out = nxf()
        .args(["note", "list", &id, "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v[0]["body"], serde_json::json!(NASTY));
}

#[test]
fn two_stdin_fields_is_an_error_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "T",
            "--priority",
            "P1",
            "--description",
            "-",
            "--design",
            "-",
            "--json",
        ])
        .current_dir(tmp.path())
        .write_stdin(NASTY)
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        msg.contains("stdin"),
        "error should mention STDIN, got: {msg}"
    );
    // No partial write: nothing was created.
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let items: serde_json::Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(
        items.as_array().unwrap().len(),
        0,
        "failed create must write nothing"
    );
}
