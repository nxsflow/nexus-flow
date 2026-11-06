//! `Host::tick` itself (§8.1): the budget, the run boundary, the held lock, and the two `busy_timeout`s (§7).

mod common;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use nexus_run_contracts::{EventBody, RunId, RunStore, StoreError};
use nexus_run_host::bridge::{self, BRIDGE_BUSY_TIMEOUT, HOST_BUSY_TIMEOUT};
use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
use nexus_run_host::test_support::{replies_pass, store_path, Rig, CLOCK_START};
use nexus_run_host::TICK_BUDGET;
use nexus_run_store_sqlite::SqliteRunStore;

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

#[test]
fn the_budget_yields_what_is_left_to_the_next_tick_and_nothing_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let runs: Vec<RunId> = (0..6)
        .map(|n| rig.commission(&format!("run {n}")).run)
        .collect();
    rig.ports.post_delay_ms.store(30, Ordering::SeqCst);
    rig.host.set_budget(Duration::from_millis(40));
    let mut yielded = 0;
    for _ in 0..60 {
        let started = Instant::now();
        let report = rig.host.tick(rig.now());
        let spent = started.elapsed();
        if report.yielded {
            yielded += 1;
        }
        // The budget plus the one post that was running, and a margin for a loaded machine.
        assert!(spent < Duration::from_secs(5), "a tick took {spent:?}");
        rig.clock.advance(100);
        if runs.iter().all(|run| rig.has(run, "root_opened")) {
            break;
        }
    }
    assert!(yielded > 0, "6 posts of 30 ms never ran out a 40 ms budget");
    for run in &runs {
        assert!(rig.has(run, "root_opened"), "{run} was starved");
    }
}

#[test]
fn the_default_budget_is_200_ms_above_the_hosts_busy_timeout() {
    assert_eq!(TICK_BUDGET, Duration::from_millis(200));
    assert_eq!(HOST_BUSY_TIMEOUT, Duration::from_millis(100));
    assert_eq!(BRIDGE_BUSY_TIMEOUT, Duration::from_secs(5));
    assert!(HOST_BUSY_TIMEOUT < TICK_BUDGET);
}

/// The held-lock test: with the write lock held, a tick with work for many runs meets `Busy` once and ends its
/// writing there. Without that, each of the 60 runs would wait out the 100 ms timeout.
#[test]
fn a_held_write_lock_ends_the_writing_for_this_tick_within_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let runs: Vec<RunId> = (0..60)
        .map(|n| rig.commission(&format!("run {n}")).run)
        .collect();
    let lock = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let report = rig.host.tick(rig.now());
    let spent = started.elapsed();
    assert!(report.busy, "the held lock answers Busy");
    assert_eq!(report.folded, 0);
    // One timeout of 100 ms for the first write, however many runs wait; 60 of them would be 6 s.
    assert!(spent < Duration::from_secs(3), "the tick took {spent:?}");
    lock.execute_batch("ROLLBACK").unwrap();
    rig.host.tick(rig.now());
    assert!(runs.iter().all(|run| rig.has(run, "commissioned")));
}

#[test]
fn the_hosts_connection_answers_busy_after_its_timeout_and_the_bridges_waits_longer() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let lock = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        rig.store.compare_and_set("x/host", None, Some(b"1")),
        Err(StoreError::Busy),
        "the host's connection gives up"
    );
    // The bridge's connection rides out a stall the host's does not.
    let bridge_store =
        SqliteRunStore::open(&store_path(dir.path()), BRIDGE_BUSY_TIMEOUT).expect("it opens");
    let release = std::thread::spawn(move || {
        std::thread::sleep(HOST_BUSY_TIMEOUT * 3);
        lock.execute_batch("ROLLBACK").unwrap();
    });
    bridge_store
        .compare_and_set("x/bridge", None, Some(b"1"))
        .expect("the bridge waited");
    release.join().unwrap();
}

