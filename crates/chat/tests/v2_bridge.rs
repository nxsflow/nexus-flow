//! The bridge between the two engines, part 1 (6j6v.b8tj; epic 6j6v.zjyv §10, §12.9): a channel declared
//! `engine: v2` is commissioned through `run_host::commission`, below both callers of `send_to`, and the old
//! engine is never entered for it; no persona and no v2 session commissions one.
//!
//! Each case runs at the `surface` seam through `Engine::send_to`, which is `surface::send_to` and nothing else,
//! and the ones the ticket names once more through the `nxc` binary.

mod common;

use std::path::{Path, PathBuf};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::model::Refs;
use nexus_chat::orchestration::Caller;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToReceipt, SendToRefs, SendToRequest, TargetKind};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nexus_run_contracts::RunStore;
use tempfile::TempDir;

const NOW: &str = "2026-11-12T10:00:00Z";
/// Reached only through the channels that cast it, so a directory offers those channels as its route in.
const CODER: &str =
    "handle: coder\njob_title: Implementer\naddressable: none\nsystem_prompt: You are the coder.\n";
const REVIEWER: &str = "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You review.\n";
/// One channel on each engine: `coding-v2` as the repository declares it in miniature, and a v1 `review`.
const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
- name: review
  members: [coder, reviewer]
";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(personas.join("coder.yaml"), CODER).unwrap();
    std::fs::write(personas.join("reviewer.yaml"), REVIEWER).unwrap();
    std::fs::write(personas.join("channels.yaml"), CHANNELS).unwrap();
    tmp
}

fn engine(dir: &Path) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

fn chat_store(dir: &Path) -> ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Every write to the chat store is an op: the count is what "before anything is written" is measured by.
fn chat_ops(dir: &Path) -> i64 {
    rusqlite::Connection::open(dir.join(".nxs").join("db.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM ops", [], |row| row.get(0))
        .unwrap()
}

/// A session the old engine started for `handle`, as its session map records it.
fn persona_session(dir: &Path, handle: &str) -> String {
    let session = format!("s-{handle}");
    chat_store(dir)
        .create_pending_session(&session, handle)
        .unwrap();
    session
}

/// A session of a run on the new engine: the owner commissions `coding-v2`, and the run's build slot is
/// recorded the way the host's `StartSession` records it — `thread/<id>` routing a step thread to the session
/// key, and `session/<key>` naming that thread (6j6v.zjyv §6.4 item 3).
fn v2_session(dir: &Path) -> String {
    send(&engine(dir), the_owner(), "coding-v2").expect("the owner commissions");
    let store = nexus_run_host::bridge::open_store(&run_sqlite(dir))
        .unwrap()
        .unwrap();
    let run = store.open_runs().unwrap()[0].run.clone();
    let key = format!("{run}/build/v1/coder/a1");
    let thread = "m-01JSTEPTHREAD0000000000000";
    let route = serde_json::json!({ "run": run, "session_key": key }).to_string();
    store
        .compare_and_set(&format!("thread/{thread}"), None, Some(route.as_bytes()))
        .unwrap();
    store
        .compare_and_set(&format!("session/{key}"), None, Some(thread.as_bytes()))
        .unwrap();
    key
}

fn run_sqlite(dir: &Path) -> PathBuf {
    dir.join(".nxs").join("run.sqlite")
}

fn the_owner() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("owner"),
        now: Some(NOW),
    }
}

fn about_the_ticket() -> SendToRefs {
    SendToRefs::declared(Refs {
        nxf_ids: vec!["6j6v.b8tj".to_owned()],
        ..Refs::default()
    })
}

fn send(engine: &Engine, caller: Caller<'_>, to: &str) -> nexus_chat::error::Result<SendToReceipt> {
    engine.send_to(
        caller,
        SendToRequest {
            to,
            body: "build it",
            refs: about_the_ticket(),
            machine: None,
        },
    )
}

