//! An unmoved v1 workspace (RFC 0008, "The installed base"): upgrading the binary moves no
//! workspace, so v2's `nxs` in a repository whose `.nxs/` has no marker (`.nxs/migrated.toml`)
//! works on the repository space as RFC 0001 describes, and says on standard error, not in its
//! output, that the v1 workspace has not moved and that `nxs migrate --check` shows the move.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{nxs, scratch, stdout};

/// A repository given its space by `nxs init`, without a v1 workspace.
fn repository(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch(test);
    let repo = base.join("repo");
    let out = nxs(&base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    (base, repo)
}

/// A v1 workspace at `root`, as v1 leaves it: `.nxs/` with its database.
fn v1_workspace(root: &Path) {
    fs::create_dir_all(root.join(".nxs")).unwrap();
    fs::write(root.join(".nxs/db.sqlite"), b"v1").unwrap();
}

fn stderr(out: &nxs_v2::Outcome) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

fn says_not_moved(err: &str) {
    assert!(err.contains("has not moved"), "{err}");
    assert!(err.contains("nxs migrate --check"), "{err}");
}

#[test]
fn an_unmarked_v1_workspace_is_named_on_stderr_and_stdout_is_unchanged() {
    let (base, repo) = repository("v1-notice-json");
    let without = nxs(&base, &repo, &["sync", "--status"]);
    assert!(without.stderr.is_empty(), "{}", stderr(&without));

    v1_workspace(&repo);
    let with = nxs(&base, &repo, &["sync", "--status"]);
    assert_eq!(with.code, 0, "{}", stdout(&with));
    assert_eq!(stdout(&with), stdout(&without));
    says_not_moved(&stderr(&with));
}

#[test]
fn with_md_the_notice_is_on_stderr_and_stdout_is_unchanged() {
    let (base, repo) = repository("v1-notice-md");
    let without = nxs(&base, &repo, &["sync", "--status", "--md"]);
    v1_workspace(&repo);
    let with = nxs(&base, &repo, &["sync", "--status", "--md"]);
    assert_eq!(stdout(&with), stdout(&without));
    says_not_moved(&stderr(&with));
}

#[test]
fn a_marked_v1_workspace_gets_no_notice() {
    let (base, repo) = repository("v1-notice-marked");
    v1_workspace(&repo);
    fs::write(repo.join(".nxs/migrated.toml"), "format = 1\n").unwrap();
    let out = nxs(&base, &repo, &["sync", "--status"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
}

#[test]
fn the_notice_is_given_in_a_folder_below_the_root() {
    let (base, repo) = repository("v1-notice-below");
    v1_workspace(&repo);
    let below = repo.join("src/deep");
    fs::create_dir_all(&below).unwrap();
    let out = nxs(&base, &below, &["sync", "--status"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    says_not_moved(&stderr(&out));
}

#[test]
fn a_nested_repository_reads_its_own_root_not_the_v1_workspace_above() {
    let (base, repo) = repository("v1-notice-nested");
    v1_workspace(&base);
    let out = nxs(&base, &repo, &["sync", "--status"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
}

#[test]
fn init_beside_a_v1_workspace_gives_the_notice_on_stderr() {
    let base = scratch("v1-notice-init");
    let repo = base.join("repo");
    v1_workspace(&repo);
    let out = nxs(&base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    let report = common::json(&out);
    assert_eq!(report["v1_workspace"], true, "{report}");
    says_not_moved(&stderr(&out));
}

#[test]
fn an_error_beside_a_v1_workspace_keeps_its_object_and_gives_the_notice() {
    let base = scratch("v1-notice-error");
    let repo = base.join("repo");
    v1_workspace(&repo);
    let out = nxs(&base, &repo, &["sync", "--status"]);
    assert_eq!(out.code, 1);
    assert_eq!(common::json(&out)["error"]["kind"], "no_workspace");
    says_not_moved(&stderr(&out));
}
