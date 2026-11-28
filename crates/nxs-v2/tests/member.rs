//! `nxs member add`, `remove` and `list` (RFC 0001, "Members", "The role record", "Public
//! surfaces this RFC brings"): adding writes the member and its `spaceMember` record together,
//! only an admin adds or removes and a refusal changes nothing, and every command prints what its
//! `Engine` counterpart returns.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, json, nxs, options, scratch, stdout};
use nexus_flow_seam::{testing, Engine, Member};
use serde_json::{json, Value};

/// A repository given its space by `nxs init`, and the factory's shipped roles `pm` and `coder` by
/// `nxs factory install`.
struct Repo {
    base: PathBuf,
    space: String,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let base = scratch(test);
        let out = nxs(&base, &base.join("repo"), &["init"]);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        let space = json(&out)["space"].as_str().unwrap().to_owned();
        let out = nxs(&base, &base.join("repo"), &["factory", "install"]);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        Repo { base, space }
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    fn nxs(&self, args: &[&str]) -> nxs_v2::Outcome {
        nxs(&self.base, &self.repo(), args)
    }

    /// The invitation of a new agent paired into the device's account, in `name/`.
    fn agent(&self, name: &str) -> String {
        let setup = options(&self.base.join("data"));
        block_on(testing::paired_agent(
            &setup.data,
            setup.keys,
            &self.base.join(name),
        ))
    }

