//! The inbox and its duplicate rules (§7, §12.3): a redelivered input can never wedge a run.

mod fixtures;

use nexus_run_contracts::{Cas, Enqueued, EventBody, RunId, RunStore, StoreError};
use nexus_run_store_sqlite::SqliteRunStore;

fn commissioned_run(store: &SqliteRunStore) -> RunId {
    let run = fixtures::run_id();
    assert_eq!(
        store
            .enqueue(&run, &fixtures::commissioned(&run), &[])
            .unwrap(),
        Enqueued::Accepted
    );
    run
}

/// Every key waiting in the run's inbox, oldest first, read through a connection of its own, because
/// the trait only shows the oldest.
fn inbox_keys(path: &std::path::Path, run: &RunId) -> Vec<String> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT key FROM inbox WHERE run = ?1 ORDER BY rowid")
        .unwrap();
    let keys = statement
        .query_map([run.as_str()], |row| row.get(0))
        .unwrap();
    keys.map(Result::unwrap).collect()
}

fn store_with_path() -> (tempfile::TempDir, std::path::PathBuf, SqliteRunStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    (dir, path, store)
}

#[test]
fn enqueue_of_a_key_in_the_inbox_is_a_duplicate_and_writes_nothing() {
    let (_dir, path, store) = store_with_path();
    let run = commissioned_run(&store);
    let first = fixtures::input(&run, "input/1");
    assert_eq!(
        store.enqueue(&run, &first, &[]).unwrap(),
        Enqueued::Accepted
    );
    let list = [Cas {
        key: "thread/x".into(),
        from: None,
        to: Some(b"v".to_vec()),
    }];
    let mut changed = first.clone();
    changed.body = EventBody::ClaimReleased;
    assert_eq!(
        store.enqueue(&run, &changed, &list).unwrap(),
        Enqueued::Duplicate
    );
    assert_eq!(store.get("thread/x").unwrap(), None, "the list did not run");
    let keys = inbox_keys(&path, &run);
    assert_eq!(keys.iter().filter(|key| *key == "input/1").count(), 1);
}

#[test]
fn enqueue_of_a_key_in_the_log_is_a_duplicate_and_writes_nothing() {
    let (_dir, path, store) = store_with_path();
    let run = commissioned_run(&store);
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let commission = fixtures::commissioned(&run);
    store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &[],
        )
        .unwrap();
    assert!(inbox_keys(&path, &run).is_empty());

    assert_eq!(
        store.enqueue(&run, &commission, &[]).unwrap(),
        Enqueued::Duplicate,
        "consumed long ago, and redelivered"
    );
    assert!(inbox_keys(&path, &run).is_empty(), "no row came back");
}

#[test]
fn append_with_consume_deletes_the_row_in_its_transaction() {
    let (_dir, path, store) = store_with_path();
    let run = commissioned_run(&store);
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let commission = fixtures::commissioned(&run);

    // A failed append consumes nothing.
    let err = store
        .append(
            &run,
            7,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &[],
        )
        .unwrap_err();
    assert_eq!(err, StoreError::SeqMismatch);
    assert_eq!(inbox_keys(&path, &run), [commission.key.clone()]);

    let list = [Cas {
        key: "thread/x".into(),
        from: Some(b"absent".to_vec()),
        to: None,
    }];
    let err = store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &list,
        )
        .unwrap_err();
    assert_eq!(err.kind(), "cas_failed");
    assert_eq!(
        inbox_keys(&path, &run),
        [commission.key.clone()],
        "a rolled-back append gives the row back"
    );

    store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &[],
        )
        .unwrap();
    assert!(inbox_keys(&path, &run).is_empty());
    assert_eq!(store.next_input(&run).unwrap(), None);
}

