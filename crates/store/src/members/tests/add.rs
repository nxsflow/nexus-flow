//! Adding a member (RFC 0001, "Members", "The role record", "Roles in a space"): the member that
//! adds another writes its `spaceMember` record in the same step, and only an admin adds. A
//! refusal changes nothing: the members and their records are as they were.

use std::fs;
use std::path::PathBuf;

use super::{block_on, data_dir, keys, role_files, scratch};
use crate::config::Config;
use crate::data_dir::DataDir;
use crate::members::{self, Member, MemberError, MemberKind};
use crate::opened;
use crate::space::init;
use crate::testing;

/// A repository with its space, the device's data directory and the space's id.
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
        role_files(&base.join("repo"), &["pm", "coder"]);
        Repo { base, data, space }
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    /// The invitation of a new agent paired into the device's account, in `name/`.
    fn agent(&self, name: &str) -> String {
        block_on(testing::paired_agent(
            &self.data,
            keys(&self.data),
            &self.base.join(name),
        ))
    }

    fn list(&self) -> Vec<Member> {
        let opened = block_on(opened::repository(
            &self.repo(),
            &self.data,
            keys(&self.data),
        ));
        block_on(members::list(&opened.unwrap())).unwrap()
    }

    fn add(
        &self,
        invitation: &str,
        role: &str,
        ticket: Option<&str>,
    ) -> Result<Member, MemberError> {
        let opened = block_on(opened::repository(
            &self.repo(),
            &self.data,
            keys(&self.data),
        ));
        block_on(members::add(&opened.unwrap(), invitation, role, ticket))
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn find<'a>(list: &'a [Member], member: &str) -> &'a Member {
    list.iter()
        .find(|m| m.member == member)
        .unwrap_or_else(|| panic!("{member} is not listed: {list:?}"))
}

#[test]
fn adding_a_paired_agent_writes_the_member_and_its_record() {
    let repo = Repo::new("member-add");
    let invitation = repo.agent("pm");

    let added = repo.add(&invitation, "pm", None).unwrap();

    assert_eq!(added.kind, MemberKind::Agent);
    assert_eq!(added.role.as_deref(), Some("pm"));
    assert_eq!(added.ticket, None);
    assert_eq!(added.parent, None);
    let list = repo.list();
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(find(&list, &added.member), &added);
    let device = list.iter().find(|m| m.kind == MemberKind::Device).unwrap();
    assert_eq!(
        device.account, added.account,
        "pm is paired into the device's account"
    );
}

#[test]
fn a_ticket_is_written_into_the_record_as_a_reference() {
    let repo = Repo::new("member-add-ticket");
    let invitation = repo.agent("coder");
    let ticket = format!("{}/00112233445566778899aabbccddeeff", repo.space);

    let added = repo.add(&invitation, "coder", Some(&ticket)).unwrap();

    assert_eq!(
        added.ticket.as_ref().map(|t| t.to_string()),
        Some(ticket.clone())
    );
    let listed = find(&repo.list(), &added.member).clone();
    assert_eq!(listed.ticket.map(|t| t.to_string()), Some(ticket));
    assert_eq!(listed.role.as_deref(), Some("coder"));
}

#[test]
fn a_ticket_that_is_no_reference_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-add-bad-ticket");
    let invitation = repo.agent("coder");
    let before = repo.list();

    let err = repo.add(&invitation, "coder", Some("NEX-42")).unwrap_err();

    assert!(matches!(err, MemberError::NotATicket { .. }), "{err:?}");
    assert!(err.to_string().contains("<space id>/<object id>"), "{err}");
    assert_eq!(repo.list(), before);
}

#[test]
fn a_member_added_twice_is_refused_and_keeps_its_record() {
    let repo = Repo::new("member-add-twice");
    let invitation = repo.agent("pm");
    repo.add(&invitation, "pm", None).unwrap();
    let before = repo.list();

    let err = repo.add(&invitation, "developer", None).unwrap_err();

    assert!(matches!(err, MemberError::AlreadyMember { .. }), "{err:?}");
    assert_eq!(repo.list(), before);
}