#[test]
fn a_send_from_the_terminal_to_a_v2_channel_commissions_a_run_and_answers_with_its_root_at_once() {
    let ws = workspace();
    let receipt =
        send(&engine(ws.path()), the_owner(), "coding-v2").expect("the owner commissions");

    assert_eq!(receipt.target, TargetKind::Channel);
    assert_eq!(receipt.to, "coding-v2");
    assert_eq!(
        receipt.message_id, None,
        "the root message is the host's to post, so the receipt names none"
    );
    let root = receipt.thread_id.clone();
    assert!(root.starts_with("m-"), "a chat thread id: {root}");

    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .expect("run.sqlite opens")
        .expect("the commission created run.sqlite");
    assert!(
        store.get(&format!("thread/{root}")).unwrap().is_some(),
        "the root thread maps to the run"
    );
    assert_eq!(store.open_runs().unwrap().len(), 1, "one run, commissioned");

    let chat = chat_store(ws.path());
    assert!(
        chat.messages_in_thread(&root).unwrap().is_empty(),
        "nothing is posted before the log can answer for it, and the old engine never posts it"
    );
    assert!(
        chat.threads_opened_by("__channel__").unwrap().is_empty(),
        "channel_open was never entered"
    );
}

/// The requester learns the result through `nxc status`, whose `open` reaches 0 when the run finishes —
/// never through `threads show`'s `complete`, which a v2 root thread does not reach (§10).
#[test]
fn the_receipt_of_a_v2_commission_says_to_poll_status_until_the_run_is_finished() {
    let ws = workspace();
    let receipt =
        send(&engine(ws.path()), the_owner(), "coding-v2").expect("the owner commissions");
    let root = receipt.thread_id.clone();
    let record = serde_json::to_value(&receipt).unwrap();

    assert_eq!(record["message_id"], serde_json::Value::Null);
    let guidance = &record["await"];
    assert_eq!(
        guidance["how"], "poll",
        "nobody wakes the person at the terminal"
    );
    assert_eq!(
        guidance["poll"],
        serde_json::json!(["nxc", "status", "--thread", root, "--json"])
    );
    assert_eq!(guidance["done_when"], serde_json::json!({ "open": 0 }));
    assert_eq!(
        guidance["stopped_when"],
        serde_json::json!({ "needs_decision": true, "awaiting_human": true })
    );
    assert_eq!(
        guidance["answer_in"],
        serde_json::json!(["nxc", "threads", "show", root, "--json"]),
        "the answer is the newest message of the root thread"
    );
}

/// The inbox exists for inputs that arrive while no owner is alive, so the send goes through either way;
/// what differs is that the receipt says nobody attends (§8.1).
#[test]
fn a_v2_commission_with_no_service_attending_says_so_and_one_attended_says_nothing() {
    let ws = workspace();
    let eng = engine(ws.path());
    let unattended = send(&eng, the_owner(), "coding-v2").expect("commissioned all the same");
    let record = serde_json::to_value(&unattended).unwrap();
    let warnings = record["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{record}");
    assert_eq!(warnings[0]["class"], "service_not_running");
    assert_eq!(
        warnings[0]["thread"], record["thread_id"],
        "about the run's root"
    );
    assert!(
        warnings[0]["detail"]
            .as_str()
            .unwrap()
            .contains("no service attends this workspace — `nxs service status`"),
        "{record}"
    );

    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .unwrap()
        .unwrap();
    store
        .mark_host_seen(
            &eng.origin(),
            nexus_run_contracts::Timestamp::parse(NOW).unwrap(),
        )
        .unwrap();
    let attended = send(&eng, the_owner(), "coding-v2").expect("commissioned");
    assert!(attended.warnings.is_empty(), "{:?}", attended.warnings);
}

/// No persona commissions a v2 run in slice 1 (owner, 2026-11-04): the list hides the channel, and the send
/// refuses it, because `coding` keeps its name when it switches and a prompt names it in prose (§10).
#[test]
fn a_persona_sessions_send_to_a_v2_channel_is_refused_before_anything_is_written() {
    let ws = workspace();
    let session = persona_session(ws.path(), "reviewer");
    let before = chat_ops(ws.path());
    let refused = send(
        &engine(ws.path()),
        Caller {
            session: Some(&session),
            actor: Some("reviewer"),
            now: Some(NOW),
        },
        "coding-v2",
    )
    .expect_err("a persona does not commission the new engine");

    assert_eq!(refused.kind, ErrorKind::Forbidden, "{refused}");
    assert!(
        refused.msg.contains("coding-v2 runs on the new engine")
            && refused.msg.contains("commission it from the terminal"),
        "says why and what to do: {refused}"
    );
    assert_eq!(
        chat_ops(ws.path()),
        before,
        "nothing written to the chat store"
    );
    assert!(
        !run_sqlite(ws.path()).exists(),
        "and nothing to the run store"
    );
}

