//! The per-run owner lease (§7), over an injected clock: acquire, takeover, renew, and a key that is
//! never deleted, so an epoch never restarts.

mod fixtures;

use nexus_run_contracts::{Cas, RunId, RunStore, Secs, StoreError};
use nexus_run_store_sqlite::{EffectEntry, Lease, LEASE_EXPIRY};

const EXPIRY_MS: i64 = 30_000;

fn owner_key(run: &RunId) -> String {
    format!("owner/{run}")
}

#[test]
fn the_lease_expiry_is_thirty_seconds() {
    assert_eq!(LEASE_EXPIRY, Secs(30));
}

#[test]
fn acquire_from_absent_is_epoch_one_and_its_value_is_the_compact_named_struct() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    assert_eq!(
        lease,
        Lease {
            owner_id: "host-a".into(),
            epoch: 1,
            expires_at: fixtures::at(EXPIRY_MS),
        }
    );
    assert_eq!(
        String::from_utf8(store.get(&owner_key(&run)).unwrap().unwrap()).unwrap(),
        r#"{"owner_id":"host-a","epoch":1,"expires_at":"2026-11-05T10:00:30.000Z"}"#
    );
}

#[test]
fn another_host_cannot_take_a_live_lease() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let just_before = fixtures::at(EXPIRY_MS - 1);
    assert_eq!(
        store
            .acquire_or_renew(&run, "host-b", just_before)
            .unwrap_err(),
        StoreError::CasFailed {
            key: owner_key(&run)
        }
    );
}

#[test]
fn takeover_after_expiry_raises_the_epoch_from_the_exact_value() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let before = store.get(&owner_key(&run)).unwrap().unwrap();

    let at_expiry = fixtures::at(EXPIRY_MS);
    let taken = store.acquire_or_renew(&run, "host-b", at_expiry).unwrap();
    assert_eq!(
        taken,
        Lease {
            owner_id: "host-b".into(),
            epoch: 2,
            expires_at: fixtures::at(2 * EXPIRY_MS),
        }
    );

    // The exact previous value no longer matches, so the old holder's CAS cannot land.
    let old_holder = store
        .compare_and_set(&owner_key(&run), Some(&before), Some(&before))
        .unwrap_err();
    assert_eq!(old_holder.kind(), "cas_failed");
}

#[test]
fn a_holder_past_its_own_expiry_comes_back_with_a_higher_epoch() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let again = store
        .acquire_or_renew(&run, "host-a", fixtures::at(EXPIRY_MS))
        .unwrap();
    assert_eq!(again.epoch, 2);
}

#[test]
fn a_renewal_keeps_the_epoch_and_writes_only_when_less_than_half_the_expiry_remains() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let first = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();

    let half_left = fixtures::at(EXPIRY_MS / 2);
    assert_eq!(
        store.acquire_or_renew(&run, "host-a", half_left).unwrap(),
        first,
        "exactly half remains: no WAL write"
    );

    let less_than_half = fixtures::at(EXPIRY_MS / 2 + 1);
    let renewed = store
        .acquire_or_renew(&run, "host-a", less_than_half)
        .unwrap();
    assert_eq!(renewed.epoch, first.epoch);
    assert_eq!(
        renewed.expires_at,
        fixtures::at(EXPIRY_MS / 2 + 1 + EXPIRY_MS)
    );
    assert_eq!(
        store.get(&owner_key(&run)).unwrap().unwrap(),
        serde_json::to_vec(&renewed).unwrap()
    );
}

#[test]
fn a_renewal_does_not_land_on_a_value_another_host_changed() {
    // The race `acquire_or_renew` loses through its CAS: a renewal against a takeover. Whoever gets
    // there first, exactly one of them holds the lease afterwards.
    let (dir, _store) = fixtures::temp_store();
    let path = dir.path().join("run.sqlite");
    for round in 0..40 {
        let run = RunId::parse(&format!("r-01JAAAAAAAAAAAAAAAAAAAAA{round:02}")).unwrap();
        let a = fixtures::open(&path);
        let b = fixtures::open(&path);
        a.acquire_or_renew(&run, "host-a", fixtures::t0()).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let (renewal, takeover) = std::thread::scope(|scope| {
            let renewal = scope.spawn(|| {
                barrier.wait();
                retry_busy(|| a.acquire_or_renew(&run, "host-a", fixtures::at(EXPIRY_MS - 1)))
            });
            let takeover = scope.spawn(|| {
                barrier.wait();
                retry_busy(|| b.acquire_or_renew(&run, "host-b", fixtures::at(EXPIRY_MS)))
            });
            (renewal.join().unwrap(), takeover.join().unwrap())
        });
        let stored: Lease =
            serde_json::from_slice(&a.get(&owner_key(&run)).unwrap().unwrap()).unwrap();
        match (renewal, takeover) {
            // A landed first and moved the expiry on, so B finds a live lease.
            (Ok(renewed), Err(StoreError::CasFailed { .. })) => {
                assert_eq!((renewed.epoch, &stored), (1, &renewed));
            }
            // B landed first, or A read before B's write and lost the CAS.
            (Err(StoreError::CasFailed { .. }), Ok(taken)) => {
                assert_eq!((taken.epoch, &stored), (2, &taken));
            }
            other => panic!("not exactly one winner: {other:?}"),
        }
    }
}

