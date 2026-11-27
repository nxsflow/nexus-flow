//! A session's role names a role file (RFC 0006, "The factory's files"): `nxs member add` refuses
//! a session's role that has no file in `factory/roles/` of the working tree, and names the
//! missing file. The frame's own roles (`owner`, `developer`, `app`, `migration`) name no file.

use std::fs;
use std::path::PathBuf;

use super::{block_on, data_dir, keys, role_files, scratch};
use crate::data_dir::DataDir;
use crate::members::role_file::role_file;
use crate::members::{self, Member, MemberError};
use crate::opened;
use crate::space::init;
use crate::testing;

struct Repo {
    base: PathBuf,
    data: DataDir,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let base = scratch(test);
        let data = data_dir(&base.join("data"));
        block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
        Repo { base, data }
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

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

    /// Adds as a member opened on the repository space by its id, with no checkout.
    fn add_without_checkout(&self, invitation: &str, role: &str) -> Result<Member, MemberError> {
        let space = block_on(opened::repository(
            &self.repo(),
            &self.data,
            keys(&self.data),
        ))
        .unwrap()
        .spaces()[0]
            .0;
        let opened = block_on(opened::space(space, None, &self.data, keys(&self.data)));
        block_on(members::add(&opened.unwrap(), invitation, role, None))
    }

    fn add(&self, invitation: &str, role: &str) -> Result<Member, MemberError> {
        let opened = block_on(opened::repository(
            &self.repo(),
            &self.data,
            keys(&self.data),
        ));
        block_on(members::add(&opened.unwrap(), invitation, role, None))
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn a_session_role_without_its_file_is_refused_names_the_file_and_writes_nothing() {
    let repo = Repo::new("member-role-file-missing");
    let invitation = repo.agent("legal");
    let before = repo.list();

    let err = repo.add(&invitation, "legal").unwrap_err();

    let missing = repo.repo().join("factory/roles/legal.md");
    assert_eq!(
        err,
        MemberError::NoRoleFile {
            role: "legal".to_owned(),
            path: missing.clone()
        }
    );
    assert!(
        err.to_string().contains(&missing.display().to_string()),
        "{err}"
    );
    assert_eq!(repo.list(), before);
}

#[test]
fn a_session_role_with_its_file_is_added() {
    let repo = Repo::new("member-role-file-present");
    role_files(&repo.repo(), &["legal"]);
    let invitation = repo.agent("legal");

    let added = repo.add(&invitation, "legal").unwrap();

    assert_eq!(added.role.as_deref(), Some("legal"));
}

#[test]
fn the_frame_roles_need_no_file() {
    for role in ["developer", "app", "migration"] {
        let repo = Repo::new(&format!("member-role-file-frame-{role}"));
        let invitation = repo.agent(role);

        let added = repo.add(&invitation, role).unwrap();

        assert_eq!(added.role.as_deref(), Some(role));
    }
}

#[test]
fn a_file_named_after_a_frame_role_is_not_a_role() {
    let repo = Repo::new("member-role-file-frame-file");
    role_files(
        &repo.repo(),
        &["owner", "developer", "app", "migration", "pm"],
    );

    for frame in ["owner", "developer", "app", "migration"] {
        assert_eq!(role_file(Some(&repo.repo()), frame), Ok(None), "{frame}");
    }
    assert_eq!(
        role_file(Some(&repo.repo()), "pm"),
        Ok(Some(repo.repo().join("factory/roles/pm.md")))
    );
}

#[test]
fn a_role_that_is_no_role_name_is_refused_even_with_a_file_of_its_name() {
    let repo = Repo::new("member-role-file-name");
    role_files(&repo.repo(), &["Legal", "1st", "a_b"]);
    let long = format!("a{}", "b".repeat(63));
    let invitation = repo.agent("legal");
    let before = repo.list();

    for role in ["Legal", "1st", "a_b", "../roles/legal", long.as_str()] {
        let err = repo.add(&invitation, role).unwrap_err();

        assert_eq!(
            err,
            MemberError::NotARoleName {
                role: role.to_owned()
            },
            "{role}"
        );
        assert!(err.to_string().contains("[a-z][a-z0-9-]{0,62}"), "{err}");
    }
    assert_eq!(repo.list(), before);
    let longest = format!("a{}", "b".repeat(62));
    role_files(&repo.repo(), &[&longest]);
    assert!(role_file(Some(&repo.repo()), &longest).unwrap().is_some());
}

#[test]
fn a_session_role_is_refused_without_a_working_tree_to_read_its_file_from() {
    let repo = Repo::new("member-role-file-no-checkout");
    role_files(&repo.repo(), &["legal"]);
    let invitation = repo.agent("legal");
    let before = repo.list();

    let err = repo.add_without_checkout(&invitation, "legal").unwrap_err();

    assert_eq!(
        err,
        MemberError::NoWorkingTree {
            role: "legal".to_owned()
        }
    );
    assert_eq!(repo.list(), before);
}
