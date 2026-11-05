//! The status projection (§6.2, contract (d)): what a person reads, from state alone.

use nexus_run_contracts::status::{NeedsKind, RunState as StatusState};
use nexus_run_contracts::{EndCause, Outcome, ParkOutcome, ReasonCode, RunId, Secs};
use nexus_run_engine::status;

const HOLDER: &str = "r-01JBBBBBBBBBBBBBBBBBBBBBBB";

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
        assert_eq!(needs.since, asked);
        assert!(got.next_action.unwrap().contains("nxc reply --thread m-root"));
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
        assert_eq!((needs.kind, needs.question, needs.since), (NeedsKind::Person, None, parked));
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
