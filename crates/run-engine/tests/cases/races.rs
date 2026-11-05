//! Inputs that arrive late or twice, and what is left for a person to read in the log (§6.2, §12.1).

use nexus_run_contracts::{EndCause, EventBody, Secs, StopKind};
use nexus_run_engine::{Effect, SlotState};

cases! {
    fn a_late_reply_after_a_restart_is_ignored_and_the_new_attempt_stands(s) {
        s.to_first_session();
        let a1 = s.key("build", 1, "coder", 1);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::Exit { status: None }));
        let a2 = s.key("build", 1, "coder", 2);
        assert!(matches!(s.slot_state(&a2), SlotState::Starting { .. }));
        s.input(s.reply(&a1, "m-old")).expect(&["ignored late_reply_after_skip"]);
        assert!(matches!(s.slot_state(&a2), SlotState::Starting { .. }));
        assert_eq!(s.state.current_step.as_deref(), Some("build"));
    }

    fn a_reply_while_the_engine_is_stopping_the_attempt_is_ignored_as_late(s) {
        s.to_first_session();
        let a1 = s.key("build", 1, "coder", 1);
        s.input(s.started(&a1));
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.reply(&a1, "m-too-late")).expect(&["ignored late_reply_after_skip"]);
    }

    fn a_duplicate_reply_is_ignored_and_an_old_verdict_is_never_counted_twice(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-1"));
        let steps = s.state.finished_steps.len();
        s.input(s.reply(&key, "m-2")).expect(&["ignored duplicate_reply"]);
        assert_eq!(s.state.finished_steps.len(), steps);
        assert_eq!(s.state.verdict_of("build").unwrap().message, "m-1");
    }

    fn a_started_for_a_superseded_attempt_owes_its_stop_and_changes_nothing_else(s) {
        s.to_first_session();
        let a1 = s.key("build", 1, "coder", 1);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::Lost));
        let a2 = s.key("build", 1, "coder", 2);
        let before = s.state.latest("build", 1, "coder").unwrap().clone();
        s.input(s.started(&a1)).expect_none();
        assert_eq!(s.effects_of(&a1), vec![Effect::StopSession {
            session_key: a1.clone(), kind: StopKind::Term, nth: 1,
        }]);
        assert_eq!(s.state.latest("build", 1, "coder"), Some(&before));
        assert!(matches!(s.effects_of(&a2)[0], Effect::StartSession { .. }));
        // Its confirmed end restarts nobody: the new attempt already stands.
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect_none();
        assert_eq!(s.state.latest("build", 1, "coder").unwrap().slot.attempt, 2);
    }

    fn a_started_on_a_slot_that_already_runs_changes_nothing(s) {
        s.to_first_session();
        let a1 = s.key("build", 1, "coder", 1);
        s.input(s.started(&a1));
        s.input(s.started(&a1)).expect(&["ignored out_of_state"]);
    }

    fn a_fact_for_a_session_nobody_knows_is_ignored(s) {
        s.to_first_session();
        s.input(s.started("no-such-session")).expect(&["ignored unknown_session"]);
        s.input(s.reply("no-such-session", "m")).expect(&["ignored unknown_session"]);
        s.input(s.nudged("no-such-session")).expect(&["ignored unknown_session"]);
        s.input(s.stop_signalled("no-such-session", 1, StopKind::Term)).expect(&["ignored unknown_session"]);
    }

    fn a_stale_timer_changes_nothing_and_its_re_arm_fires_on_its_own_instant(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        let stale = s.fired("quiet/build/v1/coder/a1");
        s.advance(60);
        s.input(s.beat(&key, nexus_run_contracts::HeartbeatKind::ModelOutput)).expect(&[
            "timer_set quiet/build/v1/coder/a1 +600s quiet",
        ]);
        s.advance(600);
        s.input(stale).expect(&["ignored stale_timer"]);
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
        s.fire("quiet/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
    }

    fn a_timer_fired_twice_acts_once(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        let once = s.fired("quiet/build/v1/coder/a1");
        s.advance(600);
        s.input(once.clone());
        s.input(once).expect(&["ignored stale_timer"]);
    }

    fn every_input_to_a_finished_run_is_ignored_final_state(s) {
        s.to_first_session();
        s.pass_until("finish");
        s.run_step_to_pass("finish");
        s.finish_clean();
        s.expect(&["finished delivered delivered"]);
        let key = s.key("build", 1, "coder", 1);
        s.input(s.withdrawn()).expect(&["ignored final_state"]);
        s.input(s.reply(&key, "m")).expect(&["ignored final_state"]);
        s.input(s.claim_released()).expect(&["ignored final_state"]);
        s.input(s.ended(&key, EndCause::Lost)).expect(&["ignored final_state"]);
    }

    fn a_reply_to_a_run_already_finishing_is_ignored(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.withdrawn());
        s.input(s.reply(&key, "m")).expect(&["ignored already_finishing"]);
    }

    fn an_input_a_state_has_no_use_for_changes_nothing(s) {
        s.input(s.commissioned());
        s.input(s.commissioned()).expect(&["ignored out_of_state"]);
        s.input(s.claim_taken()).expect(&["ignored out_of_state"]);
        s.input(s.tree_prepared()).expect(&["ignored out_of_state"]);
        s.input(s.tree_parked_clean()).expect(&["ignored out_of_state"]);
        s.input(s.root_opened());
        s.input(s.root_opened()).expect(&["ignored out_of_state"]);
    }

    fn an_unknown_input_folds_to_nothing_and_decides_ignored(s) {
        s.to_first_session();
        let before = s.state.clone();
        let unknown = nexus_run_contracts::UnknownBody {
            kind: "from_the_future".into(),
            raw: serde_json::json!({ "kind": "from_the_future" }),
        };
        s.input(EventBody::Unknown(unknown)).expect(&["ignored out_of_state"]);
        let mut after = s.state.clone();
        after.seq = before.seq;
        assert_eq!(after, before);
    }

    fn a_nudge_result_for_a_slot_no_longer_quiet_is_ignored(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.nudged(&key)).expect(&["ignored out_of_state"]);
        s.advance(Secs::minutes(1).0);
    }

    fn a_signal_record_the_log_already_holds_is_ignored(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        s.input(s.stop_signalled(&key, 1, StopKind::Term)).expect(&["ignored out_of_state"]);
        let SlotState::Stopping { ladder, .. } = s.slot_state(&key) else {
            panic!("it is stopping");
        };
        assert_eq!(ladder.signals_sent, 1);
    }
}
