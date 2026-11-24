//! `Engine::genesis_create` writes `factory/GENESIS.md` from the template only where the path is
//! free (RFC 0004, "Installing the template", "What the build must prove"): a team's GENESIS, with
//! any bytes, is never replaced; one git still tracks is reported `tracked` and not rewritten; a
//! symbolic link at the path is not followed; a case variant blocks the write. Nothing is
//! committed, and no database is opened.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nexus_flow_seam::genesis::{
    FileState, GenesisError, GenesisState, Report, OWNER_RULES_MARK, TEMPLATE,
};
use nexus_flow_seam::{Engine, RootError};

const GENESIS: &str = "factory/GENESIS.md";

/// A fresh folder for one test, holding `repo/` with a `nexus-flow.toml` whose base branch is
/// `base`.
fn repository(test: &str, base: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-seam-genesis-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("nexus-flow.toml"),
        format!(
            "format = 1\n\n[space]\nrepository = \"{}\"\n\n[git]\nbase = \"{base}\"\n",
            "0".repeat(32)
        ),
    )
    .unwrap();
    repo.canonicalize().unwrap()
}

/// Runs git in `repo`, with an identity of its own, and fails the test when it fails.
fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// GENESIS's entry of a report, as `(path, state, found)`; the report holds it exactly once.
fn only_file(report: &Report) -> (&str, GenesisState, Option<&str>) {
    let mut genesis = report.files.iter().filter(|f| f.path == GENESIS);
    let file = genesis.next().expect("the report names GENESIS");
    assert!(genesis.next().is_none(), "{report:?}");
    let FileState::Genesis(state) = file.state else {
        panic!("GENESIS has a state of its own: {report:?}");
    };
    (file.path.as_str(), state, file.found.as_deref())
}

fn cleanup(repo: &Path) {
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}

// --- written -------------------------------------------------------------------------------------

#[test]
fn a_missing_untracked_genesis_is_written_from_the_template() {
    let repo = repository("written", "main");
    git(&repo, &["init", "-q"]);

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report), (GENESIS, GenesisState::Written, None));
    assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), TEMPLATE.as_bytes());
    cleanup(&repo);
}

#[test]
fn a_genesis_is_written_from_a_subfolder_at_the_repository_root() {
    let repo = repository("subfolder", "main");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("src/deep")).unwrap();

    let report = Engine::genesis_create(&repo.join("src/deep"), false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Written);
    assert!(repo.join(GENESIS).is_file());
    assert!(!repo.join("src/deep/factory").exists());
    cleanup(&repo);
}

#[test]
fn a_genesis_is_written_outside_a_git_repository() {
    let repo = repository("no-git", "main");

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Written);
    assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), TEMPLATE.as_bytes());
    cleanup(&repo);
}

#[test]
fn writing_a_genesis_commits_nothing_and_stages_nothing() {
    let repo = repository("no-commit", "main");
    git(&repo, &["init", "-q"]);

    Engine::genesis_create(&repo, false).unwrap();

    let status = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["status", "--porcelain", "--", "factory"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "?? factory/\n");
    cleanup(&repo);
}

#[test]
fn the_template_carries_the_owner_rules_mark_with_a_list_under_it() {
    let mut lines = TEMPLATE
        .lines()
        .skip_while(|l| l.trim() != OWNER_RULES_MARK);
    assert_eq!(lines.next().map(str::trim), Some(OWNER_RULES_MARK));
    let first = lines.find(|l| !l.trim().is_empty()).unwrap_or_default();
    assert!(first.starts_with("- "), "{first:?}");
    assert_eq!(
        TEMPLATE
            .lines()
            .filter(|l| l.trim() == OWNER_RULES_MARK)
            .count(),
        1
    );
}

// --- team ----------------------------------------------------------------------------------------

/// Bytes no template holds: invalid UTF-8, CRLF and a NUL.
const ARBITRARY: &[u8] = b"# Our product\r\n\xff\xfe not utf-8 \x00 and more\r\n";

#[test]
fn a_genesis_with_arbitrary_bytes_is_unchanged_after_create_and_update() {
    let repo = repository("team-bytes", "main");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), ARBITRARY).unwrap();

    for update in [false, true] {
        let report = Engine::genesis_create(&repo, update).unwrap();
        assert_eq!(only_file(&report), (GENESIS, GenesisState::Team, None));
        assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), ARBITRARY);
    }
    cleanup(&repo);
}

#[test]
fn a_genesis_that_reads_exactly_like_the_template_is_the_teams() {
    let repo = repository("team-template", "main");
    git(&repo, &["init", "-q"]);
    Engine::genesis_create(&repo, false).unwrap();

    for update in [false, true] {
        let report = Engine::genesis_create(&repo, update).unwrap();
        assert_eq!(only_file(&report).1, GenesisState::Team);
        assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), TEMPLATE.as_bytes());
    }
    cleanup(&repo);
}

