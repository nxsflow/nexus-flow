//! nexus-flow-ykv: prefix-relative id ergonomics. A LOCAL item (this replica's prefix) is shown
//! and accepted by its bare suffix; a FOREIGN item (another replica's prefix, arrived via sync)
//! always carries its full `<prefix>.<suffix>` — in CLI text OUTPUT and in CLI INPUT alike. The
//! `--json` contract is untouched: it always emits the full id.
//!
//! The fixture manufactures a foreign item without a relay: create an item under prefix `aaaa`,
//! then re-stamp this replica's prefix to `bbbb`. The `aaaa.*` item is now foreign relative to the
//! local `bbbb` prefix, exactly as a synced item would be — the same `replica.toml` re-pin the
//! collision e2e test uses.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir);
    // Sequential suffixes (`.0001`) so the ids are predictable; the prefix we pin by hand below.
    c.env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// Re-stamp this workspace's replica prefix (site/uuid fixed), as a sync prefix reassignment would.
fn set_prefix(dir: &Path, prefix: &str) {
    std::fs::write(
        dir.join(".nxs").join("replica.toml"),
        format!("site_id = 1\nprefix = \"{prefix}\"\nreplica_uuid = \"uuid-fixture\"\n"),
    )
    .unwrap();
}

fn create(dir: &Path, title: &str) -> String {
    let out = nxf(dir)
        .args([
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
        ])
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

fn human(dir: &Path, args: &[&str]) -> String {
    let out = nxf(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

/// Build a workspace holding one FOREIGN item (`aaaa.0001`) and one LOCAL item (`bbbb.0001`),
/// with the local prefix pinned to `bbbb`. Returns the temp dir (kept alive by the caller).
fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    // Mint the soon-to-be-foreign item under prefix `aaaa`…
    set_prefix(tmp.path(), "aaaa");
    let foreign = create(tmp.path(), "Foreign work");
    assert_eq!(foreign, "aaaa.0001");
    // …then move this replica to `bbbb`, so `aaaa.0001` is now foreign, and mint a local item.
    set_prefix(tmp.path(), "bbbb");
    let local = create(tmp.path(), "Local work");
    assert_eq!(local, "bbbb.0001");
    tmp
}

#[test]
fn list_text_shows_local_bare_and_foreign_full() {
    let tmp = fixture();
    let out = human(tmp.path(), &["list"]);
    // The local prefix is stripped everywhere in human text — its bare suffix names it.
    assert!(
        !out.contains("bbbb."),
        "local prefix must never appear in human text: {out}"
    );
    // The foreign item keeps its full id (its suffix is ambiguous against the local namespace).
    assert!(
        out.contains("aaaa.0001"),
        "foreign item keeps its full id: {out}"
    );
    // Both rows are present (bare local + full foreign).
    assert!(
        out.contains("Local work") && out.contains("Foreign work"),
        "{out}"
    );
}

#[test]
fn list_json_emits_full_ids_for_local_and_foreign() {
    let tmp = fixture();
    let out = human(tmp.path(), &["list", "--json"]);
    // The machine contract is untouched: every id is the full `<prefix>.<suffix>`.
    assert!(
        out.contains("\"id\":\"bbbb.0001\""),
        "json keeps the local full id: {out}"
    );
    assert!(
        out.contains("\"id\":\"aaaa.0001\""),
        "json keeps the foreign full id: {out}"
    );
}

#[test]
fn show_accepts_a_bare_local_id_and_renders_it_bare() {
    let tmp = fixture();
    // INPUT: a bare id resolves against the local prefix → the LOCAL item.
    let out = human(tmp.path(), &["show", "0001"]);
    assert!(
        out.contains("Local work"),
        "bare id resolves locally: {out}"
    );
    // OUTPUT: the local item is rendered without its prefix.
    assert!(!out.contains("bbbb."), "local id shown bare in show: {out}");
    assert!(
        out.lines().any(|l| l.starts_with("0001 ")),
        "header carries the bare local id: {out}"
    );
}

#[test]
fn show_accepts_a_full_foreign_id_and_renders_it_full() {
    let tmp = fixture();
    // INPUT: the full foreign id is taken verbatim → the FOREIGN item.
    let out = human(tmp.path(), &["show", "aaaa.0001"]);
    assert!(
        out.contains("Foreign work"),
        "full foreign id resolves to the foreign item: {out}"
    );
    // OUTPUT: a foreign item keeps its full id in the header.
    assert!(
        out.lines().any(|l| l.starts_with("aaaa.0001 ")),
        "header carries the full foreign id: {out}"
    );
}

#[test]
fn a_full_local_id_is_also_accepted_on_input() {
    let tmp = fixture();
    // The qualified local id still works as input (back-compat) — and still renders bare.
    let out = human(tmp.path(), &["show", "bbbb.0001"]);
    assert!(out.contains("Local work"), "full local id accepted: {out}");
    assert!(!out.contains("bbbb."), "still rendered bare: {out}");
}

#[test]
fn an_unresolvable_bare_id_is_a_clean_not_found() {
    // The new resolve_id input surface must still fail LOUDLY on a miss: a bare id with no
    // matching local item (`9999` → `ab12.9999` → lookup) is a clean `not_found`, never a panic
    // or a silent success. The existing not-found tests only cover a *qualified* miss.
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let out = nxf(tmp.path())
        .args(["show", "9999", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["error"]["kind"],
        serde_json::json!("not_found"),
        "an unresolvable bare id 404s cleanly: {v}"
    );
}

#[test]
fn mutations_accept_a_bare_local_id() {
    // Bare-id input resolution is wired through the MUTATING handlers, not just `show`: a single-id
    // mutation (`claim`) and a two-id mutation (`dep add`) both resolve a bare suffix to the local
    // item and land the write. Pins the handler wiring beyond the unit-tested helper.
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let a = create(tmp.path(), "A");
    let b = create(tmp.path(), "B");
    assert_eq!((a.as_str(), b.as_str()), ("ab12.0001", "ab12.0002"));

    // `dep add 0001 0002` resolves BOTH bare endpoints; A (open) is then blocked by B.
    nxf(tmp.path())
        .args(["dep", "add", "0001", "0002"])
        .assert()
        .success();
    let blocked = human(tmp.path(), &["blocked", "--json"]);
    let bj: serde_json::Value = serde_json::from_str(&blocked).unwrap();
    let ids: Vec<&str> = bj
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"ab12.0001"),
        "bare two-id dep add resolved and landed: {ids:?}"
    );

    // `claim 0002` resolves the single bare id and lands (status → in_progress).
    nxf(tmp.path()).args(["claim", "0002"]).assert().success();
    let shown = human(tmp.path(), &["show", "0002", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(
        v["item"]["status"],
        serde_json::json!("in_progress"),
        "bare claim landed: {v}"
    );
}