fn retry_busy<T>(mut call: impl FnMut() -> Result<T, StoreError>) -> Result<T, StoreError> {
    loop {
        match call() {
            Err(StoreError::Busy) => {}
            other => return other,
        }
    }
}

#[test]
fn an_owner_key_refuses_deletion_and_the_epoch_never_restarts() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let key = owner_key(&run);
    store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    store
        .acquire_or_renew(&run, "host-b", fixtures::at(EXPIRY_MS))
        .unwrap();
    let current = store.get(&key).unwrap().unwrap();

    let err = store
        .compare_and_set(&key, Some(&current), None)
        .unwrap_err();
    assert_eq!(err, StoreError::Protected { key: key.clone() });
    // A wrong `from` is refused the same way, so a delete never reads as merely having lost a race.
    let err = store.compare_and_set(&key, Some(b"x"), None).unwrap_err();
    assert_eq!(err, StoreError::Protected { key: key.clone() });
    assert_eq!(store.get(&key).unwrap().unwrap(), current);

    let restarted = serde_json::to_vec(&Lease {
        owner_id: "host-c".into(),
        epoch: 1,
        expires_at: fixtures::at(10 * EXPIRY_MS),
    })
    .unwrap();
    let err = store
        .compare_and_set(&key, Some(&current), Some(&restarted))
        .unwrap_err();
    assert_eq!(err, StoreError::Protected { key: key.clone() });
    assert_eq!(store.get(&key).unwrap().unwrap(), current);

    let taken = store
        .acquire_or_renew(&run, "host-c", fixtures::at(2 * EXPIRY_MS))
        .unwrap();
    assert_eq!(taken.epoch, 3, "takeover after takeover keeps rising");
}

#[test]
fn an_owner_value_that_is_no_lease_is_refused() {
    let (_dir, store) = fixtures::temp_store();
    let key = owner_key(&fixtures::run_id());
    let err = store
        .compare_and_set(&key, None, Some(b"not a lease"))
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_event");
    assert_eq!(store.get(&key).unwrap(), None);
}

#[test]
fn two_hosts_own_different_runs_of_one_store() {
    let (_dir, store) = fixtures::temp_store();
    let (first, second) = (fixtures::run_id(), fixtures::other_run_id());
    let a = store
        .acquire_or_renew(&first, "host-a", fixtures::t0())
        .unwrap();
    let b = store
        .acquire_or_renew(&second, "host-b", fixtures::t0())
        .unwrap();
    assert_eq!((a.epoch, b.epoch), (1, 1));

    store
        .append(
            &first,
            0,
            a.epoch,
            &[fixtures::commissioned(&first)],
            &[],
            &[],
        )
        .unwrap();
    store
        .append(
            &second,
            0,
            b.epoch,
            &[fixtures::commissioned(&second)],
            &[],
            &[],
        )
        .unwrap();

    // Moving one run's lease on leaves the other's fence as it was.
    let taken = store
        .acquire_or_renew(&first, "host-c", fixtures::at(EXPIRY_MS))
        .unwrap();
    assert_eq!(taken.epoch, 2);
    assert_eq!(
        store
            .append(
                &first,
                1,
                a.epoch,
                &[fixtures::input(&first, "k")],
                &[],
                &[]
            )
            .unwrap_err(),
        StoreError::Fenced
    );
    store
        .append(
            &second,
            1,
            b.epoch,
            &[fixtures::input(&second, "k")],
            &[],
            &[],
        )
        .unwrap();
}

#[test]
fn open_runs_lists_a_run_with_only_an_inbox_row_for_the_host_to_acquire() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    store
        .enqueue(&run, &fixtures::commissioned(&run), &[])
        .unwrap();
    let open = store.open_runs().unwrap();
    assert_eq!(open.len(), 1);
    let lease = store
        .acquire_or_renew(&open[0].run, "host-a", fixtures::t0())
        .unwrap();
    assert_eq!(lease.epoch, 1);
}

