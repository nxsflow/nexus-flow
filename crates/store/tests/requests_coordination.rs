//! NEX-158: creating, joining and leaving a coordination space with the repository records of
//! RFC 0009, "Repositories in the space" (and "Creating it"). Creating makes a marked coordination
//! space, writes the checkout's record and names the space in `nexus-flow.toml`; it refuses a file
//! that already names one. Joining writes the record, or revives the one whose `space` is this
//! repository's, and refuses a name another repository's record holds. Leaving sets `left` and
//! removes the id from the file; a record is never deleted.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::config::Config;
use nexus_flow_store::coordination::JoinError;
use nexus_flow_store::data_dir::DataDir;
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::requests::{self, CoordinationError, Repository};
use nexus_flow_store::space::init;
use nexus_flow_store::space_kind::SpaceKind;

/// A test's folder: the data directory, and repositories beside it.
struct Group {
    base: PathBuf,
    data: DataDir,
}

impl Group {
    fn new(test: &str) -> Group {
        let base = scratch(test);
        let data = data_dir(&base.join("data"));
        Group { base, data }
    }

    /// A repository `folder` with its space and, when `origin` is given, a git remote of that url.
    fn repo(&self, folder: &str, origin: Option<&str>) -> PathBuf {
        let repo = self.base.join(folder);
        fs::create_dir_all(&repo).unwrap();
        if let Some(url) = origin {
            git(&repo, &["init", "-q"]);
            git(&repo, &["remote", "add", "origin", url]);
        }
        block_on(init(&repo, &self.data, keys(&self.data))).unwrap();
        repo
    }

