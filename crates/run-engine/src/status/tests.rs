use nexus_run_contracts::status::RunState as StatusState;
use nexus_run_contracts::{RunId, Timestamp};

use super::status;
use crate::flow::{FlowDef, WorkingTree};
use crate::state::RunState;

#[test]
fn a_run_nothing_has_folded_into_says_created_and_reports_its_pending_inputs() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let def = FlowDef {
        name: "empty".into(),
        steps: Vec::new(),
        working_tree: WorkingTree::Exclusive,
        ignored_timeouts: Vec::new(),
    };
    let got = status(&RunState::initial(run, def), 3, Timestamp::from_unix_millis(0));
    assert_eq!(got.state, StatusState::Created);
    assert_eq!(got.pending_inputs, 3);
    assert!(!got.finished);
    assert_eq!(got.outcome, None);
}
