//! The vertical seam: create + show end-to-end through the active plugin config,
//! and the config-swap proof that the seam is real (E2.5).

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

/// Create an item and return its id (parsed from the --json record).
fn create(dir: &Path, ty: &str, title: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            ty,
            "--title",
            title,
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
    let v: serde_json::Value = serde_json::from_slice(&out).expect("create json");
    v["id"].as_str().expect("id field").to_string()
}

/// Point the workspace at a different embedded plugin config.
fn set_plugin(dir: &Path, plugin: &str) {
    let cfg = dir.join(".nxs").join("config.toml");
    fs::write(&cfg, format!("plugin = \"{plugin}\"\n")).unwrap();
}

#[test]
fn create_then_show_roundtrips() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "bug", "Write the seam");

    let out = nxf()
        .args(["show", &id, "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["item"]["id"], serde_json::json!(id));
    assert_eq!(v["item"]["type"], serde_json::json!("bug"));
    assert_eq!(v["item"]["title"], serde_json::json!("Write the seam"));
}

#[test]
fn show_unknown_id_is_not_found() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxf()
        .args(["show", "nope.MISSING", "--json"])
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
fn config_swap_changes_human_view_but_not_json_contract() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "bug", "Same item");

    // --json: the CANONICAL record is identical regardless of the active plugin. The additive
    // ee2h `priority_label`/`type_label` are the plugin's PRESENTATION vocabulary and legitimately
    // differ, so strip them before asserting the plugin-independent contract.
    let show_json = |plugin: &str| -> serde_json::Value {
        set_plugin(tmp.path(), plugin);
        let out = nxf()
            .args(["show", &id, "--json"])
            .current_dir(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&out).unwrap()
    };
    let strip_labels = |mut v: serde_json::Value| {
        if let Some(item) = v.get_mut("item").and_then(|i| i.as_object_mut()) {
            item.remove("priority_label");
            item.remove("type_label");
        }
        v
    };
    let json_a = show_json("issue-tracker");
    let json_b = show_json("personal-todo");
    assert_eq!(
        strip_labels(json_a.clone()),
        strip_labels(json_b.clone()),
        "the canonical json record must not depend on the plugin"
    );
    // The additive labels ARE plugin presentation — present under each plugin, resolved through its
    // own vocabulary (issue-tracker labels the priority `P…`, personal-todo its own variant names).
    assert!(
        json_a["item"]["priority_label"].is_string()
            && json_b["item"]["priority_label"].is_string(),
        "both plugins carry the additive priority_label"
    );
    assert_ne!(
        json_a["item"]["priority_label"], json_b["item"]["priority_label"],
        "the priority_label reflects the active plugin's vocabulary"
    );

    // The human view DOES change: issue-tracker shows the status as "open",
    // personal-todo remaps the same core status to "todo".
    set_plugin(tmp.path(), "issue-tracker");
    let human_it = String::from_utf8(
        nxf()
            .args(["show", &id])
            .current_dir(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    set_plugin(tmp.path(), "personal-todo");
    let human_pt = String::from_utf8(
        nxf()
            .args(["show", &id])
            .current_dir(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_ne!(
        human_it, human_pt,
        "human view must reflect the plugin vocabulary"
    );
    assert!(
        human_it.contains("open"),
        "issue-tracker status label: {human_it}"
    );
    assert!(
        human_pt.contains("todo"),
        "personal-todo status label: {human_pt}"
    );
}
