//! `nxf show` (human) renders the FULL field model in the active plugin's language (8qv.8):
//! a header (`<id> <priority> <title>`), TYPE/STATUS, optional PARENT/BLOCKED BY, and the four
//! sections DESCRIPTION / DEFINITION OF DONE / DESIGN / NOTES whose headings carry the plugin's
//! field labels. `--json` stays the byte-exact machine contract (unchanged) and is not tested here.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path, plugin: &str) {
    nxf()
        .args(["init", "--plugin", plugin])
        .current_dir(dir)
        .assert()
        .success();
}

/// Create an item and return its id. `args` are extra flags appended after the mandatory ones.
fn create(dir: &Path, ty: &str, title: &str, extra: &[&str]) -> String {
    let mut args = vec![
        "create",
        "--type",
        ty,
        "--title",
        title,
        "--description",
        "why it exists",
        "--priority",
    ];
    // issue-tracker uses P-labels; personal-todo uses word labels. Caller passes the label via
    // extra when needed; default to a valid issue-tracker label.
    args.push("P1");
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
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn show(dir: &Path, id: &str) -> String {
    let out = nxf()
        .args(["show", id])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn renders_full_field_model_with_parent_and_blocked_by() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let blocker = create(tmp.path(), "bug", "Blocker", &[]);
    let parent = create(tmp.path(), "epic", "Epic", &[]);
    let task = create(
        tmp.path(),
        "bug",
        "The work",
        &[
            "--design",
            "the approach",
            "--dod",
            "tests pass",
            "--parent",
            &parent,
            "--depends-on",
            &blocker,
        ],
    );
    nxf()
        .args(["note", "add", &task, "a worklog entry"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let out = show(tmp.path(), &task);
    // Header: id (display form — bare suffix for a local item, ykv), priority LABEL (not the
    // ordinal), title, then an `=` rule. PARENT/BLOCKED BY likewise carry the bare local id.
    let bare = |id: &str| id.split_once('.').unwrap().1.to_string();
    assert!(
        out.contains(&bare(&task)) && out.contains("P1") && out.contains("The work"),
        "{out}"
    );
    assert!(
        !out.contains("ab12."),
        "local prefix is stripped from human text:\n{out}"
    );
    assert!(out.contains("===="), "missing header rule:\n{out}");
    // Inline structural labels with plugin-vocab values.
    assert!(out.contains("TYPE: bug"), "type label:\n{out}");
    assert!(out.contains("STATUS: open"), "status label:\n{out}");
    assert!(
        out.contains(&format!("PARENT: {}", bare(&parent))),
        "parent:\n{out}"
    );
    assert!(
        out.contains(&format!("BLOCKED BY: {}", bare(&blocker))),
        "blocked by:\n{out}"
    );
    // Section headings (issue-tracker field labels, uppercased) + their values.
    assert!(out.contains("DESCRIPTION"), "{out}");
    assert!(out.contains("why it exists"), "{out}");
    assert!(
        out.contains("DEFINITION OF DONE") && out.contains("tests pass"),
        "{out}"
    );
    assert!(
        out.contains("DESIGN") && out.contains("the approach"),
        "{out}"
    );
    assert!(
        out.contains("NOTES") && out.contains("a worklog entry"),
        "{out}"
    );
}

#[test]
fn omits_parent_and_blocked_by_when_absent() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let task = create(tmp.path(), "bug", "Standalone", &[]);
    let out = show(tmp.path(), &task);
    assert!(
        !out.contains("PARENT:"),
        "no parent line when unset:\n{out}"
    );
    assert!(
        !out.contains("BLOCKED BY:"),
        "no blocked-by when not blocked:\n{out}"
    );
    // The field sections are still present (the full model, not a subset).
    assert!(
        out.contains("DESCRIPTION") && out.contains("NOTES"),
        "{out}"
    );
}

#[test]
fn section_headings_use_the_plugins_field_labels() {
    // personal-todo renames the field headings — the seam: a plugin determines the field labels.
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "personal-todo");
    let o = nxf()
        .args([
            "create",
            "--type",
            "todo",
            "--title",
            "Buy milk",
            "--description",
            "we are out",
            "--priority",
            "now",
            "--design",
            "go to the shop",
            "--dod",
            "milk in fridge",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = serde_json::from_slice::<serde_json::Value>(&o).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let shown = show(tmp.path(), &id);
    // personal-todo's casual field labels, uppercased — and NOT the issue-tracker defaults.
    assert!(
        shown.contains("WHY") && shown.contains("we are out"),
        "{shown}"
    );
    assert!(
        shown.contains("DONE WHEN") && shown.contains("milk in fridge"),
        "{shown}"
    );
    assert!(
        shown.contains("PLAN") && shown.contains("go to the shop"),
        "{shown}"
    );
    assert!(shown.contains("LOG"), "{shown}");
    assert!(
        !shown.contains("DESCRIPTION"),
        "issue-tracker label must not leak:\n{shown}"
    );
    assert!(!shown.contains("DEFINITION OF DONE"), "{shown}");
}

#[test]
fn appends_parent_notice_at_the_very_end_when_parents_exist() {
    // 6j6v.zvd0: an item with a parent gets a loud, verbatim notice appended at the very END of
    // `nxf show`'s human output — after NOTES — pointing the reader at the parent(s).
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let parent = create(tmp.path(), "epic", "Epic", &[]);
    let task = create(tmp.path(), "bug", "The work", &["--parent", &parent]);
    nxf()
        .args(["note", "add", &task, "a worklog entry"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let out = show(tmp.path(), &task);
    let expected = format!(
        "This item has the following parents: {parent}. URGENT RECOMMENDATION: ALSO READ THESE \
         ITEMS TO GET THE COMPLETE PICTURE!!!"
    );
    assert!(
        out.trim_end().ends_with(expected.trim_end()),
        "notice must be the very last thing in the output:\n{out}"
    );
}

#[test]
fn omits_parent_notice_when_no_parent() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let task = create(tmp.path(), "bug", "Standalone", &[]);
    let out = show(tmp.path(), &task);
    assert!(
        !out.contains("URGENT RECOMMENDATION"),
        "no parent notice when there is no parent:\n{out}"
    );
}

#[test]
fn omits_parent_notice_for_a_contributes_to_only_edge() {
    // Test Quality #1 (PR #255 review): pin the load-bearing parent-vs-contributes_to
    // distinction at the CLI layer too — a contributes_to edge must never trigger the notice.
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let plan = create(tmp.path(), "epic", "Birthday plan", &[]);
    let cream = create(tmp.path(), "bug", "Cream", &[]);
    nxf()
        .args(["contributes", "add", &cream, &plan])
        .current_dir(tmp.path())
        .assert()
        .success();

    let out = show(tmp.path(), &cream);
    assert!(
        !out.contains("URGENT RECOMMENDATION"),
        "a contributes_to edge is not a parent, so no notice:\n{out}"
    );
}

#[test]
fn notes_are_listed_newest_first() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let task = create(tmp.path(), "bug", "T", &[]);
    for n in ["first note", "second note"] {
        nxf()
            .args(["note", "add", &task, n])
            .current_dir(tmp.path())
            .assert()
            .success();
    }
    let out = show(tmp.path(), &task);
    let first = out.find("first note").unwrap();
    let second = out.find("second note").unwrap();
    assert!(second < first, "newest note must come first:\n{out}");
}
