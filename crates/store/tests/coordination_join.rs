//! NEX-104: `coordination::join` records a coordination space in `nexus-flow.toml` (RFC 0001,
//! "Creating and finding a space"; "Spaces": a repository belongs to at most one coordination
//! space). It writes `[space] coordination` into a file that names none, leaving the rest of the
//! file as the team wrote it; joining the same space again writes nothing; and a file that already
//! names another coordination space, or that nxs cannot add the line to, is refused and left byte
//! for byte as it was.

mod common;

use std::fs;
use std::path::Path;

use common::scratch;
use nexus_flow_store::config::Config;
use nexus_flow_store::coordination::{join, JoinError};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::repo_root::RepoRoot;

const REPOSITORY: &str = "0123456789abcdef0123456789abcdef";
const COORDINATION: &str = "00112233445566778899aabbccddeeff";
const OTHER: &str = "ffeeddccbbaa99887766554433221100";

/// A file as a team may have written it: comments, and a base branch of its own.
const FILE: &str = "# nexus-flow.toml, committed. Holds no secret: a space id grants nothing.
format = 1

[space]
repository = \"0123456789abcdef0123456789abcdef\"   # the repository space

# The team's base branch.
[git]
base = \"v2\"
";

fn id(text: &str) -> SpaceId {
    text.parse().unwrap()
}

fn read(repo: &Path) -> Config {
    Config::read(&RepoRoot::find(repo).unwrap()).unwrap()
}

#[test]
fn joining_writes_the_id_and_keeps_the_rest_of_the_file() {
    let base = scratch("join-writes");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();

    let report = join(&repo, id(COORDINATION)).unwrap();

    assert!(report.written);
    assert_eq!(report.coordination, id(COORDINATION));
    let config = read(&repo);
    assert_eq!(config.coordination.as_deref(), Some(COORDINATION));
    assert_eq!(config.repository, REPOSITORY);
    assert_eq!(config.base, "v2");
    let text = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    let expected = FILE.replace(
        "# the repository space\n",
        &format!("# the repository space\ncoordination = \"{COORDINATION}\"\n"),
    );
    assert_eq!(text, expected);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_from_a_subfolder_writes_the_root_file() {
    let base = scratch("join-nested");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();

    join(&nested, id(COORDINATION)).unwrap();

    assert_eq!(read(&repo).coordination.as_deref(), Some(COORDINATION));
    assert!(!nested.join("nexus-flow.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_again_with_the_same_id_writes_nothing() {
    let base = scratch("join-again");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();
    join(&repo, id(COORDINATION)).unwrap();
    let before = fs::read(repo.join("nexus-flow.toml")).unwrap();

    let report = join(&repo, id(COORDINATION)).unwrap();

    assert!(!report.written);
    assert_eq!(report.coordination, id(COORDINATION));
    assert_eq!(fs::read(repo.join("nexus-flow.toml")).unwrap(), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_a_second_coordination_space_is_refused() {
    let base = scratch("join-other");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();
    join(&repo, id(COORDINATION)).unwrap();
    let before = fs::read(repo.join("nexus-flow.toml")).unwrap();

    let err = join(&repo, id(OTHER)).unwrap_err();

    match &err {
        JoinError::OtherCoordination { recorded, .. } => assert_eq!(recorded, COORDINATION),
        other => panic!("expected OtherCoordination, got {other:?}"),
    }
    assert!(err.to_string().contains(COORDINATION), "{err}");
    assert_eq!(fs::read(repo.join("nexus-flow.toml")).unwrap(), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_the_repository_space_as_coordination_is_refused() {
    let base = scratch("join-own");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();

    let err = join(&repo, id(REPOSITORY)).unwrap_err();

    assert!(matches!(err, JoinError::OwnSpace { .. }), "{err:?}");
    assert_eq!(
        fs::read_to_string(repo.join("nexus-flow.toml")).unwrap(),
        FILE
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_outside_a_repository_is_refused() {
    let base = scratch("join-none");
    let repo = base.join("repo");

    let err = join(&repo, id(COORDINATION)).unwrap_err();

    assert!(matches!(err, JoinError::Root(_)), "{err:?}");
    assert!(!repo.join("nexus-flow.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_layout_without_a_space_header_is_refused_and_left_as_it_was() {
    let base = scratch("join-inline");
    let repo = base.join("repo");
    let file = format!("format = 1\nspace = {{ repository = \"{REPOSITORY}\" }}\n");
    fs::write(repo.join("nexus-flow.toml"), &file).unwrap();

    let err = join(&repo, id(COORDINATION)).unwrap_err();

    assert!(matches!(err, JoinError::CannotPlace { .. }), "{err:?}");
    assert!(err.to_string().contains(COORDINATION), "{err}");
    assert_eq!(
        fs::read_to_string(repo.join("nexus-flow.toml")).unwrap(),
        file
    );
    fs::remove_dir_all(&base).unwrap();
}
