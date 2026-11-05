//! What the store's tests share: fixed ids and instants, and one constructor per event they append.
#![allow(dead_code)] // Every test binary compiles this file and uses a different part of it.

use std::path::Path;
use std::time::Duration;

use nexus_run_contracts::{
    Decision, Event, EventBody, FlowRef, LivenessPolicy, Origin, Outcome, RunId, RunStore,
    TimerPurpose, Timestamp, LOCAL_TENANT,
};
use nexus_run_store_sqlite::SqliteRunStore;

/// The host's timeout. A test that races writers retries a `Busy`, as the host's next tick would.
pub const HOST_TIMEOUT: Duration = Duration::from_millis(100);

pub fn open(path: &Path) -> SqliteRunStore {
    SqliteRunStore::open(path, HOST_TIMEOUT).unwrap()
}

pub fn temp_store() -> (tempfile::TempDir, SqliteRunStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("run.sqlite"));
    (dir, store)
}

pub fn run_id() -> RunId {
    RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap()
}

pub fn other_run_id() -> RunId {
    RunId::parse("r-01JBBBBBBBBBBBBBBBBBBBBBBB").unwrap()
}

pub const ROOT_THREAD: &str = "m-01JCCCCCCCCCCCCCCCCCCCCCCC";

pub fn t0() -> Timestamp {
    Timestamp::parse("2026-11-05T10:00:00.000Z").unwrap()
}

/// `t0` plus some milliseconds.
pub fn at(millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(t0().unix_millis() + millis)
}

pub fn commissioned_body(run: &RunId) -> EventBody {
    EventBody::Commissioned {
        run: run.clone(),
        workspace_id: "6j6v".into(),
        tenant: LOCAL_TENANT.into(),
        flow: FlowRef {
            hash: "sha256:00".into(),
        },
        policy: LivenessPolicy::default(),
        root_thread: ROOT_THREAD.into(),
        requester: "requester".into(),
    }
}

pub fn commissioned(run: &RunId) -> Event {
    Event::new(
        run.clone(),
        0,
        t0(),
        format!("commission/{ROOT_THREAD}"),
        Origin::Cli,
        commissioned_body(run),
    )
}

pub fn event(run: &RunId, key: &str, body: EventBody) -> Event {
    Event::new(run.clone(), 0, t0(), key, Origin::Host, body)
}

/// An input with a key of its own and nothing the store derives from.
pub fn input(run: &RunId, key: &str) -> Event {
    event(run, key, EventBody::ClaimTaken)
}

pub fn finished(run: &RunId, key: &str) -> Event {
    let body = EventBody::Decision(Decision::Finished {
        outcome: Outcome::Delivered,
        reason: None,
    });
    Event::new(run.clone(), 0, t0(), key, Origin::Reducer, body)
}

pub fn timer_set(run: &RunId, key: &str, timer: &str, fire_at: Timestamp) -> Event {
    let body = EventBody::Decision(Decision::TimerSet {
        timer: timer.into(),
        fire_at,
        purpose: TimerPurpose::Quiet,
    });
    Event::new(run.clone(), 0, t0(), key, Origin::Reducer, body)
}

pub fn timer_cleared(run: &RunId, key: &str, timer: &str) -> Event {
    let body = EventBody::Decision(Decision::TimerCleared {
        timer: timer.into(),
    });
    Event::new(run.clone(), 0, t0(), key, Origin::Reducer, body)
}

pub fn timer_fired(run: &RunId, timer: &str, fire_at: Timestamp) -> Event {
    let body = EventBody::TimerFired {
        timer: timer.into(),
        fire_at,
    };
    let key = format!("timer/{timer}/{fire_at}");
    Event::new(run.clone(), 0, t0(), key, Origin::Timer, body)
}

/// A store holding `run`, commissioned and appended at seq 1, owned by `host-a` from `t0`. Returns its
/// fence.
pub fn owned_run(store: &SqliteRunStore, run: &RunId) -> u64 {
    let lease = store.acquire_or_renew(run, "host-a", t0()).unwrap();
    store
        .append(run, 0, lease.epoch, &[commissioned(run)], &[], &[])
        .unwrap();
    lease.epoch
}

/// The keys of a run's log, in order.
pub fn log_keys(store: &SqliteRunStore, run: &RunId) -> Vec<String> {
    let events = store.load(run, 0).unwrap();
    events.into_iter().map(|e| e.key).collect()
}
