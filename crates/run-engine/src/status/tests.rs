use nexus_run_contracts::status::RunState as StatusState;
use nexus_run_contracts::RunId;

use super::status;
use crate::flow::{FlowDef, WorkingTree};
use crate::state::RunState;

#[test]
fn a_run_nothing_has_folded_into_says_created() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let def = FlowDef {
        name: "empty".into(),
        steps: Vec::new(),
        working_tree: WorkingTree::Exclusive,
        ignored_timeouts: Vec::new(),
    };
    let got = status(&RunState::initial(run, def));
    assert_eq!(got.state, StatusState::Created);
    assert!(!got.finished);
    assert_eq!(got.outcome, None);
}
