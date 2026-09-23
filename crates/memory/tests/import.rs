//! Integration tests for `nxm import` (aye.24): migrate an existing Claude-host memory store into
//! nexus-memory. Black-box over the built binary, mirroring how an agent (or `nxm init`'s upsell)
//! would drive it. The source is never modified (non-destructive) and a re-run is idempotent
//! (stable, name-derived keys upsert in place).

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z");
    c
}

/// A fixture Claude-host memory directory with a `MEMORY.md` index and two frontmatter facts.
fn seed_claude_memory(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("MEMORY.md"),
        "- [Auth](auth-jwt.md) — jwt\n- [Race](race.md) — testing\n",
    )
    .unwrap();
    fs::write(
        dir.join("auth-jwt.md"),
        "---\nname: auth-jwt\ndescription: auth\nmetadata:\n  type: project\n---\n\nauth uses JWT not sessions\n",
    )
    .unwrap();
    fs::write(
        dir.join("race.md"),
        "---\nname: race-flag\ndescription: testing\n---\n\nalways run tests with the -race flag\n",
    )
    .unwrap();
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

#[test]
fn import_brings_claude_host_memories_in_under_their_stable_keys() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    let source = tmp.path().join("claude-memory");
    seed_claude_memory(&source);

    let out = nxm(tmp.path())
        .args(["--json", "import", "--from"])
        .arg(&source)
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["imported"], 2, "both frontmatter facts imported");
    assert_eq!(
        v["keys"],
        serde_json::json!(["auth-jwt", "race-flag"]),
        "keys are the frontmatter names, sorted"
    );

    // The facts are now first-class nexus-memory entries under their host names.
    let recalled = nxm(tmp.path())
        .args(["--json", "recall", "auth-jwt"])
        .assert()
        .success();
    assert_eq!(
        json_of(&recalled.get_output().stdout)["body"],
        "auth uses JWT not sessions"
    );
}

#[test]
fn import_is_idempotent_and_non_destructive() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    let source = tmp.path().join("claude-memory");
    seed_claude_memory(&source);
    let before = fs::read_to_string(source.join("auth-jwt.md")).unwrap();

    for _ in 0..2 {
        nxm(tmp.path())
            .args(["import", "--from"])
            .arg(&source)
            .assert()
            .success();
    }

    let out = nxm(tmp.path())
        .args(["--json", "memories"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v.as_array().unwrap().len(),
        2,
        "a re-run upserts in place — no duplicates"
    );
    // The source is untouched (read-only migration).
    assert_eq!(
        fs::read_to_string(source.join("auth-jwt.md")).unwrap(),
        before,
        "the source file is never modified"
    );
}

#[test]
fn import_with_no_source_is_quiet_success_not_an_error() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    // No --from, and no Claude memory dir for this temp cwd → nothing to do, exit 0.
    let out = nxm(tmp.path())
        .args(["--json", "import"])
        .env("NXM_CLAUDE_MEMORY_DIR", "") // force "no source"
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["imported"], 0);
    assert_eq!(v["source"], Value::Null);
}

#[test]
fn init_surfaces_an_import_suggestion_when_a_claude_source_is_present() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("claude-memory");
    seed_claude_memory(&source);

    let out = nxm(tmp.path())
        .args(["--json", "init"])
        .env("NXM_CLAUDE_MEMORY_DIR", &source)
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v["import_suggestion"]["count"], 2,
        "init detects the 2 importable host memories and proposes the import"
    );
}

#[test]
fn init_without_a_claude_source_makes_no_suggestion() {
    let tmp = TempDir::new().unwrap();
    let out = nxm(tmp.path())
        .args(["--json", "init"])
        .env("NXM_CLAUDE_MEMORY_DIR", "")
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert!(v.get("import_suggestion").is_none(), "no source → no noise");
}
