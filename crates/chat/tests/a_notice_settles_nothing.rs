//! **The notice settles nothing** (nxf 6j6v.71gy, design 8nx4 section 6.2 and test 12).
//!
//! Under `notice_at_first_stop = true` the coordinator writes a `notice` into the operation's root
//! thread at an episode's first stop. It is read there and in `nxc status`, and it must not be read
//! as anything more: it discharges no turn, the collected answers never carry it, and the member's
//! own reply is what settles the round — exactly as for a progress note (nxf 6j6v.558h).
//!
//! The notice is seeded through the write the ladder makes (`facade::send` as the coordinator, kind
//! `Notice`); the ladder's own path to it is pinned in `the_ladder.rs`.

mod common;

use assert_cmd::Command;
use nexus_chat::channel::SUPERVISOR_HANDLE;
use nexus_chat::definitions::Definitions;
use nexus_chat::facade::{self, SendRequest};
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs, KIND_NOTICE};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-10-24T10:00:00Z";
const LATER: &str = "2026-10-24T10:15:00Z";
const ORIGIN: &str = "local";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let coder: RoleDecl =
        serde_yaml::from_str("handle: coder\nsystem_prompt: You are coder.\n").expect("role");
    let defs = Definitions::new(vec![coder], Vec::new()).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    tmp
}

/// The human at the keyboard, at `now`.
fn human(tmp: &TempDir, now: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", ORIGIN)
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

/// The coder's spawned session, at `now`.
fn coder(tmp: &TempDir, now: &str) -> Command {
    let mut c = human(tmp, now);
    c.env("NXC_ACTOR", "coder").env("NXC_SESSION", "s-coder");
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The coordinator's notice into `thread`, written as the ladder writes it.
fn post_a_notice(tmp: &TempDir, thread: &str) -> String {
    let mut store = open_store(tmp);
    let channel = store
        .thread_channel(thread)
        .or_else(|| store.thread_channel_via_message(thread))
        .expect("the thread's channel");
    facade::send(
        &mut store,
        SendRequest {
            now: LATER,
            origin: ORIGIN,
            actor: SUPERVISOR_HANDLE,
            channel: &channel,
            body: "NOTICE from the coordinator of thread t: step coder showed no sign of life for \
                   15 minutes after it was asked and was stopped at 2026-10-24T10:15:00Z. \
                   Nothing is asked of you now.",
            kind: MessageKind::Notice,
            priority: Priority::Normal,
            disposition: Disposition::NextSession,
            thread: Some(thread),
            refs: Refs::default(),
        },
    )
    .expect("the notice is posted")
    .message_id
}

/// The ids of the messages the round's collected answers are read from.
fn collected_ids(store: &ChatStore, thread: &str) -> Vec<String> {
    store
        .messages_since_the_declaration(thread)
        .unwrap()
        .into_iter()
        .map(|m| m.message_id)
        .collect()
}

/// The `nxc status --json` row of `thread`.
fn status_row(tmp: &TempDir, now: &str, thread: &str) -> Value {
    let status = json_of(human(tmp, now).args(["--json", "status", "--all"]));
    status["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|o| o["threads"].as_array().cloned().unwrap_or_default())
        .find(|t| t["thread_id"] == thread)
        .unwrap_or_else(|| panic!("no status row for {thread}: {status}"))
}

#[test]
fn a_notice_discharges_nothing_and_the_members_reply_does() {
    let tmp = workspace();
    let thread = json_of(human(&tmp, NOW).arg("--json").args([
        "send",
        "--to",
        "coder",
        "Add a --since flag.",
    ]))["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();
    let notice = post_a_notice(&tmp, &thread);

    // The notice alone: stored, as a notice, and no answer.
    let store = open_store(&tmp);
    let kinds: Vec<(String, String)> = store
        .messages_in_thread(&thread)
        .unwrap()
        .into_iter()
        .map(|m| (m.message_id, m.kind))
        .collect();
    assert!(
        kinds.contains(&(notice.clone(), KIND_NOTICE.to_string())),
        "{kinds:?}"
    );
    let q = store.thread_quorum(&thread, LATER).unwrap().unwrap();
    assert!(q.replied.is_empty(), "the notice discharged nothing: {q:?}");
    assert_eq!(q.outstanding, vec![format!("{ORIGIN}/coder")], "{q:?}");
    assert!(!q.complete, "{q:?}");
    assert_eq!(store.latest_reply_message_id(&thread).unwrap(), None);
    // The member-thread read the collected answers come from (`orchestration::collected_replies`).
    // On a direct thread it also holds the commissioning message, which sorts after the declaration.
    let collected = collected_ids(&store, &thread);
    assert!(
        !collected.contains(&notice),
        "the collected answers do not carry the notice: {collected:?}"
    );
    drop(store);
    let row = status_row(&tmp, LATER, &thread);
    assert_eq!(
        row["state"], "open",
        "nxc status lists it as not-an-answer: {row}"
    );
    assert_eq!(row["awaiting_human"], false, "{row}");

    // The member's reply is what settles the round.
    let receipt =
        json_of(coder(&tmp, LATER).args(["--json", "reply", "--thread", &thread, "done"]));
    assert_eq!(receipt["posted"], true, "{receipt}");
    let reply = receipt["message_id"]
        .as_str()
        .expect("message_id")
        .to_string();
    let store = open_store(&tmp);
    let q = store.thread_quorum(&thread, LATER).unwrap().unwrap();
    assert_eq!(q.replied, vec![format!("{ORIGIN}/coder")], "{q:?}");
    assert!(q.complete, "{q:?}");
    assert_eq!(
        store.latest_reply_message_id(&thread).unwrap().as_deref(),
        Some(reply.as_str()),
        "the completing reply is the member's, never the notice"
    );
    let collected = collected_ids(&store, &thread);
    assert!(collected.contains(&reply), "{collected:?}");
    assert!(
        !collected.contains(&notice),
        "the collected answers do not carry the notice: {collected:?}"
    );
    drop(store);
    let row = status_row(&tmp, LATER, &thread);
    assert_eq!(row["state"], "answered", "{row}");
}
