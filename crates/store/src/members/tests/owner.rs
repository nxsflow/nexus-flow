//! The record of the device that creates the space (RFC 0001, "The role record"; RFC 0002,
//! "Ticket ids"; decided by the owner, NEX-240): `init` writes it, with role `owner` and a free
//! tag, so the first device can mint ticket keys at once. Run again, `init` writes no second one.

use std::fs;

use super::{block_on, data_dir, keys, scratch};
use crate::members::{self, record, MemberKind};
use crate::opened;
use crate::space::init;

#[test]
fn init_gives_the_device_that_creates_the_space_its_owner_record_and_a_tag() {
    let base = scratch("member-owner");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    let list = block_on(members::list(&opened)).unwrap();

    assert_eq!(list.len(), 1, "{list:?}");
    let device = &list[0];
    assert_eq!(device.kind, MemberKind::Device);
    assert_eq!(device.role.as_deref(), Some("owner"));
    assert_eq!(device.ticket, None);
    let tag = device.tag.as_deref().expect("the device has a tag");
    assert!(members::parse_tag(tag).is_ok(), "{tag}");
    assert!(!device.tag_shared);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_run_twice_writes_no_second_record() {
    let base = scratch("member-owner-twice");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();
    let first = block_on(members::list(&opened)).unwrap();
    drop(opened);

    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();

    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();
    let (_, space) = &opened.spaces()[0];
    let records = block_on(record::tags(opened.db(), space.as_ref().unwrap())).unwrap();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(block_on(members::list(&opened)).unwrap(), first);
    fs::remove_dir_all(&base).unwrap();
}
