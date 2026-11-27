//! Removing a member (RFC 0001, "Ending a session's member"): an admin removes it from every open
//! space it is a member of. Its `spaceMember` record stays, so that who wrote what stays readable
//! (RFC 0001, "Who decided"), and a member added again gets its record back with the new role.

use std::fs;
use std::path::PathBuf;

use super::{block_on, data_dir, keys, scratch};
use crate::data_dir::DataDir;
use crate::members::{self, Member, MemberError};
use crate::opened::{self, Opened};
use crate::space::init;
use crate::testing;

struct Repo {
    base: PathBuf,
    data: DataDir,
    space: String,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let base = scratch(test);
        let data = data_dir(&base.join("data"));
        let space = block_on(init(&base.join("repo"), &data, keys(&data)))
            .unwrap()
            .space;
        Repo { base, data, space }
    }

    fn open(&self) -> Opened {
        block_on(opened::repository(
            &self.base.join("repo"),
            &self.data,
            keys(&self.data),
        ))
        .unwrap()
    }

    fn agent(&self, name: &str) -> String {
        block_on(testing::paired_agent(
            &self.data,
            keys(&self.data),
            &self.base.join(name),
        ))
    }

    fn list(&self) -> Vec<Member> {
        block_on(members::list(&self.open())).unwrap()
    }

    fn add(&self, invitation: &str, role: &str) -> Member {
        block_on(members::add(&self.open(), invitation, role, None, None)).unwrap()
    }

    fn remove(&self, member: &str) -> Result<Member, MemberError> {
        block_on(members::remove(&self.open(), member))
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn a_removed_member_is_no_longer_listed() {
    let repo = Repo::new("member-remove");
    let added = repo.add(&repo.agent("coder"), "coder");

    let removed = repo.remove(&added.member).unwrap();

    assert_eq!(removed, added, "remove reports the member as it was");
    let list = repo.list();
    assert_eq!(list.len(), 1, "{list:?}");
    assert!(list.iter().all(|m| m.member != added.member));
}

#[test]
fn a_member_added_again_gets_its_new_role() {
    let repo = Repo::new("member-remove-readd");
    let invitation = repo.agent("agent");
    let first = repo.add(&invitation, "coder");
    repo.remove(&first.member).unwrap();

    let again = repo.add(&invitation, "pm");

    assert_eq!(again.member, first.member);
    let list = repo.list();
    let listed = list.iter().find(|m| m.member == first.member).unwrap();
    assert_eq!(listed.role.as_deref(), Some("pm"));
}

#[test]
fn a_member_that_is_not_one_is_not_found() {
    let repo = Repo::new("member-remove-unknown");
    let unknown = "00".repeat(32);

    let err = repo.remove(&unknown).unwrap_err();

    assert!(matches!(err, MemberError::NotFound { .. }), "{err:?}");
}

#[test]
fn a_text_that_is_no_member_id_is_refused() {
    let repo = Repo::new("member-remove-not-an-id");

    let err = repo.remove("coder on NEX-42").unwrap_err();

    assert!(matches!(err, MemberError::NotAMemberId { .. }), "{err:?}");
}

#[test]
fn the_device_cannot_remove_itself() {
    let repo = Repo::new("member-remove-self");
    let me = repo.list()[0].member.clone();

    let err = repo.remove(&me).unwrap_err();

    assert!(matches!(err, MemberError::Invalid { .. }), "{err:?}");
    assert_eq!(repo.list().len(), 1);
}

#[test]
fn a_member_that_is_not_an_admin_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-remove-not-admin");
    let added = repo.add(&repo.agent("coder"), "coder");
    let admin = repo.agent("second-admin");
    block_on(testing::demote_device(
        &repo.data,
        keys(&repo.data),
        repo.space.parse().unwrap(),
        &admin,
    ));
    let before = repo.list();

    let err = repo.remove(&added.member).unwrap_err();

    assert!(matches!(err, MemberError::NotAdmin { .. }), "{err:?}");
    assert_eq!(repo.list(), before);
}
