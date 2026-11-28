//! `nxs init` (RFC 0001, "Creating and finding a space"): in a fresh git repository it writes
//! `nexus-flow.toml` and prints the space id as JSON; run again it changes nothing. It ends with
//! `nxs factory install` and then `nxs genesis create` (RFC 0006, RFC 0004): a GENESIS git still
//! tracks is not written, and `nxs init` exits non-zero with its record on stdout and the error on
//! stderr.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{json, nxs, scratch, stdout};

#[test]
fn init_in_a_fresh_git_repo_writes_the_file_and_prints_the_space_id() {
    let base = scratch("init");
    let repo = base.join("repo");
    let git = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(&repo)
        .status();
    assert!(git.unwrap().success(), "git init");

    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    let report = json(&out);
    let space = report["space"].as_str().expect("space is a string");
    assert_eq!(space.len(), 32, "{space}");
    assert_eq!(report["created"], true);
    let file = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    assert!(
        file.contains(&format!("repository = \"{space}\"")),
        "{file}"
    );
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

/// Runs git in `repo` with an identity of its own, and fails the test when it fails.
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

const GENESIS: &str = "factory/GENESIS.md";

#[test]
fn init_installs_the_factory_and_genesis() {
    let base = scratch("init-installs");
    let repo = base.join("repo");

    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(repo.join("factory/FACTORY.md").is_file());
    assert!(repo.join("factory/roles/coder.md").is_file());
    assert!(repo.join(GENESIS).is_file());
    assert!(repo.join("factory/genesis-guide.md").is_file());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_with_a_tracked_genesis_writes_nothing_there_and_exits_non_zero() {
    for md in [false, true] {
        let base = scratch(&format!("init-tracked-{md}"));
        let repo = base.join("repo");
        git(&repo, &["init", "-q"]);
        fs::create_dir_all(repo.join("factory")).unwrap();
        fs::write(repo.join(GENESIS), "ours").unwrap();
        git(&repo, &["add", GENESIS]);
        git(&repo, &["commit", "-q", "-m", "genesis"]);
        fs::remove_file(repo.join(GENESIS)).unwrap();

        let args: &[&str] = if md { &["init", "--md"] } else { &["init"] };
        let out = nxs(&base, &repo, args);

        assert_eq!(out.code, 1, "{}", stdout(&out));
        assert!(!repo.join(GENESIS).exists());
        let file = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
        if md {
            assert!(
                stdout(&out).contains("created the repository space"),
                "{}",
                stdout(&out)
            );
        } else {
            let space = json(&out)["space"].as_str().unwrap().to_owned();
            assert!(file.contains(&space), "{file}");
        }
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.starts_with("error: "), "{stderr}");
        assert!(stderr.contains(GENESIS), "{stderr}");
        assert!(stderr.contains("git"), "{stderr}");
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn after_init_member_list_shows_the_device_as_owner_with_a_tag_and_a_rerun_keeps_it() {
    let base = scratch("init-owner");
    let repo = base.join("repo");
    let out = nxs(&base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    let first = json(&nxs(&base, &repo, &["member", "list"]));

    let rerun = nxs(&base, &repo, &["init"]);

    assert_eq!(rerun.code, 0, "{}", stdout(&rerun));
    let list = json(&nxs(&base, &repo, &["member", "list"]));
    assert_eq!(list, first);
    let list = list.as_array().expect("member list prints an array");
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["kind"], "device");
    assert_eq!(list[0]["role"], "owner");
    let tag = list[0]["tag"].as_str().expect("the device has a tag");
    assert_eq!(tag.len(), 2, "{tag}");
    assert_eq!(list[0]["tag_shared"], false);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_prints_the_factory_and_genesis_reports_as_their_commands_print_them() {
    let base = scratch("init-reports");
    let repo = base.join("repo");
    let report = json(&nxs(&base, &repo, &["init"]));
    // A second repository, after the same init, has the files removed again, so the two
    // commands write them afresh and print what `nxs init` wrote.
    let other = scratch("init-reports-other");
    let again = other.join("repo");
    assert_eq!(nxs(&other, &again, &["init"]).code, 0);
    fs::remove_dir_all(again.join("factory")).unwrap();

    let factory = json(&nxs(&other, &again, &["factory", "install"]));
    let genesis = json(&nxs(&other, &again, &["genesis", "create"]));

    assert_eq!(report["factory"], factory);
    assert_eq!(report["genesis"], genesis);
    assert_eq!(report["genesis"]["files"][0]["state"], "written");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(&other).unwrap();
}

#[test]
fn init_with_md_names_the_paths_codeowners_leaves_uncovered_and_a_rerun_wrote_nothing() {
    let base = scratch("init-md-codeowners");
    let repo = base.join("repo");
    fs::write(repo.join("CODEOWNERS"), "/factory/roles/ @owner\n").unwrap();

    let out = nxs(&base, &repo, &["init", "--md"]);
    let rerun = nxs(&base, &repo, &["init", "--md"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let text = stdout(&out);
    assert!(
        text.contains(
            "- `nxs factory install` wrote `factory/CONFIG.md`, `factory/FACTORY.md`, `factory/roles/coder.md`, `factory/roles/pm.md`, `factory/status-note.md`; commit them; `CODEOWNERS` gives no code owner to `factory/CONFIG.md`, `factory/FACTORY.md`, `factory/status-note.md`\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "- `nxs genesis create` wrote `factory/GENESIS.md`, `factory/genesis-guide.md`; fill in GENESIS and commit them; no rule in `CODEOWNERS` covers `factory/GENESIS.md`; add one so that its changes come back to the owner\n"
        ),
        "{text}"
    );
    let text = stdout(&rerun);
    assert!(
        text.contains("- `nxs factory install` wrote nothing; `CODEOWNERS` gives no code owner to"),
        "{text}"
    );
    assert!(
        text.contains("- `nxs genesis create` wrote nothing; no rule in `CODEOWNERS` covers"),
        "{text}"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_with_md_says_a_tracked_genesis_was_not_written() {
    let base = scratch("init-md-tracked");
    let repo = base.join("repo");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), "ours").unwrap();
    git(&repo, &["add", GENESIS]);
    git(&repo, &["commit", "-q", "-m", "genesis"]);
    fs::remove_file(repo.join(GENESIS)).unwrap();

    let out = nxs(&base, &repo, &["init", "--md"]);

    let text = stdout(&out);
    assert!(
        text.contains(
            "- `nxs genesis create` wrote `factory/genesis-guide.md`; `factory/GENESIS.md` is missing here but tracked in git, so nothing was written there; restore it from git; "
        ),
        "{text}"
    );
    fs::remove_dir_all(&base).unwrap();
}
