//! Integration tests for `nxm`'s memory verbs (nexus-flow-aye.21): remember / recall / memories /
//! forget, black-box over the built binary. Each test seeds its own `.nxs/` workspace via `nxm
//! init`, so the verbs are exercised exactly as an agent would call them.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// A fresh workspace with `nxm` ready. Returns the temp dir (kept alive by the caller).
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(&tmp).arg("init").assert().success();
    tmp
}

fn nxm(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(tmp.path())
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

#[test]
fn remember_then_recall_round_trips_the_body() {
    let tmp = workspace();
    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let out = nxm(&tmp)
        .args(["--json", "recall", "auth-jwt"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["key"], "auth-jwt");
    assert_eq!(v["body"], "auth uses JWT not sessions");
    assert_eq!(v["author"], "alice");
    assert_eq!(v["updated"], "2026-06-20T10:00:00Z");
    assert_eq!(v["active"], true);
}

#[test]
fn remember_without_key_mints_a_deterministic_auto_key() {
    let tmp = workspace();
    let out = nxm(&tmp)
        .args([
            "--json",
            "remember",
            "always run tests with -race",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let key = v["key"].as_str().unwrap();
    assert!(key.starts_with("f-"), "auto-key is f-prefixed: {key}");

    // Byte-identical body → same auto-key (dedup); reworded → different.
    let out2 = nxm(&tmp)
        .args([
            "--json",
            "remember",
            "always run tests with -race",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    assert_eq!(
        json_of(&out2.get_output().stdout)["key"].as_str().unwrap(),
        key
    );
}

#[test]
fn a_stable_key_updates_in_place() {
    let tmp = workspace();
    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT (HS256), refresh in Redis",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    let out = nxm(&tmp).args(["--json", "memories"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v.as_array().unwrap().len(),
        1,
        "one entity under the stable key"
    );
    assert_eq!(v[0]["body"], "auth uses JWT (HS256), refresh in Redis");
}

#[test]
fn forget_hides_then_re_remember_revives() {
    let tmp = workspace();
    nxm(&tmp)
        .args([
            "remember",
            "secret",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(&tmp).args(["forget", "k"]).assert().success();

    // recall after forget is not_found (exit 1, structured error under --json).
    let out = nxm(&tmp).args(["--json", "recall", "k"]).assert().failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
    nxm(&tmp)
        .args(["--json", "memories"])
        .assert()
        .success()
        .stdout("[]\n");

    // a later remember revives it.
    nxm(&tmp)
        .args([
            "remember",
            "v2",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    let out = nxm(&tmp).args(["--json", "recall", "k"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout)["body"], "v2");
}

#[test]
fn forget_an_unknown_key_is_not_found() {
    let tmp = workspace();
    let out = nxm(&tmp)
        .args(["--json", "forget", "ghost"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
}

#[test]
fn memories_lists_sorted_and_searches_substring() {
    let tmp = workspace();
    nxm(&tmp)
        .args([
            "remember",
            "Dolt phantom DBs",
            "--key",
            "dolt-phantoms",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let out = nxm(&tmp).args(["--json", "memories"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    let keys: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["auth-jwt", "dolt-phantoms"], "key-sorted");

    // case-insensitive substring over key+body.
    let out = nxm(&tmp)
        .args(["--json", "memories", "DOLT"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["key"], "dolt-phantoms");
}

#[test]
fn empty_body_is_rejected() {
    let tmp = workspace();
    let out = nxm(&tmp)
        .args(["--json", "remember", "", "--introduction", "in one line"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
}

#[test]
fn empty_explicit_key_is_rejected() {
    // An empty `--key` (e.g. an unset shell var) would otherwise become an unaddressable
    // `memories` PK; reject it like an empty body rather than store a surprise auto-key.
    let tmp = workspace();
    let out = nxm(&tmp)
        .args([
            "--json",
            "remember",
            "some fact",
            "--key",
            "",
            "--introduction",
            "in one line",
        ])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
}

#[test]
fn verbs_outside_a_workspace_fail_loudly() {
    let tmp = TempDir::new().unwrap(); // no init
    let out = nxm(&tmp).args(["--json", "memories"]).assert().failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "no_workspace"
    );
}
