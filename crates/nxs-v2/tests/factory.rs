//! `nxs factory install [--update]` (RFC 0006, "Installing the shipped defaults"; "Public surfaces
//! this RFC brings"): writes the shipped defaults that are missing, and with `--update` replaces
//! an earlier default, under `factory/`; prints `files`, each with `path` and `state`, ordered by
//! path bytewise, and `codeowners`, `null` without a CODEOWNERS file.
//!
//! The defaults `nxs` embeds have one version each, so `older_default` and `updated` are held by
//! the seam's tests with defaults of their own (`crates/seam/tests/install_factory.rs`).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{json, nxs, scratch, stdout};
use serde_json::{json, Value};

const SHIPPED: [&str; 5] = [
    "factory/CONFIG.md",
    "factory/FACTORY.md",
    "factory/roles/coder.md",
    "factory/roles/pm.md",
    "factory/status-note.md",
];

/// A repository given its space by `nxs init`.
fn repository(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch(test);
    let repo = base.join("repo");
    let out = nxs(&base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    (base, repo)
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn files(states: &[(&str, &str)]) -> Value {
    Value::Array(
        states
            .iter()
            .map(|(path, state)| json!({ "path": path, "state": state }))
            .collect(),
    )
}

#[test]
fn install_writes_every_missing_default_and_prints_the_report() {
    let (base, repo) = repository("factory-install");

    let out = nxs(&base, &repo, &["factory", "install"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    // The fields in the RFC's order: `files` first, then `codeowners`.
    let files: Vec<String> = SHIPPED
        .iter()
        .map(|p| format!(r#"{{"path":"{p}","state":"written"}}"#))
        .collect();
    assert_eq!(
        stdout(&out),
        format!("{{\"files\":[{}],\"codeowners\":null}}\n", files.join(","))
    );
    for path in SHIPPED {
        assert!(repo.join(path).is_file(), "{path} is written");
    }
}

#[test]
fn install_again_finds_every_default_current() {
    let (base, repo) = repository("factory-install-again");
    nxs(&base, &repo, &["factory", "install"]);

    let out = nxs(&base, &repo, &["factory", "install", "--update"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let states: Vec<(&str, &str)> = SHIPPED.iter().map(|p| (*p, "current")).collect();
    assert_eq!(json(&out)["files"], files(&states));
}

#[test]
fn a_team_changed_role_file_stays_byte_identical_with_and_without_update() {
    let (base, repo) = repository("factory-install-team");
    let team = "# Our coder\r\n\r\nWe work our own way.\n";
    write(&repo, "factory/roles/coder.md", team);

    for args in [
        &["factory", "install"][..],
        &["factory", "install", "--update"][..],
    ] {
        let out = nxs(&base, &repo, args);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        let report = json(&out);
        let coder = report["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "factory/roles/coder.md")
            .unwrap();
        assert_eq!(coder["state"], "team");
        assert_eq!(
            fs::read(repo.join("factory/roles/coder.md")).unwrap(),
            team.as_bytes()
        );
    }
}

#[test]
fn a_teams_own_role_is_never_touched_or_listed() {
    let (base, repo) = repository("factory-install-legal");
    write(&repo, "factory/roles/legal.md", "# Legal\n");
    write(&repo, "CODEOWNERS", "/factory/roles/coder.md @owner\n");

    let out = nxs(&base, &repo, &["factory", "install", "--update"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(!stdout(&out).contains("legal"), "{}", stdout(&out));
    assert_eq!(
        fs::read_to_string(repo.join("factory/roles/legal.md")).unwrap(),
        "# Legal\n"
    );
}

#[test]
fn codeowners_names_its_file_and_the_uncovered_paths_in_the_order_of_files() {
    let (base, repo) = repository("factory-install-codeowners");
    write(
        &repo,
        ".github/CODEOWNERS",
        "/factory/roles/ @owner\n/factory/FACTORY.md @owner\n",
    );

    let out = nxs(&base, &repo, &["factory", "install"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(
        json(&out)["codeowners"],
        json!({
            "file": ".github/CODEOWNERS",
            "uncovered": ["factory/CONFIG.md", "factory/status-note.md"],
        })
    );
}

#[test]
fn markdown_lists_each_file_with_its_state_and_the_codeowners_gaps() {
    let (base, repo) = repository("factory-install-md");
    write(&repo, "factory/roles/coder.md", "# Our coder\n");
    write(
        &repo,
        "CODEOWNERS",
        "/factory/ @owner\n/factory/status-note.md\n",
    );

    let out = nxs(&base, &repo, &["factory", "install", "--md"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        "\
- `factory/CONFIG.md`: written
- `factory/FACTORY.md`: written
- `factory/roles/coder.md`: team
- `factory/roles/pm.md`: written
- `factory/status-note.md`: written

`CODEOWNERS` gives no code owner to:

- `factory/status-note.md`
"
    );
}

#[test]
fn markdown_says_when_there_is_no_codeowners_file() {
    let (base, repo) = repository("factory-install-md-none");

    let out = nxs(&base, &repo, &["factory", "install", "--md"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(
        stdout(&out).ends_with(
            "\nNo CODEOWNERS file: no code owner reviews a change to the factory's files.\n"
        ),
        "{}",
        stdout(&out)
    );
}

#[test]
fn outside_a_repository_install_says_to_run_nxs_init_and_writes_nothing() {
    let base = scratch("factory-install-no-repo");
    let repo = base.join("repo");

    let out = nxs(&base, &repo, &["factory", "install"]);

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "no_workspace");
    assert!(!repo.join("factory").exists());
}

#[test]
fn install_reads_no_data_directory_and_opens_no_database() {
    let (_base, repo) = repository("factory-install-no-data");
    let argv = ["nxs", "factory", "install"];

    let out = nxs_v2::run(argv, &repo, || {
        panic!("nxs factory install reads no options")
    });

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(repo.join("factory/roles/pm.md").is_file());
}
