//! The store knows a space's kind from `nexus-flow.toml` (NEX-95, until the open question of how
//! it knows the kind of a space opened by id alone is answered): `[space] repository` is the
//! repository space, `[space] coordination` the coordination space. A space opened by id with
//! no checkout that names it is of no known kind, and takes no kind-restricted write.

mod common;

use std::fs;
use std::path::Path;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::config::Config;
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened;
use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::space::init;
use nexus_flow_store::space_kind::SpaceKind;

/// A coordination space id no database of these tests holds.
const COORDINATION: &str = "00112233445566778899aabbccddeeff";

/// Names `COORDINATION` as the coordination space in the file at `repo`.
fn join_coordination(repo: &Path) {
    let mut config = Config::read(&RepoRoot::find(repo).unwrap()).unwrap();
    config.coordination = Some(COORDINATION.to_owned());
    fs::remove_file(repo.join("nexus-flow.toml")).unwrap();
    config.write_new(repo).unwrap();
}

#[test]
fn a_repository_names_the_kind_of_each_of_its_spaces() {
    let base = scratch("kind-repository");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&repo, &data, keys(&data))).unwrap();
    join_coordination(&repo);
    let repository: SpaceId = report.space.parse().unwrap();
    let coordination: SpaceId = COORDINATION.parse().unwrap();

    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&repository), Some(SpaceKind::Repository));
    assert_eq!(opened.kind(&coordination), Some(SpaceKind::Coordination));
    assert_eq!(opened.check_write(&repository, "spaceMember"), Ok(()));
    assert_eq!(opened.check_write(&coordination, "spaceMember"), Ok(()));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_opened_by_id_without_a_checkout_takes_no_kind_restricted_write() {
    let base = scratch("kind-no-git");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();

    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&space), None);
    let err = opened.check_write(&space, "spaceMember").unwrap_err();
    assert_eq!(err.space, None);
    assert_eq!(err.code(), None);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_opened_by_id_takes_its_kind_from_the_checkout_that_names_it() {
    let base = scratch("kind-git");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&repo, &data, keys(&data))).unwrap();
    join_coordination(&repo);
    let repository: SpaceId = report.space.parse().unwrap();
    let coordination: SpaceId = COORDINATION.parse().unwrap();

    let opened = block_on(opened::space(repository, Some(&repo), &data, keys(&data))).unwrap();
    assert_eq!(opened.kind(&repository), Some(SpaceKind::Repository));

    let opened = block_on(opened::space(coordination, Some(&repo), &data, keys(&data))).unwrap();
    assert_eq!(opened.kind(&coordination), Some(SpaceKind::Coordination));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_the_checkout_does_not_name_is_of_no_known_kind() {
    let base = scratch("kind-git-other");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    block_on(init(&repo, &data, keys(&data))).unwrap();
    let other: SpaceId = COORDINATION.parse().unwrap();

    let opened = block_on(opened::space(other, Some(&repo), &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&other), None);
    assert!(opened.check_write(&other, "spaceMember").is_err());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_that_is_not_open_takes_no_write() {
    let base = scratch("kind-not-open");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    block_on(init(&repo, &data, keys(&data))).unwrap();
    let other: SpaceId = COORDINATION.parse().unwrap();

    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&other), None);
    assert!(opened.check_write(&other, "spaceMember").is_err());
    fs::remove_dir_all(&base).unwrap();
}
