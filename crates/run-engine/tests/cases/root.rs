//! `Created` (§6.5): the root deadline counted in periods, the run-level park and its resume.

use nexus_run_contracts::{EventBody, ReasonCode};
use nexus_run_engine::{Effect, RunView};

cases! {
    fn root_failed_is_a_run_level_park_after_max_attempts_periods(s) {
        s.input(s.commissioned());
        s.fire("root_deadline").expect(&["timer_set root_deadline +60s root_deadline"]);
        assert_eq!(s.effects(), vec![Effect::OpenRoot { run: s.state.run.clone() }]);
        s.fire("root_deadline").expect(&["timer_set root_deadline +60s root_deadline"]);
        s.fire("root_deadline").expect(&["parked run root_failed"]);
        assert_eq!(s.state.view(), RunView::Parked { reason: ReasonCode::RootFailed });
        assert!(s.effects().is_empty(), "nothing is posted until a person acts");
    }

    fn resume_run_retries_a_failed_root_for_a_full_budget(s) {
        s.input(s.commissioned());
        for _ in 0..3 {
            s.fire("root_deadline");
        }
        s.input(EventBody::ResumeRun).expect(&["timer_set root_deadline +60s root_deadline"]);
        assert_eq!(s.state.view(), RunView::Created);
        assert_eq!(s.effects(), vec![Effect::OpenRoot { run: s.state.run.clone() }]);
        s.fire("root_deadline").expect(&["timer_set root_deadline +60s root_deadline"]);
    }

    fn a_root_that_opens_late_clears_the_park(s) {
        s.input(s.commissioned());
        for _ in 0..3 {
            s.fire("root_deadline");
        }
        s.input(s.root_opened()).expect(&["claim_requested"]);
        assert_eq!(s.effects(), vec![Effect::TakeClaim { workspace: s.workspace() }]);
    }
}
