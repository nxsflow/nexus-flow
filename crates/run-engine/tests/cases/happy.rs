//! The happy run.

use nexus_run_contracts::Outcome;
use nexus_run_engine::Effect;

cases! {
    fn a_happy_coding_run_delivers_once_and_releases_the_claim(s) {
        s.input(s.commissioned()).expect(&[
            "root_requested",
            "timer_set root_deadline +60s root_deadline",
        ]);
        assert_eq!(s.effects(), vec![Effect::OpenRoot { run: s.state.run.clone() }]);

        s.input(s.root_opened()).expect(&["timer_cleared root_deadline", "claim_requested"]);
        assert_eq!(s.effects(), vec![Effect::TakeClaim { workspace: s.workspace() }]);

        s.input(s.claim_taken()).expect(&[
            "step_started build v1 a1 [coder]",
            "timer_set tree_window +600s tree_window",
        ]);
        // No StartSession before the tree.
        assert_eq!(s.effects(), vec![Effect::PrepareTree { workspace: s.workspace() }]);

        s.input(s.tree_prepared()).expect(&[
            "timer_cleared tree_window",
            "timer_set start_deadline/build/v1/coder/a1 +180s start_deadline",
        ]);
        assert!(matches!(s.effects()[0], Effect::StartSession { .. }));

        s.run_step_to_pass("build");
        s.run_step_to_pass("verify");
        s.run_fan_out_to_pass("judge");
        assert_eq!(s.state.outcome(), None);
        s.run_step_to_pass("finish");
        assert_eq!(s.state.outcome(), Some(Outcome::Delivered));

        s.input(s.tree_parked_clean()).expect_none();
        s.input(s.delivered()).expect_none();
        s.input(s.claim_released()).expect(&["finished delivered delivered"]);
        assert!(s.effects().is_empty());
        assert_eq!(s.count("finished"), 1);
    }
}