    /// The members as the seam lists them.
    fn engine_members(&self) -> Vec<Member> {
        let setup = options(&self.base.join("data"));
        let engine = block_on(Engine::open_with(&self.repo(), setup)).unwrap();
        block_on(engine.members()).unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// A member as `nxs` prints it.
fn row(member: &Member) -> Value {
    json!({
        "member": member.member,
        "kind": member.kind.as_str(),
        "role": member.role,
        "ticket": member.ticket.map(|t| t.to_string()),
        "parent": member.parent,
        "tag": member.tag,
        "tag_shared": member.tag_shared,
        "account": member.account,
    })
}

#[test]
fn adding_prints_the_member_with_its_record_and_list_shows_both() {
    let repo = Repo::new("member-add");
    let invitation = repo.agent("pm");

    let out = repo.nxs(&["member", "add", &invitation, "--role", "pm"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let added = json(&out);
    assert_eq!(added["kind"], "agent");
    assert_eq!(added["role"], "pm");
    assert_eq!(added["ticket"], Value::Null);
    assert_eq!(added["parent"], Value::Null);
    let list = json(&repo.nxs(&["member", "list"]));
    let list = list.as_array().expect("member list prints an array");
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(list.contains(&added), "{list:?}");
    let device = list.iter().find(|m| m["kind"] == "device").unwrap();
    assert_eq!(device["role"], "owner");
    assert_eq!(device["account"], added["account"]);
}

#[test]
fn a_ticket_is_printed_as_written() {
    let repo = Repo::new("member-add-ticket");
    let invitation = repo.agent("coder");
    let ticket = format!("{}/00112233445566778899aabbccddeeff", repo.space);

    let out = repo.nxs(&[
        "member",
        "add",
        &invitation,
        "--role",
        "coder",
        "--ticket",
        &ticket,
    ]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["ticket"], ticket.as_str());
}

#[test]
fn every_command_prints_what_its_engine_counterpart_returns() {
    let repo = Repo::new("member-parity");
    let invitation = repo.agent("pm");

    let added = json(&repo.nxs(&["member", "add", &invitation, "--role", "pm"]));
    let listed = json(&repo.nxs(&["member", "list"]));
    let engine = repo.engine_members();
    let member = added["member"].as_str().unwrap().to_owned();
    let mine = engine.iter().find(|m| m.member == member).unwrap();

    assert_eq!(added, row(mine));
    assert_eq!(listed, Value::Array(engine.iter().map(row).collect()));
    let removed = json(&repo.nxs(&["member", "remove", &member]));
    assert_eq!(removed, row(mine));
    assert_eq!(
        json(&repo.nxs(&["member", "list"])),
        Value::Array(repo.engine_members().iter().map(row).collect())
    );
}

#[test]
fn a_removed_member_is_no_longer_listed() {
    let repo = Repo::new("member-remove");
    let invitation = repo.agent("coder");
    let added = json(&repo.nxs(&["member", "add", &invitation, "--role", "coder"]));

    let out = repo.nxs(&["member", "remove", added["member"].as_str().unwrap()]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out), added);
    let list = json(&repo.nxs(&["member", "list"]));
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
}

/// The error object's kind.
fn kind(out: &nxs_v2::Outcome) -> String {
    assert_eq!(out.code, 1, "{}", stdout(out));
    json(out)["error"]["kind"].as_str().unwrap().to_owned()
}

#[test]
fn a_member_that_is_not_an_admin_is_refused_and_nothing_changes() {
    let repo = Repo::new("member-not-admin");
    let setup = options(&repo.base.join("data"));
    let admin = repo.agent("second-admin");
    block_on(testing::demote_device(
        &setup.data,
        setup.keys,
        repo.space.parse().unwrap(),
        &admin,
    ));
    let before = repo.nxs(&["member", "list"]).stdout;
    let invitation = repo.agent("pm");

    let out = repo.nxs(&["member", "add", &invitation, "--role", "pm"]);

    assert_eq!(kind(&out), "forbidden");
    assert_eq!(repo.nxs(&["member", "list"]).stdout, before);
    let member = json(&repo.nxs(&["member", "list"]))[1]["member"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(kind(&repo.nxs(&["member", "remove", &member])), "forbidden");
    assert_eq!(repo.nxs(&["member", "list"]).stdout, before);
}

#[test]
fn each_refusal_has_its_error_kind() {
    let repo = Repo::new("member-refusals");
    let invitation = repo.agent("pm");
    repo.nxs(&["member", "add", &invitation, "--role", "pm"]);
    let unknown = "00".repeat(32);

    let cases = [
        (
            vec!["member", "add", &invitation, "--role", "pm"],
            "conflict",
        ),
        (vec!["member", "add", "nope", "--role", "pm"], "validation"),
        (
            vec!["member", "add", &invitation, "--role", ""],
            "validation",
        ),
        (
            vec![
                "member",
                "add",
                &invitation,
                "--role",
                "pm",
                "--ticket",
                "NEX-1",
            ],
            "validation",
        ),
        (vec!["member", "add", &invitation], "validation"),
        (vec!["member", "remove", &unknown], "not_found"),
        (vec!["member", "remove", "pm"], "validation"),
    ];
    for (args, expected) in cases {
        assert_eq!(kind(&repo.nxs(&args)), expected, "{args:?}");
    }
}

#[test]
fn outside_a_repository_it_says_to_run_nxs_init() {
    let base = scratch("member-no-workspace");

    let out = nxs(&base, &base.join("repo"), &["member", "list"]);

    assert_eq!(kind(&out), "no_workspace");
    fs::remove_dir_all(&base).unwrap();
}

/// The golden file `name`, with the device's member id, tag and account filled in: all three are
/// drawn anew in every test, the tag picked by the member id.
fn golden(name: &str, device: &Member) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("{member}", &device.member)
        .replace(
            "{tag}",
            device.tag.as_deref().expect("the device has a tag"),
        )
        .replace("{account}", &device.account)
}

#[test]
fn the_same_local_copy_prints_the_same_bytes_as_the_golden_files() {
    let repo = Repo::new("member-golden");
    let device = repo.engine_members().remove(0);

    let json_first = repo.nxs(&["member", "list"]);
    let json_second = repo.nxs(&["member", "list"]);
    let md_first = repo.nxs(&["member", "list", "--md"]);
    let md_second = repo.nxs(&["--md", "member", "list"]);

    assert_eq!(stdout(&json_first), golden("member-list.json", &device));
    assert_eq!(json_second.stdout, json_first.stdout);
    assert_eq!(stdout(&md_first), golden("member-list.md", &device));
    assert_eq!(md_second.stdout, md_first.stdout);
}

#[test]
fn several_members_print_the_same_bytes_every_time() {
    let repo = Repo::new("member-same-bytes");
    for name in ["a", "b"] {
        let invitation = repo.agent(name);
        repo.nxs(&["member", "add", &invitation, "--role", "coder"]);
    }

    for args in [&["member", "list"][..], &["member", "list", "--md"][..]] {
        let first = repo.nxs(args);
        let second = repo.nxs(args);
        assert_eq!(first.code, 0, "{}", stdout(&first));
        assert_eq!(second.stdout, first.stdout, "{args:?}");
    }
}

#[test]
fn a_session_role_without_its_role_file_is_a_validation_error_naming_the_file() {
    let repo = Repo::new("member-no-role-file");
    let invitation = repo.agent("legal");
    let before = repo.nxs(&["member", "list"]).stdout;

    let out = repo.nxs(&["member", "add", &invitation, "--role", "legal"]);

    assert_eq!(kind(&out), "validation");
    let message = json(&out)["error"]["msg"].as_str().unwrap().to_owned();
    let missing = repo.repo().join("factory/roles/legal.md");
    assert!(
        message.contains(&missing.display().to_string()),
        "{message}"
    );
    assert_eq!(repo.nxs(&["member", "list"]).stdout, before);
}
