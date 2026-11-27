//! Listing the members (RFC 0001, "Public surfaces this RFC brings", `nxs member list`): every
//! current member of the repository space, in member-id order, with its kind and account from its
//! certificate and its role and ticket from its `spaceMember` record. A member nexus-flow did not
//! add, such as the device that created the space, has no record, and its role is `None`.

use std::fs;

use super::{block_on, data_dir, keys, role_files, scratch};
use crate::members::{self, MemberKind};
use crate::opened;
use crate::space::init;
use crate::testing;

#[test]
fn a_new_space_lists_its_device_without_a_record() {
    let base = scratch("member-list-new");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    let list = block_on(members::list(&opened)).unwrap();

    assert_eq!(list.len(), 1, "{list:?}");
    let device = &list[0];
    assert_eq!(device.kind, MemberKind::Device);
    assert_eq!(device.member.len(), 64, "{}", device.member);
    assert_eq!(device.role, None);
    assert_eq!(device.ticket, None);
    assert_eq!(device.parent, None);
    let blocks: Vec<&str> = device.account.split(' ').collect();
    assert_eq!(blocks.len(), 12, "{}", device.account);
    assert!(blocks
        .iter()
        .all(|b| b.len() == 5 && b.bytes().all(|c| c.is_ascii_digit())));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn members_are_listed_in_member_id_order() {
    let base = scratch("member-list-order");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    role_files(&base.join("repo"), &["coder"]);
    for name in ["a", "b", "c"] {
        let invitation = block_on(testing::paired_agent(&data, keys(&data), &base.join(name)));
        let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data)));
        block_on(members::add(
            &opened.unwrap(),
            &invitation,
            "coder",
            None,
            None,
        ))
        .unwrap();
    }
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    let list = block_on(members::list(&opened)).unwrap();

    let ids: Vec<&str> = list.iter().map(|m| m.member.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(list.len(), 4);
    assert_eq!(ids, sorted);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_kinds_print_as_rfc_0001_names_them() {
    assert_eq!(MemberKind::Device.as_str(), "device");
    assert_eq!(MemberKind::Agent.as_str(), "agent");
    assert_eq!(MemberKind::SubAgent.as_str(), "subAgent");
}
