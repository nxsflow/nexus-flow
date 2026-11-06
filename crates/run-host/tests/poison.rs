//! Poison (§13, must-fix round 3 and 4): what counts, what never counts, and that the run keeps its human exit.
//!
//! The decision k8cw left to this ticket: an inbox row that does not decode COUNTS towards `poison_input`,
//! because retrying cannot mend it and the run would otherwise never get past it. A damaged row in the LOG cannot
//! be consumed, so it never counts: that run is read-only and keeps the withdraw exit.

mod common;

use nexus_run_contracts::{Decision, Event, EventBody, Origin, ReasonCode, RunId, RunStore, Slot};
use nexus_run_host::bridge;
use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
use nexus_run_host::test_support::{
    panics, replies_pass, store_path, Rig, StoreProbe, CLOCK_START,
};

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

fn rig_with_a_running_coder(dir: &std::path::Path) -> (Rig, RunId, String) {
    let mut rig = Rig::scripted(dir, CLOCK_START, Box::new(silent));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(300, move |rig| {
        rig.events(&check).iter().any(|e| {
            matches!(
                &e.body,
                EventBody::SessionFact {
                    fact: nexus_run_contracts::SessionFact::Started { .. },
                    ..
                }
            )
        })
    });
    (rig, run, receipt.root_thread)
}

fn reply_key(run: &RunId, name: &str) -> String {
    format!("input/{run}/{name}")
}

/// A healthy reply of the running coder, as the bridge would enqueue it.
fn enqueue_reply(rig: &Rig, run: &RunId, name: &str) -> String {
    let key = reply_key(run, name);
    let event = Event::new(
        run.clone(),
        0,
        rig.now(),
        key.clone(),
        Origin::Cli,
        EventBody::Reply {
            session_key: Slot {
                step: "build".to_owned(),
                visit: 1,
                member: "coder".to_owned(),
                attempt: 1,
            }
            .session_key(run),
            message: "m-reply".to_owned(),
            escalate: false,
            needs_rework: false,
        },
    );
    rig.store.enqueue(run, &event, &[]).unwrap();
    key
}

fn parked_on_poison(rig: &Rig, run: &RunId) -> bool {
    rig.events(run).iter().any(|e| {
        matches!(&e.body, EventBody::Decision(Decision::Parked { reason, .. })
            if *reason == ReasonCode::PoisonInput)
    })
}

fn waiting(rig: &Rig, run: &RunId) -> usize {
    rig.store.waiting_inputs(run).unwrap().len()
}

fn sql(dir: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open(store_path(dir)).unwrap()
}

/// The exit a person always has: withdraw, and the claim is free.
fn withdraw_and_finish(rig: &mut Rig, run: &RunId, root: &str) {
    bridge::withdraw(&rig.bridge, root, &Rig::requester(), "stop", rig.now()).unwrap();
    let check = run.clone();
    rig.run_until(600, move |rig| rig.is_finished(&check));
    assert_eq!(rig.store.get_claim_holder(), None, "the claim was released");
}

#[test]
fn a_panic_on_an_input_is_counted_then_parks_and_the_withdraw_exit_works() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, root) = rig_with_a_running_coder(dir.path());
    let key = enqueue_reply(&rig, &run, "panics");
    panics::panic_on_input(&key);

    rig.tick();
    assert_eq!(
        rig.store
            .get(&format!("poison/{run}/{key}"))
            .unwrap()
            .as_deref(),
        Some(b"1".as_slice()),
        "the count is in the store, so it survives a restart"
    );
    assert_eq!(waiting(&rig, &run), 1, "not consumed below the budget");
    // The restart must not reset it.
    rig.clock.advance(31_000);
    rig.restart();
    for _ in 0..3 {
        rig.tick();
    }
    panics::clear(&key);
    assert!(parked_on_poison(&rig, &run));
    assert_eq!(waiting(&rig, &run), 0, "the row is consumed");
    assert_eq!(
        rig.store.get(&format!("poison/{run}/{key}")).unwrap(),
        None,
        "and its count goes with it"
    );
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(status.reason, Some(ReasonCode::PoisonInput));
    assert!(
        status.next_action.unwrap_or_default().contains(&key),
        "status names the row"
    );
    withdraw_and_finish(&mut rig, &run, &root);
}

#[test]
fn busy_never_consumes_a_healthy_reply_and_never_counts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _) = rig_with_a_running_coder(dir.path());
    let key = enqueue_reply(&rig, &run, "healthy");
    let lock = sql(dir.path());
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    for _ in 0..6 {
        let report = rig.host.tick(rig.now());
        assert!(report.busy, "the held lock answers Busy");
        assert_eq!(report.folded, 0);
    }
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!(waiting(&rig, &run), 1, "the reply is still there");
    assert_eq!(rig.store.get(&format!("poison/{run}/{key}")).unwrap(), None);
    assert!(!parked_on_poison(&rig, &run));
    rig.tick();
    assert_eq!(waiting(&rig, &run), 0);
    assert!(rig.events(&run).iter().any(|e| e.key == key), "it folded");
}

/// `Busy` on the append alone, with the store free for everything else, so a count would land if one were taken.
#[test]
fn a_busy_append_writes_no_count_however_often_it_happens() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _) = rig_with_a_running_coder(dir.path());
    let key = enqueue_reply(&rig, &run, "busy-append");
    panics::busy_on_append(&key);
    for _ in 0..6 {
        let report = rig.host.tick(rig.now());
        assert!(report.busy);
    }
    assert_eq!(rig.store.get(&format!("poison/{run}/{key}")).unwrap(), None);
    assert_eq!(waiting(&rig, &run), 1, "the reply is still there");
    assert!(!parked_on_poison(&rig, &run));
    panics::clear(&key);
    rig.tick();
    assert!(rig.events(&run).iter().any(|e| e.key == key), "it folded");
}