/// A v2 session sends to nobody in this slice — persona or channel, either engine: no slot state says
/// "waiting on my own commission", and the old engine's answer would wake a second session beside the slot
/// (§10). Its `NXC_SESSION` is unknown to the old session map, so without the lookup it would pass as the
/// owner at the terminal.
#[test]
fn a_v2_sessions_send_to_anything_is_refused_before_anything_is_written() {
    let ws = workspace();
    let session = v2_session(ws.path());
    let eng = engine(ws.path());
    let runs = || {
        nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
            .unwrap()
            .unwrap()
            .open_runs()
            .unwrap()
            .len()
    };
    let (ops, open) = (chat_ops(ws.path()), runs());
    for to in ["reviewer", "review", "coding-v2"] {
        let refused = send(
            &eng,
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            to,
        )
        .expect_err("a v2 session does not commission");
        assert_eq!(refused.kind, ErrorKind::Forbidden, "{to}: {refused}");
        assert!(
            refused.msg.contains("cannot commission in this slice")
                && refused.msg.contains("`nxc reply --escalate`"),
            "{to}: says why and what to do instead: {refused}"
        );
    }
    assert_eq!(
        chat_ops(ws.path()),
        ops,
        "nothing written to the chat store"
    );
    assert_eq!(runs(), open, "and no run commissioned");
}

/// The one view behind a persona's `nxc list` and its prompt's "Who you can address" omits channels on the new
/// engine, and the casting it offers as a route in never names one; the person at the terminal keeps the full
/// directory and commissions them (§10).
#[test]
fn a_personas_directory_omits_v2_channels_and_the_terminals_keeps_them() {
    let ws = workspace();
    let eng = engine(ws.path());
    let names = |directory: nexus_chat::persona::Directory| -> Vec<String> {
        directory.channels.into_iter().map(|c| c.name).collect()
    };
    let full = eng
        .directory(nexus_chat::facade::Reader::Owner, None)
        .unwrap();
    let route = |directory: &nexus_chat::persona::Directory| -> Vec<String> {
        directory
            .personas
            .iter()
            .find(|p| p.handle == "coder")
            .map(|p| p.channels.clone())
            .unwrap()
    };
    assert_eq!(
        route(&full),
        ["coding-v2", "review"],
        "the terminal's route in"
    );
    assert!(
        names(full).contains(&"coding-v2".to_owned()),
        "the terminal sees it"
    );

    let reviewer = eng
        .directory(nexus_chat::facade::Reader::Owner, Some("reviewer"))
        .unwrap();
    assert_eq!(
        route(&reviewer),
        ["review"],
        "no route in through a v2 channel"
    );
    assert!(
        !names(reviewer).contains(&"coding-v2".to_owned()),
        "a persona's list does not offer it"
    );
}

/// A refusal that names a route in must not send a persona to a channel whose own send would refuse it.
#[test]
fn a_refusal_never_suggests_a_v2_channel_to_a_persona_and_does_to_the_terminal() {
    let ws = workspace();
    let session = persona_session(ws.path(), "reviewer");
    let eng = engine(ws.path());
    let to_persona = send(
        &eng,
        Caller {
            session: Some(&session),
            actor: Some("reviewer"),
            now: Some(NOW),
        },
        "coder",
    )
    .expect_err("coder admits no direct message");
    assert!(
        to_persona.msg.contains("review") && !to_persona.msg.contains("coding-v2"),
        "{to_persona}"
    );
    let to_owner = send(&eng, the_owner(), "coder").expect_err("nor from the terminal");
    assert!(to_owner.msg.contains("coding-v2"), "{to_owner}");
}

// ---- once through the CLI --------------------------------------------------------------------

/// The person at the terminal: no session stamps.
fn nxc(dir: &Path) -> assert_cmd::Command {
    let mut cmd = nxs_test_support::cargo_bin("nxc");
    cmd.current_dir(dir)
        .env_remove("NXC_SESSION")
        .env_remove("NXC_DB")
        .env("NXC_ACTOR", "owner")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", dir.join("dry.log"));
    cmd
}

