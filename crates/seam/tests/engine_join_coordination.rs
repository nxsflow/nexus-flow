//! NEX-104: `Engine::space_join_coordination`, the seam's counterpart of `nxs space
//! join-coordination <space id>` (RFC 0001, "Creating and finding a space"). Like the other seam
//! functions of commands that open no database (NEX-232), it takes the repository, not an open
//! engine, and writes only `nexus-flow.toml`.

mod common;

use std::fs;

use common::scratch;
use nexus_flow_seam::{Engine, Error, JoinError, SpaceId};

const FILE: &str = "format = 1\n\n[space]\nrepository = \"0123456789abcdef0123456789abcdef\"\n";
const COORDINATION: &str = "00112233445566778899aabbccddeeff";
const OTHER: &str = "ffeeddccbbaa99887766554433221100";

fn id(text: &str) -> SpaceId {
    text.parse().unwrap()
}

#[test]
fn joining_records_the_space_once_and_refuses_a_second() {
    let base = scratch("join-coordination");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();

    let first = Engine::space_join_coordination(&repo, id(COORDINATION)).unwrap();
    let again = Engine::space_join_coordination(&repo, id(COORDINATION)).unwrap();
    let other = Engine::space_join_coordination(&repo, id(OTHER)).unwrap_err();

    assert!(first.written);
    assert!(!again.written);
    assert_eq!(again.coordination, id(COORDINATION));
    assert!(
        matches!(
            other,
            Error::Coordination(JoinError::OtherCoordination { .. })
        ),
        "{other:?}"
    );
    let text = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    assert_eq!(text, format!("{FILE}coordination = \"{COORDINATION}\"\n"));
    fs::remove_dir_all(&base).unwrap();
}
