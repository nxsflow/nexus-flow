//! `Engine::member_add`, `Engine::member_remove` and `Engine::members` (RFC 0001, "The library
//! seam"): the seam's counterparts of `nxs member add`, `remove` and `list`, on an open engine.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{Engine, Error, MemberError, MemberKind};
use nexus_flow_store::space::init;
use nexus_flow_store::testing;

#[test]
fn an_engine_adds_lists_and_removes_a_member_with_its_role_record() {
    let base = scratch("engine-members");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let invitation = block_on(testing::paired_agent(
        &setup.data,
        setup.keys.clone(),
        &base.join("pm"),
    ));
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();

    let added = block_on(engine.member_add(&invitation, "pm", None)).unwrap();
    let listed = block_on(engine.members()).unwrap();
    let removed = block_on(engine.member_remove(&added.member)).unwrap();
    let after = block_on(engine.members()).unwrap();

    assert_eq!(added.kind, MemberKind::Agent);
    assert_eq!(added.role.as_deref(), Some("pm"));
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&added));
    assert_eq!(removed, added);
    assert_eq!(after.len(), 1);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_refused_add_is_a_member_error() {
    let base = scratch("engine-members-refused");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();

    let err = block_on(engine.member_add("not an invitation", "pm", None)).unwrap_err();

    assert!(
        matches!(err, Error::Member(MemberError::NotAnInvitation { .. })),
        "{err:?}"
    );
    fs::remove_dir_all(&base).unwrap();
}
