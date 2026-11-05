//! The status projection (§6.2, contract (d)): what a person reads, from state alone.

use nexus_run_contracts::status::{NeedsKind, RunState as StatusState};
use nexus_run_contracts::{Disposition, EndCause, Outcome, ParkOutcome, ReasonCode, RunId, Secs};
use nexus_run_engine::status;

const HOLDER: &str = "r-01JBBBBBBBBBBBBBBBBBBBBBBB";

/// Asking "why is this stopped?" must not restart a run: a plain reply is an answer only while a question
/// is open, so the texts of a hold and a park send a person to the explicit verb (p71m, the objection).
fn assert_names_the_explicit_action(got: &nexus_run_contracts::RunStatus) {
    let action = got.next_action.as_deref().unwrap();
    assert!(action.contains("nxc resume --thread m-root"), "{action}");
    assert!(action.contains("nxc withdraw --thread m-root"), "{action}");
    assert!(!action.contains("nxc reply"), "{action}");
    let waiting = got.waiting_for.as_deref().unwrap();
    assert!(!waiting.contains("a decision of"), "{waiting}");
}

cases! {
    fn status_carries_the_flow_and_the_claim(s) {
        s.input(s.commissioned());
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Created);
        assert_eq!(got.flow.name, "coding");
        assert_eq!(got.flow.hash, s.def.flow_ref().hash);
        assert_eq!(got.claim.workspace_id, s.workspace());
        assert!(!got.claim.held);
        assert_eq!(got.seq, s.log.last().unwrap().seq);
    }

    fn a_queued_run_names_its_holder(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(nexus_run_contracts::EventBody::ClaimBusy { holder: RunId::parse(HOLDER).unwrap() });
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Queued { behind: RunId::parse(HOLDER) });
        assert_eq!(got.waiting_for, Some(format!("the holder {HOLDER}")));
    }

    fn a_running_step_names_who_owes_the_reply_and_when_the_step_began(s) {
        s.reach_first_session();
        let began = s.now;
        s.advance(90);
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Running);
        assert!(got.claim.held);
        assert_eq!(got.claim.since, Some(began));
        assert_eq!(got.waiting_for.as_deref(), Some("the reply of coder"));
        assert_eq!(got.steps.len(), 1);
        assert_eq!(got.steps[0].id, "build");
        assert_eq!(got.steps[0].started, Some(began));
        assert_eq!(got.steps[0].ended, None);
        assert_eq!(got.steps[0].members[0].state, "starting");
        assert_eq!(got.steps[0].members[0].attempt, 1);
    }

    fn the_last_fact_is_as_of_the_log_and_a_later_clock_changes_nothing(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        let told = s.now;
        let before = status(&s.state);
        s.advance(90);
        let got = status(&s.state);
        assert_eq!(got, before, "status reads no clock");
        assert_eq!(got.steps[0].members[0].liveness.last_fact, Some(told));
    }

    fn a_status_holds_no_commit_and_no_branch_and_leaves_local_to_the_host(s) {
        s.reach_first_session();
        s.pass_until("finish");
        s.run_step_to_pass("finish");
        let parked = ParkOutcome::Committed {
            branch: "nxs/park/a-branch-name".into(),
            commit: "a-commit-id".into(),
        };
        s.input(s.tree_parked(parked));
        let got = status(&s.state);
        assert_eq!(got.local, None);
        let wire = serde_json::to_string(&got).unwrap();
        for evidence in ["a-branch-name", "a-commit-id", "\"commit\"", "\"branch\"", "\"local\""] {
            assert!(!wire.contains(evidence), "{evidence} is in {wire}");
        }
    }

    fn a_finished_step_says_so_with_its_outcome(s) {
        s.reach_first_session();
        s.run_step_to_pass("build");
        let got = status(&s.state);
        let build = got.steps.iter().find(|step| step.id == "build").unwrap();
        assert_eq!(build.state, "done");
        assert_eq!(build.outcome, Some(nexus_run_contracts::StepOutcome::Pass));
        assert_eq!(build.ended, Some(s.now));
    }

    fn a_question_is_named_as_a_decision_with_its_age(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        let asked = s.now;
        s.input(s.reply_with(&key, "m", true, false));
        s.advance(600);
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::NeedsDecision);
        let needs = got.needs.unwrap();
        assert_eq!(needs.kind, NeedsKind::Decision);
        assert_eq!(needs.question, Some(format!("escalated/{key}")));
        assert_eq!(needs.message.as_deref(), Some("m"));
        assert_eq!(needs.since, asked);
        assert!(got.next_action.unwrap().contains("nxc reply --thread m-root"));
    }

    fn the_question_status_names_is_the_key_an_answer_takes(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-why", true, false));
        let question = status(&s.state).needs.unwrap().question.unwrap();
        s.input(s.answer(&question, Disposition::Continue, "m-answer"));
        assert!(
            !s.decisions().iter().any(|line| line.starts_with("ignored")),
            "the answer was taken: {:?}", s.decisions()
        );
        assert!(s.state.open_questions().next().is_none());
    }

    fn once_the_last_question_is_answered_status_no_longer_needs_a_decision(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-why", true, false));
        let question = status(&s.state).needs.unwrap().question.unwrap();
        assert_eq!(status(&s.state).state, StatusState::NeedsDecision);
        s.input(s.answer(&question, Disposition::Continue, "m-answer"));
        let got = status(&s.state);
        assert_eq!(got.needs, None, "a stale needs_decision must not exist");
        assert_eq!(got.state, StatusState::Running);
    }

    fn a_question_is_answered_by_a_reply_and_waits_for_a_decision(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-why", true, false));
        let got = status(&s.state);
        assert_eq!(got.waiting_for.as_deref(), Some("a decision of person"));
        assert!(got.next_action.unwrap().contains("nxc reply --thread m-root"));
    }

    fn a_held_run_names_nxc_resume_and_invites_no_reply(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.ended(&key, EndCause::ProviderUnavailable));
        let got = status(&s.state);
        assert!(matches!(got.state, StatusState::Held { .. }));
        assert_names_the_explicit_action(&got);
    }

    fn a_parked_run_names_nxc_resume_and_invites_no_reply(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.ended(&key, EndCause::PermissionRefused { rule: "r".into(), program: "p".into() }));
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Parked);
        assert_names_the_explicit_action(&got);
    }

    fn a_run_level_park_names_nxc_resume_too(s) {
        s.input(s.commissioned());
        s.fire("root_deadline");
        s.fire("root_deadline");
        s.fire("root_deadline");
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Parked);
        assert_names_the_explicit_action(&got);
    }

    fn a_hold_says_until_when_and_has_no_question(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        let until = s.now.plus(Secs::hours(1));
        s.input(s.ended(&key, EndCause::UsageLimit { resets_at: Some(until) }));
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Held { until });
        assert_eq!(got.reason, Some(ReasonCode::UsageLimit));
        assert_eq!(got.needs, None, "a hold its timer retries leaves nothing open");
    }

    fn a_park_needs_a_person_and_says_since_when(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.advance(600);
        let parked = s.now;
        s.input(s.ended(&key, EndCause::PermissionRefused { rule: "r".into(), program: "p".into() }));
        s.advance(600);
        let got = status(&s.state);
        assert_eq!(got.state, StatusState::Parked);
        assert_eq!(got.reason, Some(ReasonCode::PermissionRefused));
        let needs = got.needs.unwrap();
        assert_eq!(
            (needs.kind, needs.question, needs.message, needs.since),
            (NeedsKind::Person, None, None, parked)
        );
    }

    fn a_run_level_park_says_since_when_too(s) {
        s.input(s.commissioned());
        s.fire("root_deadline");
        s.fire("root_deadline");
        s.fire("root_deadline");
        let got = status(&s.state);
        assert_eq!(got.reason, Some(ReasonCode::RootFailed));
        assert_eq!(got.needs.unwrap().since, s.now);
    }

    fn outcome_is_set_exactly_when_finished_is_true(s) {
        s.reach_first_session();
        s.pass_until("finish");
        s.run_step_to_pass("finish");
        let finishing = status(&s.state);
        assert_eq!(finishing.state, StatusState::Finishing);
        assert!(!finishing.finished);
        assert_eq!(finishing.outcome, None);
        assert_eq!(finishing.waiting_for.as_deref(), Some("the park"));

        s.finish_clean();
        let finished = status(&s.state);
        assert!(finished.finished);
        assert_eq!(finished.state, StatusState::Finished);
        assert_eq!(finished.outcome, Some(Outcome::Delivered));
        assert_eq!(finished.reason, Some(ReasonCode::Delivered));
        assert_eq!(finished.waiting_for, None);
        assert!(!finished.claim.held);
        assert_eq!(s.state.parked, Some(ParkOutcome::Clean { branch: None }));
    }
}