/// A panic in one run's step stays in that run: logged once with its id, skipped until the next tick.
#[test]
fn a_panic_in_a_step_stays_in_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    *rig.ports.panic_on_body.lock().unwrap() = Some("PANICS".to_owned());
    let bad = rig.commission("PANICS").run;
    rig.clock.advance(1_000);
    let good = rig.commission("works").run;
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    rig.host.set_log_sink(std::sync::Arc::new(move |line| {
        sink.lock().unwrap().push(line.to_owned())
    }));
    for _ in 0..30 {
        rig.tick();
    }
    assert!(rig.has(&good, "root_opened"), "the good run went on");
    assert!(!rig.has(&bad, "root_opened"));
    assert!(lines
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains(bad.as_str()) && line.contains("panicked")));
    // The port recovers and the same effect, still owed, runs.
    *rig.ports.panic_on_body.lock().unwrap() = None;
    let check = bad.clone();
    rig.run_until(30, move |rig| rig.has(&check, "root_opened"));
}

#[test]
fn a_refused_post_is_retried_and_status_says_the_chat_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let run = rig.commission("work").run;
    rig.ports.refuse_posts.store(true, Ordering::SeqCst);
    for _ in 0..5 {
        rig.tick();
    }
    assert!(!rig.has(&run, "root_opened"));
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(status.waiting_for.as_deref(), Some("the chat store"));
    rig.ports.refuse_posts.store(false, Ordering::SeqCst);
    let check = run.clone();
    rig.run_until(20, move |rig| rig.has(&check, "root_opened"));
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_ne!(status.waiting_for.as_deref(), Some("the chat store"));
}

/// An idle tick costs no write at all (§7), measured on the file: `data_version` moves when ANOTHER connection
/// commits. The clock stands still, so no lease is due for renewal and no `host-seen`; those are the only writes
/// an idle workspace owes, and each is at most once per half an expiry.
#[test]
fn an_idle_tick_writes_nothing_to_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(100, move |rig| {
        rig.events(&check)
            .iter()
            .any(|event| matches!(event.body, EventBody::SessionFact { .. }))
    });
    // What is on its way folds first.
    rig.step_ms = 0;
    for _ in 0..5 {
        rig.tick();
    }
    let probe = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    let version = || -> i64 {
        probe
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .unwrap()
    };
    let before = version();
    for _ in 0..50 {
        let report = rig.tick();
        assert_eq!(report.folded, 0);
    }
    assert_eq!(version(), before, "an idle tick wrote to the store");
}

/// The facts reach the inbox before the offset moves (§9): a fact the store refused is read again, never skipped.
#[test]
fn a_fact_the_store_refused_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(100, move |rig| {
        rig.events(&check)
            .iter()
            .any(|event| matches!(event.body, EventBody::SessionFact { .. }))
    });
    let key = format!("{run}/build/v1/coder/a1");
    let offset = dir.path().join("agent-logs/v2").join(&key).join("offset");
    let before = std::fs::read_to_string(&offset).unwrap();
    rig.backend.push_fact(
        &key,
        &nexus_run_contracts::SessionFact::Heartbeat {
            beat: nexus_run_contracts::HeartbeatKind::ModelOutput,
        },
    );
    let lock = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let report = rig.host.tick(rig.now());
    assert!(report.busy, "the held lock refused the fact");
    assert_eq!(
        std::fs::read_to_string(&offset).unwrap(),
        before,
        "the offset moved past a fact that is not in the inbox"
    );
    lock.execute_batch("ROLLBACK").unwrap();
    let facts = |rig: &Rig| {
        rig.events(&run)
            .iter()
            .filter(|event| {
                matches!(
                    &event.body,
                    EventBody::SessionFact {
                        fact: nexus_run_contracts::SessionFact::Heartbeat { .. },
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(facts(&rig), 0);
    rig.host.tick(rig.now());
    assert_eq!(facts(&rig), 1, "the fact was delivered after all");
    assert_ne!(std::fs::read_to_string(&offset).unwrap(), before);
}
