//! The back edges, the two counters and the ceiling (§6.4 items 5 and 6). The count is the old engine's:
//! the verdict's pass is `1 + hops`, so `max_passes: 3` is three reworks and the fourth verdict asks
//! (`orchestration.rs:10407`, "sent the work back 3 time(s)").

use nexus_run_contracts::{Disposition, Outcome, RunId};
use nexus_run_engine::{DeliverBody, Effect, RunView, SlotState, TurnKind, TurnPart};

use crate::script::{Script, RUN};

fn ceiling_id(step: &str, visit: u32, pass: u32) -> String {
    format!(
        "ceiling/{}/{step}/v{visit}/p{pass}",
        RunId::parse(RUN).unwrap()
    )
}

/// The back edge taken from `step`, for its verdict `pass`; ends with `build` running the visit it opened.
fn send_back(s: &mut Script, step: &str) {
    s.pass_until(step);
    if step == "judge" {
        s.run_fan_out(step, &["integrity"]);
    } else {
        s.run_step_to_rework(step);
    }
}

/// Reworks up to the ceiling, asserting both counters on every hop, then the verdict that hits it.
fn to_the_ceiling(s: &mut Script, step: &str, first_build_visit: u32) -> String {
    for (pass, visit) in (1..=3).zip(first_build_visit..) {
        send_back(s, step);
        assert_eq!(s.state.passes[step], pass, "{step}'s own count");
        assert_eq!(s.state.visits["build"], visit, "build's visit");
    }
    s.pass_until(step);
    let visit = s.state.visits[step];
    if step == "judge" {
        s.run_fan_out(step, &["integrity"]);
    } else {
        s.run_step_to_rework(step);
    }
    assert_eq!(s.state.passes[step], 3, "the ceiling starts no pass");
    ceiling_id(step, visit, 4)
}

/// The whole list of a verdict that sends the work back, at each hop.
fn walk_edge(s: &mut Script, step: &str, member: &str) {
    s.reach_first_session();
    for pass in 1..=3u32 {
        s.pass_until(step);
        let visit = s.state.visits[step];
        let build = s.state.visits["build"] + 1;
        let key = s.latest_key(step, Some(member));
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-verdict", false, true));
        s.expect(&[
            format!("timer_cleared quiet/{step}/v{visit}/{member}/a1"),
            format!("timer_set wind_down/{step}/v{visit}/{member}/a1 +300s wind_down"),
            format!("step_finished {step} v{visit} needs_rework"),
            format!("rework {step}->build declaring={step} pass={pass}"),
            format!("step_started build v{build} a1 [coder]"),
            format!("timer_set start_deadline/build/v{build}/coder/a1 +180s start_deadline"),
        ]);
        s.input(s.ended(&key, nexus_run_contracts::EndCause::TurnComplete));
    }
    s.pass_until(step);
    let visit = s.state.visits[step];
    let key = s.latest_key(step, Some(member));
    s.input(s.started(&key));
    s.input(s.reply_with(&key, "m-last", false, true));
    s.expect(&[
        format!("timer_cleared quiet/{step}/v{visit}/{member}/a1"),
        format!("timer_set wind_down/{step}/v{visit}/{member}/a1 +300s wind_down"),
        format!("step_finished {step} v{visit} needs_rework"),
        format!(
            "ceiling question={} step={step} pass=4 verdict=m-last",
            ceiling_id(step, visit, 4)
        ),
    ]);
    assert!(matches!(s.state.view(), RunView::NeedsDecision { .. }));
}

