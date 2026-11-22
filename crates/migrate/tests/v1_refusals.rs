//! NEX-177: the workspaces the reader refuses, each with its code, and each refusal changing nothing
//! (RFC 0008, "Workspaces the move refuses" and "The command").

mod fixture;

use fixture::Fixture;
use nexus_flow_migrate::v1_reader::{Refusal, V1Workspace};
use rusqlite::Connection;

/// Opens `f`'s workspace, expects a refusal, and checks that the workspace is as it was.
fn refused(f: &Fixture) -> Refusal {
    let before = f.snapshot();
    let err = V1Workspace::open(&f.nxs()).expect_err("the workspace is refused");
    assert_eq!(f.snapshot(), before, "a refusal changes nothing");
    err.refusal()
        .unwrap_or_else(|| panic!("a refusal, not {err}"))
        .clone()
}

#[test]
fn no_workspace_directory_is_nothing_to_migrate() {
    let f = Fixture::empty();
    assert_eq!(refused(&f).code(), "nothing_to_migrate");
    assert!(!f.nxs().exists(), "no .nxs/ is created");
}

#[test]
fn a_workspace_directory_without_a_database_is_nothing_to_migrate() {
    let f = Fixture::v1();
    f.remove("db.sqlite");
    assert_eq!(refused(&f).code(), "nothing_to_migrate");
    assert!(!f.db().exists(), "no database is created");
}

/// SQLite keeps `db.sqlite-wal` and `db.sqlite-shm` beside a WAL database for as long as a
/// connection holds it open, as v1's sync service or a chat session does.
#[test]
fn a_database_a_v1_process_holds_open_is_v1_in_use() {
    let f = Fixture::v1();
    let v1 = Connection::open(f.db()).unwrap();
    v1.execute_batch("SELECT COUNT(*) FROM ops").unwrap();
    assert_eq!(refused(&f).code(), "v1_in_use");
    drop(v1);
    V1Workspace::open(&f.nxs()).unwrap();
}

/// A process that ended without closing the database leaves its `-wal` behind, with writes that
/// may not be in `db.sqlite` yet; v1 takes them in the next time it opens the workspace.
#[test]
fn a_write_ahead_log_left_behind_is_v1_in_use() {
    let f = Fixture::v1();
    f.write("db.sqlite-wal", "");
    assert_eq!(refused(&f).code(), "v1_in_use");
}

#[test]
fn a_database_opened_by_v1_while_it_is_read_is_v1_in_use() {
    let f = Fixture::v1();
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    let v1 = Connection::open(f.db()).unwrap();
    v1.execute_batch("SELECT COUNT(*) FROM ops").unwrap();
    let err = ws
        .ops()
        .expect_err("the log is not read while v1 has it open");
    assert_eq!(err.refusal().map(Refusal::code), Some("v1_in_use"));
}

#[test]
fn a_plugin_other_than_issue_tracker_is_unsupported() {
    let f = Fixture::v1();
    f.write("config.toml", "[flow]\nplugin = \"personal-todo\"\n");
    let refusal = refused(&f);
    assert_eq!(refusal.code(), "unsupported_plugin");
    assert_eq!(
        refusal,
        Refusal::UnsupportedPlugin {
            plugin: "personal-todo".into()
        }
    );

    f.write("config.toml", "plugin = \"personal-todo\"\n");
    assert_eq!(refused(&f).code(), "unsupported_plugin");
}

#[test]
fn a_schema_below_9_is_v1_format_old() {
    let f = Fixture::v1();
    f.sql("PRAGMA user_version = 8;");
    assert_eq!(refused(&f), Refusal::V1FormatOld { schema: 8 });
    assert_eq!(Refusal::V1FormatOld { schema: 8 }.code(), "v1_format_old");
}

#[test]
fn a_schema_above_9_is_v1_format_unknown() {
    let f = Fixture::v1();
    f.sql("PRAGMA user_version = 10;");
    assert_eq!(refused(&f), Refusal::V1FormatUnknown { schema: 10 });
    assert_eq!(
        Refusal::V1FormatUnknown { schema: 10 }.code(),
        "v1_format_unknown"
    );
}

#[test]
fn a_store_started_from_a_snapshot_is_v1_based_store() {
    let f = Fixture::v1();
    f.sql("INSERT INTO log_base(id, relay_position, clock) VALUES(1, 40, 1200);");
    assert_eq!(refused(&f).code(), "v1_based_store");
}

/// A v1 process that opened the database, wrote and closed it again between two checks leaves no
/// file beside it, but the database itself changed under the read.
#[test]
fn a_database_v1_changed_while_it_is_read_is_v1_in_use() {
    let f = Fixture::v1();
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    f.insert(&[fixture::Op::title("late", 1, 1)]);
    let err = ws
        .ops()
        .expect_err("a log that changed under the read is not returned");
    assert_eq!(err.refusal().map(Refusal::code), Some("v1_in_use"));
}
