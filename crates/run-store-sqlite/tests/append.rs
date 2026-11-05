//! `append` as one transaction: the fence, the sequence, the keys and the CAS list (§7).

mod fixtures;

use nexus_run_contracts::{Cas, RunStore, StoreError};
use nexus_run_store_sqlite::SqliteRunStore;

fn cas(key: &str, from: Option<&[u8]>, to: Option<&[u8]>) -> Cas {
    Cas {
        key: key.into(),
        from: from.map(<[u8]>::to_vec),
        to: to.map(<[u8]>::to_vec),
    }
}

#[test]
fn fenced_beats_seq_mismatch_beats_duplicate_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteRunStore::open(
        &dir.path().join("run.sqlite"),
        std::time::Duration::from_millis(100),
    )
    .unwrap();
    let run = fixtures::run_id();
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    store
        .append(
            &run,
            0,
            lease.epoch,
            &[fixtures::commissioned(&run)],
            &[],
            &[],
        )
        .unwrap();
    // wrong fence AND wrong seq AND duplicate key: Fenced wins
    let err = store
        .append(
            &run,
            0,
            lease.epoch + 1,
            &[fixtures::commissioned(&run)],
            &[],
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, StoreError::Fenced));
    let err = store
        .append(
            &run,
            0,
            lease.epoch,
            &[fixtures::commissioned(&run)],
            &[],
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, StoreError::SeqMismatch));
    let err = store
        .append(
            &run,
            1,
            lease.epoch,
            &[fixtures::commissioned(&run)],
            &[],
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, StoreError::DuplicateKey { seq: 1 }));
}

#[test]
fn duplicate_key_beats_a_failing_cas_and_applies_none_of_the_list() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let list = [
        cas("thread/applied", None, Some(b"x")),
        cas("thread/absent", Some(b"not there"), Some(b"y")),
    ];
    let err = store
        .append(&run, 1, fence, &[fixtures::commissioned(&run)], &[], &list)
        .unwrap_err();
    assert_eq!(err, StoreError::DuplicateKey { seq: 1 });
    assert_eq!(store.get("thread/applied").unwrap(), None);
}

#[test]
fn a_failed_cas_rolls_the_whole_append_back() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let list = [
        cas("thread/applied", None, Some(b"x")),
        cas("thread/absent", Some(b"not there"), Some(b"y")),
    ];
    let err = store
        .append(&run, 1, fence, &[fixtures::input(&run, "k")], &[], &list)
        .unwrap_err();
    assert_eq!(
        err,
        StoreError::CasFailed {
            key: "thread/absent".into()
        }
    );
    assert_eq!(store.get("thread/applied").unwrap(), None);
    assert_eq!(fixtures::log_keys(&store, &run).len(), 1, "no event landed");
    let row = &store.open_runs().unwrap()[0];
    assert_eq!(row.last_seq, 1, "the run row did not move");
}

#[test]
fn append_assigns_a_gap_free_sequence_whatever_the_events_carry() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let mut claimed = fixtures::input(&run, "a");
    claimed.seq = 99;
    let appended = store
        .append(
            &run,
            1,
            fence,
            &[claimed, fixtures::input(&run, "b")],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(appended.last_seq, 3);
    let seqs: Vec<_> = store.load(&run, 0).unwrap().iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    let after_first: Vec<_> = store
        .load(&run, 1)
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(after_first, ["a", "b"]);
}

#[test]
fn an_event_reads_back_whole_with_what_this_version_does_not_know() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let wire = serde_json::json!({
        "run": run.as_str(), "seq": 0, "at": "2026-11-05T10:00:00.000Z", "v": 1, "key": "newer",
        "origin": "reducer", "from_a_newer_writer": 7,
        "body": { "kind": "a_kind_from_the_future", "field": "kept" },
    });
    let newer: nexus_run_contracts::Event = serde_json::from_value(wire).unwrap();
    assert!(newer.is_unknown_decision());
    store
        .append(&run, 1, fence, std::slice::from_ref(&newer), &[], &[])
        .unwrap();
    let read = store.load(&run, 1).unwrap().remove(0);
    assert_eq!(read.seq, 2);
    assert!(read.is_unknown_decision());
    assert_eq!(read.extra["from_a_newer_writer"], 7);
    let written = serde_json::to_value(&read).unwrap();
    assert_eq!(written["body"]["field"], "kept");
}

#[test]
fn an_append_without_an_owner_is_fenced() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let err = store
        .append(&run, 0, 1, &[fixtures::commissioned(&run)], &[], &[])
        .unwrap_err();
    assert_eq!(err, StoreError::Fenced);
}

#[test]
fn only_commissioned_creates_a_run() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let err = store
        .append(
            &run,
            0,
            lease.epoch,
            &[fixtures::input(&run, "k")],
            &[],
            &[],
        )
        .unwrap_err();
    assert_eq!(err, StoreError::UnknownRun);
    assert!(store.open_runs().unwrap().is_empty());
}

#[test]
fn a_finished_event_closes_the_run_row() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let row = store.open_runs().unwrap().remove(0);
    assert_eq!(row.workspace_id, "6j6v");
    assert_eq!(row.commissioned_at, fixtures::t0());
    assert_eq!((row.last_seq, row.max_v, row.finished), (1, 1, false));
    store
        .append(&run, 1, fence, &[fixtures::finished(&run, "end")], &[], &[])
        .unwrap();
    assert!(store.open_runs().unwrap().is_empty());
}

#[test]
fn an_event_of_another_run_is_refused_before_anything_is_written() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let foreign = fixtures::input(&fixtures::other_run_id(), "k");
    let err = store
        .append(&run, 1, fence, &[foreign], &[], &[])
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_event");
    assert_eq!(fixtures::log_keys(&store, &run).len(), 1);
}

#[test]
fn two_events_with_one_key_in_a_call_are_refused() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let twice = [fixtures::input(&run, "k"), fixtures::input(&run, "k")];
    let err = store.append(&run, 1, fence, &twice, &[], &[]).unwrap_err();
    assert_eq!(err.kind(), "invalid_event");
}

#[test]
fn two_cas_on_one_key_in_a_call_are_refused() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let list = [
        cas("thread/x", None, Some(b"1")),
        cas("thread/x", Some(b"1"), Some(b"2")),
    ];
    let err = store
        .append(&run, 1, fence, &[fixtures::input(&run, "k")], &[], &list)
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_event");
    assert_eq!(store.get("thread/x").unwrap(), None);
}

#[test]
fn a_cas_list_deletes_and_writes_in_the_transaction_of_the_append() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    store
        .compare_and_set("pending/x", None, Some(b"body"))
        .unwrap();
    let list = [
        cas("pending/x", Some(b"body"), None),
        cas("persona_session/x/coder", None, Some(b"s")),
    ];
    store
        .append(&run, 1, fence, &[fixtures::input(&run, "k")], &[], &list)
        .unwrap();
    assert_eq!(store.get("pending/x").unwrap(), None);
    assert_eq!(
        store.get("persona_session/x/coder").unwrap(),
        Some(b"s".to_vec())
    );
}

#[test]
fn the_sqlite_store_declares_no_bounds() {
    let (_dir, store) = fixtures::temp_store();
    assert_eq!(store.limits(), nexus_run_contracts::StoreLimits::default());
}