cases! {
    fn a_verify_verdict_sends_the_work_back_with_the_verdict_in_the_turn(s) {
        s.reach_first_session();
        s.run_step_to_pass("build");
        let key = s.latest_key("verify", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-red", false, true));
        s.expect(&[
            "timer_cleared quiet/verify/v1/verifier/a1",
            "timer_set wind_down/verify/v1/verifier/a1 +300s wind_down",
            "step_finished verify v1 needs_rework",
            "rework verify->build declaring=verify pass=1",
            "step_started build v2 a1 [coder]",
            "timer_set start_deadline/build/v2/coder/a1 +180s start_deadline",
        ]);
        let build = s.latest_key("build", None);
        let Effect::StartSession { turn, resume, .. } = s.effects_of(&build).remove(0) else {
            panic!("the coder is owed a start");
        };
        assert!(resume, "build declares resume");
        assert_eq!(turn, vec![
            TurnPart { kind: TurnKind::Root, message: Some("m-root-message".into()) },
            TurnPart {
                kind: TurnKind::Reply { step: "verify".into(), member: "verifier".into() },
                message: Some("m-red".into()),
            },
            TurnPart { kind: TurnKind::Obligation, message: None },
        ]);
    }

    fn the_verify_edge_counts_three_passes_and_the_fourth_verdict_asks(s) {
        walk_edge(s, "verify", "verifier");
    }

    fn the_finish_edge_counts_three_passes_and_the_fourth_verdict_asks(s) {
        walk_edge(s, "finish", "finisher");
    }

    fn the_judge_edge_counts_three_passes_and_the_fourth_verdict_asks(s) {
        s.reach_first_session();
        let question = to_the_ceiling(s, "judge", 2);
        assert_eq!(s.state.visits["build"], 4);
        let synth_message = s.state.verdict_of("judge").unwrap().message;
        assert!(s.decided(&format!(
            "ceiling question={question} step=judge pass=4 verdict={synth_message}"
        )));
        assert_eq!(status_reason(s), "ceiling");
    }

    fn verify_rework_then_judge_rework_starts_build_v2_and_v3_on_two_counts(s) {
        s.reach_first_session();
        send_back(s, "verify");
        assert_eq!(s.key("build", 2, "coder", 1), s.latest_key("build", None));
        send_back(s, "judge");
        assert_eq!(s.key("build", 3, "coder", 1), s.latest_key("build", None));
        assert_eq!(s.state.passes["verify"], 1);
        assert_eq!(s.state.passes["judge"], 1);
        assert_eq!(s.state.passes.get("finish"), None);
    }

    fn verify_reworks_never_spend_the_ceiling_of_judge(s) {
        s.reach_first_session();
        for _ in 0..3 {
            send_back(s, "verify");
        }
        for pass in 1..=3 {
            send_back(s, "judge");
            assert_eq!(s.state.passes["judge"], pass);
            assert!(matches!(s.state.view(), RunView::Running { .. }));
        }
        assert_eq!(s.state.passes["verify"], 3);
        // verify, judge and finish each still have their own count: build ran seven times in all.
        assert_eq!(s.state.visits["build"], 7);
    }

    fn the_ceiling_holds_the_claim_and_owes_one_question(s) {
        s.reach_first_session();
        let question = to_the_ceiling(s, "verify", 2);
        let key = format!("question/{question}");
        assert_eq!(s.deliveries(), vec![(key.clone(), DeliverBody::Question(question.clone()))]);
        assert!(s.state.claim_since.is_some());
        assert!(!s.effects().iter().any(|e| matches!(e, Effect::ReleaseClaim { .. })));
        assert_eq!(s.state.outcome(), None);

        s.input(s.delivered()).expect_none();
        assert!(s.deliveries().is_empty(), "the question is delivered once");
    }

    fn continue_at_the_ceiling_buys_one_pass_and_the_next_ceiling_asks_again(s) {
        s.reach_first_session();
        let question = to_the_ceiling(s, "verify", 2);
        s.input(s.answer(&question, Disposition::Continue, "m-go-on")).expect(&[
            "rework verify->build declaring=verify pass=4",
            "step_started build v5 a1 [coder]",
            "timer_set start_deadline/build/v5/coder/a1 +180s start_deadline",
        ]);
        assert!(matches!(s.state.view(), RunView::Running { .. }));

        // One more verdict is over the ceiling at once: exactly one pass was bought.
        s.pass_until("verify");
        let key = s.latest_key("verify", None);
        let visit = s.state.visits["verify"];
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-again", false, true));
        let second = ceiling_id("verify", visit, 5);
        assert!(s.decisions().contains(&format!(
            "ceiling question={second} step=verify pass=5 verdict=m-again"
        )), "{:?}", s.decisions());
        assert!(matches!(s.state.view(), RunView::NeedsDecision { .. }));
    }

    fn accept_at_the_ceiling_starts_the_next_step_with_the_overruled_verdict_and_the_reasons(s) {
        s.reach_first_session();
        let question = to_the_ceiling(s, "verify", 2);
        let verdict = s.state.verdict_of("verify").unwrap().message;
        s.input(s.answer(&question, Disposition::Accept, "m-reasons")).expect(&[
            "step_started judge v1 a1 [code-quality,test-quality,integrity]",
            "timer_set start_deadline/judge/v1/code-quality/a1 +180s start_deadline",
            "timer_set start_deadline/judge/v1/test-quality/a1 +180s start_deadline",
            "timer_set start_deadline/judge/v1/integrity/a1 +180s start_deadline",
        ]);
        let key = s.key("judge", 1, "integrity", 1);
        let Effect::StartSession { turn, .. } = s.effects_of(&key).remove(0) else {
            panic!("a start is owed");
        };
        assert_eq!(turn, vec![
            TurnPart { kind: TurnKind::Root, message: Some("m-root-message".into()) },
            TurnPart { kind: TurnKind::Answer, message: Some("m-reasons".into()) },
            TurnPart {
                kind: TurnKind::Reply { step: "verify".into(), member: "verifier".into() },
                message: Some(verdict),
            },
            TurnPart { kind: TurnKind::Task, message: None },
            TurnPart { kind: TurnKind::Obligation, message: None },
        ]);
    }

    fn accept_at_the_last_steps_ceiling_ends_the_run_accepted_by_requester(s) {
        s.reach_first_session();
        let question = to_the_ceiling(s, "finish", 2);
        let verdict = s.state.verdict_of("finish").unwrap().message;
        s.input(s.answer(&question, Disposition::Accept, "m-reasons")).expect(&[
            "finishing accepted accepted_by_requester",
        ]);
        s.input(s.tree_parked_clean()).expect_none();
        let key = format!("result/{}", s.state.run);
        assert_eq!(s.deliveries(), vec![(key, DeliverBody::Message(verdict))]);
        s.input(s.delivered()).expect_none();
        s.input(s.claim_released()).expect(&["finished accepted accepted_by_requester"]);
        assert_eq!(s.state.outcome(), Some(Outcome::Accepted));
    }

    fn a_needs_rework_bit_on_a_step_with_no_back_edge_passes(s) {
        s.reach_first_session();
        let key = s.latest_key("build", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-x", false, true));
        assert!(s.decisions().contains(&"step_finished build v1 pass".to_owned()));
    }

    fn a_later_ceiling_of_the_same_step_and_pass_after_an_accept_is_a_new_question(s) {
        s.reach_first_session();
        let first = to_the_ceiling(s, "verify", 2);
        s.input(s.answer(&first, Disposition::Accept, "m-accept"));
        send_back(s, "judge");
        s.pass_until("verify");
        let key = s.latest_key("verify", None);
        s.input(s.started(&key));
        s.input(s.reply_with(&key, "m-again", false, true));
        let open: Vec<String> = s.state.open_questions().map(|q| q.id.clone()).collect();
        assert_eq!(open.len(), 1);
        assert_ne!(open[0], first);
        assert!(matches!(s.slot_state(&key), SlotState::Replied { .. }));
    }
}

fn status_reason(s: &Script) -> String {
    nexus_run_engine::status(&s.state, 0, s.now)
        .reason
        .map(|r| r.as_str().to_owned())
        .unwrap_or_default()
}
