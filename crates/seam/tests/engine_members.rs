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
    fs::create_dir_all(base.join("repo/factory/roles")).unwrap();
    fs::write(base.join("repo/factory/roles/pm.md"), "# pm\n").unwrap();
    let invitation = block_on(testing::paired_agent(
        &setup.data,
        setup.keys.clone(),
        &base.join("pm"),
    ));
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();

    let added = block_on(engine.member_add(&invitation, "pm", None, None)).unwrap();
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

    let err = block_on(engine.member_add("not an invitation", "pm", None, None)).unwrap_err();

    assert!(
        matches!(err, Error::Member(MemberError::NotAnInvitation { .. })),
        "{err:?}"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_engine_adds_with_a_tag_and_retags() {
    let (base, invitations) = coder_space("engine-members-retag", &["coder"]);
    let engine = engine(&base);

    let added = block_on(engine.member_add(&invitations[0], "coder", None, Some("ka"))).unwrap();
    let retagged = block_on(engine.member_retag(&added.member, "kb")).unwrap();
    let held = block_on(engine.member_retag(&added.member, "K1")).unwrap_err();

    assert_eq!(added.tag.as_deref(), Some("ka"));
    assert_eq!(retagged.tag.as_deref(), Some("kb"));
    assert!(block_on(engine.members()).unwrap().contains(&retagged));
    assert!(
        matches!(held, Error::Member(MemberError::NotATag { .. })),
        "{held:?}"
    );
    fs::remove_dir_all(&base).unwrap();
}

/// The tag the device holds in every [`coder_space`]: `init` picks the device's tag by its random
/// id, so the tests give it one that no test names (NEX-248).
const DEVICE_TAG: &str = "zy";

#[test]
fn the_device_of_a_coder_space_holds_the_tag_no_test_names() {
    let (base, _) = coder_space("engine-members-device", &[]);

    let members = block_on(engine(&base).members()).unwrap();

    let device = members.iter().find(|m| m.kind == MemberKind::Device);
    assert_eq!(device.and_then(|m| m.tag.as_deref()), Some(DEVICE_TAG));
    fs::remove_dir_all(&base).unwrap();
}

/// An engine on a fresh repository space whose roles include `coder`, and the invitations of
/// `agents` new agents paired into its account.
fn coder_space(test: &str, agents: &[&str]) -> (std::path::PathBuf, Vec<String>) {
    let base = scratch(test);
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    fs::create_dir_all(base.join("repo/factory/roles")).unwrap();
    fs::write(base.join("repo/factory/roles/coder.md"), "# coder\n").unwrap();
    let invitations = agents
        .iter()
        .map(|a| {
            block_on(testing::paired_agent(
                &setup.data,
                setup.keys.clone(),
                &base.join(a),
            ))
        })
        .collect();
    retag_device(&engine(&base));
    (base, invitations)
}

/// Gives the device [`DEVICE_TAG`], in place of the tag `init` picked for it.
fn retag_device(engine: &Engine) {
    let device = block_on(engine.members())
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MemberKind::Device)
        .expect("the device that created the space is a member");
    block_on(engine.member_retag(&device.member, DEVICE_TAG)).unwrap();
}

fn engine(base: &std::path::Path) -> Engine {
    block_on(Engine::open_with(
        &base.join("repo"),
        options(&base.join("data")),
    ))
    .unwrap()
}

#[test]
fn a_tag_only_a_removed_member_held_is_handed_out_by_tag_and_by_retag() {
    let (base, invitations) = coder_space("engine-members-freed", &["a", "b", "c"]);
    let engine = engine(&base);
    let removed = block_on(engine.member_add(&invitations[0], "coder", None, Some("ka"))).unwrap();
    block_on(engine.member_remove(&removed.member)).unwrap();

    let named = block_on(engine.member_add(&invitations[1], "coder", None, Some("ka"))).unwrap();
    block_on(engine.member_remove(&named.member)).unwrap();
    let other = block_on(engine.member_add(&invitations[2], "coder", None, Some("kb"))).unwrap();
    let retagged = block_on(engine.member_retag(&other.member, "ka")).unwrap();

    assert_eq!(named.tag.as_deref(), Some("ka"));
    assert_eq!(retagged.tag.as_deref(), Some("ka"));
    assert!(block_on(engine.members()).unwrap().contains(&retagged));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_tag_only_a_removed_member_held_is_picked_and_then_no_tag_is_free() {
    let (base, invitations) = coder_space("engine-members-freed-pick", &["a", "b", "c"]);
    let removed = {
        let engine = engine(&base);
        let removed =
            block_on(engine.member_add(&invitations[0], "coder", None, Some("ka"))).unwrap();
        block_on(engine.member_remove(&removed.member)).unwrap();
        removed
    };
    let setup = options(&base.join("data"));
    block_on(testing::hold_every_tag_but(
        &base.join("repo"),
        &setup.data,
        setup.keys,
        Some("ka"),
    ));
    let engine = engine(&base);

    let picked = block_on(engine.member_add(&invitations[1], "coder", None, None)).unwrap();
    let none = block_on(engine.member_add(&invitations[2], "coder", None, None)).unwrap_err();

    assert_ne!(picked.member, removed.member);
    assert_eq!(picked.tag.as_deref(), Some("ka"));
    assert!(
        matches!(none, Error::Member(MemberError::NoFreeTag)),
        "{none:?}"
    );
    fs::remove_dir_all(&base).unwrap();
}
