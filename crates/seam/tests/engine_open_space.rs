//! `Engine::open_space` (RFC 0001, "The library seam"): opens a space by id for a caller without
//! a checkout, such as manufakt.io, so that "on main" is `null`. The store's tests check that the
//! engine then holds no checkout (`crates/store/tests/opened_space.rs`).
//!
//! The space's kind comes from the marker written when it was created (RFC 0001, "The model"),
//! so an engine on a space opened by id alone takes a write of the space's own kind and refuses
//! one of another kind.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{DecisionError, Engine, Error, NewDecision, SpaceId};
use nexus_flow_store::space::{self, init};
use nexus_flow_store::space_kind::SpaceKind;

#[test]
fn a_space_opens_by_id_without_a_checkout() {
    let base = scratch("space");
    let setup = options(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    fs::remove_dir_all(base.join("repo")).unwrap();

    block_on(Engine::open_space_with(
        space,
        None,
        options(&base.join("data")),
    ))
    .unwrap();
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_opens_by_id_with_a_checkout() {
    let base = scratch("space-git");
    let setup = options(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let space: SpaceId = report.space.parse().unwrap();

    block_on(Engine::open_space_with(
        space,
        Some(&base.join("repo")),
        options(&base.join("data")),
    ))
    .unwrap();
    fs::remove_dir_all(&base).unwrap();
}

fn decision() -> NewDecision {
    NewDecision {
        title: "Written by id alone".to_owned(),
        why: "Because it holds".to_owned(),
        instead_of: "The option not taken".to_owned(),
        topics: vec!["spaces".to_owned()],
        tickets: vec!["NEX-242".to_owned()],
        date: Some("2026-11-28".to_owned()),
        decided_by: Some("Carsten".to_owned()),
        ..NewDecision::default()
    }
}

#[test]
fn a_space_opened_by_id_alone_takes_a_write_of_its_own_kind() {
    let base = scratch("space-kind-own");
    let setup = options(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    fs::remove_dir_all(base.join("repo")).unwrap();
    let engine = block_on(Engine::open_space_with(
        space,
        None,
        options(&base.join("data")),
    ))
    .unwrap();

    let added = block_on(engine.decision_add(decision())).unwrap();

    assert_eq!(
        block_on(engine.decisions_with_handle(&added.handle)).unwrap(),
        [added.id]
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_opened_by_id_alone_refuses_a_write_of_another_kind() {
    let base = scratch("space-kind-other");
    let setup = options(&base.join("data"));
    let coordination = block_on(space::create(
        &setup.data,
        setup.keys.clone(),
        SpaceKind::Coordination,
    ))
    .unwrap();
    let engine = block_on(Engine::open_space_with(coordination, None, setup)).unwrap();

    let err = block_on(engine.decision_add(decision())).unwrap_err();

    let Error::Decision(DecisionError::WrongSpace(err)) = err else {
        panic!("a wrong-kind refusal, got {err:?}");
    };
    assert_eq!(err.collection, "decisionsDecision");
    assert_eq!(err.space, Some(SpaceKind::Coordination));
    fs::remove_dir_all(&base).unwrap();
}
