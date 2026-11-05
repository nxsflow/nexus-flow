//! The happy run.

use nexus_run_contracts::Outcome;
use nexus_run_engine::{Effect, TurnKind};

use crate::script::Script;

fn turn_of(s: &Script, key: &str) -> Vec<(TurnKind, Option<String>)> {
    match s.effects_of(key).remove(0) {
        Effect::StartSession { turn, .. } => turn
            .into_iter()
            .map(|part| (part.kind, part.message))
            .collect(),
        other => panic!("a start is owed, not {other:?}"),
    }
}

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
        assert_eq!(s.deliveries().len(), 1, "one result is owed");
        s.input(s.delivered()).expect_none();
        assert_eq!(s.state.delivered.len(), 1);
        assert_eq!(s.effects(), vec![Effect::ReleaseClaim { workspace: s.workspace() }]);
        s.input(s.delivered_key(&format!("result/{}", s.state.run))).expect(&["ignored out_of_state"]);
        s.input(s.claim_released()).expect(&["finished delivered delivered"]);
        assert!(s.effects().is_empty());
        assert_eq!(s.count("finished"), 1);
    }

    fn the_first_step_gets_the_root_and_its_obligation(s) {
        s.to_first_session();
        let key = s.latest_key("build", None);
        assert_eq!(turn_of(s, &key), vec![
            (TurnKind::Root, Some("m-root-message".to_owned())),
            (TurnKind::Obligation, None),
        ]);
    }

    fn verify_declares_no_input_and_is_handed_no_reply(s) {
        s.to_first_session();
        s.run_step_to_pass("build");
        let key = s.latest_key("verify", None);
        assert_eq!(turn_of(s, &key), vec![
            (TurnKind::Root, Some("m-root-message".to_owned())),
            (TurnKind::Obligation, None),
        ]);
    }

    fn judge_gets_the_preceding_reply_and_its_task(s) {
        s.to_first_session();
        s.run_step_to_pass("build");
        let verified = s.run_step_to_pass("verify");
        let key = s.key("judge", 1, "integrity", 1);
        assert_eq!(turn_of(s, &key), vec![
            (TurnKind::Root, Some("m-root-message".to_owned())),
            (TurnKind::Reply { step: "verify".into(), member: "verifier".into() }, Some(verified)),
            (TurnKind::Task, None),
            (TurnKind::Obligation, None),
        ]);
    }

    fn finish_gets_the_replies_of_verify_and_judge_as_it_declares(s) {
        s.to_first_session();
        s.run_step_to_pass("build");
        let verified = s.run_step_to_pass("verify");
        let judged = s.run_fan_out_to_pass("judge");
        let key = s.latest_key("finish", None);
        assert_eq!(turn_of(s, &key), vec![
            (TurnKind::Root, Some("m-root-message".to_owned())),
            (TurnKind::Reply { step: "verify".into(), member: "verifier".into() }, Some(verified)),
            (TurnKind::Reply { step: "judge".into(), member: "__synth__".into() }, Some(judged)),
            (TurnKind::Obligation, None),
        ]);
    }

    fn only_a_step_that_declares_resume_starts_with_it(s) {
        s.to_first_session();
        let build = s.latest_key("build", None);
        assert!(matches!(s.effects_of(&build)[0], Effect::StartSession { resume: true, .. }));
        s.run_step_to_pass("build");
        let verify = s.latest_key("verify", None);
        assert!(matches!(s.effects_of(&verify)[0], Effect::StartSession { resume: false, .. }));
    }
}
