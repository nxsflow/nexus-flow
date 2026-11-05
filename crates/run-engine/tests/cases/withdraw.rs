//! Withdrawal (§6.4 item 10) and the hung-session exit walked by decision list (§12.1): SIGTERM first,
//! SIGKILL only once forced, the claim released only after the stop, the park and the discharge.

use nexus_run_contracts::{
    EndCause, EventBody, Outcome, ParkOutcome, ReasonCode, StopKind, WithdrawnBy,
};
use nexus_run_engine::{DeliverBody, Effect, RunView, SlotState};

use crate::script::Script;

fn running(s: &mut Script) -> String {
    s.to_first_session();
    let key = s.latest_key("build", None);
    s.input(s.started(&key));
    key
}

/// Takes a slot to `parked { would_not_stop }`, by the ladder.
fn to_would_not_stop(s: &mut Script, key: &str) {
    s.fire("quiet/build/v1/coder/a1");
    s.input(s.nudged(key));
    s.fire("confirm/build/v1/coder/a1");
    s.input(s.stop_signalled(key, 1, StopKind::Term));
    s.fire("stop_grace/1/build/v1/coder/a1");
    s.input(s.stop_signalled(key, 2, StopKind::Term));
    s.fire("stop_grace/2/build/v1/coder/a1");
}

