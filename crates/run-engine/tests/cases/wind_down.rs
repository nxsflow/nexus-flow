//! A replied slot's process (§6.4 item 9): it winds down on its own or is stopped; its end folds onto the
//! slot; a resume of the same persona waits for it.

use nexus_run_contracts::{EndCause, Outcome, ParkOutcome, ReasonCode, SessionFact, StopKind};
use nexus_run_engine::{DeliverBody, Effect, Process, RunView, SlotState};

use crate::script::Script;

fn to_finisher_replied(s: &mut Script) -> String {
    s.to_first_session();
    s.pass_until("finish");
    let key = s.latest_key("finish", None);
    s.input(s.started(&key));
    s.input(s.reply(&key, "m-pr"));
    key
}

cases! {
    fn a_reply_then_its_end_folds_onto_the_slot_and_owes_no_stop(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-1"));
        assert!(matches!(s.slot_state(&key), SlotState::Replied { process: Process::Live, .. }));
        s.input(s.ended(&key, EndCause::TurnComplete)).expect(&[
            "timer_cleared wind_down/build/v1/coder/a1",
        ]);
        assert!(matches!(s.slot_state(&key), SlotState::Replied { process: Process::Ended, .. }));
        assert!(s.effects_of(&key).is_empty());
        // Its second end is a fact about a slot already ended.
        s.input(s.ended(&key, EndCause::TurnComplete)).expect(&["ignored final_state"]);
    }

    fn an_end_then_its_reply_is_ignored_final_state_and_the_restart_stands(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.ended(&key, EndCause::Exit { status: None }));
        s.input(s.reply(&key, "m-late")).expect(&["ignored late_reply_after_skip"]);
    }

    fn with_budget_spent_an_end_then_its_reply_is_ignored_final_state(s) {
        s.to_first_session();
        for attempt in 1..=3 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::Exit { status: None }));
        }
        let key = s.key("build", 1, "coder", 3);
        s.input(s.reply(&key, "m-late")).expect(&["ignored final_state"]);
    }

    fn the_next_step_with_another_persona_starts_while_a_replied_process_winds_down(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-1")).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "timer_set wind_down/build/v1/coder/a1 +300s wind_down",
            "step_finished build v1 pass",
            "step_started verify v1 a1 [verifier]",
            "timer_set start_deadline/verify/v1/verifier/a1 +180s start_deadline",
        ]);
    }

    fn a_resume_waits_for_the_previous_process_of_the_same_persona(s) {
        s.to_first_session();
        let coder = s.latest_key("build", None);
        s.input(s.started(&coder));
        s.input(s.reply(&coder, "m-1"));
        let verifier = s.latest_key("verify", None);
        s.input(s.started(&verifier));
        s.input(s.reply_with(&verifier, "m-red", false, true)).expect(&[
            "timer_cleared quiet/verify/v1/verifier/a1",
            "timer_set wind_down/verify/v1/verifier/a1 +300s wind_down",
            "step_finished verify v1 needs_rework",
            "rework verify->build declaring=verify pass=1",
            "step_started build v2 a1 [coder]",
        ]);
        let again = s.key("build", 2, "coder", 1);
        assert!(s.effects_of(&again).is_empty(), "no second process on one backend session");
        assert!(matches!(s.slot_state(&again), SlotState::Pending));

        s.input(s.ended(&coder, EndCause::TurnComplete)).expect(&[
            "timer_cleared wind_down/build/v1/coder/a1",
            "timer_set start_deadline/build/v2/coder/a1 +180s start_deadline",
        ]);
        assert!(matches!(s.effects_of(&again)[0], Effect::StartSession { .. }));
    }

    fn an_overdue_reply_is_stopped_by_the_same_ladder(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-1"));
        assert!(s.effects_of(&key).is_empty(), "inside its wind-down it owes nothing");
        s.fire("wind_down/build/v1/coder/a1").expect_none();
        assert!(matches!(s.slot_state(&key), SlotState::Replied { overdue: true, .. }));
        assert_eq!(s.effects_of(&key), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.input(s.stop_signalled(&key, 1, StopKind::Term)).expect(&[
            "timer_set stop_grace/1/build/v1/coder/a1 +300s stop_grace",
        ]);
        assert!(s.effects_of(&key).is_empty());
        s.fire("stop_grace/1/build/v1/coder/a1").expect_none();
        assert_eq!(s.effects_of(&key), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Term, nth: 2,
        }]);
        s.input(s.stop_signalled(&key, 2, StopKind::Term)).expect(&[
            "timer_set stop_grace/2/build/v1/coder/a1 +300s stop_grace",
        ]);
        s.fire("stop_grace/2/build/v1/coder/a1").expect(&[
            "parked build/v1/coder/a1 would_not_stop",
        ]);
        // Its step stays done, and the run says it needs a person.
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::WouldNotStop });
        assert_eq!(s.state.finished_steps[0].step, "build");
        assert!(s.effects_of(&key).is_empty());
    }

    fn a_finisher_whose_end_never_comes_is_stopped_and_the_claim_goes_only_after_it(s) {
        let key = to_finisher_replied(s);
        s.expect(&[
            "timer_cleared quiet/finish/v1/finisher/a1",
            "timer_set wind_down/finish/v1/finisher/a1 +300s wind_down",
            "step_finished finish v1 pass",
            "finishing delivered delivered",
        ]);
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
        assert!(s.effects().is_empty(), "a just-replied finisher is given its grace");

        s.fire("wind_down/finish/v1/finisher/a1").expect_none();
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        assert!(s.effects().is_empty(), "the claim, the park and the delivery all wait for the end");

        s.input(s.ended(&key, EndCause::StoppedByHost)).expect(&[
            "timer_cleared stop_grace/1/finish/v1/finisher/a1",
        ]);
        assert_eq!(s.effects(), vec![Effect::ParkTree {
            workspace: s.workspace(), occasion: Outcome::Delivered,
        }]);
        s.finish_clean();
        s.expect(&["finished delivered delivered"]);
    }

    fn a_finisher_that_will_not_stop_holds_the_claim_with_the_outcome_kept_and_a_withdrawal_forces_it(s) {
        let key = to_finisher_replied(s);
        s.fire("wind_down/finish/v1/finisher/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        s.fire("stop_grace/1/finish/v1/finisher/a1");
        s.input(s.stop_signalled(&key, 2, StopKind::Term));
        s.fire("stop_grace/2/finish/v1/finisher/a1").expect(&[
            "parked finish/v1/finisher/a1 would_not_stop",
        ]);
        assert!(s.effects().is_empty());
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
        assert_eq!(
            nexus_run_engine::status(&s.state, 0, s.now).reason,
            Some(ReasonCode::WouldNotStop)
        );

        s.input(s.withdrawn()).expect(&["withdraw_forced"]);
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered), "the outcome stays");
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Kill, nth: 3,
        }]);
        s.input(s.stop_signalled(&key, 3, StopKind::Kill)).expect_none();
        assert!(s.effects().is_empty(), "a kill that does not confirm leaves the claim held");
        s.input(s.ended(&key, EndCause::StoppedByHost)).expect_none();
        s.finish_clean();
        s.expect(&["finished delivered delivered"]);
    }

    fn a_dead_leader_with_a_live_group_moves_a_running_slot_to_stopping_and_is_no_end(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.fact(&key, SessionFact::GroupLingering { pids: vec![11, 12] })).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "liveness build/v1/coder/a1 lost since=+0s",
        ]);
        let SlotState::Stopping { ladder, .. } = s.slot_state(&key) else {
            panic!("it is stopping");
        };
        assert!(!ladder.forced, "only a withdrawal forces a stop");
        assert_eq!(s.state.visits["build"], 1);
        let status = nexus_run_engine::status(&s.state, 0, s.now);
        assert_eq!(status.steps[0].members[0].pids, vec![11, 12]);
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        // A repeat of the fact restarts nothing and moves no rung.
        s.input(s.fact(&key, SessionFact::GroupLingering { pids: vec![12] })).expect_none();
        let SlotState::Stopping { ladder, .. } = s.slot_state(&key) else {
            panic!("it is stopping");
        };
        assert_eq!(ladder.signals_sent, 1);

        s.input(s.ended(&key, EndCause::StoppedByHost)).expect(&[
            "timer_cleared stop_grace/1/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
    }

    fn a_dead_leader_with_a_live_group_on_a_replied_slot_starts_its_stop_and_keeps_the_step_done(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-1"));
        s.input(s.fact(&key, SessionFact::GroupLingering { pids: vec![11] })).expect(&[
            "timer_cleared wind_down/build/v1/coder/a1",
        ]);
        assert!(matches!(s.effects_of(&key)[0], Effect::StopSession { kind: StopKind::Term, .. }));
        assert_eq!(s.state.finished_steps[0].step, "build");
    }

    fn a_committed_park_of_a_delivered_run_owes_the_result_and_one_notice(s) {
        let key = to_finisher_replied(s);
        s.input(s.ended(&key, EndCause::TurnComplete));
        s.input(s.tree_parked(ParkOutcome::Committed { branch: "nxs/park/x".into(), commit: "abc".into() }));
        let run = s.state.run.clone();
        assert_eq!(s.deliveries(), vec![
            (format!("result/{run}"), DeliverBody::Message("m-pr".into())),
            (format!("park/{run}"), DeliverBody::ParkNotice { branch: "nxs/park/x".into(), commit: "abc".into() }),
        ]);
        s.input(s.delivered());
        assert_eq!(s.deliveries().len(), 1);
        assert!(!s.effects().iter().any(|e| matches!(e, Effect::ReleaseClaim { .. })));
        s.input(s.delivered());
        assert_eq!(s.effects(), vec![Effect::ReleaseClaim { workspace: s.workspace() }]);
    }

    fn local_commits_on_its_own_branch_owe_a_notice_and_a_clean_branch_owes_none(s) {
        let key = to_finisher_replied(s);
        s.input(s.ended(&key, EndCause::TurnComplete));
        s.input(s.tree_parked(ParkOutcome::Clean { branch: Some("feat/x".into()) }));
        let run = s.state.run.clone();
        assert_eq!(s.deliveries(), vec![(format!("result/{run}"), DeliverBody::Message("m-pr".into()))]);

        let mut other = Script::coding();
        let key = to_finisher_replied(&mut other);
        other.input(other.ended(&key, EndCause::TurnComplete));
        other.input(other.tree_parked(ParkOutcome::OnOwnBranch { branch: "feat/x".into(), commit: "abc".into() }));
        assert_eq!(other.deliveries()[1].1, DeliverBody::ParkNotice { branch: "feat/x".into(), commit: "abc".into() });
    }

    fn a_delivery_key_nobody_owes_is_ignored_and_a_release_before_its_time_too(s) {
        let key = to_finisher_replied(s);
        s.input(s.delivered_key("result/not-this-run")).expect(&["ignored out_of_state"]);
        s.input(s.claim_released()).expect(&["ignored out_of_state"]);
        s.input(s.tree_parked_clean()).expect_none();
        s.input(s.tree_parked_clean()).expect(&["ignored out_of_state"]);
        s.input(s.ended(&key, EndCause::TurnComplete));
    }
}
