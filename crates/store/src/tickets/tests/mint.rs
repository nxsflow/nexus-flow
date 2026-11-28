//! Minting a key (RFC 0002, "Ticket ids", "The number").

use nocentraldb::Value;

use crate::members::tests::{block_on, Repo};
use crate::members::{self, record};
use crate::tickets::keys::{self, Key, KeyError};

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

/// A member id no member of the space has: a removed member's.
const GONE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// A space in which a removed member held the tag `zz` and created `zz.5`, and the device has
/// taken `zz` over with `nxs member retag`.
fn taken_over(test: &str) -> Repo {
    let repo = Repo::new(test);
    let opened = repo.opened();
    let (id, _) = &opened.spaces()[0];
    block_on(opened.write(id, |w| {
        w.create(
            record::COLLECTION,
            [
                ("member", Value::from(GONE)),
                ("role", Value::from("coder")),
                ("tag", Value::from("zz")),
            ],
        )?;
        w.create(
            keys::TICKETS,
            [
                ("key", Value::from("zz.5")),
                ("type", Value::from("feature")),
                ("title", Value::from("The removed member's ticket")),
                ("status", Value::from("open")),
                ("createdBy", Value::from(GONE)),
            ],
        )
        .map(drop)
    }))
    .unwrap();
    let me = repo.list().into_iter().next().unwrap().member;
    block_on(members::retag(&opened, &me, "zz")).unwrap();
    repo
}

#[test]
fn a_member_waits_for_its_first_sync_before_minting_with_a_freed_tag() {
    let repo = taken_over("key-mint-freed-unsynced");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];

    let refused = block_on(keys::mint(&opened, space));

    assert_eq!(
        refused,
        Err(KeyError::NotSynced {
            space: *space,
            tag: "zz".to_owned()
        })
    );
}
