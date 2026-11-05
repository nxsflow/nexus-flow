//! The review fan-out and its synthesis (§6.4 item 7): never fewer than all three, and the routing bit from
//! the members only.

use nexus_run_contracts::{EndCause, StopKind};
use nexus_run_engine::{Effect, RunView, SlotState, TurnKind};

use crate::script::Script;

fn to_judge(s: &mut Script) {
    s.to_first_session();
    s.pass_until("judge");
}

cases! {
    fn three_sessions_start_at_once_and_two_replies_finish_nothing(s) {
        to_judge(s);
        let starts = s.effects().iter()
            .filter(|e| matches!(e, Effect::StartSession { .. })).count();
        assert_eq!(starts, 3);
        for member in ["code-quality", "test-quality"] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply(&key, "m-review")).expect(&[
                format!("timer_cleared quiet/judge/v1/{member}/a1"),
                format!("timer_set wind_down/judge/v1/{member}/a1 +300s wind_down"),
            ]);
        }
        assert!(matches!(s.state.view(), RunView::Running { .. }));
    }

    fn the_third_reply_starts_the_consolidator_with_no_resume_and_the_three_replies(s) {
        to_judge(s);
        for member in ["code-quality", "test-quality"] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply(&key, &format!("m-{member}")));
        }
        let key = s.key("judge", 1, "integrity", 1);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-integrity")).expect(&[
            "timer_cleared quiet/judge/v1/integrity/a1",
            "timer_set wind_down/judge/v1/integrity/a1 +300s wind_down",
            "step_started judge v1 a1 [__synth__]",
            "timer_set start_deadline/judge/v1/__synth__/a1 +180s start_deadline",
        ]);
        let synth = s.key("judge", 1, "__synth__", 1);
        let Effect::StartSession { turn, resume, .. } = s.effects_of(&synth).remove(0) else {
            panic!("the consolidator is owed a start");
        };
        assert!(!resume);
        let replies: Vec<&TurnKind> = turn.iter().map(|p| &p.kind)
            .filter(|k| matches!(k, TurnKind::Reply { .. })).collect();
        assert_eq!(replies.len(), 3);
        let messages: Vec<&str> = turn.iter().filter_map(|p| p.message.as_deref()).collect();
        assert_eq!(messages, ["m-code-quality", "m-test-quality", "m-integrity"]);
    }

    fn the_outcome_is_counted_from_the_members_and_a_bit_on_the_synthesis_routes_nothing(s) {
        to_judge(s);
        for member in ["code-quality", "test-quality", "integrity"] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply(&key, "m-pass"));
        }
        let synth = s.key("judge", 1, "__synth__", 1);
        s.input(s.started(&synth));
        s.input(s.reply_with(&synth, "m-summary", true, true)).expect(&[
            "timer_cleared quiet/judge/v1/__synth__/a1",
            "timer_set wind_down/judge/v1/__synth__/a1 +300s wind_down",
            "step_finished judge v1 pass",
            "step_started finish v1 a1 [finisher]",
            "timer_set start_deadline/finish/v1/finisher/a1 +180s start_deadline",
        ]);
        assert!(s.state.open_questions().next().is_none(), "the consolidator cannot ask");
    }

    fn one_member_sending_it_back_routes_it_back_and_the_synthesis_is_its_verdict(s) {
        to_judge(s);
        for (member, bit) in [("code-quality", false), ("test-quality", true), ("integrity", false)] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply_with(&key, "m-review", false, bit));
        }
        let synth = s.key("judge", 1, "__synth__", 1);
        s.input(s.started(&synth));
        s.input(s.reply(&synth, "m-summary")).expect(&[
            "timer_cleared quiet/judge/v1/__synth__/a1",
            "timer_set wind_down/judge/v1/__synth__/a1 +300s wind_down",
            "step_finished judge v1 needs_rework",
            "rework judge->build declaring=judge pass=1",
            "step_started build v2 a1 [coder]",
            "timer_set start_deadline/build/v2/coder/a1 +180s start_deadline",
        ]);
        let build = s.latest_key("build", None);
        let Effect::StartSession { turn, .. } = s.effects_of(&build).remove(0) else {
            panic!("a start is owed");
        };
        assert!(turn.iter().any(|p| p.message.as_deref() == Some("m-summary")));
    }

    fn a_reviewer_that_ends_without_replying_is_restarted_only_after_its_confirmed_end_then_parked(s) {
        to_judge(s);
        for member in ["code-quality", "test-quality"] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply(&key, "m-review"));
        }
        let a1 = s.key("judge", 1, "integrity", 1);
        s.input(s.started(&a1));
        // A dead leader with a live group is no end: nothing restarts yet.
        s.input(s.fact(&a1, nexus_run_contracts::SessionFact::GroupLingering { pids: vec![9] })).expect(&[
            "timer_cleared quiet/judge/v1/integrity/a1",
            "liveness judge/v1/integrity/a1 lost since=+0s",
        ]);
        assert_eq!(s.state.visits["judge"], 1);
        assert_eq!(s.effects_of(&a1), vec![Effect::StopSession {
            session_key: a1.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect(&[
            "step_started judge v1 a2 [integrity]",
            "timer_set start_deadline/judge/v1/integrity/a2 +180s start_deadline",
        ]);

        let a2 = s.key("judge", 1, "integrity", 2);
        s.input(s.started(&a2));
        s.input(s.ended(&a2, EndCause::Exit { status: Some(1) })).expect(&[
            "timer_cleared quiet/judge/v1/integrity/a2",
            "step_started judge v1 a3 [integrity]",
            "timer_set start_deadline/judge/v1/integrity/a3 +180s start_deadline",
        ]);

        let a3 = s.key("judge", 1, "integrity", 3);
        s.input(s.started(&a3));
        s.input(s.ended(&a3, EndCause::Lost)).expect(&[
            "timer_cleared quiet/judge/v1/integrity/a3",
            "parked judge/v1/integrity/a3 fan_out_incomplete",
        ]);
        assert_eq!(
            s.state.view(),
            RunView::Parked { reason: nexus_run_contracts::ReasonCode::FanOutIncomplete }
        );
        // Two of three: the step never completes, and the run asks nobody for a synthesis.
        assert!(s.state.latest("judge", 1, "__synth__").is_none());
        assert!(s.state.finished_steps.iter().all(|f| f.step != "judge"));
    }

    fn a_resumed_reviewer_with_a_fresh_budget_lets_the_fan_out_complete(s) {
        to_judge(s);
        for member in ["code-quality", "test-quality"] {
            let key = s.key("judge", 1, member, 1);
            s.input(s.started(&key));
            s.input(s.reply(&key, "m-review"));
        }
        for attempt in 1..=3 {
            let key = s.key("judge", 1, "integrity", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::Lost));
        }
        s.input(nexus_run_contracts::EventBody::Resume { slot: None }).expect(&[
            "step_started judge v1 a4 [integrity]",
            "timer_set start_deadline/judge/v1/integrity/a4 +180s start_deadline",
        ]);
        assert!(matches!(s.state.view(), RunView::Running { .. }));
        let a4 = s.key("judge", 1, "integrity", 4);
        let record = s.state.find(&a4).unwrap();
        assert_eq!(s.state.slots[record].restarts, 0, "the budget starts over");
        s.input(s.started(&a4));
        s.input(s.reply(&a4, "m-late"));
        assert!(s.decisions().contains(&"step_started judge v1 a1 [__synth__]".to_owned()));
        assert!(matches!(s.slot_state(&a4), SlotState::Replied { .. }));
    }
}
