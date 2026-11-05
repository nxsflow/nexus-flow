//! A row that does not decode is named, so the host can act on it, and nothing of it reaches the error:
//! a damaged row can hold anything, and errors reach log lines.

mod fixtures;

use nexus_run_contracts::{RunId, RunStore, StoreError};
use nexus_run_store_sqlite::SqliteRunStore;

const MARKER: &str = "CANARY-in-a-damaged-row";

fn store_with_run() -> (
    tempfile::TempDir,
    rusqlite::Connection,
    SqliteRunStore,
    RunId,
    u64,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let raw = rusqlite::Connection::open(&path).unwrap();
    (dir, raw, store, run, fence)
}

fn known_kind_with_a_bad_field() -> String {
    format!(r#"{{"kind":"claim_busy","holder":"{MARKER}"}}"#)
}

fn assert_says_nothing_of_the_row(err: &StoreError) {
    assert!(!err.to_string().contains(MARKER), "{err}");
    assert!(!format!("{err:?}").contains(MARKER), "{err:?}");
}

#[test]
fn a_damaged_inbox_row_is_named_by_its_key_and_can_be_consumed() {
    let (_dir, raw, store, run, fence) = store_with_run();
    raw.execute(
        "INSERT INTO inbox (run, key, at, origin, body) VALUES (?1, 'damaged', 0, 'cli', ?2)",
        [run.as_str(), &known_kind_with_a_bad_field()],
    )
    .unwrap();
    store
        .enqueue(&run, &fixtures::input(&run, "behind"), &[])
        .unwrap();

    let err = store.next_input(&run).unwrap_err();
    assert_eq!(
        err,
        StoreError::DamagedRow {
            key: "damaged".into(),
            seq: None
        }
    );
    assert_says_nothing_of_the_row(&err);

    // Whatever the host decides to do about it (m8we), it can name the row in `consume`, and the run
    // moves on to the input behind it.
    store
        .append(
            &run,
            1,
            fence,
            &[fixtures::input(&run, "the-hosts-answer")],
            &["damaged".to_owned()],
            &[],
        )
        .unwrap();
    assert_eq!(store.next_input(&run).unwrap().unwrap().key, "behind");
}

#[test]
fn a_damaged_log_row_is_named_by_its_key_and_seq() {
    let (_dir, raw, store, run, _fence) = store_with_run();
    raw.execute(
        "INSERT INTO events (run, seq, at, v, key, origin, body)
         VALUES (?1, 2, 0, 1, 'damaged', 'host', ?2)",
        [run.as_str(), &known_kind_with_a_bad_field()],
    )
    .unwrap();
    let err = store.load(&run, 0).unwrap_err();
    assert_eq!(
        err,
        StoreError::DamagedRow {
            key: "damaged".into(),
            seq: Some(2)
        }
    );
    assert_says_nothing_of_the_row(&err);
    assert_eq!(
        store.load(&run, 2).unwrap().len(),
        0,
        "a later read is fine"
    );
}

#[test]
fn a_body_that_is_not_even_text_is_named_the_same_way() {
    let (_dir, raw, store, run, _fence) = store_with_run();
    raw.execute(
        "INSERT INTO inbox (run, key, at, origin, body) VALUES (?1, 'blob', 0, 'cli', x'FF00')",
        [run.as_str()],
    )
    .unwrap();
    assert_eq!(
        store.next_input(&run).unwrap_err(),
        StoreError::DamagedRow {
            key: "blob".into(),
            seq: None
        }
    );
}

#[test]
fn an_unknown_kind_is_not_damage() {
    let (_dir, raw, store, run, _fence) = store_with_run();
    let body = format!(
        r#"{{"run":"{run}","seq":0,"at":"2026-11-05T10:00:00.000Z","v":1,"key":"future","origin":"cli","body":{{"kind":"from_the_future"}}}}"#
    );
    raw.execute(
        "INSERT INTO inbox (run, key, at, origin, body) VALUES (?1, 'future', 0, 'cli', ?2)",
        [run.as_str(), &body],
    )
    .unwrap();
    assert!(store.next_input(&run).unwrap().unwrap().is_unknown_input());
}

#[test]
fn a_damaged_lease_is_named_and_never_passes_the_fence() {
    let (_dir, raw, store, run, fence) = store_with_run();
    let key = format!("owner/{run}");
    raw.execute(
        "UPDATE kv SET value = ?2 WHERE key = ?1",
        rusqlite::params![key, MARKER.as_bytes()],
    )
    .unwrap();
    let expected = StoreError::DamagedRow {
        key: key.clone(),
        seq: None,
    };
    let err = store
        .append(&run, 1, fence, &[fixtures::input(&run, "k")], &[], &[])
        .unwrap_err();
    assert_eq!(err, expected);
    assert_says_nothing_of_the_row(&err);
    assert_eq!(
        store
            .acquire_or_renew(&run, "host-a", fixtures::t0())
            .unwrap_err(),
        expected
    );
}
