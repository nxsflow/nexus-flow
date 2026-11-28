//! NEX-245: the device that creates a coordination space gets its `spaceMember` record there, with
//! role `owner` and its tag, as `nxs init` gives it one in the repository space (RFC 0001, "The
//! role record"; RFC 0002, "Ticket ids"; RFC 0009, "Request ids"; NEX-240).

mod common;

use std::fs;
use std::path::Path;

use common::{block_on, options, scratch};
use nexus_flow_seam::{init_with, Engine};

#[test]
fn the_device_that_creates_a_coordination_space_is_listed_there_as_owner_with_its_tag() {
    let base = scratch("engine-coordination-owner");
    let repo = base.join("repo");
    let data = base.join("data");
    block_on(init_with(&repo, options(&data))).unwrap();
    let mut engine = block_on(Engine::open_with(&repo, options(&data))).unwrap();
    let in_repository = block_on(engine.members()).unwrap();

    let created = block_on(engine.coordination_create(Some("central-infra"))).unwrap();
    drop(engine);
    let coordination =
        block_on(Engine::open_space_with(created.coordination, None, options(&data))).unwrap();
    let listed = block_on(coordination.members()).unwrap();

    assert_eq!(listed.len(), 1, "{listed:?}");
    let owner = &listed[0];
    assert_eq!(owner.member, in_repository[0].member);
    assert_eq!(owner.role.as_deref(), Some("owner"));
    assert_eq!(owner.ticket, None);
    assert!(owner.tag.is_some(), "{owner:?}");
    assert_eq!(owner.tag, in_repository[0].tag);
    assert!(!owner.tag_shared);
    remove(&base);
}

fn remove(base: &Path) {
    fs::remove_dir_all(base).unwrap();
}
