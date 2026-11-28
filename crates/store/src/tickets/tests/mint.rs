//! Minting a key (RFC 0002, "Ticket ids", "The number").

use crate::members::tests::{block_on, Repo};
use crate::tickets::keys::{self, Key};

/// The tag of the device's own record in `repo`, which `nxs init` wrote.
fn own_tag(repo: &Repo) -> String {
    let me = repo.list().into_iter().next().expect("the device is listed");
    me.tag.expect("nxs init gave the device a tag")
}

#[test]
fn a_member_with_no_keys_starts_at_1() {
    let repo = Repo::new("key-mint-first");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];

    let key = block_on(keys::mint(&opened, space)).unwrap();

    assert_eq!(key.to_string(), format!("{}.1", own_tag(&repo)));
    assert_eq!(key.to_string().parse::<Key>(), Ok(key));
}