/// A session, by the stamps a spawned session runs under.
fn nxc_as(dir: &Path, handle: &str, session: &str) -> assert_cmd::Command {
    let mut cmd = nxc(dir);
    cmd.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    cmd
}

fn json_out(cmd: &mut assert_cmd::Command) -> serde_json::Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).expect("one JSON record")
}

fn channel_names(listing: &serde_json::Value) -> Vec<String> {
    listing["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn nxc_send_to_a_v2_channel_prints_the_root_id_at_once_and_the_status_read() {
    let ws = workspace();
    let receipt = json_out(nxc(ws.path()).args([
        "--json",
        "send",
        "--to",
        "coding-v2",
        "--ref",
        "nxf_ids=6j6v.b8tj",
        "build it",
    ]));
    let root = receipt["thread_id"].as_str().unwrap().to_owned();
    assert!(root.starts_with("m-"), "{receipt}");
    assert_eq!(receipt["message_id"], serde_json::Value::Null);
    assert_eq!(receipt["await"]["poll"][1], "status");
    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .unwrap()
        .unwrap();
    assert!(store.get(&format!("thread/{root}")).unwrap().is_some());

    let out = nxc(ws.path())
        .args(["send", "--to", "coding-v2", "--no-ref", "and again"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8_lossy(&out);
    assert!(human.contains("nxc status --thread m-"), "{human}");
    assert!(
        !human.contains("--stream"),
        "a run is not followed by --stream yet: {human}"
    );
}

#[test]
fn nxc_list_hides_v2_channels_from_a_persona_and_a_v2_session_and_shows_them_at_the_terminal() {
    let ws = workspace();
    let persona = persona_session(ws.path(), "reviewer");
    let v2 = v2_session(ws.path());
    assert!(
        channel_names(&json_out(nxc(ws.path()).args(["--json", "list"])))
            .contains(&"coding-v2".to_owned())
    );
    for (handle, session) in [("reviewer", persona.as_str()), ("coder", v2.as_str())] {
        let listing = json_out(nxc_as(ws.path(), handle, session).args(["--json", "list"]));
        assert!(
            !channel_names(&listing).contains(&"coding-v2".to_owned()),
            "{handle}: {listing}"
        );
    }
}

#[test]
fn nxc_send_refuses_a_persona_to_a_v2_channel_and_a_v2_session_to_anything() {
    let ws = workspace();
    let persona = persona_session(ws.path(), "reviewer");
    let v2 = v2_session(ws.path());
    let before = chat_ops(ws.path());
    let refused = |cmd: &mut assert_cmd::Command| {
        let out = cmd.assert().failure().get_output().stderr.clone();
        String::from_utf8_lossy(&out).into_owned()
    };
    let persona_says = refused(nxc_as(ws.path(), "reviewer", &persona).args([
        "send",
        "--to",
        "coding-v2",
        "--no-ref",
        "build it",
    ]));
    assert!(
        persona_says.contains("runs on the new engine"),
        "{persona_says}"
    );
    let v2_says = refused(
        nxc_as(ws.path(), "coder", &v2).args(["send", "--to", "reviewer", "--no-ref", "look"]),
    );
    assert!(v2_says.contains("`nxc reply --escalate`"), "{v2_says}");
    assert_eq!(chat_ops(ws.path()), before, "nothing written");
}

/// A v2 session is a SESSION to the old engine's own gate too: withdrawing a v1 operation belongs to the
/// person who started it (owner decision s8gf), and a session's `NXC_SESSION` the old map does not know must
/// not make it that person (§10).
#[test]
fn a_v2_session_cannot_withdraw_a_v1_operation() {
    let ws = workspace();
    let session = v2_session(ws.path());
    let eng = engine(ws.path());
    let v1 = send(&eng, the_owner(), "review").expect("a v1 round");
    let before = chat_ops(ws.path());
    let refused = eng
        .withdraw(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            &v1.thread_id,
        )
        .expect_err("not the owner");
    assert_eq!(refused.kind, ErrorKind::Forbidden, "{refused}");
    assert!(
        refused.msg.contains("withdraw is not a verb for a session")
            && refused.msg.contains("--escalate"),
        "{refused}"
    );
    assert_eq!(
        chat_ops(ws.path()),
        before,
        "nothing withdrawn, nothing written"
    );
}
