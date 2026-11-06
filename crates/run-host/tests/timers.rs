//! Timers and clocks (§12.6, epic DoD 11): a timer 80 minutes ahead fires exactly once, at its instant, across a
//! restart, a forward jump and a clock restarted behind; and a healthy slot survives the wake, `Nudged` and
//! `Starting` included. Then the stop table across a restart (§6.5).

mod common;

use nexus_run_contracts::{
    Decision, EventBody, HeartbeatKind, ReasonCode, RunId, Secs, SessionFact, TimerPurpose,
};
use nexus_run_engine::{slot_timer, SlotState};
use nexus_run_host::bridge;
use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
use nexus_run_host::test_support::{Rig, StoreProbe, CLOCK_START};

const QUIET_AFTER: u64 = 80 * 60;
const STEP: i64 = 30_000;

fn silent_coder(info: &SessionInfo) -> Cue {
    match info.step.as_str() {
        "build" => Cue::Silent,
        _ => Cue::Silent,
    }
}

fn hung_coder(info: &SessionInfo) -> Cue {
    match info.step.as_str() {
        "build" => Cue::Hung,
        _ => Cue::Silent,
    }
}

/// A rig with the coder `Running`, its `quiet` timer 80 minutes ahead.
fn coder_running(dir: &std::path::Path) -> (Rig, RunId, String, nexus_run_contracts::Timestamp) {
    let mut rig = Rig::scripted(dir, CLOCK_START, Box::new(silent_coder));
    rig.policy.quiet_after = Secs(QUIET_AFTER);
    rig.step_ms = 1_000;
    let run = rig.commission("work").run;
    let key = format!("{run}/build/v1/coder/a1");
    let probe = key.clone();
    let check = run.clone();
    rig.run_until(500, move |rig| {
        let state = rig.state(&check);
        state
            .find(&probe)
            .is_some_and(|index| matches!(state.slots[index].state, SlotState::Running { .. }))
    });
    let fire_at = armed_quiet(&rig, &run, &key).expect("the quiet timer is armed");
    (rig, run, key, fire_at)
}

fn quiet_timer(key: &str) -> String {
    let info = SessionInfo::parse(key).unwrap();
    slot_timer(
        &TimerPurpose::Quiet,
        &nexus_run_contracts::Slot {
            step: info.step,
            visit: info.visit,
            member: info.member,
            attempt: info.attempt,
        },
    )
}

fn armed_quiet(rig: &Rig, run: &RunId, key: &str) -> Option<nexus_run_contracts::Timestamp> {
    rig.state(run)
        .timers
        .get(&quiet_timer(key))
        .map(|armed| armed.fire_at)
}

/// Every `timer_fired` of this timer id, as (`at`, `fire_at`).
fn firings(rig: &Rig, run: &RunId, timer: &str) -> Vec<(i64, i64)> {
    rig.events(run)
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::TimerFired { timer: t, fire_at } if t == timer => {
                Some((event.at.unix_millis(), fire_at.unix_millis()))
            }
            _ => None,
        })
        .collect()
}

fn was_stopped(rig: &Rig, run: &RunId) -> bool {
    rig.events(run).iter().any(|event| {
        matches!(
            &event.body,
            EventBody::Decision(Decision::Liveness { reason, .. })
                if *reason == ReasonCode::Hung || *reason == ReasonCode::StartFailed
        )
    })
}

#[test]
fn eighty_minutes_ahead_fires_once_at_its_instant_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, key, fire_at) = coder_running(dir.path());
    let timer = quiet_timer(&key);
    // In jumps, with a restart in the middle: a new host on the same store, the clock continued.
    rig.clock.advance(40 * 60 * 1_000);
    for _ in 0..5 {
        rig.tick();
    }
    assert!(firings(&rig, &run, &timer).is_empty(), "fired early");
    rig.restart();
    rig.clock.advance(35 * 60 * 1_000);
    rig.step_ms = STEP;
    let start = rig.now().unix_millis();
    let (check, name) = (run.clone(), timer.clone());
    rig.run_until(400, move |rig| !firings(rig, &check, &name).is_empty());
    let fired = firings(&rig, &run, &timer);
    assert_eq!(fired.len(), 1);
    let (at, named) = fired[0];
    assert_eq!(named, fire_at.unix_millis(), "it names its own instant");
    assert!(at >= named, "it fired at or after its instant");
    assert!(
        at - start < 6 * 60 * 1_000 + 31_000,
        "and not long after it"
    );
    // The next rung: quiet, then a nudge; the healthy session answers it.
    let (check, probe) = (run.clone(), key.clone());
    rig.run_until(60, move |rig| {
        rig.events(&check)
            .iter()
            .any(|e| matches!(&e.body, EventBody::Nudged { session_key } if *session_key == probe))
    });
    for _ in 0..40 {
        rig.tick();
    }
    assert_eq!(
        firings(&rig, &run, &timer).len(),
        1,
        "fired once, whatever came after"
    );
    assert!(!was_stopped(&rig, &run), "a healthy session was stopped");
}

