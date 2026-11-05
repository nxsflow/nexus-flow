//! Three signs of life, and the destructive side only when all three are silent (§6.5, §12.7).

use nexus_run_contracts::{EndCause, HeartbeatKind::*, Secs, StopKind};
use nexus_run_engine::{Effect, SlotState};

use crate::script::Script;

fn running(s: &mut Script) -> String {
    s.reach_first_session();
    let key = s.latest_key("build", None);
    s.input(s.started(&key)).expect(&[
        "timer_cleared start_deadline/build/v1/coder/a1",
        "timer_set quiet/build/v1/coder/a1 +600s quiet",
    ]);
    key
}

cases! {
    fn model_output_re_arms_quiet(s) {
        let key = running(s);
        s.advance(300);
        s.input(s.beat(&key, ModelOutput)).expect(&["timer_set quiet/build/v1/coder/a1 +600s quiet"]);
    }

    fn no_output_for_quiet_after_is_quiet_and_owes_a_nudge(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
        assert!(matches!(s.effects_of(&key)[0], Effect::NudgeSession { .. }));
        s.input(s.nudged(&key)).expect(&["timer_set confirm/build/v1/coder/a1 +300s confirm"]);
        assert!(s.effects_of(&key).is_empty(), "the nudge is sent once");
        assert!(matches!(s.slot_state(&key), SlotState::Nudged { .. }));
    }

    fn any_fact_returns_a_nudged_slot_to_running(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.input(s.nudged(&key));
        s.input(s.beat(&key, ModelOutput)).expect(&[
            "timer_cleared confirm/build/v1/coder/a1",
            "timer_set quiet/build/v1/coder/a1 +600s quiet",
        ]);
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
    }

    fn a_usage_fact_also_counts_as_a_sign_of_life(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.input(s.fact(&key, nexus_run_contracts::SessionFact::Usage {
            model: "m".into(), input_tokens: 1, output_tokens: 1,
            cache_read_tokens: 0, cache_write_tokens: 0, turns: 1,
        })).expect(&[
            "timer_cleared confirm/build/v1/coder/a1",
            "timer_set quiet/build/v1/coder/a1 +600s quiet",
        ]);
    }

    fn no_fact_after_the_nudge_is_hung_and_stops_it(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.input(s.nudged(&key));
        s.fire("confirm/build/v1/coder/a1").expect(&["liveness build/v1/coder/a1 hung since=+0s"]);
        assert_eq!(s.effects_of(&key), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Term, nth: 1,
        }]);
    }

    fn a_slot_in_a_thirty_minute_call_with_moving_subprocesses_is_never_nudged(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen)).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "timer_set subprocess_idle/build/v1/coder/a1 +1800s subprocess_idle",
            "timer_set call_bound/build/v1/coder/a1 +10800s call_bound",
        ]);
        for _ in 0..30 {
            s.advance(60);
            s.input(s.beat(&key, SubprocessMoved));
            assert!(s.effects_of(&key).is_empty());
        }
        assert!(matches!(s.slot_state(&key), SlotState::Running { open_call: Some(_), .. }));
    }

    fn the_first_sample_clears_the_bound_for_a_host_that_samples(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.advance(60);
        s.input(s.beat(&key, SubprocessMoved)).expect(&[
            "timer_cleared call_bound/build/v1/coder/a1",
            "timer_set subprocess_idle/build/v1/coder/a1 +1800s subprocess_idle",
        ]);
    }

    fn a_call_at_call_bound_plus_an_hour_with_sparse_movement_runs_on_a_sampling_host(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        for _ in 0..16 {
            s.advance(Secs::minutes(15).0);
            s.input(s.beat(&key, SubprocessMoved));
        }
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
        assert!(!s.state.timers.contains_key("call_bound/build/v1/coder/a1"));
    }

    fn the_same_call_with_no_movement_is_nudged_then_stopped(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.fire("subprocess_idle/build/v1/coder/a1").expect(&[
            "timer_cleared call_bound/build/v1/coder/a1",
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
        assert!(matches!(s.effects_of(&key)[0], Effect::NudgeSession { .. }));
        s.input(s.nudged(&key));
        s.fire("confirm/build/v1/coder/a1").expect(&["liveness build/v1/coder/a1 hung since=+0s"]);
    }

    fn a_call_past_call_bound_is_nudged_on_a_blind_host_and_only_after_it(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.input(s.beat(&key, Blind)).expect(&[
            "timer_cleared subprocess_idle/build/v1/coder/a1",
            "timer_set call_bound/build/v1/coder/a1 +10800s call_bound",
        ]);
        s.advance(Secs::minutes(60).0);
        assert!(s.effects_of(&key).is_empty(), "blind, it is never taken before the bound");
        s.fire("call_bound/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
        assert!(matches!(s.effects_of(&key)[0], Effect::NudgeSession { .. }));
    }

    fn call_bound_firing_on_a_host_not_known_to_be_blind_takes_no_rung(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.fire("call_bound/build/v1/coder/a1").expect_none();
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
    }

    fn a_call_that_returns_to_running_re_arms_its_bound_from_that_moment(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.input(s.beat(&key, Blind));
        s.fire("call_bound/build/v1/coder/a1");
        s.input(s.nudged(&key));
        s.advance(120);
        s.input(s.beat(&key, ModelOutput)).expect(&[
            "timer_cleared confirm/build/v1/coder/a1",
            "timer_set call_bound/build/v1/coder/a1 +10800s call_bound",
        ]);
        assert!(matches!(s.slot_state(&key), SlotState::Running { open_call: Some(_), .. }));
    }

    fn a_closed_busy_period_clears_its_timers_and_re_arms_quiet(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.advance(60);
        s.input(s.beat(&key, CallsClosed)).expect(&[
            "timer_cleared subprocess_idle/build/v1/coder/a1",
            "timer_cleared call_bound/build/v1/coder/a1",
            "timer_set quiet/build/v1/coder/a1 +600s quiet",
        ]);
    }

    fn model_output_inside_a_call_moves_no_timer(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.input(s.beat(&key, ModelOutput)).expect_none();
    }

    fn a_second_calls_open_keeps_the_period_it_is_inside(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        let opened = match s.slot_state(&key) {
            SlotState::Running { open_call, .. } => open_call,
            other => panic!("{other:?}"),
        };
        s.advance(60);
        s.input(s.beat(&key, CallsOpen));
        match s.slot_state(&key) {
            SlotState::Running { open_call, .. } => assert_eq!(open_call, opened),
            other => panic!("{other:?}"),
        }
    }

    fn a_killed_leader_is_lost_at_once_with_or_without_an_open_call(s) {
        let key = running(s);
        s.input(s.beat(&key, CallsOpen));
        s.input(s.ended(&key, EndCause::Lost)).expect(&[
            "timer_cleared call_bound/build/v1/coder/a1",
            "timer_cleared subprocess_idle/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
    }

    fn a_heartbeat_on_a_slot_that_already_replied_changes_nothing(s) {
        let key = running(s);
        s.input(s.reply(&key, "m"));
        s.input(s.beat(&key, ModelOutput)).expect(&["ignored out_of_state"]);
    }
}
