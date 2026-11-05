//! Unit tests of what the cases cannot reach through the harness: `decide` on a decision handed as input.

use nexus_run_contracts::{
    Decision, Event, EventBody, Origin, Outcome, ReasonCode, RunId, Slot, StepOutcome,
    TimerPurpose, Timestamp, UnknownBody,
};

use super::{decide, decision_key};
use crate::evolve::{evolve, UnknownDecision};
use crate::flow::{FlowDef, WorkingTree};
use crate::state::RunState;

fn def() -> FlowDef {
    FlowDef {
        name: "empty".into(),
        steps: Vec::new(),
        working_tree: WorkingTree::Exclusive,
        ignored_timeouts: Vec::new(),
    }
}

#[test]
fn a_decision_handed_in_as_an_input_changes_nothing() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let state = RunState::initial(run.clone(), def());
    let event = Event::new(
        run,
        7,
        Timestamp::from_unix_millis(0),
        "k",
        Origin::Host,
        EventBody::Decision(Decision::ClaimRequested),
    );
    assert_eq!(
        decide(&state, &event, &state.def),
        vec![Decision::Ignored {
            input_seq: 7,
            reason: ReasonCode::OutOfState
        }]
    );
}

fn unknown(origin: Origin) -> Event {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let unknown = UnknownBody {
        kind: "from_the_future".into(),
        raw: serde_json::json!({ "kind": "from_the_future" }),
    };
    Event::new(
        run,
        9,
        Timestamp::from_unix_millis(0),
        "k",
        origin,
        EventBody::Unknown(unknown),
    )
}

#[test]
fn an_unknown_decision_refuses_to_fold_naming_its_kind_and_seq() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let state = RunState::initial(run, def());
    assert_eq!(
        evolve(state, &unknown(Origin::Reducer)),
        Err(UnknownDecision {
            kind: "from_the_future".into(),
            seq: 9
        })
    );
}

/// A newer writer's value in a decision this version knows the kind of. Folding past it would diverge as
/// silently as an unknown kind, so it refuses the same way, and an input's unknown value does not.
#[test]
fn a_known_decision_holding_an_unknown_value_refuses_to_fold_naming_its_kind_and_seq() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let later = "a_later_value".to_owned();
    let at = Timestamp::from_unix_millis(0);
    for (decision, kind) in [
        (
            Decision::StepFinished {
                step: "s".into(),
                visit: 1,
                outcome: StepOutcome::Unknown(later.clone()),
            },
            "step_finished",
        ),
        (
            Decision::Finishing {
                outcome: Outcome::Unknown(later.clone()),
                reason: None,
            },
            "finishing",
        ),
        (
            Decision::Finished {
                outcome: Outcome::Unknown(later.clone()),
                reason: None,
            },
            "finished",
        ),
        (
            Decision::TimerSet {
                timer: "t".into(),
                fire_at: at,
                purpose: TimerPurpose::Unknown(later.clone()),
            },
            "timer_set",
        ),
    ] {
        let event = Event::new(
            run.clone(),
            9,
            at,
            "k",
            Origin::Reducer,
            EventBody::Decision(decision),
        );
        assert_eq!(
            evolve(RunState::initial(run.clone(), def()), &event),
            Err(UnknownDecision {
                kind: kind.into(),
                seq: 9
            })
        );
    }
}

#[test]
fn an_unknown_input_folds_to_nothing_but_its_place_in_the_log() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let state = RunState::initial(run, def());
    let mut expected = state.clone();
    expected.seq = 9;
    assert_eq!(evolve(state, &unknown(Origin::Host)), Ok(expected));
}

#[test]
fn a_question_is_written_under_its_own_id_and_every_other_decision_under_its_inputs_key() {
    let slot = Slot {
        step: "s".into(),
        visit: 1,
        member: "m".into(),
        attempt: 1,
    };
    let asked = Decision::Escalated {
        question: "escalated/x".into(),
        slot,
        message: "m-1".into(),
    };
    assert_eq!(decision_key("input/7", 2, &asked), "escalated/x");
    let ceiling = Decision::Ceiling {
        question: "ceiling/x".into(),
        step: "s".into(),
        pass: 4,
        last_verdict: "m-1".into(),
    };
    assert_eq!(decision_key("input/7", 0, &ceiling), "ceiling/x");
    assert_eq!(
        decision_key("input/7", 2, &Decision::ClaimRequested),
        "input/7/d2"
    );
}
