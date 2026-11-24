//! `nxs genesis create [--update]` (RFC 0004, "Installing the template", "Output"): writes
//! `factory/GENESIS.md` only where the path is free, installs the guide, and prints the report,
//! its keys in the RFC's order and `codeowners` `null` without a CODEOWNERS file; a GENESIS git
//! still tracks exits non-zero with the report on stdout; without `nexus-flow.toml` it names
//! `nxs init`. It never opens the database: the options are never asked for.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{json, stdout};
use nxs_v2::Outcome;

const GENESIS: &str = "factory/GENESIS.md";
const GUIDE: &str = "factory/genesis-guide.md";

/// A fresh git repository with a `nexus-flow.toml`.
fn repository(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-v2-genesis-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("nexus-flow.toml"),
        format!(
            "format = 1\n\n[space]\nrepository = \"{}\"\n",
            "0".repeat(32)
        ),
    )
    .unwrap();
    git(&repo, &["init", "-q"]);
    repo.canonicalize().unwrap()
}

fn git(repo: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .expect("git runs")
        .success();
    assert!(ok, "git {args:?}");
}

/// Runs `nxs <args>` in `dir` with options that fail the test if asked for: `genesis create`
/// opens no database.
fn nxs(dir: &Path, args: &[&str]) -> Outcome {
    let argv = std::iter::once("nxs").chain(args.iter().copied());
    nxs_v2::run(argv, dir, || {
        panic!("genesis create asked for the database's options")
    })
}

fn cleanup(repo: &Path) {
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}

#[test]
fn create_writes_genesis_and_prints_the_report() {
    let repo = repository("written");

    let out = nxs(&repo, &["genesis", "create"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    assert_eq!(
        stdout(&out),
        concat!(
            r#"{"files":[{"path":"factory/GENESIS.md","state":"written"},"#,
            r#"{"path":"factory/genesis-guide.md","state":"written"}],"codeowners":null}"#,
            "\n"
        )
    );
    assert!(repo.join(GENESIS).is_file());
    assert!(repo.join(GUIDE).is_file());
    cleanup(&repo);
}

#[test]
fn create_with_update_leaves_the_teams_genesis_as_it_is() {
    let repo = repository("team");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), b"ours\xff").unwrap();

    let out = nxs(&repo, &["genesis", "create", "--update"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["files"][0]["state"], "team");
    assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), b"ours\xff");
    cleanup(&repo);
}

#[test]
fn a_case_variant_is_reported_with_the_path_found() {
    let repo = repository("case");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join("factory/Genesis.md"), "ours").unwrap();

    let out = nxs(&repo, &["genesis", "create"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        concat!(
            r#"{"files":[{"path":"factory/GENESIS.md","state":"team","found":"factory/Genesis.md"},"#,
            r#"{"path":"factory/genesis-guide.md","state":"written"}],"codeowners":null}"#,
            "\n"
        )
    );
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn a_tracked_but_missing_genesis_exits_non_zero_with_the_report() {
    let repo = repository("tracked");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), "ours").unwrap();
    git(&repo, &["add", GENESIS]);
    git(&repo, &["commit", "-q", "-m", "genesis"]);
    fs::remove_file(repo.join(GENESIS)).unwrap();

    let out = nxs(&repo, &["genesis", "create"]);

    assert_ne!(out.code, 0);
    assert_eq!(
        stdout(&out),
        concat!(
            r#"{"files":[{"path":"factory/GENESIS.md","state":"tracked"},"#,
            r#"{"path":"factory/genesis-guide.md","state":"written"}],"codeowners":null}"#,
            "\n"
        )
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains(GENESIS), "{stderr}");
    assert!(stderr.contains("git"), "{stderr}");
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn markdown_names_each_file_and_its_state() {
    let repo = repository("md");

    let out = nxs(&repo, &["genesis", "create", "--md"]);

    assert_eq!(out.code, 0);
    let text = stdout(&out);
    assert!(text.contains("`factory/GENESIS.md`"), "{text}");
    assert!(text.contains("`factory/genesis-guide.md`"), "{text}");
    assert!(text.contains("written"), "{text}");
    assert!(text.contains("no CODEOWNERS file"), "{text}");
    cleanup(&repo);
}

#[test]
fn the_report_names_genesis_when_codeowners_leaves_it_uncovered() {
    let repo = repository("codeowners");
    fs::create_dir_all(repo.join(".github")).unwrap();
    fs::write(repo.join(".github/CODEOWNERS"), "/src/ @owner\n").unwrap();

    let out = nxs(&repo, &["genesis", "create"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        concat!(
            r#"{"files":[{"path":"factory/GENESIS.md","state":"written"},"#,
            r#"{"path":"factory/genesis-guide.md","state":"written"}],"#,
            r#""codeowners":{"file":".github/CODEOWNERS","uncovered":["factory/GENESIS.md"]}}"#,
            "\n"
        )
    );
    let md = stdout(&nxs(&repo, &["genesis", "create", "--md"]));
    assert!(md.contains("`.github/CODEOWNERS`"), "{md}");
    assert!(md.contains("`factory/GENESIS.md`"), "{md}");
    cleanup(&repo);
}

#[test]
fn update_reports_the_teams_guide_and_leaves_it_as_it_is() {
    let repo = repository("guide-team");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GUIDE), "our guide").unwrap();

    let out = nxs(&repo, &["genesis", "create", "--update"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["files"][1]["path"], GUIDE);
    assert_eq!(json(&out)["files"][1]["state"], "team");
    assert_eq!(fs::read_to_string(repo.join(GUIDE)).unwrap(), "our guide");
    cleanup(&repo);
}

#[test]
fn without_nexus_flow_toml_create_names_nxs_init() {
    let dir = std::env::temp_dir().join(format!("nxs-v2-genesis-no-toml-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let out = nxs(&dir, &["genesis", "create"]);

    assert_eq!(out.code, 1);
    let error = json(&out);
    assert_eq!(error["error"]["kind"], "no_workspace");
    let msg = error["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("nxs init"), "{msg}");
    assert!(!dir.join("factory").exists());
    fs::remove_dir_all(&dir).unwrap();
}
