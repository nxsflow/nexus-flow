//! Black-box: an op nobody on this machine can vouch for is SHOWN and carries NO action, through
//! the real `nxc` binary (nxf 6j6v.pzkb, spec `docs/specs/E4-auth-identity-and-signed-ops.md` §2.4).
//!
//! The foreign ops come from a second replica — an in-memory store with a key of its own — and
//! land in the workspace the way a sync pull lands them: `apply`, which verifies each one and
//! records the verdict. Trust is then granted through the store, the same substrate call
//! `nxs sync trust add` makes.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-07-22T00:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW);
    c
}

fn json(cmd: &mut Command) -> Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_str(String::from_utf8_lossy(&out).trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// A board alice opened, asking bob — on a channel no declaration names, the shape `ask` leaves.
fn board(tmp: &TempDir) -> String {
    let mut store = open_store(tmp);
    store.set_channel_field("c-raw", "kind", "group", "local/alice");
    store.add_member("c-raw", "local/alice", "local/alice");
    nexus_chat::facade::ask(
        &mut store,
        nexus_chat::facade::AskRequest {
            now: NOW,
            origin: "local",
            actor: "alice",
            channel: "c-raw",
            body: "please review",
            expect: &["local/bob".to_string()],
            deadline: None,
            kind: nexus_chat::model::MessageKind::Question,
            priority: nexus_chat::model::Priority::Normal,
            refs: nexus_chat::model::Refs::default(),
        },
    )
    .expect("the board opens")
    .thread_id
}

/// A second replica that has seen everything the workspace holds, so what it writes next is
/// causally after it.
fn peer_of(tmp: &TempDir) -> ChatStore {
    let mut peer = ChatStore::open_in_memory(2);
    peer.apply(&open_store(tmp).export());
    peer
}

fn deliver(tmp: &TempDir, peer: &ChatStore) {
    let written: Vec<_> = peer.export().into_iter().filter(|o| o.site == 2).collect();
    open_store(tmp).apply(&written);
}

#[test]
fn a_reply_claiming_the_expected_handle_from_an_untrusted_key_is_shown_and_completes_nothing() {
    let tmp = workspace();
    let tid = board(&tmp);
    let mut peer = peer_of(&tmp);
    nexus_chat::facade::reply(
        &mut peer,
        nexus_chat::facade::ReplyRequest {
            now: NOW,
            origin: "local",
            actor: "bob",
            target: &tid,
            body: "lgtm",
            kind: nexus_chat::model::MessageKind::Report,
            priority: nexus_chat::model::Priority::Normal,
            disposition: nexus_chat::model::Disposition::InTurn,
            refs: nexus_chat::model::Refs::default(),
            if_unanswered: false,
        },
    )
    .expect("the peer replies as bob");
    deliver(&tmp, &peer);

    let shown = json(nxc(&tmp).args(["--json", "threads", "show", &tid]));
    let bodies: Vec<&str> = shown["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(bodies.contains(&"lgtm"), "the reply is shown: {shown}");
    assert_eq!(
        shown["messages"][1]["unvouched"], true,
        "marked as such: {shown}"
    );
    assert!(
        shown["messages"][0].get("unvouched").is_none(),
        "while alice's own request reads as before: {shown}"
    );
    assert_eq!(shown["complete"], false, "and completes nothing: {shown}");

    open_store(&tmp)
        .trust_key(peer.key_id(), "bob's laptop", NOW)
        .unwrap();
    let shown = json(nxc(&tmp).args(["--json", "threads", "show", &tid]));
    assert_eq!(shown["complete"], true, "trusted, it does: {shown}");
    assert!(shown["messages"][1].get("unvouched").is_none(), "{shown}");
}

#[test]
fn a_tick_on_a_thread_an_untrusted_op_redeclared_says_held() {
    let tmp = workspace();
    let tid = board(&tmp);
    let mut peer = peer_of(&tmp);
    peer.set_expects_reply_from(&tid, "[\"local/mallory\"]", "local/alice");
    deliver(&tmp, &peer);

    let shown = json(nxc(&tmp).args(["--json", "threads", "show", &tid]));
    assert_eq!(shown["held"], true, "{shown}");
    let tick = json(nxc(&tmp).args(["--json", "tick", "--thread", &tid]));
    assert_eq!(tick["acted"], false, "{tick}");
    assert_eq!(tick["reason"], "held", "{tick}");
}
