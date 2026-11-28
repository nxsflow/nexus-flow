//! NEX-158: the seam's counterparts of `nxs space create-coordination`, `join-coordination`,
//! `leave-coordination` and `nxs repository list` (RFC 0009, "Public surfaces this RFC brings",
//! library seam): `Engine::coordination_create`, `coordination_join`, `coordination_leave` and
//! `repositories`, on an engine open on a checkout. They replace NEX-104's
//! `Engine::space_join_coordination`, which opened no database: the join now writes the
//! repository record into the coordination space.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, options, scratch};
use nexus_flow_seam::{init_with, CoordinationError, Engine, Error, JoinError, Repository};

fn repo(base: &Path, folder: &str) -> PathBuf {
    let repo = base.join(folder);
    fs::create_dir_all(&repo).unwrap();
    block_on(init_with(&repo, options(&base.join("data")))).unwrap();
    repo
}

fn open(base: &Path, repo: &Path) -> Engine {
    block_on(Engine::open_with(repo, options(&base.join("data")))).unwrap()
}

fn space_of(repo: &Path) -> String {
    let text = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    let line = text
        .lines()
        .find(|line| line.starts_with("repository"))
        .unwrap();
    line.split('"').nth(1).unwrap().to_owned()
}

#[test]
fn an_engine_creates_joins_lists_and_leaves_a_coordination_space() {
    let base = scratch("engine-coordination");
    let first = repo(&base, "first");
    let second = repo(&base, "second");

    let mut creator = open(&base, &first);
    let created = block_on(creator.coordination_create(Some("central-infra"))).unwrap();
    let refused = block_on(creator.coordination_create(Some("again"))).unwrap_err();
    let mut joiner = open(&base, &second);
    let joined = block_on(joiner.coordination_join(created.coordination, Some("app"))).unwrap();
    let listed = block_on(joiner.repositories()).unwrap();
    let left = block_on(joiner.coordination_leave()).unwrap();
    let after = block_on(open(&base, &first).repositories()).unwrap();

    assert!(created.written && joined.written && left.written);
    assert!(
        matches!(
            refused,
            Error::Coordination(CoordinationError::File(JoinError::OtherCoordination { .. }))
        ),
        "{refused:?}"
    );
    let record = |name: &str, repo: &Path, left: bool| Repository {
        name: name.to_owned(),
        space: space_of(repo),
        left,
    };
    assert_eq!(
        listed,
        [
            record("app", &second, false),
            record("central-infra", &first, false)
        ]
    );
    assert_eq!(left.repository, Some(record("app", &second, true)));
    assert_eq!(
        after,
        [
            record("app", &second, true),
            record("central-infra", &first, false)
        ]
    );
    fs::remove_dir_all(&base).unwrap();
}
