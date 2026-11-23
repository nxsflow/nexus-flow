//! `Engine::open_space` (RFC 0001, "The library seam"): opens a space by id for a caller without
//! a checkout, such as manufakt.io, so that "on main" is `null`. The store's tests check that the
//! engine then holds no checkout (`crates/store/tests/opened_space.rs`).

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{Engine, SpaceId};
use nexus_flow_store::space::init;

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
