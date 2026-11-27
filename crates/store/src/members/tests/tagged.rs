//! A member's ticket tag (RFC 0002, "Ticket ids"): `nxs member add` writes one into the new
//! member's `spaceMember` record, a free one unless `--tag` names it, and refuses a tag a current
//! member holds; with no tag left it fails with `no_free_tag`. Two current members holding one
//! tag are both marked `tag_shared`.
//!
//! Until the owner decides when a freed tag may be handed out again (see the PR of NEX-108), a
//! tag that a removed member held is neither picked nor taken by `--tag`; a member added again
//! gets its own tag back.

use nocentraldb::{MemberId, Value};

use super::add::find;
use super::{block_on, Repo};
use crate::members::record;
use crate::members::tags::{self, Tag};
use crate::members::{Member, MemberError, MemberKind};

/// The device's own member: the one that created the space.
fn device(repo: &Repo) -> Member {
    let list = repo.list();
    list.into_iter()
        .find(|m| m.kind == MemberKind::Device)
        .expect("the device that created the space is a member")
}

/// Writes one role record for `member` per tag in `tags`, in the repository space, as two
/// inviters adding offline at once, or a member's earlier records, leave them.
fn write_records(repo: &Repo, member: &str, tags: impl IntoIterator<Item = Tag>) {
    let opened = repo.opened();
    let (id, _) = &opened.spaces()[0];
    let tags: Vec<Tag> = tags.into_iter().collect();
    block_on(opened.write(id, |w| {
        for tag in &tags {
            w.create(
                record::COLLECTION,
                [
                    ("member", Value::from(member)),
                    ("role", Value::from("coder")),
                    ("tag", Value::from(tag.as_str())),
                ],
            )?;
        }
        Ok(())
    }))
    .unwrap();
}

/// Rewrites `member`'s record to hold `tag`, as a second inviter's offline add would.
fn set_tag(repo: &Repo, member: &str, tag: &str) {
    let opened = repo.opened();
    let (id, space) = &opened.spaces()[0];
    let space = space.as_ref().unwrap();
    let member: MemberId = member.parse().unwrap();
    let tag: Tag = tag.parse().unwrap();
    block_on(record::write(
        &opened,
        id,
        space,
        member,
        "coder",
        None,
        Some(&tag),
    ))
    .unwrap();
}

fn is_tag(text: Option<&str>) -> bool {
    text.is_some_and(|t| t.parse::<Tag>().is_ok())
}

#[test]
fn an_added_member_gets_a_free_tag_in_its_record() {
    let repo = Repo::new("member-tag-free");

    let first = repo.add(&repo.agent("a"), "coder", None).unwrap();
    let second = repo.add(&repo.agent("b"), "pm", None).unwrap();

    assert!(is_tag(first.tag.as_deref()), "{first:?}");
    assert!(is_tag(second.tag.as_deref()), "{second:?}");
    assert_ne!(first.tag, second.tag);
    let list = repo.list();
    assert_eq!(find(&list, &first.member).tag, first.tag);
    assert_eq!(find(&list, &second.member).tag, second.tag);
    assert!(list.iter().all(|m| !m.tag_shared), "{list:?}");
}

#[test]
fn a_named_tag_is_written() {
    let repo = Repo::new("member-tag-named");

    let added = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();

    assert_eq!(added.tag.as_deref(), Some("ka"));
    assert_eq!(find(&repo.list(), &added.member).tag.as_deref(), Some("ka"));
}

#[test]
fn a_text_that_is_no_tag_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-tag-bad");
    let invitation = repo.agent("a");
    let before = repo.list();

    for bad in ["K1", "1k", "kab", "k", ""] {
        let err = repo.add_tagged(&invitation, "coder", bad).unwrap_err();
        assert!(
            matches!(err, MemberError::NotATag { .. }),
            "{bad:?}: {err:?}"
        );
        assert!(err.to_string().contains("^[a-z][a-z0-9]$"), "{err}");
    }
    assert_eq!(repo.list(), before);
}

#[test]
fn a_tag_a_current_member_holds_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-tag-held");
    repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    let before = repo.list();

    let err = repo
        .add_tagged(&repo.agent("b"), "coder", "ka")
        .unwrap_err();

    assert!(matches!(err, MemberError::TagHeld { .. }), "{err:?}");
    assert!(err.to_string().contains("ka"), "{err}");
    assert_eq!(repo.list(), before);
}

#[test]
fn with_every_tag_held_adding_fails_with_no_free_tag_and_nothing_changes() {
    let repo = Repo::new("member-tag-none-free");
    let device = device(&repo);
    write_records(&repo, &device.member, tags::all());
    let before = repo.list();

    let err = repo.add(&repo.agent("a"), "coder", None).unwrap_err();

    assert!(matches!(err, MemberError::NoFreeTag), "{err:?}");
    assert_eq!(repo.list(), before);
}

#[test]
fn a_free_tag_is_picked_even_when_only_one_is_left() {
    let repo = Repo::new("member-tag-last-free");
    let device = device(&repo);
    write_records(
        &repo,
        &device.member,
        tags::all().filter(|t| t.as_str() != "q3"),
    );

    let added = repo.add(&repo.agent("a"), "coder", None).unwrap();

    assert_eq!(added.tag.as_deref(), Some("q3"));
}

#[test]
fn two_members_holding_one_tag_are_both_marked_shared() {
    let repo = Repo::new("member-tag-shared");
    let first = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    let second = repo.add_tagged(&repo.agent("b"), "coder", "kb").unwrap();

    set_tag(&repo, &second.member, "ka");

    let list = repo.list();
    let (first, second) = (find(&list, &first.member), find(&list, &second.member));
    assert_eq!(first.tag.as_deref(), Some("ka"));
    assert_eq!(second.tag.as_deref(), Some("ka"));
    assert!(first.tag_shared && second.tag_shared, "{list:?}");
    assert!(!device(&repo).tag_shared);
}

#[test]
fn a_tag_a_removed_member_held_is_not_handed_out_again() {
    let repo = Repo::new("member-tag-freed");
    let removed = repo.add_tagged(&repo.agent("a"), "coder", "ka").unwrap();
    block_on(crate::members::remove(&repo.opened(), &removed.member)).unwrap();
    let device = device(&repo);
    write_records(
        &repo,
        &device.member,
        tags::all().filter(|t| t.as_str() != "ka"),
    );

    let err = repo
        .add_tagged(&repo.agent("b"), "coder", "ka")
        .unwrap_err();
    assert!(matches!(err, MemberError::TagFreed { .. }), "{err:?}");
    let err = repo.add(&repo.agent("c"), "coder", None).unwrap_err();
    assert!(matches!(err, MemberError::NoFreeTag), "{err:?}");
}

#[test]
fn a_member_added_again_gets_its_own_tag_back() {
    let repo = Repo::new("member-tag-again");
    let invitation = repo.agent("a");
    let first = repo.add_tagged(&invitation, "coder", "ka").unwrap();
    block_on(crate::members::remove(&repo.opened(), &first.member)).unwrap();

    let again = repo.add(&invitation, "coder", None).unwrap();

    assert_eq!(again.tag.as_deref(), Some("ka"));
}
