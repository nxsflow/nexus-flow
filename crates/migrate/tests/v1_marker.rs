//! NEX-180: the move marks a v1 workspace before it replays anything, writes `.nxs/migrated.toml`
//! and raises the header of `db.sqlite` to 10, and `--undo` restores the header from the marker
//! (RFC 0008, "The marker").

mod fixture;

use fixture::{Fixture, Op};
use nexus_flow_migrate::marker::{self, Header, Mark};
use nexus_flow_migrate::v1_reader::V1Workspace;
use rusqlite::Connection;

/// Where SQLite keeps `PRAGMA user_version` and `PRAGMA application_id` in the database header,
/// each a 4-byte big-endian integer.
const USER_VERSION: std::ops::Range<usize> = 60..64;
const APPLICATION_ID: std::ops::Range<usize> = 68..72;

fn mark() -> Mark {
    Mark {
        space: "space-0001".into(),
        run: "run-0001".into(),
        migrated_at: "2026-11-20T21:00:00Z".into(),
        by: "nxs 2.0.0".into(),
    }
}

fn marked(f: &Fixture) -> V1Workspace {
    let mut ws = V1Workspace::open(&f.nxs()).expect("the workspace opens");
    ws.mark(&mark()).expect("the workspace is marked");
    ws
}

/// The database's header, read through SQLite as v1 reads it.
fn header(f: &Fixture) -> (i64, i64) {
    let conn = Connection::open(f.db()).unwrap();
    let user_version = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let application_id = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    (user_version, application_id)
}

fn db_bytes(f: &Fixture) -> Vec<u8> {
    std::fs::read(f.db()).unwrap()
}

#[test]
fn the_marker_names_the_space_and_the_run_and_records_the_old_header() {
    let f = Fixture::v1();
    marked(&f);

    let text = std::fs::read_to_string(f.nxs().join("migrated.toml")).unwrap();
    assert!(
        text.starts_with(
            "# .nxs/migrated.toml, written by nxs migrate. Remove only with nxs migrate --undo.\n"
        ),
        "the marker opens with the comment RFC 0008 gives it:\n{text}"
    );
    let value: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(value["format"].as_integer(), Some(1));
    assert_eq!(value["space"].as_str(), Some("space-0001"));
    assert_eq!(value["run"].as_str(), Some("run-0001"));
    assert_eq!(value["migrated_at"].as_str(), Some("2026-11-20T21:00:00Z"));
    assert_eq!(value["by"].as_str(), Some("nxs 2.0.0"));
    assert_eq!(value["header"]["user_version"].as_integer(), Some(9));
    assert_eq!(value["header"]["application_id"].as_integer(), Some(4));

    let read = marker::read(&f.nxs())
        .unwrap()
        .expect("the workspace is marked");
    assert_eq!(read.space, "space-0001");
    assert_eq!(read.run, "run-0001");
    assert_eq!(
        read.header,
        Header {
            user_version: 9,
            application_id: 4
        }
    );
}

#[test]
fn an_unmarked_workspace_has_no_marker() {
    let f = Fixture::v1();
    assert!(marker::read(&f.nxs()).unwrap().is_none());
}

#[test]
fn marking_raises_the_schema_and_the_floor_to_10() {
    let f = Fixture::v1();
    marked(&f);
    assert_eq!(header(&f), (10, 10));
}

#[test]
fn marking_changes_only_the_marker_file_and_the_two_header_fields() {
    let f = Fixture::v1();
    f.insert(&[Op::title("op-1", 1, 1), Op::title("op-2", 2, 1)]);
    let files_before = f.snapshot();
    let before = db_bytes(&f);

    marked(&f);

    let after = db_bytes(&f);
    assert_eq!(after.len(), before.len(), "the database keeps its length");
    let changed: Vec<usize> = (0..before.len())
        .filter(|&i| before[i] != after[i])
        .collect();
    assert!(
        changed
            .iter()
            .all(|i| USER_VERSION.contains(i) || APPLICATION_ID.contains(i)),
        "only the two header fields change, but bytes {changed:?} did"
    );
    assert_eq!(&after[USER_VERSION], &10u32.to_be_bytes());
    assert_eq!(&after[APPLICATION_ID], &10u32.to_be_bytes());

    let others = |files: Vec<(String, usize, u64)>| -> Vec<(String, usize, u64)> {
        files
            .into_iter()
            .filter(|(name, _, _)| name != "db.sqlite" && name != "migrated.toml")
            .collect()
    };
    let files_after = f.snapshot();
    assert_eq!(
        others(files_after.clone()),
        others(files_before),
        "no other file in .nxs/ changes"
    );
    let names: Vec<&str> = files_after.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"migrated.toml"), "the marker is written");
    assert!(
        !names.iter().any(|n| n.starts_with("db.sqlite-")),
        "no -wal, -shm or -journal file is left: {names:?}"
    );
}

#[test]
fn the_ops_are_still_read_after_marking() {
    let f = Fixture::v1();
    f.insert(&[Op::title("op-1", 1, 1), Op::title("op-2", 2, 1)]);
    let ws = marked(&f);
    let ids: Vec<String> = ws.ops().unwrap().into_iter().map(|o| o.op_id).collect();
    assert_eq!(ids, ["op-1", "op-2"]);
}

#[test]
fn a_workspace_v1_holds_open_is_not_marked() {
    let f = Fixture::v1();
    let mut ws = V1Workspace::open(&f.nxs()).unwrap();
    let before = f.snapshot();
    let v1 = Connection::open(f.db()).unwrap();
    v1.query_row("SELECT count(*) FROM ops", [], |r| r.get::<_, i64>(0))
        .unwrap();

    let err = ws.mark(&mark()).expect_err("marking is refused");
    assert_eq!(err.refusal().map(|r| r.code()), Some("v1_in_use"));
    drop(v1);
    assert_eq!(f.snapshot(), before, "a refused mark changes nothing");
}

#[test]
fn undo_restores_the_header_byte_for_byte_and_removes_the_marker() {
    let f = Fixture::v1();
    f.insert(&[Op::title("op-1", 1, 1)]);
    let before = db_bytes(&f);
    drop(marked(&f));

    let restored = marker::undo(&f.nxs()).expect("the mark is undone");
    assert_eq!(
        restored,
        Header {
            user_version: 9,
            application_id: 4
        }
    );
    assert_eq!(header(&f), (9, 4));
    assert!(db_bytes(&f) == before, "the database is as it was");
    assert!(
        !f.nxs().join("migrated.toml").exists(),
        "the marker is gone"
    );
    assert!(marker::read(&f.nxs()).unwrap().is_none());
    V1Workspace::open(&f.nxs()).expect("the move reads the workspace again");
}

#[test]
fn undo_refuses_a_workspace_without_a_marker() {
    let f = Fixture::v1();
    let before = f.snapshot();
    let err = marker::undo(&f.nxs()).expect_err("nothing to undo");
    assert!(
        matches!(err, marker::Error::NotMarked),
        "not marked, got {err}"
    );
    assert_eq!(f.snapshot(), before);
}
