//! Priority is a **named variant** (8qv.2): the active plugin's `[priority] labels` are the
//! valid set AND the ranking order. The agent SETS a label (e.g. `P1`); the record stores the
//! plugin-independent canonical ordinal (`"1"`) so `--json` stays byte-identical across plugins,
//! and the human surface maps it back to the label. Every write path validates the supplied
//! label against the set, rejecting unknowns loudly with the structured `validation` envelope
//! whose message lists the accepted labels. The set is config-driven (issue-tracker: P0..P4).

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

/// stdout JSON of a failed command, asserting it failed.
fn fail_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn create_accepts_named_priority_and_stores_canonical_ordinal() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // P0 is variant 0, P4 is variant 4 — the agent sets the label, the record keeps the ordinal.
    for (label, ordinal) in [("P0", "0"), ("P4", "4")] {
        let out = nxf()
            .args([
                "create",
                "--type",
                "bug",
                "--title",
                "ok",
                "--description",
                "d",
                "--priority",
                label,
                "--json",
            ])
            .current_dir(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v["priority"],
            serde_json::json!(ordinal),
            "{label} stores ordinal {ordinal} in the plugin-independent --json record"
        );
    }
}

#[test]
fn human_show_renders_priority_as_the_named_label() {
    // The label round-trips on the HUMAN surface: set P3, see P3 (the canonical ordinal "3" is a
    // --json detail). `show` is plain (no --json).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "ok",
            "--description",
            "d",
            "--priority",
            "P3",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    nxf()
        .args(["show", &id])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("P3"));
}

#[test]
fn create_rejects_unknown_priority_and_lists_the_labels() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // `=` form so a leading-`-` value isn't mistaken for a flag (clap convention).
    for p in ["P9", "high", "5"] {
        let arg = format!("--priority={p}");
        let v = fail_json(
            tmp.path(),
            &[
                "create",
                "--type",
                "bug",
                "--title",
                "x",
                "--description",
                "d",
                &arg,
            ],
        );
        assert_eq!(
            v["error"]["kind"],
            serde_json::json!("validation"),
            "priority {p} must be a validation error"
        );
        let msg = v["error"]["msg"].as_str().unwrap_or_default();
        assert!(
            msg.contains("P0") && msg.contains("P4"),
            "error must list the accepted labels, got: {msg}"
        );
    }
}

#[test]
fn update_accepts_named_priority_and_stores_canonical_ordinal() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    for (label, ordinal) in [("P0", "0"), ("P4", "4")] {
        let out = nxf()
            .args([
                "update",
                &id,
                "--set",
                &format!("priority={label}"),
                "--json",
            ])
            .current_dir(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["priority"], serde_json::json!(ordinal));
    }
}

#[test]
fn update_rejects_unknown_priority_and_lists_the_labels() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    for p in ["P9", "5"] {
        let v = fail_json(
            tmp.path(),
            &["update", &id, "--set", &format!("priority={p}")],
        );
        assert_eq!(
            v["error"]["kind"],
            serde_json::json!("validation"),
            "priority {p} must be a validation error"
        );
        let msg = v["error"]["msg"].as_str().unwrap_or_default();
        assert!(
            msg.contains("P0") && msg.contains("P4"),
            "error must list the accepted labels, got: {msg}"
        );
    }
}