#[test]
fn an_empty_genesis_is_the_teams() {
    let repo = repository("team-empty", "main");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), b"").unwrap();

    let report = Engine::genesis_create(&repo, true).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Team);
    assert_eq!(fs::read(repo.join(GENESIS)).unwrap(), b"");
    cleanup(&repo);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_at_the_path_is_not_followed() {
    let repo = repository("symlink", "main");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("factory")).unwrap();
    let target = repo.join("elsewhere/target.md");
    std::os::unix::fs::symlink(&target, repo.join(GENESIS)).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Team);
    assert!(!target.exists(), "the link's target was written");
    assert!(!repo.join("elsewhere").exists());
    let link = fs::symlink_metadata(repo.join(GENESIS)).unwrap();
    assert!(link.file_type().is_symlink());
    assert_eq!(fs::read_link(repo.join(GENESIS)).unwrap(), target);
    cleanup(&repo);
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_is_not_written_through() {
    let repo = repository("symlink-file", "main");
    fs::create_dir_all(repo.join("factory")).unwrap();
    let target = repo.join("product.md");
    fs::write(&target, ARBITRARY).unwrap();
    std::os::unix::fs::symlink(&target, repo.join(GENESIS)).unwrap();

    let report = Engine::genesis_create(&repo, true).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Team);
    assert_eq!(fs::read(&target).unwrap(), ARBITRARY);
    cleanup(&repo);
}

#[test]
fn a_lowercase_genesis_blocks_the_write_and_is_named() {
    let repo = repository("case", "main");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join("factory/genesis.md"), ARBITRARY).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(
        only_file(&report),
        (GENESIS, GenesisState::Team, Some("factory/genesis.md"))
    );
    let names: Vec<_> = fs::read_dir(repo.join("factory"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["genesis.md"]);
    assert_eq!(
        fs::read(repo.join("factory/genesis.md")).unwrap(),
        ARBITRARY
    );
    cleanup(&repo);
}

// --- tracked -------------------------------------------------------------------------------------

/// Commits a GENESIS of `ARBITRARY` bytes on the current branch.
fn commit_genesis(repo: &Path) {
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), ARBITRARY).unwrap();
    git(repo, &["add", GENESIS]);
    git(repo, &["commit", "-q", "-m", "genesis"]);
}

#[test]
fn a_genesis_deleted_but_tracked_in_head_is_reported_and_not_written() {
    let repo = repository("tracked-head", "main");
    git(&repo, &["init", "-q"]);
    commit_genesis(&repo);
    git(&repo, &["rm", "-q", "--cached", GENESIS]);
    fs::remove_file(repo.join(GENESIS)).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report), (GENESIS, GenesisState::Tracked, None));
    assert!(report.is_refused());
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn a_genesis_deleted_but_still_in_the_index_is_reported_and_not_written() {
    let repo = repository("tracked-index", "main");
    git(&repo, &["init", "-q"]);
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(repo.join(GENESIS), ARBITRARY).unwrap();
    git(&repo, &["add", GENESIS]);
    fs::remove_file(repo.join(GENESIS)).unwrap();

    let report = Engine::genesis_create(&repo, true).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Tracked);
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn a_genesis_only_on_the_base_branch_is_reported_and_not_written() {
    let repo = repository("tracked-base", "trunk");
    git(&repo, &["init", "-q", "-b", "trunk"]);
    commit_genesis(&repo);
    git(&repo, &["checkout", "-q", "--orphan", "feature"]);
    git(&repo, &["rm", "-q", "-r", "--cached", "."]);
    fs::remove_dir_all(repo.join("factory")).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Tracked);
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn a_genesis_only_on_the_remote_base_branch_is_reported_and_not_written() {
    let repo = repository("tracked-remote", "trunk");
    git(&repo, &["init", "-q", "-b", "trunk"]);
    commit_genesis(&repo);
    git(&repo, &["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
    git(&repo, &["checkout", "-q", "--orphan", "feature"]);
    git(&repo, &["branch", "-q", "-D", "trunk"]);
    git(&repo, &["rm", "-q", "-r", "--cached", "."]);
    fs::remove_dir_all(repo.join("factory")).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Tracked);
    assert!(!repo.join(GENESIS).exists());
    cleanup(&repo);
}

#[test]
fn a_genesis_on_another_branch_than_the_base_does_not_block_the_write() {
    let repo = repository("other-branch", "main");
    git(&repo, &["init", "-q", "-b", "elsewhere"]);
    commit_genesis(&repo);
    git(&repo, &["checkout", "-q", "--orphan", "feature"]);
    git(&repo, &["rm", "-q", "-r", "--cached", "."]);
    fs::remove_dir_all(repo.join("factory")).unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(only_file(&report).1, GenesisState::Written);
    cleanup(&repo);
}

// --- no repository -------------------------------------------------------------------------------

#[test]
fn without_nexus_flow_toml_create_fails_and_writes_nothing() {
    let dir = std::env::temp_dir().join(format!("nxs-seam-genesis-no-toml-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();

    let err = Engine::genesis_create(&dir, false).unwrap_err();

    assert!(
        matches!(err, GenesisError::Root(RootError::NotFound { .. })),
        "{err:?}"
    );
    assert!(err.to_string().contains("nxs init"), "{err}");
    assert!(!dir.join("factory").exists());
    fs::remove_dir_all(&dir).unwrap();
}
