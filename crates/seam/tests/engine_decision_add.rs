//! `Engine::decision_add` (RFC 0003, "The library seam"): the seam's counterpart of
//! `nxs decision add` writes a decision after the checks of "Writing a decision" and returns it
//! in full; `Engine::decisions_with_handle` finds every decision of a handle, which is how a
//! caller learns that the handle it wrote was taken already.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{DecisionError, Engine, Error, NewDecision, SubjectKind};
use nexus_flow_store::space::init;

fn new(title: &str) -> NewDecision {
    NewDecision {
        title: title.to_owned(),
        why: "Because it holds".to_owned(),
        instead_of: "The option not taken".to_owned(),
        topics: vec!["decisions".to_owned()],
        tickets: vec!["NEX-6 to NEX-8".to_owned()],
        date: Some("2026-11-24".to_owned()),
        decided_by: Some("Carsten".to_owned()),
        ..NewDecision::default()
    }
}

#[test]
fn an_engine_writes_a_decision_and_returns_it_in_full() {
    let base = scratch("engine-decision-add");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();

    let added = block_on(engine.decision_add(new("An engine writes it"))).unwrap();

    assert_eq!(added.handle, "2026-11-24-NEX-6-an-engine-writes-it");
    assert_eq!(added.tickets, "NEX-6 to NEX-8");
    assert_eq!(added.subjects.len(), 3);
    assert_eq!(added.subjects[0].kind, SubjectKind::Ticket);
    assert_eq!(added.subjects[0].object, None, "known only by its label");
    assert!(added.in_force);
    assert_eq!(added.on_main, Some(true), "no pr: on main once made");
    assert!(added.superseded_by.is_empty());
    assert!(!added.edited && !added.deleted);
    assert_eq!(
        block_on(engine.decisions_with_handle(&added.handle)).unwrap(),
        [added.id.clone()]
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_refused_decision_is_a_decision_error() {
    let base = scratch("engine-decision-refused");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();
    let mut decision = new("Refused");
    decision.topics.clear();

    let err = block_on(engine.decision_add(decision)).unwrap_err();

    assert!(
        matches!(err, Error::Decision(DecisionError::NoTopic)),
        "{err:?}"
    );
    fs::remove_dir_all(&base).unwrap();
}