#[test]
fn duplicate_key_consumes_the_row_so_the_run_is_not_wedged() {
    let (dir, path, store) = store_with_path();
    let run = commissioned_run(&store);
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let commission = fixtures::commissioned(&run);
    store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &[],
        )
        .unwrap();

    // A row whose key is already in the log can only come from a store written by something other than
    // `enqueue`, which refuses it: a crash of an older write path, a restore. Put it there by hand.
    let conn = rusqlite::Connection::open(dir.path().join("run.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO inbox (run, key, at, origin, body) SELECT run, key, at, origin, body
         FROM events WHERE run = ?1 AND key = ?2",
        [run.as_str(), commission.key.as_str()],
    )
    .unwrap();
    drop(conn);
    let wedged = store.next_input(&run).unwrap().expect("the row is back");
    assert_eq!(wedged.key, commission.key);

    let err = store
        .append(
            &run,
            1,
            lease.epoch,
            std::slice::from_ref(&wedged),
            std::slice::from_ref(&wedged.key),
            &[],
        )
        .unwrap_err();
    assert_eq!(err, StoreError::DuplicateKey { seq: 1 });
    assert!(inbox_keys(&path, &run).is_empty(), "the host moves on");
    assert_eq!(store.next_input(&run).unwrap(), None);
    assert_eq!(fixtures::log_keys(&store, &run).len(), 1);
}

#[test]
fn enqueue_with_a_cas_list_is_one_transaction() {
    let (_dir, path, store) = store_with_path();
    let run = fixtures::run_id();
    store
        .compare_and_set("pending/taken", None, Some(b"x"))
        .unwrap();
    let list = [
        Cas {
            key: "flow/h".into(),
            from: None,
            to: Some(b"definition".to_vec()),
        },
        Cas {
            key: "pending/taken".into(),
            from: None,
            to: Some(b"body".to_vec()),
        },
    ];
    let err = store
        .enqueue(&run, &fixtures::commissioned(&run), &list)
        .unwrap_err();
    assert_eq!(
        err,
        StoreError::CasFailed {
            key: "pending/taken".into()
        }
    );
    assert!(
        inbox_keys(&path, &run).is_empty(),
        "a failed CAS leaves no row"
    );
    assert_eq!(store.get("flow/h").unwrap(), None, "and no first CAS");
    assert!(store.open_runs().unwrap().is_empty(), "and no run row");

    let list = [Cas {
        key: "flow/h".into(),
        from: None,
        to: Some(b"definition".to_vec()),
    }];
    assert_eq!(
        store
            .enqueue(&run, &fixtures::commissioned(&run), &list)
            .unwrap(),
        Enqueued::Accepted
    );
    assert_eq!(store.get("flow/h").unwrap(), Some(b"definition".to_vec()));
    assert_eq!(inbox_keys(&path, &run).len(), 1);
}

#[test]
fn claim_busy_keyed_by_the_holder_change_count_enqueued_twice_is_a_duplicate() {
    let (_dir, _path, store) = store_with_path();
    let run = commissioned_run(&store);
    let busy = |changes: u32| {
        fixtures::event(
            &run,
            &format!("claim_busy/{run}/{changes}"),
            EventBody::ClaimBusy {
                holder: fixtures::other_run_id(),
            },
        )
    };
    assert_eq!(
        store.enqueue(&run, &busy(1), &[]).unwrap(),
        Enqueued::Accepted
    );
    assert_eq!(
        store.enqueue(&run, &busy(1), &[]).unwrap(),
        Enqueued::Duplicate,
        "an unchanged holder writes nothing"
    );
    assert_eq!(
        store.enqueue(&run, &busy(2), &[]).unwrap(),
        Enqueued::Accepted,
        "the next holder change is a new input"
    );
}

#[test]
fn next_input_is_the_oldest_row_and_stays_until_consumed() {
    let (_dir, _path, store) = store_with_path();
    let run = commissioned_run(&store);
    for key in ["z", "a", "m"] {
        store
            .enqueue(&run, &fixtures::input(&run, key), &[])
            .unwrap();
    }
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let mut expected_seq = 0;
    let mut seen = Vec::new();
    while let Some(input) = store.next_input(&run).unwrap() {
        assert_eq!(store.next_input(&run).unwrap().unwrap().key, input.key);
        store
            .append(
                &run,
                expected_seq,
                lease.epoch,
                std::slice::from_ref(&input),
                std::slice::from_ref(&input.key),
                &[],
            )
            .unwrap();
        expected_seq += 1;
        seen.push(input.key);
    }
    assert_eq!(seen[1..], ["z", "a", "m"], "arrival order, not key order");
}

