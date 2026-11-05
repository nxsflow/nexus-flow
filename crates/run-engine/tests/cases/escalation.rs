//! Escalation and answer (§6.4 item 8), and the precedence of §6.3 over sibling slots.

use nexus_run_contracts::{Disposition, EndCause, EventBody, ReasonCode};
use nexus_run_engine::{DeliverBody, Effect, RunView, TurnKind, TurnPart};

use crate::script::{slot, Script};

fn to_judge(s: &mut Script) {
    s.reach_first_session();
    s.pass_until("judge");
    for member in ["code-quality", "test-quality", "integrity"] {
        let key = s.key("judge", 1, member, 1);
        s.input(s.started(&key));
    }
}

/// A verifier asks; its process winds down; the answer resumes it.
fn verifier_asks(s: &mut Script) -> (String, String) {
    s.reach_first_session();
    s.run_step_to_pass("build");
    let key = s.latest_key("verify", None);
    s.input(s.started(&key));
    s.input(s.reply_with(&key, "m-ask", true, false));
    (key.clone(), format!("escalated/{key}"))
}

cases! {
    fn an_escalation_asks_the_moment_the_reply_folds_and_holds_the_claim(s) {
        s.reach_first_session();
        s.run_step_to_pass("build");
        let key = s.latest_key("verify", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-ask", true, false)).expect(&[
            "timer_cleared quiet/verify/v1/verifier/a1".to_owned(),
            "timer_set wind_down/verify/v1/verifier/a1 +300s wind_down".to_owned(),
            format!("escalated question=escalated/{key} slot=verify/v1/verifier/a1 message=m-ask"),
        ]);
        let question = format!("escalated/{key}");
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![question.clone()] });
        assert_eq!(
            s.deliveries(),
            vec![(format!("question/{question}"), DeliverBody::Question(question))]
        );
        assert!(s.state.claim_since.is_some());
        // The question id is the key the event that asked was written under.
        let asked = s.log.iter().find(|e| {
            matches!(&e.body, EventBody::Decision(nexus_run_contracts::Decision::Escalated { .. }))
        });
        assert_eq!(asked.unwrap().key, format!("escalated/{key}"));
    }

    fn the_answer_reaches_the_same_slot_with_resume_after_the_old_process_ended(s) {
        let (key, question) = verifier_asks(s);
        s.input(s.delivered()).expect_none();
        s.input(s.answer(&question, Disposition::Continue, "m-answer")).expect(&[
            "step_started verify v1 a2 [verifier]",
        ]);
        // Two processes never share one backend session: it waits for the old one.
        let a2 = s.key("verify", 1, "verifier", 2);
        assert!(s.effects_of(&a2).is_empty());
        s.input(s.ended(&key, EndCause::TurnComplete)).expect(&[
            "timer_cleared wind_down/verify/v1/verifier/a1",
            "timer_set start_deadline/verify/v1/verifier/a2 +180s start_deadline",
        ]);
        assert_eq!(s.effects_of(&a2), vec![Effect::StartSession {
            session_key: a2.clone(),
            turn: vec![
                TurnPart { kind: TurnKind::Answer, message: Some("m-answer".into()) },
                TurnPart { kind: TurnKind::Obligation, message: None },
            ],
            resume: true,
        }]);
    }

    fn the_next_reply_of_an_answered_attempt_completes_the_step_and_the_old_bit_never_re_escalates(s) {
        let (key, question) = verifier_asks(s);
        s.input(s.answer(&question, Disposition::Continue, "m-answer"));
        s.input(s.ended(&key, EndCause::TurnComplete));
        let a2 = s.key("verify", 1, "verifier", 2);
        s.input(s.started(&a2));
        s.input(s.reply(&a2, "m-done")).expect(&[
            "timer_cleared quiet/verify/v1/verifier/a2",
            "timer_set wind_down/verify/v1/verifier/a2 +300s wind_down",
            "step_finished verify v1 pass",
            "step_started judge v1 a1 [code-quality,test-quality,integrity]",
            "timer_set start_deadline/judge/v1/code-quality/a1 +180s start_deadline",
            "timer_set start_deadline/judge/v1/test-quality/a1 +180s start_deadline",
            "timer_set start_deadline/judge/v1/integrity/a1 +180s start_deadline",
        ]);
        assert_eq!(s.state.questions.len(), 1, "asked once");
        assert!(s.state.open_questions().next().is_none());
    }

    fn an_answered_escalation_spends_no_restart(s) {
        let (key, question) = verifier_asks(s);
        s.input(s.answer(&question, Disposition::Continue, "m-answer"));
        s.input(s.ended(&key, EndCause::TurnComplete));
        let a2 = s.state.find(&s.key("verify", 1, "verifier", 2)).unwrap();
        assert_eq!(s.state.slots[a2].restarts, 0);
    }

    fn an_attempt_that_escalates_again_asks_a_new_question(s) {
        let (key, question) = verifier_asks(s);
        s.input(s.answer(&question, Disposition::Continue, "m-answer"));
        s.input(s.ended(&key, EndCause::TurnComplete));
        let a2 = s.key("verify", 1, "verifier", 2);
        s.input(s.started(&a2));
        s.input(s.reply_with(&a2, "m-ask-2", true, false)).expect(&[
            "timer_cleared quiet/verify/v1/verifier/a2".to_owned(),
            "timer_set wind_down/verify/v1/verifier/a2 +300s wind_down".to_owned(),
            format!("escalated question=escalated/{a2} slot=verify/v1/verifier/a2 message=m-ask-2"),
        ]);
    }

    fn two_reviewers_escalating_queue_two_questions_answered_by_id_in_order(s) {
        to_judge(s);
        let (first, second) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        s.input(s.reply_with(&first, "m-1", true, false));
        s.input(s.reply_with(&second, "m-2", true, false));
        let (q1, q2) = (format!("escalated/{first}"), format!("escalated/{second}"));
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![q1.clone(), q2.clone()] });
        assert_eq!(s.deliveries().len(), 2);

        // The sibling keeps running to its reply, and the step does not complete on it.
        let other = s.key("judge", 1, "code-quality", 1);
        s.input(s.reply(&other, "m-3")).expect(&[
            "timer_cleared quiet/judge/v1/code-quality/a1",
            "timer_set wind_down/judge/v1/code-quality/a1 +300s wind_down",
        ]);

        s.input(s.ended(&first, EndCause::TurnComplete));
        s.input(s.answer(&q1, Disposition::Continue, "m-a1")).expect(&[
            "step_started judge v1 a2 [integrity]",
            "timer_set start_deadline/judge/v1/integrity/a2 +180s start_deadline",
        ]);
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![q2.clone()] });

        s.input(s.ended(&second, EndCause::TurnComplete));
        s.input(s.answer(&q2, Disposition::Continue, "m-a2")).expect(&[
            "step_started judge v1 a2 [test-quality]",
            "timer_set start_deadline/judge/v1/test-quality/a2 +180s start_deadline",
        ]);
        assert!(matches!(s.state.view(), RunView::Running { .. }));
    }

    fn accept_on_an_escalation_is_refused_even_if_it_reaches_the_reducer(s) {
        let (_, question) = verifier_asks(s);
        s.input(s.answer(&question, Disposition::Accept, "m-accept")).expect(&[
            "ignored out_of_state",
        ]);
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![question] });
    }

    fn an_answer_to_an_unknown_or_answered_question_changes_nothing(s) {
        let (_, question) = verifier_asks(s);
        s.input(s.answer("no-such-question", Disposition::Continue, "m")).expect(&["ignored out_of_state"]);
        s.input(s.answer(&question, Disposition::Continue, "m"));
        s.input(s.answer(&question, Disposition::Continue, "m")).expect(&["ignored out_of_state"]);
    }

    // ---------------------------------------------------------- the precedence of §6.3, one per row

    fn a_hold_alone_is_held_and_never_needs_a_decision(s) {
        to_judge(s);
        let key = s.key("judge", 1, "integrity", 1);
        s.input(s.ended(&key, EndCause::ProviderUnavailable));
        assert!(matches!(s.state.view(), RunView::Held { reason: ReasonCode::ProviderUnavailable, .. }));
        assert!(s.deliveries().is_empty(), "a hold is no question");
        assert!(s.state.questions.is_empty());
    }

    fn a_question_beside_a_hold_needs_a_decision_and_the_hold_shows_again_once_answered(s) {
        to_judge(s);
        let (held, asks) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        s.input(s.ended(&held, EndCause::ProviderUnavailable));
        s.input(s.reply_with(&asks, "m-ask", true, false));
        let question = format!("escalated/{asks}");
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![question.clone()] });
        s.input(s.answer(&question, Disposition::Continue, "m-a"));
        assert!(matches!(s.state.view(), RunView::Held { .. }));
    }

    fn a_question_beside_a_park_needs_a_decision_and_the_park_shows_again_once_answered(s) {
        to_judge(s);
        let (parked, asks) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        s.input(s.ended(&parked, EndCause::PermissionRefused { rule: "r".into(), program: "git".into() }));
        s.input(s.reply_with(&asks, "m-ask", true, false));
        let question = format!("escalated/{asks}");
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![question.clone()] });
        s.input(s.answer(&question, Disposition::Continue, "m-a"));
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::PermissionRefused });
    }

    fn a_hold_beside_a_park_is_held_until_the_earliest_hold_and_then_parked(s) {
        to_judge(s);
        let (parked, held) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        s.input(s.ended(&parked, EndCause::PermissionRefused { rule: "r".into(), program: "git".into() }));
        s.input(s.ended(&held, EndCause::ProviderUnavailable));
        assert!(matches!(s.state.view(), RunView::Held { .. }));
        s.fire("hold_retry/judge/v1/test-quality/a1");
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::PermissionRefused });
    }

    fn all_three_at_once_need_a_decision(s) {
        to_judge(s);
        let parked = s.key("judge", 1, "integrity", 1);
        let held = s.key("judge", 1, "test-quality", 1);
        let asks = s.key("judge", 1, "code-quality", 1);
        s.input(s.ended(&parked, EndCause::PermissionRefused { rule: "r".into(), program: "git".into() }));
        s.input(s.ended(&held, EndCause::ProviderUnavailable));
        s.input(s.reply_with(&asks, "m-ask", true, false));
        assert!(matches!(s.state.view(), RunView::NeedsDecision { .. }));
    }

    fn two_holds_report_the_earliest_until(s) {
        to_judge(s);
        let (a, b) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        let later = s.now.plus(nexus_run_contracts::Secs::hours(2));
        let sooner = s.now.plus(nexus_run_contracts::Secs::hours(1));
        s.input(s.ended(&a, EndCause::UsageLimit { resets_at: Some(later) }));
        s.input(s.ended(&b, EndCause::UsageLimit { resets_at: Some(sooner) }));
        assert_eq!(s.state.view(), RunView::Held { reason: ReasonCode::UsageLimit, until: sooner });
    }

    fn a_run_condition_shows_as_parked_with_no_slot(s) {
        s.input(s.commissioned());
        s.host_decision(nexus_run_contracts::Decision::Parked {
            slot: None,
            reason: ReasonCode::PoisonInput,
        });
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::PoisonInput });
        s.input(EventBody::ResumeRun).expect_none();
        assert_eq!(s.state.view(), RunView::Created);
    }

    fn a_superseded_attempts_condition_does_not_count(s) {
        to_judge(s);
        let a1 = s.key("judge", 1, "integrity", 1);
        s.input(s.ended(&a1, EndCause::ProviderUnavailable));
        s.fire("hold_retry/judge/v1/integrity/a1").expect(&[
            "step_started judge v1 a2 [integrity]",
            "timer_set start_deadline/judge/v1/integrity/a2 +180s start_deadline",
        ]);
        assert!(matches!(s.state.view(), RunView::Running { .. }));
        assert_eq!(s.state.find_slot(&slot("judge", 1, "integrity", 1)), s.state.find(&a1));
    }
}