#[test]
fn a_cas_failure_a_retry_does_not_clear_is_counted_then_parks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, root) = rig_with_a_running_coder(dir.path());
    let key = enqueue_reply(&rig, &run, "cas");
    panics::cas_fails_on(&key);
    rig.tick();
    assert_eq!(
        rig.store
            .get(&format!("poison/{run}/{key}"))
            .unwrap()
            .as_deref(),
        Some(b"1".as_slice())
    );
    for _ in 0..3 {
        rig.tick();
    }
    assert!(parked_on_poison(&rig, &run));
    assert_eq!(waiting(&rig, &run), 0);
    withdraw_and_finish(&mut rig, &run, &root);
}

#[test]
fn a_damaged_inbox_row_counts_is_consumed_and_the_run_moves_on_to_the_row_behind_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, root) = rig_with_a_running_coder(dir.path());
    let damaged = enqueue_reply(&rig, &run, "damaged");
    let healthy = enqueue_reply(&rig, &run, "behind");
    sql(dir.path())
        .execute(
            "UPDATE inbox SET body = 'not json' WHERE key = ?1",
            [&damaged],
        )
        .unwrap();
    for _ in 0..4 {
        rig.tick();
    }
    assert!(
        parked_on_poison(&rig, &run),
        "a row no retry can mend counts"
    );
    assert!(
        rig.events(&run).iter().any(|e| e.key == healthy),
        "the row behind it folded"
    );
    withdraw_and_finish(&mut rig, &run, &root);
}

/// A decision is not an input: folded from the inbox, a row would write the reducer's mind.
#[test]
fn a_decision_in_the_inbox_is_set_aside_and_never_folded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _) = rig_with_a_running_coder(dir.path());
    let event = Event::new(
        run.clone(),
        0,
        rig.now(),
        "input/forged",
        Origin::Cli,
        EventBody::Decision(Decision::Finished {
            outcome: nexus_run_contracts::Outcome::Delivered,
            reason: None,
        }),
    );
    rig.store.enqueue(&run, &event, &[]).unwrap();
    for _ in 0..4 {
        rig.tick();
    }
    assert!(!rig.is_finished(&run), "a row finished the run");
    assert!(parked_on_poison(&rig, &run));
}

#[test]
fn a_panic_on_replay_leaves_the_run_read_only_and_withdraw_releases_with_no_fold() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, root) = rig_with_a_running_coder(dir.path());
    let key = format!("root_opened/{run}");
    panics::panic_on_replay(&key);
    rig.clock.advance(31_000);
    rig.restart();
    rig.tick();
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(
        status.waiting_for.as_deref(),
        Some("a fold that does not panic")
    );
    assert!(rig.store.get_claim_holder().is_some());

    bridge::withdraw(&rig.bridge, &root, &Rig::requester(), "stop", rig.now()).unwrap();
    // A session of the run still lives: the exit waits for it.
    for _ in 0..5 {
        rig.tick();
    }
    assert!(!rig.is_finished(&run), "released with a live session");
    assert!(rig.store.get_claim_holder().is_some());
    let session = format!("{run}/build/v1/coder/a1");
    rig.backend
        .end(&session, nexus_run_contracts::EndCause::Lost);
    rig.tick();
    assert!(rig.is_finished(&run));
    assert_eq!(rig.store.get_claim_holder(), None);
    assert_eq!(
        rig.store.get(&format!("pending/{run}")).unwrap(),
        None,
        "nothing of the commission is left"
    );
    panics::clear(&key);
}

#[test]
fn a_damaged_log_row_is_never_counted_and_withdraw_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, root) = rig_with_a_running_coder(dir.path());
    sql(dir.path())
        .execute(
            "UPDATE events SET body = 'not json' WHERE run = ?1 AND key = ?2",
            [run.as_str(), &format!("claim_taken/{run}")],
        )
        .unwrap();
    rig.clock.advance(31_000);
    rig.restart();
    let healthy = enqueue_reply(&rig, &run, "stuck");
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(waiting(&rig, &run), 1, "nothing folds past a damaged log");
    assert_eq!(
        rig.store.get(&format!("poison/{run}/{healthy}")).unwrap(),
        None
    );
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(
        status.waiting_for.as_deref(),
        Some("a fold that does not panic")
    );

    bridge::withdraw(&rig.bridge, &root, &Rig::requester(), "stop", rig.now()).unwrap();
    rig.backend.end(
        &format!("{run}/build/v1/coder/a1"),
        nexus_run_contracts::EndCause::Lost,
    );
    rig.tick();
    assert!(rig.is_finished(&run));
    assert_eq!(rig.store.get_claim_holder(), None);
}

/// One poisoned run never stops the others (§8.1).
#[test]
fn a_poisoned_run_leaves_the_other_runs_going() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let first = rig.commission("first").run;
    let second = rig.commission("second").run;
    let row = format!("commissioned/{first}");
    panics::panic_on_input(&row);
    let check = second.clone();
    rig.run_until(4_000, move |rig| rig.is_finished(&check));
    panics::clear(&row);
    assert!(!rig.is_finished(&first));
}
