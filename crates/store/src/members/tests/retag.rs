//! Giving a member a new tag (RFC 0002, "Two members with one tag"): `nxs member retag` writes
//! the new tag into the member's records, under the rules `nxs member add` follows, and the keys
//! already written keep the tag they were written with.

use nocentraldb::Value;

use super::add::find;
use super::tagged::set_tag;
use super::{block_on, Repo};
use crate::members::{self, Member, MemberError, MemberKind};

fn retag(repo: &Repo, member: &str, tag: &str) -> Result<Member, MemberError> {
    block_on(members::retag(&repo.opened(), member, tag))
}

#[test]
fn retag_writes_the_new_tag_and_keeps_the_rest_of_the_record() {
    let repo = Repo::new("member-retag");
    let ticket = format!("{}/00112233445566778899aabbccddeeff", repo.space);
    let added = repo.add(&repo.agent("a"), "coder", Some(&ticket)).unwrap();

    let retagged = retag(&repo, &added.member, "kb").unwrap();

    assert_eq!(retagged.tag.as_deref(), Some("kb"));
    let listed = find(&repo.list(), &added.member).clone();
    assert_eq!(listed, retagged);
    assert_eq!(listed.role.as_deref(), Some("coder"));
    assert_eq!(listed.ticket.map(|t| t.to_string()), Some(ticket));
}

#[test]
fn keys_already_written_keep_their_tag() {
    let repo = Repo::new("member-retag-keys");
    let added = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    let opened = repo.opened();
    let (id, space) = &opened.spaces()[0];
    let written = block_on(opened.write(id, |w| {
        w.create(
            "ticketsTicket",
            [
                ("key", Value::from("ka.1")),
                ("type", Value::from("feature")),
                ("title", Value::from("A ticket")),
                ("status", Value::from("open")),
            ],
        )
        .map(|_| ())
    }))
    .unwrap();
    let ticket = written.ids[0];

    retag(&repo, &added.member, "kb").unwrap();

    let object = block_on(space.as_ref().unwrap().get("ticketsTicket", ticket))
        .unwrap()
        .unwrap();
    assert_eq!(object.get::<&str>("key").unwrap(), Some("ka.1"));
}

#[test]
fn the_device_without_a_record_gets_one_with_its_tag() {
    let repo = Repo::new("member-retag-device");
    let device = repo
        .list()
        .into_iter()
        .find(|m| m.kind == MemberKind::Device)
        .unwrap();

    let retagged = retag(&repo, &device.member, "ka").unwrap();

    assert_eq!(retagged.tag.as_deref(), Some("ka"));
    assert_eq!(retagged.role, None);
    assert_eq!(find(&repo.list(), &device.member), &retagged);
}

#[test]
fn retag_to_the_tag_a_member_already_holds_changes_nothing() {
    let repo = Repo::new("member-retag-same");
    let added = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();

    assert_eq!(retag(&repo, &added.member, "ka").unwrap(), added);
}

#[test]
fn retag_to_a_tag_only_a_removed_member_held_writes_it() {
    let repo = Repo::new("member-retag-freed");
    let removed = repo.add_tagged(&repo.agent("a"), "coder", "kc").unwrap();
    block_on(members::remove(&repo.opened(), &removed.member)).unwrap();
    let other = repo.add_tagged(&repo.agent("b"), "coder", "kb").unwrap();

    let retagged = retag(&repo, &other.member, "kc").unwrap();

    assert_eq!(retagged.tag.as_deref(), Some("kc"));
    assert_eq!(find(&repo.list(), &other.member), &retagged);
}

#[test]
fn each_refusal_of_retag_changes_nothing() {
    let repo = Repo::new("member-retag-refused");
    let first = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    let removed = repo.add_tagged(&repo.agent("b"), "coder", "kc").unwrap();
    block_on(members::remove(&repo.opened(), &removed.member)).unwrap();
    let second = repo.add_tagged(&repo.agent("c"), "coder", "kb").unwrap();
    let before = repo.list();

    let held = retag(&repo, &second.member, "ka").unwrap_err();
    let bad = retag(&repo, &second.member, "K1").unwrap_err();
    let gone = retag(&repo, &removed.member, "kd").unwrap_err();
    let no_id = retag(&repo, "pm", "kd").unwrap_err();

    assert!(matches!(held, MemberError::TagHeld { .. }), "{held:?}");
    assert!(matches!(bad, MemberError::NotATag { .. }), "{bad:?}");
    assert!(matches!(gone, MemberError::NotFound { .. }), "{gone:?}");
    assert!(
        matches!(no_id, MemberError::NotAMemberId { .. }),
        "{no_id:?}"
    );
    assert_eq!(repo.list(), before);
    assert_eq!(find(&before, &first.member).tag.as_deref(), Some("ka"));
}

#[test]
fn retag_resolves_a_shared_tag() {
    let repo = Repo::new("member-retag-shared");
    let first = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    let second = repo.add_tagged(&repo.agent("b"), "coder", "kb").unwrap();
    set_tag(&repo, &second.member, "ka");
    assert!(find(&repo.list(), &first.member).tag_shared);

    retag(&repo, &second.member, "kc").unwrap();

    let list = repo.list();
    assert!(list.iter().all(|m| !m.tag_shared), "{list:?}");
}
