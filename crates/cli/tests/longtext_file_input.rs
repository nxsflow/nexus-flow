//! Epic 95d.2 — file input for the long-text fields: `--<field>-file <path>` (create + close),
//! `--set-file field=<path>` (update). Replaces the `/tmp/`-workaround pattern cleanly, allows
//! several long-text fields from their own files in one call (which the single-STDIN path
//! cannot), and extends the SAME source resolver as 95d.1: one source per field, one STDIN
//! reader per call, files read verbatim and UTF-8-enforced.

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

const NASTY: &str =
    "first `line` with backticks\nsecond \"quoted\" 'line' with a bang!\nx = y && echo $(boom)\n";

/// Write `content` to `<dir>/<name>` and return the absolute path as a String.
fn write_file(dir: &Path, name: &str, content: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p.to_string_lossy().into_owned()
}

fn create_json(dir: &Path, extra: &[&str]) -> serde_json::Value {
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
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("create json")
}

fn new_task(dir: &Path) -> String {
    create_json(dir, &["--description", "seed"])["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn create_description_from_file_is_verbatim() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let f = write_file(tmp.path(), "desc.md", NASTY.as_bytes());
    let v = create_json(tmp.path(), &["--description-file", &f]);
    assert_eq!(v["description"], serde_json::json!(NASTY));
}

#[test]
fn multiple_file_flags_in_one_call() {
    // The differentiator over the single-STDIN path: every long-text field from its own file.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let d = write_file(tmp.path(), "d.md", b"the description");
    let g = write_file(tmp.path(), "g.md", b"the design");
    let c = write_file(tmp.path(), "c.md", b"the dod");
    let v = create_json(
        tmp.path(),
        &[
            "--description-file",
            &d,
            "--design-file",
            &g,
            "--dod-file",
            &c,
        ],
    );
    assert_eq!(v["description"], serde_json::json!("the description"));
    assert_eq!(v["design"], serde_json::json!("the design"));
    assert_eq!(v["completion_criterion"], serde_json::json!("the dod"));
}

#[test]
fn close_reason_from_file() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let f = write_file(tmp.path(), "reason.md", NASTY.as_bytes());
    let out = nxf()
        .args(["close", &id, "--reason-file", &f, "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["closing_comment"], serde_json::json!(NASTY));
}

#[test]
fn update_set_file() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let f = write_file(tmp.path(), "desc.md", NASTY.as_bytes());
    let out = nxf()
        .args([
            "update",
            &id,
            "--set-file",
            &format!("description={f}"),
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["description"], serde_json::json!(NASTY));
}

#[test]
fn inline_and_file_for_same_field_is_an_error_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let f = write_file(tmp.path(), "desc.md", b"from file");
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
            "inline",
            "--description-file",
            &f,
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    // Nothing created.
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn missing_file_is_an_error_and_writes_nothing() {
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
            "--description-file",
            "/no/such/file.md",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("io"));
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn non_utf8_file_is_a_validation_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let f = write_file(tmp.path(), "bin.dat", &[0xff, 0xfe, 0x00, 0x80]);
    let v = serde_json::from_slice::<serde_json::Value>(
        &nxf()
            .args([
                "create",
                "--type",
                "bug",
                "--title",
                "T",
                "--priority",
                "P1",
                "--description-file",
                &f,
                "--json",
            ])
            .current_dir(tmp.path())
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
}

#[test]
fn file_path_dash_aliases_stdin() {
    // `--description-file -` is the STDIN sentinel too, and counts toward the one-reader rule.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = serde_json::from_slice::<serde_json::Value>(
        &nxf()
            .args([
                "create",
                "--type",
                "bug",
                "--title",
                "T",
                "--priority",
                "P1",
                "--description-file",
                "-",
                "--json",
            ])
            .current_dir(tmp.path())
            .write_stdin(NASTY)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["description"], serde_json::json!(NASTY));
}

#[test]
fn two_stdin_readers_across_inline_and_file_dash_is_an_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = serde_json::from_slice::<serde_json::Value>(
        &nxf()
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
                "--design-file",
                "-",
                "--json",
            ])
            .current_dir(tmp.path())
            .write_stdin(NASTY)
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    assert!(msg.contains("stdin"), "got: {msg}");
}

#[test]
fn malformed_set_file_without_equals_is_an_error_and_writes_nothing() {
    // `--set-file` expects `field=path`; an argument with no `=` is a clear validation error,
    // and (like every rejected write) leaves the item untouched.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let v = serde_json::from_slice::<serde_json::Value>(
        &nxf()
            .args(["update", &id, "--set-file", "description", "--json"])
            .current_dir(tmp.path())
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("field=path"), "got: {msg}");
    // Unchanged: the seed description survives (no partial write).
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let items: serde_json::Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(items[0]["id"], serde_json::json!(id));
    assert_eq!(items[0]["description"], serde_json::json!("seed"));
}
