//! What opening the file settles (§7): WAL, the tables, and a file from a newer version refused.

mod fixtures;

use nexus_run_contracts::RunStore;
use nexus_run_store_sqlite::SqliteRunStore;

#[test]
fn the_file_is_in_wal_and_holds_exactly_the_tables_of_section_7() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    fixtures::open(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        tables,
        ["events", "inbox", "kv", "runs", "schema", "timers"]
    );
}

#[test]
fn reopening_keeps_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let run = fixtures::run_id();
    {
        let store = fixtures::open(&path);
        fixtures::owned_run(&store, &run);
    }
    let store = fixtures::open(&path);
    assert_eq!(store.load(&run, 0).unwrap().len(), 1);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let versions: i64 = conn
        .query_row("SELECT count(*) FROM schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!(versions, 1);
}

#[test]
fn opening_an_existing_file_needs_no_write_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    fixtures::open(&path);
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(SqliteRunStore::open(&path, fixtures::HOST_TIMEOUT).is_ok());
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_file_from_a_newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    fixtures::open(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE schema SET version = version + 1", [])
        .unwrap();
    drop(conn);
    let err = SqliteRunStore::open(&path, fixtures::HOST_TIMEOUT).unwrap_err();
    assert_eq!(err.kind(), "backend");
}

#[test]
fn a_locked_new_file_answers_busy_rather_than_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("PRAGMA journal_mode = WAL").unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let err = SqliteRunStore::open(&path, fixtures::HOST_TIMEOUT).unwrap_err();
    assert_eq!(err.kind(), "busy");
    holder.execute_batch("ROLLBACK").unwrap();
}
