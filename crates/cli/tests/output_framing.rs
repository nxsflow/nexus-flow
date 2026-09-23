//! Human CLI output gets a blank line before the first and after the last line —
//! breathing room in the terminal (nexus-flow-vwx). The `--json` machine contract is
//! NEVER framed (it must stay byte-exact and deterministic for agents). Errors are framed
//! too: the leading blank is emitted on stdout before the command runs, the trailing blank
//! follows the message on stderr — so a terminal shows blank / error / blank, matching the
//! shape of successful output.

use assert_cmd::Command;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

#[test]
fn human_success_output_is_framed_with_blank_lines() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.starts_with('\n'), "leading blank line: {s:?}");
    assert!(s.ends_with("\n\n"), "trailing blank line: {s:?}");
    assert!(
        s.contains("initialized"),
        "still carries the payload: {s:?}"
    );
}

#[test]
fn json_output_is_never_framed() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(
        !s.starts_with('\n'),
        "json must not be framed (agent contract): {s:?}"
    );
    assert!(
        s.starts_with('{'),
        "json envelope begins immediately: {s:?}"
    );
    // Proof it is still parseable, exactly as before.
    serde_json::from_str::<serde_json::Value>(s.trim()).expect("valid json");
}

#[test]
fn human_error_output_is_framed_on_stderr() {
    // Non-interactive `init` with no plugin fails by contract; the human error gets the
    // trailing blank on stderr, and the leading blank on stdout (before the command ran).
    let tmp = TempDir::new().unwrap();
    let output = nxf()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(output.stderr).unwrap();
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(
        err.starts_with("error:"),
        "error message on stderr: {err:?}"
    );
    assert!(
        err.ends_with("\n\n"),
        "trailing blank line after error: {err:?}"
    );
    assert_eq!(out, "\n", "leading blank line emitted on stdout: {out:?}");
}

#[test]
fn json_error_output_is_never_framed() {
    let tmp = TempDir::new().unwrap();
    let output = nxf()
        .args(["--json", "init"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .clone();
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(
        out.starts_with('{'),
        "json error envelope, unframed: {out:?}"
    );
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("valid json envelope");
    assert_eq!(v["error"]["kind"], "validation");
}
