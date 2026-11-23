//! `nxs init` (RFC 0001, "Creating and finding a space"): in a fresh git repository it writes
//! `nexus-flow.toml` and prints the space id as JSON; run again it changes nothing.

mod common;

use std::fs;
use std::process::Command;

use common::{json, nxs, scratch, stdout};

#[test]
fn init_in_a_fresh_git_repo_writes_the_file_and_prints_the_space_id() {
    let base = scratch("init");
    let repo = base.join("repo");
    let git = Command::new("git").arg("init").arg("-q").arg(&repo).status();
    assert!(git.unwrap().success(), "git init");

    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    let report = json(&out);
    let space = report["space"].as_str().expect("space is a string");
    assert_eq!(space.len(), 32, "{space}");
    assert_eq!(report["created"], true);
    let file = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    assert!(file.contains(&format!("repository = \"{space}\"")), "{file}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_run_twice_changes_nothing() {
    let base = scratch("init-twice");
    let repo = base.join("repo");
    let first = json(&nxs(&base, &repo, &["init"]));
    let file = fs::read(repo.join("nexus-flow.toml")).unwrap();

    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let second = json(&out);
    assert_eq!(second["space"], first["space"]);
    assert_eq!(second["created"], false);
    assert_eq!(fs::read(repo.join("nexus-flow.toml")).unwrap(), file);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_from_a_subfolder_finds_the_repository_root() {
    let base = scratch("init-nested");
    let repo = base.join("repo");
    let first = json(&nxs(&base, &repo, &["init"]));
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();

    let second = json(&nxs(&base, &nested, &["init"]));

    assert_eq!(second["space"], first["space"]);
    assert!(!nested.join("nexus-flow.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}