#[test]
fn an_inbox_row_keeps_the_version_and_the_fields_of_its_writer() {
    let (_dir, _path, store) = store_with_path();
    let run = commissioned_run(&store);
    let wire = serde_json::json!({
        "run": run.as_str(), "seq": 0, "at": "2026-11-05T10:00:00.000Z", "v": 3, "key": "newer",
        "origin": "cli", "from_a_newer_writer": true,
        "body": { "kind": "claim_taken" },
    });
    let newer: nexus_run_contracts::Event = serde_json::from_value(wire).unwrap();
    store.enqueue(&run, &newer, &[]).unwrap();
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let commission = store.next_input(&run).unwrap().unwrap();
    store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&commission),
            std::slice::from_ref(&commission.key),
            &[],
        )
        .unwrap();
    let read = store.next_input(&run).unwrap().unwrap();
    assert_eq!(read, newer);
}

#[test]
fn an_input_for_a_run_nobody_commissioned_is_unknown() {
    let (_dir, path, store) = store_with_path();
    let run = fixtures::run_id();
    let err = store
        .enqueue(&run, &fixtures::input(&run, "k"), &[])
        .unwrap_err();
    assert_eq!(err, StoreError::UnknownRun);
    assert!(inbox_keys(&path, &run).is_empty());
    assert!(store.open_runs().unwrap().is_empty());
}

#[test]
fn a_run_with_only_an_inbox_row_is_open() {
    let (_dir, _path, store) = store_with_path();
    let run = commissioned_run(&store);
    let row = store.open_runs().unwrap().remove(0);
    assert_eq!(row.run, run);
    assert_eq!(row.workspace_id, "6j6v");
    assert_eq!(row.commissioned_at, fixtures::t0());
    assert_eq!((row.last_seq, row.max_v, row.finished), (0, 0, false));
}

#[test]
fn a_second_commission_of_a_known_run_under_a_new_key_is_refused() {
    let (_dir, path, store) = store_with_path();
    let run = commissioned_run(&store);
    let mut again = fixtures::commissioned(&run);
    again.key = "commission/another".into();
    let err = store.enqueue(&run, &again, &[]).unwrap_err();
    assert_eq!(err.kind(), "invalid_event");
    assert_eq!(inbox_keys(&path, &run).len(), 1);
}

#[test]
fn a_commission_with_a_path_for_a_workspace_or_a_non_id_thread_is_refused() {
    let (_dir, path, store) = store_with_path();
    let run = fixtures::run_id();
    for bad in [
        EventBody::Commissioned {
            run: run.clone(),
            workspace_id: "/Users/someone/repo".into(),
            flow: nexus_run_contracts::FlowRef { hash: "h".into() },
            policy: Default::default(),
            root_thread: fixtures::ROOT_THREAD.into(),
            requester: "r".into(),
        },
        EventBody::Commissioned {
            run: run.clone(),
            workspace_id: "6j6v".into(),
            flow: nexus_run_contracts::FlowRef { hash: "h".into() },
            policy: Default::default(),
            root_thread: "the text of the commission".into(),
            requester: "r".into(),
        },
    ] {
        let err = store
            .enqueue(&run, &fixtures::event(&run, "c", bad), &[])
            .unwrap_err();
        assert_eq!(err.kind(), "invalid_event");
    }
    assert!(inbox_keys(&path, &run).is_empty());
}

#[test]
fn an_owner_key_never_rides_in_a_list() {
    let (_dir, _path, store) = store_with_path();
    let run = fixtures::run_id();
    let key = format!("owner/{run}");
    let list = [Cas {
        key: key.clone(),
        from: None,
        to: Some(b"{}".to_vec()),
    }];
    let err = store
        .enqueue(&run, &fixtures::commissioned(&run), &list)
        .unwrap_err();
    assert_eq!(err, StoreError::Protected { key: key.clone() });
    let err = store
        .append(&run, 0, 1, &[fixtures::commissioned(&run)], &[], &list)
        .unwrap_err();
    assert_eq!(err, StoreError::Protected { key });
}