cases! {
    fn the_hung_session_exit_walked_by_decision_list(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 quiet since=+0s",
            "timer_set confirm/build/v1/coder/a1 +300s confirm",
        ]);
        s.input(s.nudged(&key)).expect(&["timer_set confirm/build/v1/coder/a1 +300s confirm"]);
        s.fire("confirm/build/v1/coder/a1").expect(&["liveness build/v1/coder/a1 hung since=+0s"]);
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
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::WouldNotStop });
        assert!(s.effects().is_empty(), "never a SIGKILL of its own");
        assert!(s.state.claim_since.is_some());

        // A person's act: the withdrawal sets `forced`, and only now is the SIGKILL owed.
        s.input(s.withdrawn()).expect(&["finishing withdrawn withdrawn_by_requester"]);
        let SlotState::Stopping { ladder, .. } = s.slot_state(&key) else {
            panic!("it is stopping");
        };
        assert!(ladder.forced);
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Kill, nth: 3,
        }]);
        s.input(s.stop_signalled(&key, 3, StopKind::Kill)).expect_none();
        assert!(s.effects().is_empty(), "a forced stop that does not confirm leaves the claim held");

        s.input(s.ended(&key, EndCause::StoppedByHost)).expect_none();
        assert_eq!(s.effects(), vec![Effect::ParkTree {
            workspace: s.workspace(), occasion: Outcome::Withdrawn,
        }]);
        let parked = ParkOutcome::Committed { branch: "nxs/park/run/x".into(), commit: "abc".into() };
        s.input(s.tree_parked(parked.clone())).expect_none();
        let result = format!("result/{}", s.state.run);
        assert_eq!(s.deliveries(), vec![(result, DeliverBody::Discharge {
            step: Some("build".into()), pass: 0, verdict: None, parked: Some(parked),
        })]);
        s.input(s.delivered()).expect_none();
        assert_eq!(s.effects(), vec![Effect::ReleaseClaim { workspace: s.workspace() }]);
        s.input(s.claim_released()).expect(&["finished withdrawn withdrawn_by_requester"]);
        assert!(s.effects().is_empty());
    }

    fn a_withdrawal_during_the_first_grace_kills_only_after_that_grace(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        s.input(s.withdrawn());
        assert!(s.effects().is_empty(), "the withdrawal never skips the first grace");
        s.fire("stop_grace/1/build/v1/coder/a1").expect_none();
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Kill, nth: 2,
        }]);
    }

    fn a_withdrawal_during_the_second_grace_waits_for_it_and_then_kills(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        s.fire("stop_grace/1/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 2, StopKind::Term));
        s.input(s.withdrawn()).expect(&["finishing withdrawn withdrawn_by_requester"]);
        assert!(s.effects().is_empty(), "the second grace runs out first");
        assert!(s.state.timers.contains_key("stop_grace/2/build/v1/coder/a1"));
        s.fire("stop_grace/2/build/v1/coder/a1").expect_none();
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Kill, nth: 3,
        }]);
        s.input(s.stop_signalled(&key, 3, StopKind::Kill)).expect_none();
        assert!(s.effects().is_empty(), "after the third signal nothing more is owed");
    }

    fn a_withdrawal_of_a_running_session_terms_first_and_kills_after_the_grace(s) {
        let key = running(s);
        s.input(s.withdrawn()).expect(&[
            "finishing withdrawn withdrawn_by_requester",
            "timer_cleared quiet/build/v1/coder/a1",
        ]);
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.input(s.stop_signalled(&key, 1, StopKind::Term)).expect(&[
            "timer_set stop_grace/1/build/v1/coder/a1 +300s stop_grace",
        ]);
        s.fire("stop_grace/1/build/v1/coder/a1");
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: key.clone(), kind: StopKind::Kill, nth: 2,
        }]);
    }

    fn a_second_withdraw_keeps_the_outcome_and_forces_an_open_stop(s) {
        s.to_first_session();
        s.pass_until("finish");
        let key = s.latest_key("finish", None);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-pr"));
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
        s.fire("wind_down/finish/v1/finisher/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Term));

        s.input(s.withdrawn()).expect(&["withdraw_forced"]);
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
        s.fire("stop_grace/1/finish/v1/finisher/a1");
        assert!(matches!(s.effects()[0], Effect::StopSession { kind: StopKind::Kill, .. }));

        // Nothing is left to force.
        s.input(s.withdrawn()).expect(&["ignored already_finishing"]);
    }

    fn a_withdraw_with_nothing_to_force_is_already_finishing(s) {
        s.to_first_session();
        s.pass_until("finish");
        s.run_step_to_pass("finish");
        s.input(s.withdrawn()).expect(&["ignored already_finishing"]);
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
    }

    fn a_run_withdrawn_in_created_delivers_nothing(s) {
        s.input(s.commissioned());
        s.input(s.withdrawn()).expect(&[
            "finishing withdrawn withdrawn_by_requester",
            "timer_cleared root_deadline",
        ]);
        s.input(s.tree_parked(ParkOutcome::NotHeld)).expect_none();
        assert!(s.deliveries().is_empty());
        assert_eq!(s.effects(), vec![Effect::ReleaseClaim { workspace: s.workspace() }]);
        s.input(s.claim_released()).expect(&["finished withdrawn withdrawn_by_requester"]);
    }

    fn a_root_that_opens_after_a_withdrawal_carries_the_discharge(s) {
        s.input(s.commissioned());
        s.input(s.withdrawn());
        s.input(s.root_opened()).expect_none();
        s.input(s.tree_parked(ParkOutcome::NotHeld));
        assert_eq!(s.deliveries().len(), 1);
    }

    fn a_withdrawal_by_the_owner_says_so(s) {
        s.input(s.commissioned());
        s.input(EventBody::Withdrawn { message: None, by: WithdrawnBy::Owner }).expect(&[
            "finishing withdrawn withdrawn_by_owner",
            "timer_cleared root_deadline",
        ]);
    }

    fn a_withdrawn_slot_replied_inside_its_wind_down_is_given_its_grace_then_forced(s) {
        let key = running(s);
        s.input(s.reply(&key, "m"));
        let verifier = s.latest_key("verify", None);
        s.input(s.withdrawn());
        // The verifier never started and is stopped; the coder is inside its wind-down.
        assert_eq!(s.effects(), vec![Effect::StopSession {
            session_key: verifier.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.fire("wind_down/build/v1/coder/a1").expect_none();
        s.input(s.stop_signalled(&key, 1, StopKind::Term));
        s.fire("stop_grace/1/build/v1/coder/a1");
        assert!(s.effects_of(&key).iter().any(|e| matches!(e, Effect::StopSession { kind: StopKind::Kill, .. })));
    }

    fn the_stop_of_a_session_that_would_not_stop_stays_forced_through_a_withdrawal(s) {
        let key = running(s);
        to_would_not_stop(s, &key);
        s.input(s.withdrawn());
        let SlotState::Stopping { ladder, .. } = s.slot_state(&key) else {
            panic!("it is stopping");
        };
        assert_eq!((ladder.forced, ladder.signals_sent), (true, 2));
    }

    fn a_withdrawal_while_a_slot_waits_for_its_personas_end_starts_nothing(s) {
        s.to_first_session();
        let coder = s.latest_key("build", None);
        s.input(s.started(&coder));
        s.input(s.reply(&coder, "m"));
        let verifier = s.latest_key("verify", None);
        s.input(s.started(&verifier));
        s.input(s.reply_with(&verifier, "m", false, true));
        let again = s.key("build", 2, "coder", 1);
        s.input(s.withdrawn());
        assert!(matches!(s.slot_state(&again), SlotState::Ended { cause: EndCause::StoppedByHost }));
        s.input(s.ended(&coder, EndCause::TurnComplete));
        assert!(s.effects_of(&again).is_empty());
    }
}
