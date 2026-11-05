//! An unknown value in an INPUT folds by the rule of its enum (contract (a), p71m H1). A decision holding
//! one refuses ownership instead, which `decide/tests.rs` holds: the two rules are not one rule.

use nexus_run_contracts::{
    Disposition, EndCause, EventBody, HeartbeatKind, Outcome, ParkOutcome, ReasonCode,
    SequenceKind, StopKind, TreeRefusal, UnknownBody, WithdrawnBy,
};
use nexus_run_engine::{DeliverBody, Effect, RunView, SlotState};

use crate::script::Script;

const LATER: &str = "a_later_value";

fn later(tag: &str) -> UnknownBody {
    UnknownBody {
        kind: LATER.to_owned(),
        raw: serde_json::json!({ tag: LATER }),
    }
}

fn running(s: &mut Script) -> String {
    s.reach_first_session();
    let key = s.latest_key("build", None);
    s.input(s.started(&key));
    key
}

fn escalated(s: &mut Script) -> String {
    let key = running(s);
    s.input(s.reply_with(&key, "m-question", true, false));
    format!("escalated/{key}")
}

fn unprepared(s: &mut Script) {
    s.input(s.commissioned());
    s.input(s.root_opened());
    s.input(s.claim_taken());
}

cases! {
    fn an_unknown_disposition_is_ignored_and_the_question_stays_open(s) {
        let question = escalated(s);
        s.input(s.answer(&question, Disposition::Unknown(LATER.into()), "m-answer"))
            .expect(&["ignored out_of_state"]);
        assert!(matches!(s.state.view(), RunView::NeedsDecision { .. }));
        s.input(s.answer(&question, Disposition::Continue, "m-answer-2"));
        assert!(!matches!(s.state.view(), RunView::NeedsDecision { .. }));
    }

    fn a_withdrawal_by_an_unknown_party_still_withdraws_with_the_raw_value_as_the_reason(s) {
        running(s);
        s.input(EventBody::Withdrawn { message: None, by: WithdrawnBy::Unknown(LATER.into()) });
        assert_eq!(s.state.outcome(), Some(Outcome::Withdrawn));
        assert!(s.decisions().contains(&format!("finishing withdrawn {LATER}")));
    }

    fn an_unknown_stop_signal_counts_as_sent_and_is_never_taken_for_a_kill(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&key, 1, StopKind::Unknown(LATER.into())))
            .expect(&["timer_set stop_grace/1/build/v1/coder/a1 +300s stop_grace"]);
        s.fire("stop_grace/1/build/v1/coder/a1");
        let Effect::StopSession { kind, nth, .. } = s.effects_of(&key).remove(0) else {
            panic!("the second signal is owed");
        };
        assert_eq!((kind, nth), (StopKind::Term, 2));
    }

    fn an_unknown_heartbeat_counts_as_a_fact_and_re_arms_nothing(s) {
        let key = running(s);
        let quiet = s.state.timers["quiet/build/v1/coder/a1"].fire_at;
        s.advance(60);
        s.input(s.beat(&key, HeartbeatKind::Unknown(LATER.into()))).expect::<&str>(&[]);
        assert_eq!(s.state.timers["quiet/build/v1/coder/a1"].fire_at, quiet);
        let index = s.state.find(&key).unwrap();
        assert_eq!(s.state.slots[index].last_fact, Some(s.now));
    }

    fn an_unknown_heartbeat_returns_a_quiet_slot_to_running_as_any_fact_does(s) {
        let key = running(s);
        s.fire("quiet/build/v1/coder/a1");
        s.input(s.beat(&key, HeartbeatKind::Unknown(LATER.into()))).expect(&[
            "timer_cleared confirm/build/v1/coder/a1",
            "timer_set quiet/build/v1/coder/a1 +600s quiet",
        ]);
        assert!(matches!(s.slot_state(&key), SlotState::Running { .. }));
    }

    fn an_unknown_end_cause_takes_the_exit_row_restart_then_park(s) {
        let key = running(s);
        s.input(s.ended(&key, EndCause::Unknown(later("code")))).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        let key = s.key("build", 1, "coder", 2);
        s.input(s.started(&key));
        s.input(s.ended(&key, EndCause::Unknown(later("code"))));
        let key = s.key("build", 1, "coder", 3);
        s.input(s.started(&key));
        s.input(s.ended(&key, EndCause::Unknown(later("code")))).expect(&[
            "timer_cleared quiet/build/v1/coder/a3",
            "parked build/v1/coder/a3 start_failed",
        ]);
    }

    fn an_unknown_tree_refusal_is_permanent_and_parks(s) {
        unprepared(s);
        s.input(s.tree_refused(TreeRefusal::Unknown(later("code")))).expect(&[
            "timer_cleared tree_window",
            "parked run tree_refused",
        ]);
    }

    fn an_unknown_sequence_stays_mid_sequence_which_is_transient(s) {
        unprepared(s);
        s.input(s.tree_refused(TreeRefusal::MidSequence { what: SequenceKind::Unknown(LATER.into()) }))
            .expect::<&str>(&[]);
        assert!(s.state.condition.is_none());
    }

    fn an_unknown_park_outcome_is_recorded_the_run_goes_on_and_no_notice_is_owed(s) {
        s.reach_first_session();
        s.pass_until("finish");
        s.run_step_to_pass("finish");
        let outcome = ParkOutcome::Unknown(later("kind"));
        s.input(s.tree_parked(outcome.clone()));
        assert_eq!(s.state.parked, Some(outcome));
        let notices = s.effects().into_iter().filter(|effect| {
            matches!(effect, Effect::Deliver { body: DeliverBody::ParkNotice { .. }, .. })
        });
        assert_eq!(notices.count(), 0);
        s.finish_clean();
        assert!(s.state.is_finished());
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));
        assert_eq!(s.state.view(), RunView::Finished { outcome: Outcome::Delivered });
    }

    fn a_reason_code_this_version_does_not_know_is_rendered_as_it_reads(s) {
        running(s);
        s.input(EventBody::Withdrawn { message: None, by: WithdrawnBy::Unknown(LATER.into()) });
        let status = nexus_run_engine::status(&s.state, 0, s.now);
        assert_eq!(status.reason, Some(ReasonCode::Unknown(LATER.into())));
    }
}
