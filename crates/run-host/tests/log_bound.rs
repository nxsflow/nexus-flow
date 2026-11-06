//! The log's size (§7, §12.4, epic DoD 19's engine half): the scripted worst case stays under 5,000 events
//! however many tool calls it makes, and a queued run logs a holder change, never a tick.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nexus_run_contracts::{EventBody, HeartbeatKind, RunId, RunStore, SessionFact};
use nexus_run_host::bridge::ReplyFlags;
use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
use nexus_run_host::test_support::{replies_pass, Rig, CLOCK_START, WORKSPACE};
use ulid::Ulid;

const MINUTE: i64 = 60_000;

/// How long each session of the worst case works, in minutes: a two-hour judge inside a four-hour run, nine
/// session-hours over the seven sessions.
fn minutes_of(info: &SessionInfo) -> i64 {
    match info.step.as_str() {
        "build" => 105,
        "verify" => 45,
        "judge" if info.member == "__synth__" => 15,
        "judge" => 120,
        _ => 15,
    }
}

/// The facts of one session-minute under §12.4's worst shape of 100 overlapping tool calls per session-hour. A
/// busy period closes only when the set of open calls is empty at the end of a minute, so overlapping calls make
/// the most edges as 50 periods of two calls an hour, each opened and closed within its minute (§6.5). Each minute
/// also carries the sidecar's `model_output`, and each busy one its `subprocess_moved` (§7).
///
/// Measured, not asserted: with an edge in EVERY minute, which 100 calls an hour cannot produce while they
/// overlap, nine session-hours log 5,459 events, because an edge makes three decisions and not the one `timer_set`
/// §7 counts. The ticket's note and a board item carry that.
fn facts_of_the_minute(minute: i64) -> Vec<SessionFact> {
    let beat = |beat| SessionFact::Heartbeat { beat };
    if minute % 6 == 5 {
        return vec![beat(HeartbeatKind::ModelOutput)];
    }
    vec![
        beat(HeartbeatKind::ModelOutput),
        beat(HeartbeatKind::CallsOpen),
        beat(HeartbeatKind::SubprocessMoved),
        beat(HeartbeatKind::CallsClosed),
    ]
}

#[test]
fn the_scripted_worst_case_stays_under_5000_events() {
    let dir = tempfile::tempdir().unwrap();
    let done: Arc<Mutex<BTreeMap<String, bool>>> = Arc::default();
    let released = Arc::clone(&done);
    let mut rig = Rig::scripted(
        dir.path(),
        CLOCK_START,
        Box::new(move |info| {
            let key = format!("{}/{}/{}", info.step, info.visit, info.member);
            if *released.lock().unwrap().get(&key).unwrap_or(&false) {
                Cue::Reply {
                    text: "done".to_owned(),
                    flags: ReplyFlags::default(),
                }
            } else {
                Cue::Silent
            }
        }),
    );
    rig.step_ms = MINUTE;
    let run = rig.commission("work").run;
    let mut minutes: BTreeMap<String, i64> = BTreeMap::new();
    for _ in 0..4_000 {
        if rig.is_finished(&run) {
            break;
        }
        rig.tick();
        // A few ticks between minutes: a tick folds what the last one enqueued.
        rig.step_ms = 0;
        for _ in 0..8 {
            rig.tick();
        }
        rig.step_ms = MINUTE;
        for session in live_sessions(&rig) {
            let Some(info) = SessionInfo::parse(&session) else {
                continue;
            };
            let spent = minutes.entry(session.clone()).or_insert(0);
            *spent += 1;
            let seat = format!("{}/{}/{}", info.step, info.visit, info.member);
            if *spent >= minutes_of(&info) {
                done.lock().unwrap().insert(seat, true);
            } else {
                for fact in facts_of_the_minute(*spent) {
                    rig.backend.push_fact(&session, &fact);
                }
            }
        }
    }
    assert!(
        rig.is_finished(&run),
        "the run did not finish: {:?}",
        rig.kinds()
    );
    let sessions = minutes.len();
    let session_minutes: i64 = minutes.values().sum();
    let events = rig.events(&run).len();
    assert_eq!(sessions, 7, "the coding flow's seven sessions");
    assert!(
        session_minutes >= 9 * 60,
        "nine session-hours: {session_minutes} min"
    );
    assert!(
        events < 5_000,
        "{events} events over {session_minutes} session-minutes"
    );
    eprintln!("worst case: {events} events over {session_minutes} session-minutes");
}

fn live_sessions(rig: &Rig) -> Vec<String> {
    nexus_run_host::bridge::live_sessions(&rig.bridge)
        .unwrap()
        .into_iter()
        .filter(|session| rig.spawned().contains(&session.session_key))
        .map(|session| session.session_key)
        .collect()
}

fn claim_busy_count(rig: &Rig, run: &RunId) -> usize {
    rig.events(run)
        .iter()
        .filter(|event| matches!(event.body, EventBody::ClaimBusy { .. }))
        .count()
}

/// A -> B -> A logs three, and many ticks behind one holder log nothing (§6.4 item 2).
#[test]
fn a_queued_run_logs_holder_changes_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let key = format!("claim/{WORKSPACE}");
    let holder_a = RunId::new(Ulid::new());
    let holder_b = RunId::new(Ulid::new());
    let set = |rig: &Rig, from: Option<&RunId>, to: &RunId| {
        rig.store
            .compare_and_set(
                &key,
                from.map(|run| run.as_str().as_bytes()),
                Some(to.as_str().as_bytes()),
            )
            .unwrap();
    };
    set(&rig, None, &holder_a);
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| claim_busy_count(rig, &check) == 1);

    let events = rig.events(&run).len();
    for _ in 0..200 {
        rig.tick();
    }
    assert_eq!(
        rig.events(&run).len(),
        events,
        "a tick behind one holder logged something"
    );

    set(&rig, Some(&holder_a), &holder_b);
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(claim_busy_count(&rig, &run), 2);
    set(&rig, Some(&holder_b), &holder_a);
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(claim_busy_count(&rig, &run), 3, "A -> B -> A logs three");
    let keys: Vec<String> = rig
        .events(&run)
        .iter()
        .filter(|event| matches!(event.body, EventBody::ClaimBusy { .. }))
        .map(|event| event.key.clone())
        .collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().all(|k| k.starts_with("claim_busy/")));
}

/// A queued workspace costs no write per tick: the oldest queued run takes a freed claim, and the younger
/// waits behind the older.
#[test]
fn the_oldest_queued_run_takes_a_freed_claim() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let first = rig.commission("first").run;
    rig.clock.advance(1_000);
    let second = rig.commission("second").run;
    let check = first.clone();
    rig.run_until(4_000, move |rig| rig.is_finished(&check));
    assert!(
        claim_busy_count(&rig, &second) >= 1,
        "the younger waited behind the older"
    );
    rig.run_to_finished(&second);
    assert_eq!(claim_busy_count(&rig, &first), 0, "the older never waited");
}
