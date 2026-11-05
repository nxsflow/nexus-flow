use std::collections::BTreeSet;

use nexus_run_contracts::status::{RunState as StatusState, MEMBER_STATES, STEP_STATES};
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

/// Every row of the slot table, so the eight words are not only what the scripted cases happened to reach.
#[test]
fn the_slot_table_writes_only_the_published_words_and_all_of_them() {
    let mut seen = BTreeSet::new();
    for example in RunState::slot_examples() {
        for step in status(&example.run).steps {
            assert!(STEP_STATES.contains(&step.state.as_str()), "{}", step.state);
            seen.extend(step.members.into_iter().map(|member| member.state));
        }
    }
    let published: BTreeSet<String> = MEMBER_STATES.iter().map(|w| (*w).to_owned()).collect();
    assert_eq!(seen, published);
}
