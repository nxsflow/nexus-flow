//! The table of how a session ends with no reply (§6.5), holds counted apart from attempts, and what a
//! person's `resume` means per parked reason.

use nexus_run_contracts::{
    Decision, EndCause, EventBody, HeartbeatKind, ReasonCode, Secs, StopKind,
};
use nexus_run_engine::{Effect, RunView, SlotState, TurnKind};

use crate::script::{slot, Script};

fn first(s: &mut Script) -> String {
    s.reach_first_session();
    s.key("build", 1, "coder", 1)
}

fn resume_none() -> EventBody {
    EventBody::Resume { slot: None }
}

/// `attempts` sessions in a row that start and end this way.
fn end_each(s: &mut Script, attempts: u32, cause: &EndCause) {
    for attempt in 1..=attempts {
        let key = s.key("build", 1, "coder", attempt);
        s.input(s.started(&key));
        s.input(s.ended(&key, cause.clone()));
    }
}

cases! {
    fn an_exit_without_a_reply_restarts_up_to_max_attempts_then_parks_start_failed(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::Exit { status: Some(1) })).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        let a2 = s.key("build", 1, "coder", 2);
        s.input(s.started(&a2));
        s.input(s.ended(&a2, EndCause::Exit { status: None })).expect(&[
            "timer_cleared quiet/build/v1/coder/a2",
            "step_started build v1 a3 [coder]",
            "timer_set start_deadline/build/v1/coder/a3 +180s start_deadline",
        ]);
        let a3 = s.key("build", 1, "coder", 3);
        s.input(s.started(&a3));
        s.input(s.ended(&a3, EndCause::Exit { status: Some(2) })).expect(&[
            "timer_cleared quiet/build/v1/coder/a3",
            "parked build/v1/coder/a3 start_failed",
        ]);
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::StartFailed });
        assert!(s.state.claim_since.is_some(), "the claim stays held");
    }

    fn a_lost_session_restarts_up_to_max_attempts_then_parks_lost(s) {
        first(s);
        end_each(s, 2, &EndCause::Lost);
        let a3 = s.key("build", 1, "coder", 3);
        s.input(s.started(&a3));
        s.input(s.ended(&a3, EndCause::Lost)).expect(&[
            "timer_cleared quiet/build/v1/coder/a3",
            "parked build/v1/coder/a3 lost",
        ]);
    }

    fn a_turn_ended_without_a_reply_gets_one_reminder_on_a_resumed_session_then_a_restart_then_parks(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::TurnComplete)).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        let a2 = s.key("build", 1, "coder", 2);
        let Effect::StartSession { turn, resume, .. } = s.effects_of(&a2).remove(0) else {
            panic!("a start is owed");
        };
        assert!(resume);
        assert_eq!(turn[0].kind, TurnKind::Reminder);

        s.input(s.started(&a2));
        s.input(s.ended(&a2, EndCause::TurnComplete));
        let a3 = s.key("build", 1, "coder", 3);
        let Effect::StartSession { turn, .. } = s.effects_of(&a3).remove(0) else {
            panic!("a start is owed");
        };
        assert_eq!(turn[0].kind, TurnKind::Root, "the plain restart goes back to the first turn");

        s.input(s.started(&a3));
        s.input(s.ended(&a3, EndCause::TurnComplete)).expect(&[
            "timer_cleared quiet/build/v1/coder/a3",
            "parked build/v1/coder/a3 ended_without_reply",
        ]);
    }

    fn a_session_that_never_started_is_stopped_then_retried_then_parked_start_failed(s) {
        let a1 = first(s);
        s.fire("start_deadline/build/v1/coder/a1").expect(&[
            "liveness build/v1/coder/a1 start_failed since=+0s",
        ]);
        assert_eq!(s.effects_of(&a1), vec![Effect::StopSession {
            session_key: a1.clone(), kind: StopKind::Term, nth: 1,
        }]);
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect(&[
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        for attempt in 2..=3 {
            let key = s.key("build", 1, "coder", attempt);
            s.fire(&format!("start_deadline/build/v1/coder/a{attempt}"));
            s.input(s.ended(&key, EndCause::StoppedByHost));
        }
        s.expect(&["parked build/v1/coder/a3 start_failed"]);
    }

    fn a_hung_session_is_restarted_up_to_max_attempts_then_parked_hung(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect(&[
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        for attempt in 2..=3 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.fire(&format!("quiet/build/v1/coder/a{attempt}"));
            s.fire(&format!("confirm/build/v1/coder/a{attempt}"));
            s.input(s.ended(&key, EndCause::StoppedByHost));
        }
        s.expect(&["parked build/v1/coder/a3 hung"]);
    }

    fn stopped_by_host_that_no_state_ordered_counts_as_lost_rather_than_stalling_the_run(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
    }

    fn stopped_by_host_on_an_ordered_stop_is_what_that_stop_said_and_nothing_more(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.withdrawn());
        s.input(s.ended(&a1, EndCause::StoppedByHost)).expect_none();
        assert!(s.state.is_finishing());
    }

    fn a_permission_refusal_parks_with_the_rule_and_the_program_only(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::PermissionRefused {
            rule: "Bash(git push:*)".into(),
            program: "git".into(),
        })).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "parked build/v1/coder/a1 permission_refused",
        ]);
        let status = nexus_run_engine::status(&s.state);
        let member = &status.steps[0].members[0];
        assert_eq!(member.refused.as_deref(), Some("Bash(git push:*): git"));
    }

    fn the_quota_hold_and_the_second_hit_go_back_on_hold_on_a_resumed_session(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        let resets = s.now.plus(Secs::hours(2));
        s.input(s.ended(&a1, EndCause::UsageLimit { resets_at: Some(resets) })).expect(&[
            "timer_cleared quiet/build/v1/coder/a1",
            "held build/v1/coder/a1 usage_limit until=+7200s resets_at=+7200s",
            "timer_set hold_retry/build/v1/coder/a1 +7200s hold_retry",
        ]);
        assert_eq!(s.state.view(), RunView::Held { reason: ReasonCode::UsageLimit, until: resets });

        s.fire("hold_retry/build/v1/coder/a1").expect(&[
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
        let a2 = s.key("build", 1, "coder", 2);
        let Effect::StartSession { resume, .. } = s.effects_of(&a2).remove(0) else {
            panic!("a start is owed");
        };
        assert!(resume, "a hold restarts on the resumed session");

        // cn4p: the resumed session hits the limit at once. That is a hold, not a normal end.
        s.input(s.started(&a2));
        s.input(s.ended(&a2, EndCause::UsageLimit { resets_at: None })).expect(&[
            "timer_cleared quiet/build/v1/coder/a2",
            "held build/v1/coder/a2 usage_limit until=+1800s",
            "timer_set hold_retry/build/v1/coder/a2 +1800s hold_retry",
        ]);
        let index = s.state.find(&a2).unwrap();
        assert_eq!(s.state.slots[index].holds, 1, "only the hold with no named reset counts");
        assert_eq!(s.state.slots[index].restarts, 0, "holds spend no attempts");
    }

    fn a_hold_past_max_holds_parks_hold_limit_and_a_person_starts_the_count_over(s) {
        first(s);
        for attempt in 1..=5 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::ProviderUnavailable));
            s.fire(&format!("hold_retry/build/v1/coder/a{attempt}"));
        }
        let a6 = s.key("build", 1, "coder", 6);
        s.input(s.started(&a6));
        s.input(s.ended(&a6, EndCause::ProviderUnavailable)).expect(&[
            "timer_cleared quiet/build/v1/coder/a6",
            "parked build/v1/coder/a6 hold_limit",
        ]);
        s.input(resume_none()).expect(&[
            "step_started build v1 a7 [coder]",
            "timer_set start_deadline/build/v1/coder/a7 +180s start_deadline",
        ]);
        let a7 = s.state.find(&s.key("build", 1, "coder", 7)).unwrap();
        assert_eq!(s.state.slots[a7].holds, 0);
        assert!(s.state.slots[a7].resume);
    }

    fn a_seven_day_limit_holds_through_its_daily_retries_without_parking(s) {
        first(s);
        let resets = s.now.plus(Secs::hours(7 * 24));
        for attempt in 1..=7 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::UsageLimit { resets_at: Some(resets) }));
            assert!(matches!(s.state.view(), RunView::Held { reason: ReasonCode::UsageLimit, .. }), "day {attempt}");
            assert_eq!(s.state.slots[s.state.find(&key).unwrap()].holds, 0, "a named reset spends no hold");
            s.fire(&format!("hold_retry/build/v1/coder/a{attempt}"));
        }
        assert_eq!(s.now, resets, "the seventh retry is the reset itself");
        let key = s.key("build", 1, "coder", 8);
        s.input(s.started(&key));
        s.input(s.reply(&key, "m-after-the-reset"));
        assert!(matches!(s.slot_state(&key), SlotState::Replied { .. }));
    }

    fn a_usage_limit_with_no_reset_named_still_parks_at_max_holds(s) {
        first(s);
        for attempt in 1..=5 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::UsageLimit { resets_at: None }));
            s.fire(&format!("hold_retry/build/v1/coder/a{attempt}"));
        }
        let a6 = s.key("build", 1, "coder", 6);
        s.input(s.started(&a6));
        s.input(s.ended(&a6, EndCause::UsageLimit { resets_at: None })).expect(&[
            "timer_cleared quiet/build/v1/coder/a6",
            "parked build/v1/coder/a6 hold_limit",
        ]);
    }

    fn holds_with_a_named_reset_leave_the_budget_of_the_ones_without(s) {
        first(s);
        for attempt in 1..=5 {
            let key = s.key("build", 1, "coder", attempt);
            s.input(s.started(&key));
            s.input(s.ended(&key, EndCause::ProviderUnavailable));
            s.fire(&format!("hold_retry/build/v1/coder/a{attempt}"));
        }
        let a6 = s.key("build", 1, "coder", 6);
        s.input(s.started(&a6));
        let resets = s.now.plus(Secs::hours(1));
        s.input(s.ended(&a6, EndCause::UsageLimit { resets_at: Some(resets) }));
        assert!(matches!(s.state.view(), RunView::Held { .. }), "a named reset holds even at the limit");
        s.fire("hold_retry/build/v1/coder/a6");
        let a7 = s.key("build", 1, "coder", 7);
        s.input(s.started(&a7));
        s.input(s.ended(&a7, EndCause::ProviderUnavailable));
        assert!(s.decided("parked build/v1/coder/a7 hold_limit"), "the count stood where it was");
    }

    fn the_hold_retry_is_clamped_to_the_policy_window(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::UsageLimit { resets_at: Some(s.now.plus(Secs(5))) }));
        assert!(s.decisions().contains(&"held build/v1/coder/a1 usage_limit until=+60s resets_at=+5s".to_owned()));
        s.fire("hold_retry/build/v1/coder/a1");
        let a2 = s.key("build", 1, "coder", 2);
        s.input(s.started(&a2));
        s.input(s.ended(&a2, EndCause::UsageLimit { resets_at: Some(s.now.plus(Secs::hours(100))) }));
        assert!(s.decisions().contains(&"held build/v1/coder/a2 usage_limit until=+86400s resets_at=+360000s".to_owned()));
    }

    fn a_hold_retried_by_a_person_before_its_time_clears_the_timer_and_restarts(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::ProviderUnavailable));
        let held = slot("build", 1, "coder", 1);
        s.input(EventBody::Resume { slot: Some(held) }).expect(&[
            "timer_cleared hold_retry/build/v1/coder/a1",
            "step_started build v1 a2 [coder]",
            "timer_set start_deadline/build/v1/coder/a2 +180s start_deadline",
        ]);
    }

    fn a_hold_retried_then_an_escalation_answered_runs_on(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.ended(&a1, EndCause::ProviderUnavailable));
        s.fire("hold_retry/build/v1/coder/a1");
        let a2 = s.key("build", 1, "coder", 2);
        s.input(s.started(&a2));
        s.input(s.reply_with(&a2, "m-ask", true, false));
        let question = format!("escalated/{a2}");
        assert_eq!(s.state.view(), RunView::NeedsDecision { questions: vec![question.clone()] });
        s.input(s.ended(&a2, EndCause::TurnComplete));
        s.input(s.answer(&question, nexus_run_contracts::Disposition::Continue, "m-a")).expect(&[
            "step_started build v1 a3 [coder]",
            "timer_set start_deadline/build/v1/coder/a3 +180s start_deadline",
        ]);
        let record = s.state.find(&s.key("build", 1, "coder", 3)).unwrap();
        assert_eq!(s.state.slots[record].holds, 1, "an answer keeps the hold count");
    }

    fn resume_without_a_slot_picks_the_oldest_condition(s) {
        s.reach_first_session();
        s.pass_until("judge");
        let (old, new) = (s.key("judge", 1, "integrity", 1), s.key("judge", 1, "test-quality", 1));
        s.input(s.ended(&old, EndCause::PermissionRefused { rule: "r".into(), program: "p".into() }));
        s.input(s.ended(&new, EndCause::PermissionRefused { rule: "r".into(), program: "p".into() }));
        s.input(resume_none());
        assert!(s.decisions().contains(&"step_started judge v1 a2 [integrity]".to_owned()));
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::PermissionRefused });
    }

    fn resume_of_a_slot_with_no_condition_changes_nothing(s) {
        first(s);
        s.input(resume_none()).expect(&["ignored out_of_state"]);
        s.input(EventBody::Resume { slot: Some(slot("build", 1, "coder", 1)) }).expect(&["ignored out_of_state"]);
        s.input(EventBody::ResumeRun).expect(&["ignored out_of_state"]);
    }

    fn resume_of_would_not_stop_signals_again_and_never_restarts(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.fire("quiet/build/v1/coder/a1");
        s.fire("confirm/build/v1/coder/a1");
        s.input(s.stop_signalled(&a1, 1, StopKind::Term));
        s.fire("stop_grace/1/build/v1/coder/a1");
        s.input(s.stop_signalled(&a1, 2, StopKind::Term));
        s.fire("stop_grace/2/build/v1/coder/a1").expect(&["parked build/v1/coder/a1 would_not_stop"]);
        assert!(s.effects_of(&a1).is_empty(), "nothing is owed past the second SIGTERM");

        s.input(resume_none()).expect_none();
        assert_eq!(s.effects_of(&a1), vec![Effect::StopSession {
            session_key: a1.clone(), kind: StopKind::Term, nth: 3,
        }]);
        assert!(s.state.latest("build", 1, "coder").is_some_and(|r| r.slot.attempt == 1), "no restart");
        s.input(s.stop_signalled(&a1, 3, StopKind::Term)).expect(&[
            "timer_set stop_grace/1/build/v1/coder/a1 +300s stop_grace",
        ]);
        s.fire("stop_grace/1/build/v1/coder/a1");
        assert_eq!(s.effects_of(&a1), vec![Effect::StopSession {
            session_key: a1.clone(), kind: StopKind::Term, nth: 4,
        }]);
    }

    fn poison_input_parks_the_run_with_the_withdraw_exit_open(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.host_decision(Decision::Parked { slot: None, reason: ReasonCode::PoisonInput });
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::PoisonInput });
        s.input(s.withdrawn());
        assert!(s.decisions().contains(&"finishing withdrawn withdrawn_by_requester".to_owned()));
    }

    fn usage_and_refused_facts_change_no_timer_on_a_running_slot(s) {
        let a1 = first(s);
        s.input(s.started(&a1));
        s.input(s.fact(&a1, nexus_run_contracts::SessionFact::Usage {
            model: "m".into(), input_tokens: 1, output_tokens: 2,
            cache_read_tokens: 3, cache_write_tokens: 4, turns: 1,
        })).expect_none();
        s.input(s.fact(&a1, nexus_run_contracts::SessionFact::PermissionRefused {
            rule: "r".into(), program: "p".into(),
        })).expect_none();
        assert!(matches!(s.slot_state(&a1), SlotState::Running { .. }));
        s.input(s.beat(&a1, HeartbeatKind::ModelOutput));
    }
}