fn effect(state: &str, epoch: u64) -> Vec<u8> {
    serde_json::to_vec(&EffectEntry {
        state: state.into(),
        epoch,
    })
    .unwrap()
}

#[test]
fn every_effect_entry_records_the_epoch_that_wrote_it() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let key = format!("effect/{run}/root/{run}");

    let bare = store
        .compare_and_set(&key, None, Some(b"pending"))
        .unwrap_err();
    assert_eq!(bare.kind(), "invalid_event", "an entry with no epoch");
    assert_eq!(store.get(&key).unwrap(), None);

    let pending = effect("pending", lease.epoch);
    store.compare_and_set(&key, None, Some(&pending)).unwrap();
    assert_eq!(
        String::from_utf8(store.get(&key).unwrap().unwrap()).unwrap(),
        r#"{"state":"pending","epoch":1}"#
    );

    // The next holder picks it up, and what it writes carries its own epoch.
    let taken = store
        .acquire_or_renew(&run, "host-b", fixtures::at(EXPIRY_MS))
        .unwrap();
    assert_eq!(
        store
            .compare_and_set(&key, Some(&pending), Some(&effect("done:m-1", lease.epoch)))
            .unwrap_err(),
        StoreError::Fenced,
        "the epoch that wrote it is not the run's any more"
    );
    assert_eq!(store.get(&key).unwrap().unwrap(), pending);
    store
        .compare_and_set(&key, Some(&pending), Some(&effect("done:m-1", taken.epoch)))
        .unwrap();

    let no_run = store
        .compare_and_set("effect/no-run-here", None, Some(&effect("pending", 1)))
        .unwrap_err();
    assert_eq!(no_run.kind(), "invalid_event");
}

#[test]
fn an_effect_entry_in_a_list_passes_the_same_check() {
    let (_dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let list = [Cas {
        key: format!("effect/{run}/x"),
        from: None,
        to: Some(effect("pending", fence + 1)),
    }];
    let err = store
        .append(&run, 1, fence, &[fixtures::input(&run, "k")], &[], &list)
        .unwrap_err();
    assert_eq!(err, StoreError::Fenced);
    assert_eq!(fixtures::log_keys(&store, &run).len(), 1, "rolled back");
}

#[test]
fn host_seen_writes_only_when_older_than_half_the_attended_window() {
    let (_dir, store) = fixtures::temp_store();
    let workspace = "6j6v";
    assert_eq!(store.host_seen(workspace).unwrap(), None);

    store.mark_host_seen(workspace, fixtures::t0()).unwrap();
    assert_eq!(store.host_seen(workspace).unwrap(), Some(fixtures::t0()));

    // Attended means seen within two expiries, so half that window is one expiry.
    store
        .mark_host_seen(workspace, fixtures::at(EXPIRY_MS - 1))
        .unwrap();
    assert_eq!(store.host_seen(workspace).unwrap(), Some(fixtures::t0()));

    store
        .mark_host_seen(workspace, fixtures::at(EXPIRY_MS))
        .unwrap();
    assert_eq!(
        store.host_seen(workspace).unwrap(),
        Some(fixtures::at(EXPIRY_MS))
    );
}

#[test]
fn host_seen_from_a_clock_restarted_far_behind_moves_back_with_it() {
    let (_dir, store) = fixtures::temp_store();
    store
        .mark_host_seen("6j6v", fixtures::at(10 * EXPIRY_MS))
        .unwrap();
    store.mark_host_seen("6j6v", fixtures::t0()).unwrap();
    assert_eq!(store.host_seen("6j6v").unwrap(), Some(fixtures::t0()));
}

#[test]
fn host_seen_that_loses_a_race_to_another_host_is_no_error() {
    // The stored value is what the winner just put there, so nothing is owed.
    let (dir, _store) = fixtures::temp_store();
    let path = dir.path().join("run.sqlite");
    for round in 0..40 {
        let workspace = format!("workspace-{round}");
        let a = fixtures::open(&path);
        let b = fixtures::open(&path);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for store in [&a, &b] {
                let (barrier, workspace) = (&barrier, &workspace);
                scope.spawn(move || {
                    barrier.wait();
                    retry_busy(|| store.mark_host_seen(workspace, fixtures::t0())).unwrap();
                });
            }
        });
        assert_eq!(a.host_seen(&workspace).unwrap(), Some(fixtures::t0()));
    }
}
