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

/// A service last seen `at_millis_from_now` away from NOW: the window is two lease expiries, so a host that stopped stepping
/// just inside it still attends, and one a millisecond past it does not (§8.1).
fn send_with_host_seen(at_millis_from_now: i64) -> SendToReceipt {
    let ws = workspace();
    let eng = engine(ws.path());
    // A first send creates run.sqlite; the attendance record is then written into it.
    send(&eng, the_owner(), "coding-v2").expect("commissioned");
    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .unwrap()
        .unwrap();
    let now = nexus_run_contracts::Timestamp::parse(NOW).unwrap();
    store
        .mark_host_seen(
            &eng.origin(),
            nexus_run_contracts::Timestamp::from_unix_millis(
                now.unix_millis() + at_millis_from_now,
            ),
        )
        .unwrap();
    send(&eng, the_owner(), "coding-v2").expect("commissioned")
}

const WINDOW_MILLIS: i64 = 2 * nexus_run_host::bridge::LEASE_EXPIRY.0 as i64 * 1000;

fn says_nobody_attends(receipt: &SendToReceipt) -> bool {
    let record = serde_json::to_value(receipt).unwrap();
    record["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["class"] == "service_not_running")
}

#[test]
fn a_service_last_seen_exactly_two_lease_expiries_ago_still_attends() {
    let receipt = send_with_host_seen(-WINDOW_MILLIS);
    assert!(!says_nobody_attends(&receipt), "{:?}", receipt.warnings);
}

#[test]
fn a_service_last_seen_a_millisecond_beyond_two_lease_expiries_no_longer_attends() {
    let receipt = send_with_host_seen(-WINDOW_MILLIS - 1);
    assert!(says_nobody_attends(&receipt), "{:?}", receipt.warnings);
}

/// After clock skew a record from the future proves nothing about now: past the window either way, the
/// receipt says nobody attends rather than staying silent.
#[test]
fn a_service_seen_further_in_the_future_than_the_window_does_not_count_as_attending() {
    assert!(!says_nobody_attends(&send_with_host_seen(WINDOW_MILLIS)));
    assert!(says_nobody_attends(&send_with_host_seen(WINDOW_MILLIS + 1)));
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

/// The receipt carries the warning and the CLI prints the receipt's warnings, so the library prints none
/// itself: one line on stderr, not two, and an embedder of `Engine::send` gets no stderr write at all.
#[test]
fn nxc_send_to_an_unattended_v2_channel_warns_exactly_once_on_stderr() {
    let ws = workspace();
    let out = nxc(ws.path())
        .args(["send", "--to", "coding-v2", "--no-ref", "build it"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&out);
    assert_eq!(
        stderr.matches("no service attends this workspace").count(),
        1,
        "{stderr}"
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

// ---- a run.sqlite that does not answer -------------------------------------------------------

/// What every refusal of an unreadable `run.sqlite` must carry: where it is, why, and the way out.
fn assert_names_the_recovery(refused: &nexus_chat::error::NxfError, dir: &Path, why: &str) {
    let msg = &refused.msg;
    assert_eq!(refused.kind, ErrorKind::Conflict, "{refused}");
    assert!(
        msg.contains(&run_sqlite(dir).display().to_string()),
        "the file: {msg}"
    );
    assert!(msg.contains(why), "the reason ({why}): {msg}");
    assert!(msg.contains("Nothing was written"), "{msg}");
    assert!(
        msg.contains("move it aside") && msg.contains("abandons every run on the new engine"),
        "the recovery, and what it costs: {msg}"
    );
}

/// Only a MISSING file means "not v2": a damaged one never lets a session pass as the terminal, and never
/// lets the terminal's commission fall through to the old engine (§10; round 3 IR #2, round 4 IR #10).
#[test]
fn a_damaged_run_sqlite_refuses_a_session_and_a_commission_with_the_way_out() {
    let ws = workspace();
    let session = persona_session(ws.path(), "reviewer");
    std::fs::write(
        run_sqlite(ws.path()),
        b"this is not a database, and it is long enough",
    )
    .unwrap();
    let eng = engine(ws.path());
    let before = chat_ops(ws.path());
    let as_session = send(
        &eng,
        Caller {
            session: Some(&session),
            actor: Some("reviewer"),
            now: Some(NOW),
        },
        "review",
    )
    .expect_err("the lookup cannot answer");
    assert_names_the_recovery(&as_session, ws.path(), "not a database");
    let commission = send(&eng, the_owner(), "coding-v2").expect_err("nor can the commission");
    assert_names_the_recovery(&commission, ws.path(), "not a database");
    assert_eq!(chat_ops(ws.path()), before, "nothing written");
}

#[test]
fn a_run_sqlite_from_a_newer_nxs_refuses_and_says_so() {
    let ws = workspace();
    send(&engine(ws.path()), the_owner(), "coding-v2").expect("a first run creates the file");
    rusqlite::Connection::open(run_sqlite(ws.path()))
        .unwrap()
        .execute("UPDATE schema SET version = 99", [])
        .unwrap();
    let refused = send(&engine(ws.path()), the_owner(), "coding-v2").expect_err("newer");
    assert_names_the_recovery(&refused, ws.path(), "schema 99");
    assert!(refused.msg.contains("nxs self-update"), "{refused}");
}

/// `run.sqlite` is in WAL mode, so a held write lock does not stop the session lookup — reads go on beside
/// it. What a lock stops is the commission's write: the bridge waits its 5 s, then refuses with the reason,
/// and nothing falls through to the old engine.
#[test]
fn a_locked_run_sqlite_refuses_the_commission_after_the_bridges_timeout() {
    let ws = workspace();
    send(&engine(ws.path()), the_owner(), "coding-v2").expect("a first run creates the file");
    let before = chat_ops(ws.path());
    let holder = rusqlite::Connection::open(run_sqlite(ws.path())).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let refused = send(&engine(ws.path()), the_owner(), "coding-v2").expect_err("locked");
    let waited = started.elapsed();
    holder.execute_batch("ROLLBACK").unwrap();
    assert_names_the_recovery(&refused, ws.path(), "locked");
    assert!(
        waited >= std::time::Duration::from_secs(4),
        "it waited for the writer first: {waited:?}"
    );
    assert_eq!(
        chat_ops(ws.path()),
        before,
        "channel_open was not entered instead"
    );
}

/// What the send hands over, the host can open (§6.4 item 1): a real `Host` over `ports_v1` opens the minted
/// root thread in the chat store and posts the commission into it as the requester's, once — the post the
/// send itself never makes. Its sessions are scripted; the claim and the tree play no part before the root.
#[test]
fn the_host_opens_the_root_the_send_minted_and_posts_the_commission_into_it_once() {
    use nexus_run_host::test_support::scripted_backend::{pass, ScriptedBackend};
    use nexus_run_host::test_support::scripted_tree::ScriptedTree;
    use nexus_run_host::test_support::SimClock;
    use nexus_run_host::{Host, Layout, Ports, TreeLedger};

    let ws = workspace();
    let eng = engine(ws.path());
    let receipt = send(&eng, the_owner(), "coding-v2").expect("the owner commissions");
    let root = receipt.thread_id;
    let db = ws.path().join(".nxs").join("db.sqlite");
    let db = db.to_str().unwrap();
    let ports = std::sync::Arc::new(
        nexus_chat::ports_v1::V1Ports::for_root(ws.path(), db, &eng.origin()).unwrap(),
    );
    let layout = Layout {
        root: ws.path().to_owned(),
        agent_logs: ws.path().join(".nxs").join("agent-logs"),
        session_env: Vec::new(),
    };
    let store = std::sync::Arc::new(
        nexus_run_host::bridge::SqliteRunStore::open(
            &run_sqlite(ws.path()),
            nexus_run_host::bridge::HOST_BUSY_TIMEOUT,
        )
        .unwrap(),
    );
    let now = nexus_run_contracts::Timestamp::parse(NOW).unwrap();
    let mut host = Host::new(
        store,
        Ports {
            messages: ports.clone(),
            declarations: ports.clone(),
            sidecar: ports,
            layout: layout.clone(),
        },
        std::sync::Arc::new(ScriptedBackend::new(
            &layout.agent_logs,
            &ws.path().join("timeline.jsonl"),
            &layout.v2_dir(),
            Box::new(|_| pass()),
        )),
        std::sync::Arc::new(ScriptedTree::new(TreeLedger::new(
            &layout.agent_logs,
            &eng.origin(),
        ))),
        std::sync::Arc::new(SimClock::at(now.unix_millis())),
    );
    let chat = || chat_store(ws.path()).messages_in_thread(&root).unwrap();
    for _ in 0..5 {
        host.tick(now);
        if !chat().is_empty() {
            break;
        }
    }
    host.tick(now);

    let posted = chat();
    assert_eq!(posted.len(), 1, "the root message, once: {posted:?}");
    assert_eq!(posted[0].body, "build it");
    assert_eq!(posted[0].sender, format!("{}/owner", eng.origin()));
    let refs: Refs = serde_json::from_str(posted[0].refs.as_deref().unwrap()).unwrap();
    assert_eq!(refs.nxf_ids, ["6j6v.b8tj"], "the --ref the send named");
    assert!(
        refs.run_key
            .as_deref()
            .is_some_and(|key| key.starts_with("root/")),
        "the host's key, which its find-or-post finds it by: {refs:?}"
    );
    // The substrate channel of a declared channel, as the old engine opens it: the read filters resolve
    // visibility and membership from `decl:<name>` (6j6v.y6v7), and a bare name is a channel nobody reads.
    assert_eq!(
        chat_store(ws.path()).thread_channel(&root).as_deref(),
        Some("decl:coding-v2")
    );
}

// ---- identity: qualified callers, bare declarations, and the precedence -----------------------

/// Who a caller is comes from PROVENANCE — the session it presents, looked up — and never from the handle it
/// states (memory declared-members-bare-caller-qualified). A caller presenting its handle QUALIFIED
/// (`<origin>/reviewer`) against a BARE declaration (`reviewer`) is therefore still the persona: the refusal
/// fires and nothing is written. Were the persona line keyed on the stated handle compared with `==`, this
/// caller would read as the terminal and its send would be commissioned.
#[test]
fn a_persona_presenting_a_qualified_handle_is_still_refused_a_v2_channel_before_any_write() {
    let ws = workspace();
    let session = persona_session(ws.path(), "reviewer");
    let eng = engine(ws.path());
    let qualified = format!("{}/reviewer", eng.origin());
    let before = chat_ops(ws.path());
    let refused = send(
        &eng,
        Caller {
            session: Some(&session),
            actor: Some(&qualified),
            now: Some(NOW),
        },
        "coding-v2",
    )
    .expect_err("still the persona");
    assert!(refused.msg.contains("runs on the new engine"), "{refused}");
    assert_eq!(chat_ops(ws.path()), before);
    assert!(!run_sqlite(ws.path()).exists());
}

#[test]
fn a_v2_session_presenting_a_qualified_handle_is_still_refused_before_any_write() {
    let ws = workspace();
    let session = v2_session(ws.path());
    let eng = engine(ws.path());
    let qualified = format!("{}/coder", eng.origin());
    let before = chat_ops(ws.path());
    for to in ["reviewer", "coding-v2"] {
        let refused = send(
            &eng,
            Caller {
                session: Some(&session),
                actor: Some(&qualified),
                now: Some(NOW),
            },
            to,
        )
        .expect_err("still a v2 session");
        assert!(
            refused.msg.contains("cannot commission in this slice"),
            "{to}: {refused}"
        );
    }
    assert_eq!(chat_ops(ws.path()), before);
}

/// `resolve_caller` answers the BARE handle the declarations use, from the slot the session runs — so every
/// later comparison against a declaration (`for_persona`, `addressable`) is between two bare handles.
#[test]
fn resolve_caller_answers_the_bare_declared_handle_for_a_v2_session() {
    use nexus_chat::persona::Identity;
    use nexus_chat::v2_bridge::resolve_caller;
    let ws = workspace();
    let session = real_v2_session(ws.path());
    let (store, defs, db) = resolver_inputs(ws.path());
    let (identity, v2) = resolve_caller(&store, &defs, None, Some(&session), &db, "local").unwrap();
    assert_eq!(identity, Identity::Persona("coder".to_owned()));
    assert_eq!(v2.expect("a v2 session").handle, "coder");
}

/// The precedence, pinned: an explicit `--persona` is the host's claim and stands; else the run store's
/// `session/<key>`; else the old session map. A key BOTH stores know is the new engine's, because that is the
/// session that is actually running.
#[test]
fn the_precedence_is_explicit_persona_then_the_run_store_then_the_old_session_map() {
    use nexus_chat::persona::Identity;
    use nexus_chat::v2_bridge::resolve_caller;
    let ws = workspace();
    let session = real_v2_session(ws.path());
    chat_store(ws.path())
        .create_pending_session(&session, "reviewer")
        .unwrap();
    let old_only = persona_session(ws.path(), "reviewer");
    let (store, defs, db) = resolver_inputs(ws.path());
    let resolve = |declared: Option<&str>, session: Option<&str>| {
        resolve_caller(&store, &defs, declared, session, &db, "local").unwrap()
    };

    let (explicit, v2) = resolve(Some("reviewer"), Some(&session));
    assert_eq!(
        explicit,
        Identity::Persona("reviewer".to_owned()),
        "--persona stands"
    );
    assert_eq!(v2, None);
    let (run_store, v2) = resolve(None, Some(&session));
    assert_eq!(
        run_store,
        Identity::Persona("coder".to_owned()),
        "the run store before the old map"
    );
    assert!(v2.is_some());
    let (old_map, v2) = resolve(None, Some(&old_only));
    assert_eq!(old_map, Identity::Persona("reviewer".to_owned()));
    assert_eq!(v2, None);
    let (nobody, _) = resolve(None, None);
    assert_eq!(nobody, Identity::Human);
}

fn resolver_inputs(dir: &Path) -> (ChatStore, nexus_chat::definitions::Definitions, String) {
    let db = dir.join(".nxs").join("db.sqlite");
    (
        chat_store(dir),
        nexus_chat::definitions::Definitions::resolve(dir).unwrap(),
        db.to_str().unwrap().to_owned(),
    )
}

/// A v2 session whose slot the run's log places: the owner commissions, and a real `Host` (scripted
/// backend and tree) steps the run until the build slot's session is recorded.
fn real_v2_session(dir: &Path) -> String {
    use nexus_run_host::test_support::scripted_backend::{pass, ScriptedBackend};
    use nexus_run_host::test_support::scripted_tree::ScriptedTree;
    use nexus_run_host::test_support::SimClock;
    use nexus_run_host::{Host, Layout, Ports, TreeLedger};

    let eng = engine(dir);
    send(&eng, the_owner(), "coding-v2").expect("the owner commissions");
    let db = dir.join(".nxs").join("db.sqlite");
    let ports = std::sync::Arc::new(
        nexus_chat::ports_v1::V1Ports::for_root(dir, db.to_str().unwrap(), &eng.origin()).unwrap(),
    );
    let layout = Layout {
        root: dir.to_owned(),
        agent_logs: dir.join(".nxs").join("agent-logs"),
        session_env: Vec::new(),
    };
    let store = std::sync::Arc::new(
        nexus_run_host::bridge::SqliteRunStore::open(
            &run_sqlite(dir),
            nexus_run_host::bridge::HOST_BUSY_TIMEOUT,
        )
        .unwrap(),
    );
    let mut now = nexus_run_contracts::Timestamp::parse(NOW).unwrap();
    let clock = std::sync::Arc::new(SimClock::at(now.unix_millis()));
    let tree = std::sync::Arc::new(ScriptedTree::new(TreeLedger::new(
        &layout.agent_logs,
        &eng.origin(),
    )));
    let mut host = Host::new(
        store.clone(),
        Ports {
            messages: ports.clone(),
            declarations: ports.clone(),
            sidecar: ports,
            layout: layout.clone(),
        },
        std::sync::Arc::new(ScriptedBackend::new(
            &layout.agent_logs,
            &dir.join("timeline.jsonl"),
            &layout.v2_dir(),
            Box::new(|_| pass()),
        )),
        tree.clone(),
        clock.clone(),
    );
    let run = store.open_runs().unwrap()[0].run.clone();
    let key = format!("{run}/build/v1/coder/a1");
    // Only ticks the run could use count: one the tree's worker thread is working through is real
    // time, and a loaded runner decides how much (the clock stands still meanwhile, as `Rig::tick`).
    let ceiling = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let mut counted = 0;
    while counted < 40 && std::time::Instant::now() < ceiling {
        host.tick(now);
        if store.get(&format!("session/{key}")).unwrap().is_some() {
            return key;
        }
        if tree.busy() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            clock.advance(1_000);
            now = nexus_run_contracts::Timestamp::from_unix_millis(now.unix_millis() + 1_000);
            counted += 1;
        }
    }
    panic!("the host never started the build slot");
}

/// A host may hand its acting user's handle qualified (`<origin>/owner`) or bare (`owner`): it is one
/// requester either way, recorded in the one qualified form every v1 sender has — never `<origin>/<origin>/owner`.
/// The requester checks of the root (6j6v.y6v7) compare against this value.
#[test]
fn a_requester_handed_qualified_or_bare_is_recorded_as_the_same_qualified_handle() {
    let ws = workspace();
    let eng = engine(ws.path());
    let qualified = format!("{}/owner", eng.origin());
    send(&eng, the_owner(), "coding-v2").unwrap();
    send(
        &eng,
        Caller {
            session: None,
            actor: Some(&qualified),
            now: Some(NOW),
        },
        "coding-v2",
    )
    .unwrap();
    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .unwrap()
        .unwrap();
    let requesters: Vec<String> = store
        .open_runs()
        .unwrap()
        .iter()
        .map(|row| {
            nexus_run_host::bridge::view(&store, &row.run)
                .unwrap()
                .unwrap()
                .state
                .requester
                .to_string()
        })
        .collect();
    assert_eq!(
        requesters,
        [qualified.clone(), qualified],
        "one requester, one form"
    );
}

/// A persona's own declared route (`addressable: [<channels>]`) is filtered the same way the derived one is:
/// a refusal never offers a persona a channel whose send it would refuse (6j6v.b8tj review, CQ #3).
#[test]
fn a_declared_route_through_a_v2_channel_is_not_offered_to_a_persona() {
    let ws = workspace();
    std::fs::write(
        ws.path().join(".nxs-personas").join("lister.yaml"),
        "handle: lister\naddressable: [coding-v2, review]\nsystem_prompt: p\n",
    )
    .unwrap();
    let session = persona_session(ws.path(), "reviewer");
    let eng = engine(ws.path());
    let to_persona = send(
        &eng,
        Caller {
            session: Some(&session),
            actor: Some("reviewer"),
            now: Some(NOW),
        },
        "lister",
    )
    .expect_err("lister admits no direct message");
    assert!(
        to_persona.msg.contains("review") && !to_persona.msg.contains("coding-v2"),
        "{to_persona}"
    );
    let to_owner = send(&eng, the_owner(), "lister").expect_err("nor from the terminal");
    assert!(
        to_owner.msg.contains("coding-v2"),
        "the terminal keeps it: {to_owner}"
    );
}

/// A session key the run store routes but whose slot its log cannot place is still a session, never the
/// owner — under a placeholder handle no declaration has, not the key itself posing as a handle
/// (6j6v.b8tj review, CQ #1). The key stays on the `V2Session`.
#[test]
fn an_unplaced_v2_session_is_a_persona_under_the_placeholder_and_keeps_its_key() {
    use nexus_chat::persona::Identity;
    use nexus_chat::v2_bridge::{resolve_caller, UNPLACED_V2_SESSION};
    let ws = workspace();
    let key = v2_session(ws.path());
    let (store, defs, db) = resolver_inputs(ws.path());
    let (identity, v2) = resolve_caller(&store, &defs, None, Some(&key), &db, "local").unwrap();
    assert_eq!(identity, Identity::Persona(UNPLACED_V2_SESSION.to_owned()));
    let v2 = v2.unwrap();
    assert_eq!(v2.handle, UNPLACED_V2_SESSION);
    assert_eq!(v2.session_key, key);
}

// ---- the `engine:` key ------------------------------------------------------------------------

fn load(engine: &str) -> nexus_chat::error::Result<nexus_chat::definitions::Definitions> {
    let ws = workspace();
    std::fs::write(
        ws.path().join(".nxs-personas").join("channels.yaml"),
        format!("- name: coding-v2\n  engine: {engine}\n  members: [coder]\n"),
    )
    .unwrap();
    nexus_chat::definitions::Definitions::resolve(ws.path())
}

/// An engine nobody built is refused at load, naming the value — never read as v1, which would run the
/// channel on the old engine in the root while its author meant the new one.
#[test]
fn an_unknown_engine_value_is_refused_at_load_rather_than_read_as_v1() {
    for bad in ["v3", "V2", "\"\""] {
        let refused = load(bad).expect_err(bad);
        assert!(
            refused.msg.contains("engine")
                || refused.msg.contains("v3")
                || refused.msg.contains("variant"),
            "{bad}: {refused}"
        );
    }
    assert!(load("v1").is_ok());
    assert!(load("v2").is_ok());
}

/// A v1 declaration serializes exactly as before the key existed; a v2 one carries it.
#[test]
fn a_v1_declaration_serializes_without_an_engine_key() {
    let v1 = load("v1").unwrap();
    let v1 = serde_json::to_value(v1.channel("coding-v2").unwrap()).unwrap();
    assert!(v1.get("engine").is_none(), "{v1}");
    let v2 = load("v2").unwrap();
    let v2 = serde_json::to_value(v2.channel("coding-v2").unwrap()).unwrap();
    assert_eq!(v2["engine"], "v2");
}

/// A v2 channel the new engine cannot run in this slice — `working_tree: shared` is refused at compile time
/// (6j6v.zjyv §6.1) — is refused by the send before anything is written, as a validation error naming why.
/// (Declaration-time validation of v2 shapes is not part of the switch; the compile is the one judge.)
#[test]
fn a_v2_channel_the_new_engine_cannot_run_is_refused_before_anything_is_written() {
    let ws = workspace();
    std::fs::write(
        ws.path().join(".nxs-personas").join("channels.yaml"),
        "- name: coding-v2\n  engine: v2\n  members: []\n  steps:\n    - id: build\n      target: coder\n",
    )
    .unwrap();
    let before = chat_ops(ws.path());
    let refused = send(&engine(ws.path()), the_owner(), "coding-v2").expect_err("shared tree");
    assert_eq!(refused.kind, ErrorKind::Validation, "{refused}");
    assert!(refused.msg.contains("does not compile"), "{refused}");
    assert_eq!(chat_ops(ws.path()), before);
    let runs = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .unwrap()
        .map(|store| store.open_runs().unwrap().len())
        .unwrap_or(0);
    assert_eq!(runs, 0, "no run was opened");
}
