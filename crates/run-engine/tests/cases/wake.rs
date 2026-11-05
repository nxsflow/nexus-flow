//! The wake rule (§12.6): after a jump larger than `wake_jump` a destructive timer waits one more sample,
//! so a healthy slot survives a laptop waking, and one still silent after it takes the rung.

use nexus_run_contracts::{EndCause, HeartbeatKind, ReasonCode, Secs};
use nexus_run_engine::{RunView, SlotState};

use crate::script::Script;

const HOURS: u64 = 3 * 3600;

fn running(s: &mut Script) -> String {
    s.to_first_session();
    let key = s.latest_key("build", None);
    s.input(s.started(&key));
    key
}

/// Fires a timer `HOURS` after it was due, as a host does that just woke.
fn fire_late(s: &mut Script, timer: &str) {
    let due = s.state.timers[timer].fire_at;
    let input = s.fired(timer);
    s.now = due.plus(Secs(HOURS));
    s.input(input);
}

cases! {
    fn a_starting_slot_survives_the_wake_and_a_start_after_it_counts(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        fire_late(s, "start_deadline/build/v1/coder/a1");
        s.expect(&["timer_set start_deadline/build/v1/coder/a1 +120s start_deadline"]);
        assert!(matches!(s.slot_state(&key), SlotState::Starting { .. }));
        s.input(s.started(&key));
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
    }

    fn a_starting_slot_still_silent_one_sample_after_the_wake_takes_the_rung(s) {
        s.to_first_session();
        fire_late(s, "start_deadline/build/v1/coder/a1");
        s.fire("start_deadline/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 start_failed since=+0s",
        ]);
    }

    fn a_nudged_slot_survives_the_wake_and_a_fact_after_it_returns_it_to_running(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.input(s.nudged(&key));
        fire_late(s, "confirm/build/v1/coder/a1");
        s.expect(&["timer_set confirm/build/v1/coder/a1 +120s confirm"]);
        assert!(matches!(s.slot_state(&key), SlotState::Nudged { .. }));
        s.input(s.beat(&key, HeartbeatKind::ModelOutput));
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
    }

    fn a_quiet_timer_due_at_the_wake_fires_once_and_only_takes_the_harmless_rung(s) {
        let key = running(s);
        fire_late(s, "quiet/build/v1/coder/a1");
        s.expect(&[
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
        assert!(matches!(s.slot_state(&key), SlotState::Quiet { .. }));
    }

    fn a_second_grace_due_at_the_wake_parks_only_after_one_more_sample(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 1, nexus_run_contracts::StopKind::Term));
        s.fire("stop_grace/1/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 2, nexus_run_contracts::StopKind::Term));
        fire_late(s, "stop_grace/2/build/v1/coder/a1");
        s.expect(&["timer_set stop_grace/2/build/v1/coder/a1 +120s stop_grace"]);
        assert!(s.state.condition.is_none());
        s.input(s.ended(&key, EndCause::StoppedByHost));
        assert!(!matches!(s.state.view(), RunView::Parked { .. }));
    }

    fn the_tree_window_at_a_wake_waits_one_more_sample(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(s.claim_taken());
        fire_late(s, "tree_window");
        s.expect(&["timer_set tree_window +120s tree_window"]);
        assert!(!s.state.tree_cancel_owed);
        s.fire("tree_window").expect(&["parked run tree_refused"]);
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::TreeRefused });
    }

    fn a_timer_fired_inside_the_jump_is_no_wake(s) {
        s.to_first_session();
        let timer = "start_deadline/build/v1/coder/a1";
        let due = s.state.timers[timer].fire_at;
        let input = s.fired(timer);
        s.now = due.plus(Secs(120)); // exactly wake_jump late: not larger
        s.input(input);
        s.expect(&["liveness build/v1/coder/a1 start_failed since=+0s"]);
    }

    fn a_clock_set_behind_fires_a_timer_when_it_passes_it_and_defers_nothing(s) {
        s.to_first_session();
        let timer = "start_deadline/build/v1/coder/a1";
        let due = s.state.timers[timer].fire_at;
        let input = s.fired(timer);
        s.now = due;
        s.input(input);
        s.expect(&["liveness build/v1/coder/a1 start_failed since=+0s"]);
    }
}