    fn open(&self, repo: &Path) -> Opened {
        block_on(opened::repository(repo, &self.data, keys(&self.data))).unwrap()
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn config(repo: &Path) -> Config {
    Config::read(&RepoRoot::find(repo).unwrap()).unwrap()
}

fn file(repo: &Path) -> String {
    fs::read_to_string(repo.join("nexus-flow.toml")).unwrap()
}

fn repository(name: &str, repo: &Path, left: bool) -> Repository {
    Repository {
        name: name.to_owned(),
        space: config(repo).repository,
        left,
    }
}

/// Creates a coordination space from `repo` and returns its id.
fn create(group: &Group, repo: &Path, name: Option<&str>) -> SpaceId {
    let mut opened = group.open(repo);
    block_on(requests::create(&mut opened, name))
        .unwrap()
        .coordination
}

#[test]
fn creating_names_the_space_writes_the_record_and_takes_the_name_from_origin() {
    let group = Group::new("coordination-create");
    let repo = group.repo("nexus-flow", Some("git@github.com:nxsflow/nexus-flow.git"));
    let mut opened = group.open(&repo);

    let created = block_on(requests::create(&mut opened, None)).unwrap();

    assert!(created.written);
    assert_eq!(
        config(&repo).coordination,
        Some(created.coordination.to_string())
    );
    assert_eq!(
        created.repository,
        Some(repository("nexus-flow", &repo, false))
    );
    assert_eq!(opened.kind(&created.coordination), Some(SpaceKind::Coordination));
    assert_eq!(
        block_on(requests::list(&opened)).unwrap(),
        [repository("nexus-flow", &repo, false)]
    );
}

#[test]
fn the_default_name_is_the_last_segment_of_origin_without_dot_git() {
    let group = Group::new("coordination-default-name");
    for (folder, url, name) in [
        ("a", "https://github.com/nxsflow/central-infra.git", "central-infra"),
        ("b", "https://github.com/nxsflow/app-foundations", "app-foundations"),
        ("c", "git@github.com:nxsflow/nexflowit-app.git", "nexflowit-app"),
        ("d", "/srv/git/manufakt-io.git/", "manufakt-io"),
    ] {
        let repo = group.repo(folder, Some(url));
        let mut opened = group.open(&repo);
        let created = block_on(requests::create(&mut opened, None)).unwrap();
        assert_eq!(created.repository.unwrap().name, name, "{url}");
    }
}

#[test]
fn without_origin_or_name_creating_is_refused_and_writes_nothing() {
    let group = Group::new("coordination-no-origin");
    let repo = group.repo("repo", None);
    let before = file(&repo);
    let mut opened = group.open(&repo);

    let refused = block_on(requests::create(&mut opened, None)).unwrap_err();

    assert!(matches!(refused, CoordinationError::NoName { .. }), "{refused:?}");
    assert_eq!(file(&repo), before);
    let created = block_on(requests::create(&mut opened, Some("own-name"))).unwrap();
    assert_eq!(created.repository.unwrap().name, "own-name");
}

#[test]
fn creating_is_refused_when_the_file_already_names_a_coordination_space() {
    let group = Group::new("coordination-create-twice");
    let repo = group.repo("repo", Some("https://example.com/repo.git"));
    let first = create(&group, &repo, None);
    let before = file(&repo);
    let mut opened = group.open(&repo);

    let refused = block_on(requests::create(&mut opened, None)).unwrap_err();

    assert!(
        matches!(
            &refused,
            CoordinationError::File(JoinError::OtherCoordination { recorded, .. })
                if *recorded == first.to_string()
        ),
        "{refused:?}"
    );
    assert_eq!(file(&repo), before);
}

#[test]
fn joining_writes_the_record_of_the_joining_repository() {
    let group = Group::new("coordination-join");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app-foundations.git"));
    let space = create(&group, &first, None);
    let mut opened = group.open(&second);

    let joined = block_on(requests::join(&mut opened, space, None)).unwrap();

    assert!(joined.written);
    assert_eq!(config(&second).coordination, Some(space.to_string()));
    assert_eq!(
        joined.repository,
        Some(repository("app-foundations", &second, false))
    );
    assert_eq!(
        block_on(requests::list(&opened)).unwrap(),
        [
            repository("app-foundations", &second, false),
            repository("central-infra", &first, false),
        ]
    );
}

#[test]
fn joining_again_writes_nothing_and_a_new_name_renames_the_record() {
    let group = Group::new("coordination-join-again");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app.git"));
    let space = create(&group, &first, None);
    let mut opened = group.open(&second);
    block_on(requests::join(&mut opened, space, None)).unwrap();
    let before = file(&second);

    let again = block_on(requests::join(&mut opened, space, None)).unwrap();
    let renamed = block_on(requests::join(&mut opened, space, Some("app-foundations"))).unwrap();

    assert!(!again.written);
    assert_eq!(again.repository, Some(repository("app", &second, false)));
    assert!(renamed.written);
    assert_eq!(
        renamed.repository,
        Some(repository("app-foundations", &second, false))
    );
    assert_eq!(file(&second), before);
    assert_eq!(block_on(requests::list(&opened)).unwrap().len(), 2);
}

#[test]
fn a_name_another_repository_holds_is_refused_and_nothing_is_written() {
    let group = Group::new("coordination-join-taken");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app.git"));
    let space = create(&group, &first, None);
    let before = file(&second);
    let mut opened = group.open(&second);

    let refused =
        block_on(requests::join(&mut opened, space, Some("central-infra"))).unwrap_err();

    match refused {
        CoordinationError::NameTaken { name, holders } => {
            assert_eq!(name, "central-infra");
            assert_eq!(holders, [repository("central-infra", &first, false)]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(file(&second), before);
    let opened = group.open(&first);
    assert_eq!(block_on(requests::list(&opened)).unwrap().len(), 1);
}

#[test]
fn a_space_the_device_does_not_hold_is_refused_and_nothing_is_written() {
    let group = Group::new("coordination-join-unheld");
    let repo = group.repo("repo", Some("https://example.com/repo.git"));
    let before = file(&repo);
    let mut opened = group.open(&repo);
    let unheld: SpaceId = "00112233445566778899aabbccddeeff".parse().unwrap();

    let refused = block_on(requests::join(&mut opened, unheld, None)).unwrap_err();

    assert!(
        matches!(refused, CoordinationError::NotHeld { space } if space == unheld),
        "{refused:?}"
    );
    assert_eq!(file(&repo), before);
}

#[test]
fn leaving_sets_left_keeps_the_record_and_removes_the_id_from_the_file() {
    let group = Group::new("coordination-leave");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app.git"));
    let space = create(&group, &first, None);
    let mut opened = group.open(&second);
    let before = file(&second);
    block_on(requests::join(&mut opened, space, None)).unwrap();

    let left = block_on(requests::leave(&mut opened)).unwrap();

    assert_eq!(left.coordination, space);
    assert!(left.written);
    assert_eq!(left.repository, Some(repository("app", &second, true)));
    assert_eq!(config(&second).coordination, None);
    assert_eq!(file(&second), before, "the file is as it was before the join");
    let opened = group.open(&first);
    assert_eq!(
        block_on(requests::list(&opened)).unwrap(),
        [
            repository("app", &second, true),
            repository("central-infra", &first, false),
        ]
    );
}

#[test]
fn joining_again_after_leaving_clears_left_on_the_same_record() {
    let group = Group::new("coordination-rejoin");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app.git"));
    let space = create(&group, &first, None);
    let mut opened = group.open(&second);
    block_on(requests::join(&mut opened, space, None)).unwrap();
    block_on(requests::leave(&mut opened)).unwrap();

    let rejoined = block_on(requests::join(&mut opened, space, None)).unwrap();

    assert_eq!(rejoined.repository, Some(repository("app", &second, false)));
    assert_eq!(block_on(requests::list(&opened)).unwrap().len(), 2);
}

#[test]
fn leaving_a_repository_in_no_coordination_space_is_refused() {
    let group = Group::new("coordination-leave-none");
    let repo = group.repo("repo", None);
    let mut opened = group.open(&repo);

    let refused = block_on(requests::leave(&mut opened)).unwrap_err();

    assert!(matches!(refused, CoordinationError::NotJoined { .. }), "{refused:?}");
    let listed = block_on(requests::list(&opened)).unwrap_err();
    assert!(matches!(listed, CoordinationError::NotJoined { .. }), "{listed:?}");
}

#[test]
fn a_name_two_records_hold_is_ambiguous_and_names_both() {
    let group = Group::new("coordination-ambiguous");
    let first = group.repo("first", Some("https://example.com/central-infra.git"));
    let second = group.repo("second", Some("https://example.com/app.git"));
    let space = create(&group, &first, None);
    let mut opened = group.open(&second);
    block_on(requests::join(&mut opened, space, None)).unwrap();
    assert_eq!(
        block_on(requests::named(&opened, "app")).unwrap(),
        repository("app", &second, false)
    );
    // Two repositories that join offline at once can end up with one name (RFC 0009,
    // "Joining"); the store's own check refuses it, so the test renames the record directly.
    let own = config(&second).repository;
    let record = block_on(opened.find(&space, "requestsRepository", "bySpace", vec![own.into()]))
        .unwrap()[0]
        .id();
    block_on(opened.write(&space, |w| {
        w.update("requestsRepository", record, [("name", "central-infra".into())])
    }))
    .unwrap();

    let missing = block_on(requests::named(&opened, "app")).unwrap_err();
    assert!(
        matches!(&missing, CoordinationError::NotFound { name } if name == "app"),
        "{missing:?}"
    );
    match block_on(requests::named(&opened, "central-infra")).unwrap_err() {
        CoordinationError::Ambiguous { name, matching } => {
            assert_eq!(name, "central-infra");
            assert_eq!(matching.len(), 2);
        }
        other => panic!("{other:?}"),
    }
}