#[test]
fn a_text_that_is_no_invitation_is_refused() {
    let repo = Repo::new("member-add-not-invitation");

    let err = repo.add("ncdb-invite-1:nope", "pm", None).unwrap_err();

    assert!(
        matches!(err, MemberError::NotAnInvitation { .. }),
        "{err:?}"
    );
    assert_eq!(repo.list().len(), 1);
}

#[test]
fn an_empty_role_is_refused() {
    let repo = Repo::new("member-add-empty-role");
    let invitation = repo.agent("pm");

    let err = repo.add(&invitation, " ", None).unwrap_err();

    assert!(matches!(err, MemberError::EmptyRole), "{err:?}");
    assert_eq!(repo.list().len(), 1);
}

#[test]
fn a_device_cannot_add_a_sub_agent_and_nothing_changes() {
    let repo = Repo::new("member-add-sub-agent");
    let invitation = block_on(testing::sub_agent(&repo.base.join("coder")));

    let err = repo.add(&invitation, "coder", None).unwrap_err();

    assert!(matches!(err, MemberError::NotAllowed { .. }), "{err:?}");
    assert_eq!(repo.list().len(), 1);
}

#[test]
fn a_device_of_an_account_this_device_does_not_know_is_refused() {
    let repo = Repo::new("member-add-unknown-account");
    let invitation = block_on(testing::other_device(&repo.base.join("other")));

    let err = repo.add(&invitation, "developer", None).unwrap_err();

    assert!(matches!(err, MemberError::UnknownAccount { .. }), "{err:?}");
    assert_eq!(repo.list().len(), 1);
}

#[test]
fn a_member_that_is_not_an_admin_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-add-not-admin");
    let admin = repo.agent("second-admin");
    block_on(testing::demote_device(
        &repo.data,
        keys(&repo.data),
        repo.space.parse().unwrap(),
        &admin,
    ));
    let before = repo.list();
    let invitation = repo.agent("pm");

    let err = repo.add(&invitation, "pm", None).unwrap_err();

    assert!(matches!(err, MemberError::NotAdmin { .. }), "{err:?}");
    assert_eq!(repo.list(), before);
}

/// The repository's `nexus-flow.toml`, rewritten to name `coordination` too.
fn name_coordination(repo: &Repo, coordination: &str) {
    fs::remove_file(repo.repo().join("nexus-flow.toml")).unwrap();
    let mut config = Config::new(&repo.space);
    config.coordination = Some(coordination.to_owned());
    config.write_new(&repo.repo()).unwrap();
}

fn coordination_members(repo: &Repo, coordination: &str) -> Vec<Member> {
    let root = repo.repo();
    let opened = opened::space(
        coordination.parse().unwrap(),
        Some(&root),
        &repo.data,
        keys(&repo.data),
    );
    block_on(members::list(&block_on(opened).unwrap())).unwrap()
}

#[test]
fn pm_is_added_to_the_coordination_space_too_with_its_record() {
    let repo = Repo::new("member-add-coordination");
    let coordination = block_on(testing::create_space(&repo.data, keys(&repo.data))).to_string();
    name_coordination(&repo, &coordination);
    let invitation = repo.agent("pm");

    let added = repo.add(&invitation, "pm", None).unwrap();

    let there = coordination_members(&repo, &coordination);
    assert_eq!(find(&there, &added.member).role.as_deref(), Some("pm"));
}

#[test]
fn a_role_session_is_not_added_to_the_coordination_space() {
    let repo = Repo::new("member-add-coordination-session");
    let coordination = block_on(testing::create_space(&repo.data, keys(&repo.data))).to_string();
    name_coordination(&repo, &coordination);
    let invitation = repo.agent("coder");

    let added = repo.add(&invitation, "coder", None).unwrap();

    let there = coordination_members(&repo, &coordination);
    assert!(there.iter().all(|m| m.member != added.member), "{there:?}");
    assert!(repo.list().iter().any(|m| m.member == added.member));
}
