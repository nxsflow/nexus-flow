//! A member's ticket tag at the surface (RFC 0002, "Ticket ids", "Public surfaces this RFC
//! brings"): `nxs member add --tag <tag>`, `nxs member retag <member> <tag>`, `tag` and
//! `tag_shared` in every member `nxs member` prints, and the error kinds of a refused tag:
//! `validation` for a text that is no tag, `conflict` for a tag a current member holds, and
//! `no_free_tag` when every tag is held.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{block_on, json, nxs, options, scratch, stdout};
use nexus_flow_seam::{testing, Engine};
use serde_json::Value;

struct Repo {
    base: PathBuf,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let base = scratch(test);
        let out = nxs(&base, &base.join("repo"), &["init"]);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        let out = nxs(&base, &base.join("repo"), &["factory", "install"]);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        let repo = Repo { base };
        repo.retag_device();
        repo
    }

    /// Gives the device [`DEVICE_TAG`], in place of the tag `nxs init` picked for it.
    fn retag_device(&self) {
        let list = self.ok(&["member", "list"]);
        let device = list
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "device")
            .expect("the device that created the space is a member");
        let member = device["member"].as_str().unwrap();
        self.ok(&["member", "retag", member, DEVICE_TAG]);
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    fn nxs(&self, args: &[&str]) -> nxs_v2::Outcome {
        nxs(&self.base, &self.repo(), args)
    }

    fn agent(&self, name: &str) -> String {
        let setup = options(&self.base.join("data"));
        block_on(testing::paired_agent(
            &setup.data,
            setup.keys,
            &self.base.join(name),
        ))
    }

    fn ok(&self, args: &[&str]) -> Value {
        let out = self.nxs(args);
        assert_eq!(out.code, 0, "{args:?}: {}", stdout(&out));
        json(&out)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The tag the device holds in every [`Repo`]: `nxs init` picks the device's tag by its random
/// id, so the tests give it one that no test names (NEX-248).
const DEVICE_TAG: &str = "zy";

#[test]
fn the_device_of_a_test_repo_holds_the_tag_no_test_names() {
    let repo = Repo::new("member-tag-device");

    let list = repo.ok(&["member", "list"]);

    let device = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "device");
    assert_eq!(
        device.map(|m| &m["tag"]),
        Some(&Value::from(DEVICE_TAG)),
        "{list}"
    );
}

fn kind(out: &nxs_v2::Outcome) -> String {
    assert_eq!(out.code, 1, "{}", stdout(out));
    json(out)["error"]["kind"].as_str().unwrap().to_owned()
}

#[test]
fn add_prints_a_free_tag_and_list_prints_tag_and_tag_shared() {
    let repo = Repo::new("member-tag-add");

    let added = repo.ok(&["member", "add", &repo.agent("a"), "--role", "coder"]);

    let tag = added["tag"].as_str().expect("a tag");
    assert_eq!(tag.len(), 2, "{added}");
    assert_eq!(added["tag_shared"], false);
    let list = repo.ok(&["member", "list"]);
    let list = list.as_array().unwrap();
    assert!(list.contains(&added), "{list:?}");
    let device = list.iter().find(|m| m["kind"] == "device").unwrap();
    assert_ne!(device["tag"], added["tag"], "{list:?}");
    assert_eq!(device["tag_shared"], false);
}

#[test]
fn add_with_tag_writes_it_and_retag_writes_the_new_one() {
    let repo = Repo::new("member-tag-retag");

    let added = repo.ok(&[
        "member",
        "add",
        &repo.agent("a"),
        "--role",
        "coder",
        "--tag",
        "ka",
    ]);
    assert_eq!(added["tag"], "ka");
    let member = added["member"].as_str().unwrap();

    let retagged = repo.ok(&["member", "retag", member, "kb"]);

    assert_eq!(retagged["tag"], "kb");
    assert_eq!(retagged["member"], added["member"]);
    assert_eq!(retagged["role"], "coder");
    let list = repo.ok(&["member", "list"]);
    assert!(list.as_array().unwrap().contains(&retagged), "{list}");
}

#[test]
fn each_refused_tag_has_its_error_kind() {
    let repo = Repo::new("member-tag-refused");
    let held = repo.ok(&[
        "member",
        "add",
        &repo.agent("a"),
        "--role",
        "coder",
        "--tag",
        "ka",
    ]);
    let other = repo.ok(&["member", "add", &repo.agent("b"), "--role", "coder"]);
    let other = other["member"].as_str().unwrap();
    let invitation = repo.agent("c");
    let before = repo.nxs(&["member", "list"]).stdout;

    let cases = [
        (
            vec![
                "member",
                "add",
                &invitation,
                "--role",
                "coder",
                "--tag",
                "K1",
            ],
            "validation",
        ),
        (
            vec![
                "member",
                "add",
                &invitation,
                "--role",
                "coder",
                "--tag",
                "ka",
            ],
            "conflict",
        ),
        (vec!["member", "retag", other, "ka"], "conflict"),
        (vec!["member", "retag", other, "1k"], "validation"),
        (vec!["member", "retag", other], "validation"),
    ];
    for (args, expected) in cases {
        assert_eq!(kind(&repo.nxs(&args)), expected, "{args:?}");
    }
    assert_eq!(repo.nxs(&["member", "list"]).stdout, before);
    assert_eq!(held["tag"], "ka");
}

#[test]
fn with_every_tag_held_add_fails_with_no_free_tag() {
    let repo = Repo::new("member-tag-none-free");
    let setup = options(&repo.base.join("data"));
    block_on(testing::hold_every_tag(
        &repo.repo(),
        &setup.data,
        setup.keys,
    ));
    let before = repo.nxs(&["member", "list"]).stdout;

    let out = repo.nxs(&["member", "add", &repo.agent("a"), "--role", "coder"]);

    assert_eq!(kind(&out), "no_free_tag");
    assert!(
        json(&out)["error"]["msg"].as_str().unwrap().contains("936"),
        "{}",
        stdout(&out)
    );
    assert_eq!(repo.nxs(&["member", "list"]).stdout, before);
}

#[test]
fn retag_prints_what_its_engine_counterpart_returns() {
    let repo = Repo::new("member-tag-parity");
    let added = repo.ok(&["member", "add", &repo.agent("a"), "--role", "coder"]);
    let member = added["member"].as_str().unwrap();
    let setup = options(&repo.base.join("data"));
    let engine = block_on(Engine::open_with(&repo.repo(), setup)).unwrap();

    let by_engine = block_on(engine.member_retag(member, "kb")).unwrap();
    let printed = repo.ok(&["member", "retag", member, "kb"]);

    assert_eq!(printed["tag"], by_engine.tag.as_deref().unwrap());
    assert_eq!(printed["tag_shared"], by_engine.tag_shared);
}
