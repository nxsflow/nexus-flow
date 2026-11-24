//! NEX-171: `Engine::merge_gate_rules`, the counterpart of `nxs merge-gate rules` (RFC 0010): the
//! rules at `base`, or at the branch `[git] base` names in `nexus-flow.toml` when no base is given,
//! for the repository at or above a folder. It opens no database.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nexus_flow_seam::merge_gate::{Effect, GitError};
use nexus_flow_seam::{Engine, Error, InitError, RootError};

const SPACE: &str = "0123456789abcdef0123456789abcdef";
const GENESIS: &str = "<!-- nexus-flow: owner-rules -->\n\n- Anything a user sees first.\n";
const GATE: &str = "format = 1\ndefault = \"owner\"\nowners = [\"o\"]\nverdict_authors = [\"v\"]\n\n[[rule]]\nid = \"decided-design\"\neffect = \"pm\"\ntext = \"Only decided work.\"\n";

/// A fresh git repository for one test, on `main`, with `nexus-flow.toml` naming `base`.
fn repository(test: &str, base: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-seam-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(
        dir.join("nexus-flow.toml"),
        format!("format = 1\n\n[space]\nrepository = \"{SPACE}\"\n\n[git]\nbase = \"{base}\"\n"),
    )
    .unwrap();
    dir
}

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Commits everything, and returns the commit's full id.
fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A repository whose branch `trunk` holds the rules, and whose `main` has dropped them since.
fn rules_on_trunk(test: &str) -> (PathBuf, String) {
    let dir = repository(test, "trunk");
    write(&dir, "factory/GENESIS.md", GENESIS);
    write(&dir, "factory/merge-gate/gate.toml", GATE);
    let trunk = commit(&dir, "rules");
    git(&dir, &["branch", "trunk"]);
    write(&dir, "factory/GENESIS.md", "no rules\n");
    commit(&dir, "drop the rules");
    (dir, trunk)
}

#[test]
fn without_a_base_it_reads_the_branch_nexus_flow_toml_names() {
    let (dir, trunk) = rules_on_trunk("gate-rules-default");

    let report = Engine::merge_gate_rules(&dir, None).unwrap();

    assert_eq!(report.base, trunk);
    assert!(report.readable(), "{:?}", report.errors);
    let ids: Vec<(&str, Effect)> = report
        .rules
        .iter()
        .map(|r| (r.id.as_str(), r.effect))
        .collect();
    assert_eq!(
        ids,
        [("genesis.1", Effect::Owner), ("decided-design", Effect::Pm)]
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_base_given_wins_over_nexus_flow_toml() {
    let (dir, _) = rules_on_trunk("gate-rules-base");

    let report = Engine::merge_gate_rules(&dir, Some("main")).unwrap();

    assert_eq!(report.base, git(&dir, &["rev-parse", "main"]));
    assert!(!report.readable());
    assert_eq!(report.errors[0].code, "mark_missing");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn from_a_subfolder_it_reads_the_repository_root() {
    let (dir, trunk) = rules_on_trunk("gate-rules-nested");
    let nested = dir.join("src");
    fs::create_dir_all(&nested).unwrap();

    let report = Engine::merge_gate_rules(&nested, None).unwrap();

    assert_eq!(report.base, trunk);
    assert!(report.readable(), "{:?}", report.errors);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_base_that_is_no_commit_fails() {
    let (dir, _) = rules_on_trunk("gate-rules-no-base");

    let error = Engine::merge_gate_rules(&dir, Some("nowhere")).unwrap_err();

    assert!(
        matches!(&error, Error::Git(GitError::NotACommit { reference }) if reference == "nowhere"),
        "{error:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn outside_a_repository_it_says_so() {
    let dir = std::env::temp_dir().join(format!("nxs-seam-gate-rules-none-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let error = Engine::merge_gate_rules(&dir, Some("main")).unwrap_err();

    assert!(
        matches!(error, Error::Open(InitError::Root(RootError::NotFound { .. }))),
        "{error:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}
