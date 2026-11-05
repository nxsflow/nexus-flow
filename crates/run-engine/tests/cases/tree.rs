//! The claim, the working directory and the window (§6.5, §6.6): no session before the tree, refusals, the
//! run-level resume, and a late result in `Finishing`.

use nexus_run_contracts::{
    Base, EndCause, EventBody, GitReason, Outcome, ParkOutcome, ReasonCode, RunId, SequenceKind,
    TreeRefusal,
};
use nexus_run_engine::{DeliverBody, Effect, RunView};

use crate::script::Script;

const HOLDER_A: &str = "r-01JBBBBBBBBBBBBBBBBBBBBBBB";
const HOLDER_B: &str = "r-01JCCCCCCCCCCCCCCCCCCCCCCC";

fn busy(holder: &str) -> EventBody {
    EventBody::ClaimBusy {
        holder: RunId::parse(holder).unwrap(),
    }
}

fn unprepared(s: &mut Script) {
    s.input(s.commissioned());
    s.input(s.root_opened());
    s.input(s.claim_taken());
}

fn transient() -> TreeRefusal {
    TreeRefusal::Git {
        reason: GitReason::FetchFailed,
    }
}

cases! {
    fn no_timer_and_no_session_before_the_tree_is_prepared(s) {
        unprepared(s);
        assert_eq!(s.effects(), vec![Effect::PrepareTree { workspace: s.workspace() }]);
        assert_eq!(s.state.timers.keys().collect::<Vec<_>>(), ["tree_window"]);
        assert_eq!(s.state.view(), RunView::Running { tree: nexus_run_engine::TreePhase::Unprepared });
        s.input(s.tree_prepared()).expect(&[
            "timer_cleared tree_window",
            "timer_set start_deadline/build/v1/coder/a1 +180s start_deadline",
        ]);
        assert_eq!(s.state.base, Some(Base { branch: "main".into(), commit: "c0".into() }));
    }

    fn what_prepare_rescued_is_named_in_state_for_the_host_to_report(s) {
        unprepared(s);
        let found = ParkOutcome::Committed { branch: "nxs/park/old".into(), commit: "abc".into() };
        s.input(EventBody::TreePrepared { base: s.base(), found: Some(found.clone()) });
        assert_eq!(s.state.found, Some(found));
    }

    fn a_permanent_refusal_parks_the_run_at_once(s) {
        unprepared(s);
        s.input(s.tree_refused(TreeRefusal::ForeignDirectory)).expect(&[
            "timer_cleared tree_window",
            "parked run tree_refused",
        ]);
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::TreeRefused });
        assert!(s.effects().is_empty(), "no retry of a directory a person has to move");

        let mut other = Script::coding();
        unprepared(&mut other);
        other.input(other.tree_refused(TreeRefusal::NotARepository));
        assert_eq!(other.state.view(), RunView::Parked { reason: ReasonCode::TreeRefused });
    }

    fn a_transient_refusal_inside_the_window_is_counted_and_prepare_stays_owed(s) {
        unprepared(s);
        s.input(s.tree_refused(transient())).expect_none();
        s.input(s.tree_refused(TreeRefusal::MidSequence { what: SequenceKind::Merge })).expect_none();
        assert_eq!(s.state.tree_refusals, 2);
        assert_eq!(s.effects(), vec![Effect::PrepareTree { workspace: s.workspace() }]);
        assert_eq!(s.state.view(), RunView::Running { tree: nexus_run_engine::TreePhase::Unprepared });
    }

    fn the_window_closing_parks_the_run_and_owes_the_cancel_of_what_is_in_flight(s) {
        unprepared(s);
        s.fire("tree_window").expect(&["parked run tree_refused"]);
        assert_eq!(s.effects(), vec![Effect::CancelTree { workspace: s.workspace() }]);
        // The host cancels, and its refusal is the count; nothing more is owed until a person acts.
        s.input(s.tree_refused(TreeRefusal::Git { reason: GitReason::BoundExceeded })).expect_none();
        assert_eq!(s.state.tree_refusals, 1);
        assert!(s.effects().is_empty());
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::TreeRefused });
    }

    fn a_result_that_outran_the_cancel_still_prepares_the_run(s) {
        unprepared(s);
        s.fire("tree_window");
        s.input(s.tree_prepared()).expect(&[
            "timer_set start_deadline/build/v1/coder/a1 +180s start_deadline",
        ]);
        assert_eq!(s.state.view(), RunView::Running { tree: nexus_run_engine::TreePhase::Prepared });
        assert!(!s.effects().iter().any(|e| matches!(e, Effect::CancelTree { .. })));
    }

    fn refuse_resume_refuse_resume_prepare(s) {
        unprepared(s);
        s.input(s.tree_refused(TreeRefusal::ForeignDirectory)).expect(&[
            "timer_cleared tree_window",
            "parked run tree_refused",
        ]);
        let run = s.state.run.clone();
        assert_eq!(s.state.tree_refused_key(), format!("tree_refused/{run}/1"));

        s.input(EventBody::ResumeRun).expect(&["timer_set tree_window +600s tree_window"]);
        assert_eq!(s.effects(), vec![Effect::PrepareTree { workspace: s.workspace() }]);
        assert_eq!(s.state.view(), RunView::Running { tree: nexus_run_engine::TreePhase::Unprepared });

        s.input(s.tree_refused(TreeRefusal::ForeignDirectory)).expect(&[
            "timer_cleared tree_window",
            "parked run tree_refused",
        ]);
        assert_eq!(s.state.tree_refused_key(), format!("tree_refused/{run}/2"));

        s.input(EventBody::ResumeRun).expect(&["timer_set tree_window +600s tree_window"]);
        assert_eq!(s.effects(), vec![Effect::PrepareTree { workspace: s.workspace() }]);

        s.input(s.tree_prepared()).expect(&[
            "timer_cleared tree_window",
            "timer_set start_deadline/build/v1/coder/a1 +180s start_deadline",
        ]);
    }

    fn a_late_tree_prepared_folds_as_already_finishing(s) {
        unprepared(s);
        s.input(s.withdrawn());
        s.input(s.tree_prepared()).expect(&["ignored already_finishing"]);
        assert_eq!(s.state.base, None);
        assert_eq!(s.effects(), vec![Effect::ParkTree {
            workspace: s.workspace(), occasion: Outcome::Withdrawn,
        }]);
    }

    fn a_late_tree_refused_folds_as_already_finishing(s) {
        unprepared(s);
        s.input(s.withdrawn());
        s.input(s.tree_refused(transient())).expect(&["ignored already_finishing"]);
        assert_eq!(s.state.tree_refusals, 0);
    }

    fn a_claim_taken_that_a_withdrawal_overtook_folds_as_already_finishing(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(s.withdrawn());
        s.input(s.claim_taken()).expect(&["ignored already_finishing"]);
    }

    fn a_queued_run_withdrawn_records_not_held_and_releases_without_touching_a_tree(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(busy(HOLDER_A)).expect(&[format!("run_queued behind={HOLDER_A}")]);
        s.input(s.withdrawn()).expect(&["finishing withdrawn withdrawn_by_requester"]);
        assert_eq!(s.effects(), vec![Effect::ParkTree {
            workspace: s.workspace(), occasion: Outcome::Withdrawn,
        }]);
        s.input(s.tree_parked(ParkOutcome::NotHeld)).expect_none();
        let key = format!("result/{}", s.state.run);
        assert_eq!(s.deliveries(), vec![(key, DeliverBody::Discharge {
            step: None, pass: 0, verdict: None, parked: Some(ParkOutcome::NotHeld),
        })]);
        s.input(s.delivered()).expect_none();
        assert_eq!(s.effects(), vec![Effect::ReleaseClaim { workspace: s.workspace() }]);
        s.input(s.claim_released()).expect(&["finished withdrawn withdrawn_by_requester"]);
    }

    fn a_claim_busy_logs_one_per_holder_change_and_an_unchanged_holder_is_nothing(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(busy(HOLDER_A)).expect(&[format!("run_queued behind={HOLDER_A}")]);
        assert_eq!(s.state.claim_changes, 1);
        s.input(busy(HOLDER_A)).expect(&["ignored out_of_state"]);
        assert_eq!(s.state.claim_changes, 1);
        s.input(busy(HOLDER_B)).expect(&[format!("run_queued behind={HOLDER_B}")]);
        s.input(busy(HOLDER_A)).expect(&[format!("run_queued behind={HOLDER_A}")]);
        let run = s.state.run.clone();
        assert_eq!(s.state.claim_busy_key(), format!("claim_busy/{run}/3"), "A -> B -> A logs three");
        assert_eq!(s.effects(), vec![Effect::TakeClaim { workspace: s.workspace() }]);
    }

    fn the_queued_run_takes_the_claim_when_the_holder_lets_go(s) {
        s.input(s.commissioned());
        s.input(s.root_opened());
        s.input(busy(HOLDER_A));
        s.input(s.claim_taken()).expect(&[
            "step_started build v1 a1 [coder]",
            "timer_set tree_window +600s tree_window",
        ]);
    }

    fn a_missing_directory_at_the_park_releases_because_nothing_is_left_to_save(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.withdrawn());
        s.input(s.ended(&key, EndCause::StoppedByHost));
        s.input(s.tree_parked(ParkOutcome::Missing)).expect_none();
        s.input(s.delivered());
        s.input(s.claim_released()).expect(&["finished withdrawn withdrawn_by_requester"]);
    }
}