/// A timer is how a stuck run gets out, so it does not wait for a tick with budget left: a budget of zero is the
/// machine that never has any.
#[test]
fn a_due_timer_fires_with_the_budget_gone_before_the_first_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, key, _) = coder_running(dir.path());
    let timer = quiet_timer(&key);
    rig.host.set_budget(std::time::Duration::ZERO);
    rig.clock.advance(6 * 3_600 * 1_000);
    let (check, name) = (run.clone(), timer.clone());
    rig.run_until(5, move |rig| !firings(rig, &check, &name).is_empty());
    assert_eq!(firings(&rig, &run, &timer).len(), 1);
}

#[test]
fn a_forward_jump_of_hours_fires_once_and_a_healthy_slot_survives_the_wake() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, key, _) = coder_running(dir.path());
    let timer = quiet_timer(&key);
    rig.clock.advance(6 * 3_600 * 1_000);
    for _ in 0..80 {
        rig.tick();
    }
    assert_eq!(firings(&rig, &run, &timer).len(), 1, "due once, fired once");
    assert!(
        !was_stopped(&rig, &run),
        "the wake stopped a healthy session"
    );
}

#[test]
fn a_clock_restarted_behind_still_fires_once_when_it_passes_the_instant() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, key, fire_at) = coder_running(dir.path());
    let timer = quiet_timer(&key);
    rig.clock.advance(-3_600 * 1_000);
    rig.restart();
    rig.step_ms = STEP;
    for _ in 0..20 {
        rig.tick();
    }
    assert!(
        firings(&rig, &run, &timer).is_empty(),
        "fired behind its instant"
    );
    let (check, name) = (run.clone(), timer.clone());
    rig.run_until(2_000, move |rig| !firings(rig, &check, &name).is_empty());
    for _ in 0..40 {
        rig.tick();
    }
    let fired = firings(&rig, &run, &timer);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].1, fire_at.unix_millis());
    assert!(!was_stopped(&rig, &run));
}

/// The wake rule (§6.5): a destructive timer due across a jump is put off, and a session that shows any fact
/// in that time was never in danger. The first case is `Nudged`, the second `Starting`.
#[test]
fn a_nudged_slot_survives_a_jump_past_its_confirm_timer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, key, fire_at) = coder_running(dir.path());
    let millis = fire_at.unix_millis() - rig.now().unix_millis();
    rig.clock.advance(millis);
    // Host ticks only: the session has been nudged and has not answered yet.
    let (check, probe) = (run.clone(), key.clone());
    for _ in 0..100 {
        let now = rig.now();
        rig.host.tick(now);
        let state = rig.state(&check);
        if state
            .find(&probe)
            .is_some_and(|i| matches!(state.slots[i].state, SlotState::Nudged { .. }))
        {
            break;
        }
    }
    let state = rig.state(&run);
    let index = state.find(&key).unwrap();
    assert!(matches!(state.slots[index].state, SlotState::Nudged { .. }));
    // The machine sleeps through the confirm window.
    rig.clock.advance(5 * 3_600 * 1_000);
    for _ in 0..80 {
        rig.tick();
    }
    assert!(
        !was_stopped(&rig, &run),
        "the wake stopped a nudged session"
    );
    let state = rig.state(&run);
    let index = state.find(&key).unwrap();
    assert!(
        matches!(state.slots[index].state, SlotState::Running { .. }),
        "the session answered: {:?}",
        state.slots[index].state
    );
}

#[test]
fn a_starting_slot_survives_a_jump_past_its_start_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent_coder));
    rig.policy.quiet_after = Secs(QUIET_AFTER);
    let run = rig.commission("work").run;
    let key = format!("{run}/build/v1/coder/a1");
    // Host ticks only, so the session has not reported `started`.
    for _ in 0..50 {
        let now = rig.now();
        rig.host.tick(now);
        rig.clock.advance(1_000);
        let state = rig.state(&run);
        if state
            .find(&key)
            .is_some_and(|i| matches!(state.slots[i].state, SlotState::Starting { .. }))
            && rig.spawned().contains(&key)
        {
            break;
        }
    }
    assert!(rig.spawned().contains(&key));
    rig.clock.advance(5 * 3_600 * 1_000);
    for _ in 0..80 {
        rig.tick();
    }
    assert!(
        !was_stopped(&rig, &run),
        "the wake stopped a starting session"
    );
    assert_eq!(
        rig.spawned().iter().filter(|k| **k == key).count(),
        1,
        "one spawn for one key"
    );
    let state = rig.state(&run);
    let index = state.find(&key).unwrap();
    assert!(matches!(
        state.slots[index].state,
        SlotState::Running { .. }
    ));
}

