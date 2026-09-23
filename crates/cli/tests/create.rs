//! `nxf create` with the full option set and its validation (E2.6).

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

fn create_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = vec!["create"];
    full.extend_from_slice(args);
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("create json")
}

#[test]
fn create_sets_all_provided_fields() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "Full",
            "--description",
            "d",
            "--priority",
            "P2",
            "--due",
            "2026-01-02",
            "--defer",
            "2026-01-01",
        ],
    );
    assert_eq!(v["type"], serde_json::json!("bug"));
    // The label P2 is stored as its canonical ordinal "2" in the plugin-independent record.
    assert_eq!(v["priority"], serde_json::json!("2"));
    assert_eq!(v["due"], serde_json::json!("2026-01-02"));
    assert_eq!(v["defer_until"], serde_json::json!("2026-01-01"));
}

#[test]
fn unknown_priority_label_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P9",
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
}

#[test]
fn garbage_due_date_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P1",
            "--due",
            "not-a-date",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure();
}

#[test]
fn parent_sets_belongs_to_and_must_exist() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let parent = create_json(
        tmp.path(),
        &[
            "--type",
            "epic",
            "--title",
            "P",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let pid = parent["id"].as_str().unwrap();

    let child = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "C",
            "--description",
            "d",
            "--priority",
            "P1",
            "--parent",
            pid,
        ],
    );
    assert_eq!(child["belongs_to"], serde_json::json!(pid));

    // Unknown parent is a not_found error.
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P1",
            "--parent",
            "nope.MISSING",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}

// ---- 8qv.3: agent-complete create in a single call ----

#[test]
fn one_call_sets_title_description_priority_design_dod_parent() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let parent = create_json(
        tmp.path(),
        &[
            "--type",
            "epic",
            "--title",
            "Epic",
            "--description",
            "the epic",
            "--priority",
            "P0",
        ],
    );
    let pid = parent["id"].as_str().unwrap().to_string();

    let v = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "Full",
            "--description",
            "why and goal",
            "--priority",
            "P1",
            "--design",
            "the approach",
            "--dod",
            "tests pass",
            "--parent",
            &pid,
        ],
    );
    assert_eq!(v["title"], serde_json::json!("Full"));
    assert_eq!(v["description"], serde_json::json!("why and goal"));
    assert_eq!(v["priority"], serde_json::json!("1"));
    assert_eq!(v["design"], serde_json::json!("the approach"));
    assert_eq!(v["completion_criterion"], serde_json::json!("tests pass"));
    assert_eq!(v["belongs_to"], serde_json::json!(pid));
}

#[test]
fn depends_on_creates_directed_edges_in_one_call() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "A",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let b = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "B",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let aid = a["id"].as_str().unwrap().to_string();
    let bid = b["id"].as_str().unwrap().to_string();

    // New task depends on BOTH A and B (repeatable flag) at creation.
    let c = create_json(
        tmp.path(),
        &[
            "--type",
            "bug",
            "--title",
            "C",
            "--description",
            "d",
            "--priority",
            "P1",
            "--depends-on",
            &aid,
            "--depends-on",
            &bid,
        ],
    );
    let cid = c["id"].as_str().unwrap().to_string();

    // `show --json` lists both deps; C is blocked until they close.
    let show_out = nxf()
        .args(["show", &cid, "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_out).unwrap();
    let deps: Vec<&str> = show["deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert!(
        deps.contains(&aid.as_str()) && deps.contains(&bid.as_str()),
        "deps: {deps:?}"
    );
}

#[test]
fn depends_on_nonexistent_is_not_found_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P1",
            "--depends-on",
            "nope.MISSING",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("not_found"));
}

#[test]
fn missing_description_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // No --description: a clear "required" failure (clap), nothing created.
    nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--priority",
            "P1",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure();
}

// ---- nexus-flow-82h: create -q/--id-only (id-capture ergonomics) ----

/// A deterministic `nxf` so the first-created id is the stable bare `0001` (prefix `ab12`).
fn nxf_det() -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1");
    c
}

fn init_det(dir: &Path) {
    nxf_det()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

fn create_q(dir: &Path, extra: &[&str]) -> Vec<u8> {
    let mut args = vec![
        "create",
        "--type",
        "bug",
        "--title",
        "x",
        "--description",
        "d",
        "--priority",
        "P1",
    ];
    args.extend_from_slice(extra);
    nxf_det()
        .args(args)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

#[test]
fn id_only_prints_exactly_the_bare_id_one_line() {
    // -q prints ONLY the new id — bare (display form, ykv), one line, no JSON, no output framing.
    let tmp = TempDir::new().unwrap();
    init_det(tmp.path());
    let out = String::from_utf8(create_q(tmp.path(), &["-q"])).unwrap();
    assert_eq!(out, "0001\n", "exactly the bare id, one line, unframed");
}

#[test]
fn id_only_long_flag_matches_the_short_flag() {
    let tmp = TempDir::new().unwrap();
    init_det(tmp.path());
    let out = String::from_utf8(create_q(tmp.path(), &["--id-only"])).unwrap();
    assert_eq!(out, "0001\n");
}

#[test]
fn q_takes_precedence_over_json() {
    // -q + --json: -q wins (just the id), --json is ignored — documented precedence, not an error.
    let tmp = TempDir::new().unwrap();
    init_det(tmp.path());
    let out = String::from_utf8(create_q(tmp.path(), &["-q", "--json"])).unwrap();
    assert_eq!(out, "0001\n", "-q wins; no JSON object emitted");
    assert!(!out.contains('{'), "no JSON framing leaks through: {out}");
}

#[test]
fn id_only_writes_nothing_to_stdout_on_failure() {
    // `-q` disables output framing, so its error branch is a DISTINCT path: a failed create must
    // exit non-zero and print NO id (not a partial line) — otherwise `id=$(nxf create … -q)` would
    // capture garbage. The dangling `--parent` is rejected before any id is minted.
    let tmp = TempDir::new().unwrap();
    init_det(tmp.path());
    let out = nxf_det()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P1",
            "--parent",
            "nope.MISSING",
            "-q",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    assert!(
        out.is_empty(),
        "no id (nor partial line) on stdout when the create fails: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn id_only_output_feeds_straight_back_into_the_next_command() {
    // The captured id is directly re-usable as an id argument (the whole point of -q).
    let tmp = TempDir::new().unwrap();
    init_det(tmp.path());
    let id = String::from_utf8(create_q(tmp.path(), &["-q"]))
        .unwrap()
        .trim()
        .to_string();
    let shown = nxf_det()
        .args(["show", &id])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(shown).unwrap().contains("x"),
        "the captured bare id resolves back to the created item"
    );
}

// ---- plugin custom fields (T3, 6j6v.p7b2): the create `--set` flag ----

#[test]
fn create_set_flag_reaches_the_custom_field_validation() {
    // T3: `nxf create --set name=value` threads through CreateArgs → the shared write layer, which
    // rejects a name no active plugin declares (issue-tracker declares no `[fields]`). This proves
    // the new create `--set` flag is WIRED to the facade custom-field path, not silently dropped.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
            "--priority",
            "P1",
            "--set",
            "nope=whatever",
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
    assert!(
        v["error"]["msg"]
            .as_str()
            .unwrap_or_default()
            .contains("custom field"),
        "the error explains --set carries custom fields: {}",
        v["error"]
    );
}

#[test]
fn missing_priority_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "x",
            "--description",
            "d",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure();
}
