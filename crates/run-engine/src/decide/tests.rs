//! Unit tests of what the cases cannot reach through the harness: `decide` on a decision handed as input.

use nexus_run_contracts::{Decision, Event, EventBody, Origin, ReasonCode, RunId, Timestamp};

use super::decide;
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
