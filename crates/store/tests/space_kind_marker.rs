//! A space's kind is a marker record written when the space is created (RFC 0001, "The model",
//! NEX-205): one `spaceKind` object naming the kind. The store reads it for every open space, so
//! a space opened by id alone, without a checkout that names it, has its kind too. A space
//! without a marker, created before the marker existed, keeps the kind `nexus-flow.toml` gives
//! it, or none.

mod common;

use std::fs;
use std::path::Path;

use common::{block_on, data_dir, keys, scratch, unmarked_space};
use nexus_flow_store::config::Config;
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened;
use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::space::{self, init};
use nexus_flow_store::space_kind::SpaceKind;
use nexus_flow_store::write::WriteError;
use nocentraldb::Value;

/// Names `coordination` as the coordination space in the file at `repo`.
fn name_coordination(repo: &Path, coordination: SpaceId) {
    let mut config = Config::read(&RepoRoot::find(repo).unwrap()).unwrap();
    config.coordination = Some(coordination.to_string());
    fs::remove_file(repo.join("nexus-flow.toml")).unwrap();
    config.write_new(repo).unwrap();
}

#[test]
fn the_space_nxs_init_creates_is_a_repository_space_when_opened_by_id_alone() {
    let base = scratch("marker-init");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    fs::remove_dir_all(base.join("repo")).unwrap();

    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&space), Some(SpaceKind::Repository));
    assert_eq!(opened.check_write(&space, "decisionsDecision"), Ok(()));
    let err = opened.check_write(&space, "goalsGoal").unwrap_err();
    assert_eq!(err.space, Some(SpaceKind::Repository));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_created_space_of_each_kind_has_that_kind_when_opened_by_id_alone() {
    let base = scratch("marker-create");
    let data = data_dir(&base.join("data"));

    for kind in SpaceKind::ALL {
        let space = block_on(space::create(&data, keys(&data), kind)).unwrap();

        let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

        assert_eq!(opened.kind(&space), Some(kind));
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_marker_names_the_kind_when_nexus_flow_toml_names_another() {
    let base = scratch("marker-over-toml");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    block_on(init(&repo, &data, keys(&data))).unwrap();
    let other = block_on(space::create(&data, keys(&data), SpaceKind::Repository)).unwrap();
    name_coordination(&repo, other);

    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();

    assert_eq!(opened.kind(&other), Some(SpaceKind::Repository));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_without_a_marker_keeps_the_kind_nexus_flow_toml_gives_it() {
    let base = scratch("marker-none");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    block_on(init(&repo, &data, keys(&data))).unwrap();
    let unmarked = unmarked_space(&data);
    name_coordination(&repo, unmarked);

    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();
    assert_eq!(opened.kind(&unmarked), Some(SpaceKind::Coordination));

    let opened = block_on(opened::space(unmarked, None, &data, keys(&data))).unwrap();
    assert_eq!(opened.kind(&unmarked), None);
    assert!(opened.check_write(&unmarked, "spaceMember").is_err());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_marker_takes_no_write_after_the_space_is_created() {
    let base = scratch("marker-no-write");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&repo, &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();
    let marker = block_on(opened.find(&space, "spaceKind", "byKind", Vec::new())).unwrap()[0].id();

    let created = block_on(opened.write(&space, |w| {
        w.create("spaceKind", [("kind", Value::from("coordination"))])
            .map(drop)
    }));
    let set = block_on(opened.write(&space, |w| {
        w.update("spaceKind", marker, [("kind", Value::from("coordination"))])
    }));
    let deleted = block_on(opened.write(&space, |w| w.delete("spaceKind", marker)));

    for result in [created, set, deleted] {
        let err = result.unwrap_err();
        assert!(
            matches!(err, WriteError::Marker(ref e) if e.collection == "spaceKind"),
            "{err:?}"
        );
    }
    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();
    assert_eq!(opened.kind(&space), Some(SpaceKind::Repository));
    fs::remove_dir_all(&base).unwrap();
}