/// A session that shows no fact after the wake takes the rung: the wake rule puts a stop off, never away.
#[test]
fn a_silent_session_still_takes_the_rung_after_the_wake() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(hung_coder));
    rig.policy.quiet_after = Secs(600);
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.step_ms = 5_000;
    rig.run_until(800, move |rig| was_stopped(rig, &check));
}

/// Exactly the table's signals across a restart (§6.5): 10 minutes in `Stopping` under the simulated clock give
/// two SIGTERMs and no kill, then `parked { would_not_stop }` with the claim held, and only a withdrawal kills.
#[test]
fn ten_minutes_in_stopping_give_two_sigterms_and_no_kill_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(hung_coder));
    rig.policy = nexus_run_contracts::LivenessPolicy {
        quiet_after: Secs(60),
        confirm_within: Secs(60),
        stop_grace: Secs(300),
        ..rig.policy.clone()
    };
    rig.step_ms = 5_000;
    let receipt = rig.commission("work");
    let run = receipt.run;
    let key = format!("{run}/build/v1/coder/a1");
    let check = run.clone();
    let probe = key.clone();
    rig.run_until(800, move |rig| {
        rig.backend
            .timeline()
            .iter()
            .any(|e| e.session_key == probe && e.what == "term")
    });
    // The restart, between the first signal and the second.
    rig.restart();
    let probe = key.clone();
    rig.run_until(800, move |rig| {
        rig.backend
            .timeline()
            .iter()
            .filter(|e| e.session_key == probe && e.what == "term")
            .count()
            == 2
    });
    let parks = |rig: &Rig| {
        rig.events(&check).iter().any(|e| {
            matches!(&e.body, EventBody::Decision(Decision::Parked { reason, .. })
                if *reason == ReasonCode::WouldNotStop)
        })
    };
    let probe_run = run.clone();
    rig.run_until(800, move |rig| {
        rig.events(&probe_run).iter().any(|e| {
            matches!(&e.body, EventBody::Decision(Decision::Parked { reason, .. })
                if *reason == ReasonCode::WouldNotStop)
        })
    });
    assert!(parks(&rig));
    // More time changes nothing: no third signal, ever, from the host alone.
    for _ in 0..200 {
        rig.tick();
    }
    let timeline = common::Timeline(rig.backend.timeline());
    assert_eq!(
        timeline.signals(&key),
        ["term", "term"],
        "the table's signals"
    );
    assert!(rig.store.get_claim_holder().is_some(), "the claim is held");

    // A person's act: the kill comes through a withdrawal, by the table.
    bridge::withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    rig.run_to_finished(&run);
    let timeline = common::Timeline(rig.backend.timeline());
    assert_eq!(timeline.signals(&key), ["term", "term", "kill"]);
}

/// `model_output` and the facts of a busy period keep a slot off the ladder in the host as in the reducer: a
/// 30-minute tool call with movement is never nudged.
#[test]
fn a_tool_call_whose_subprocesses_move_is_never_nudged() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent_coder));
    rig.policy.quiet_after = Secs(600);
    rig.step_ms = 60_000;
    let run = rig.commission("work").run;
    let key = format!("{run}/build/v1/coder/a1");
    let check = run.clone();
    let probe = key.clone();
    rig.step_ms = 1_000;
    rig.run_until(200, move |rig| {
        let state = rig.state(&check);
        state
            .find(&probe)
            .is_some_and(|i| matches!(state.slots[i].state, SlotState::Running { .. }))
    });
    rig.step_ms = 60_000;
    let beat = |kind| SessionFact::Heartbeat { beat: kind };
    rig.backend.push_fact(&key, &beat(HeartbeatKind::CallsOpen));
    for _ in 0..90 {
        rig.backend
            .push_fact(&key, &beat(HeartbeatKind::SubprocessMoved));
        rig.tick();
        rig.tick();
    }
    assert!(
        !rig.events(&run)
            .iter()
            .any(|e| matches!(e.body, EventBody::Nudged { .. })),
        "a working session was nudged"
    );
}
