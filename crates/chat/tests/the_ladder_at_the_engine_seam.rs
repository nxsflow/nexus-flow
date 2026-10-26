//! **The ladder's two new calls, at the `Engine` seam** (nxf 6j6v.558h) — `Engine::reply_progress`
//! (`nxc reply --progress`) and `Engine::session_ended_with` (`nxc session ended --cause
//! --attempts`). The products speak `Engine::*`, so the CLI and the compute-layer tests in
//! `the_ladder.rs` do not count for these two.
//!
//! **And a session inside one tool call** (nxf 6j6v.j6yk): what the coordinator reads as work comes
//! in through `Engine::transcript_append` — the call a host makes with what its sidecar flushed —
//! and what the coordinator made of it goes out through `Engine::status`. The look between them is
//! `nxc tick` / the service's timer, which has no `Engine` verb; it is run here on the engine's own
//! workspace with the engine's own worker.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{Reader, StatusScope};
use nexus_chat::ladder::{SILENCE_SECS, TOOL_CALL_BOUND_SECS};
use nexus_chat::orchestration::{self, Caller, SessionEnd, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest, PROGRESS_UNREQUESTED};
use nexus_chat::timer::DisabledTimer;
use nexus_chat::timer::TimerConfig;
use nexus_chat::transcript::TranscriptEntry;
use nexus_chat::worker::{
    AskOutcome, SessionAsk, SessionStop, TriggerOutcome, TriggerRequest, TriggerResult, Worker,
    WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-25T09:00:00Z";

#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    asked: Mutex<Vec<SessionAsk>>,
}

impl Worker for Hand {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn answers_liveness(&self) -> bool {
        true
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn asks_sessions(&self) -> bool {
        true
    }
    fn stop_session(&self, _internal_session: &str) -> Result<SessionStop, String> {
        Ok(SessionStop::Requested)
    }
    fn ask_session(&self, ask: &SessionAsk) -> AskOutcome {
        if !self.session_is_running(&ask.session) {
            return AskOutcome::NoProcess;
        }
        self.asked.lock().unwrap().push(ask.clone());
        AskOutcome::Asked
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn engine() -> (TempDir, Engine, Arc<Hand>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(Hand::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine, worker)
}

fn at<'a>(actor: Option<&'a str>, session: Option<&'a str>, now: &'a str) -> Caller<'a> {
    Caller {
        session,
        actor,
        now: Some(now),
    }
}

fn commissioned(engine: &Engine, worker: &Hand) -> (String, String) {
    let thread = engine
        .send_to(
            at(Some("carsten"), None, NOW),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "Add a --since flag.",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission")
        .thread_id;
    let session = worker.seen.lock().unwrap()[0].internal_session.clone();
    (thread, session)
}

/// `Engine::reply_progress` posts a progress note that discharges nothing, and with no ask pending
/// its receipt says it counted for nothing.
#[test]
fn engine_reply_progress_posts_a_note_that_discharges_nothing() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = commissioned(&engine, &worker);
    let receipt = engine
        .reply_progress(at(None, Some(&session), NOW), &thread, "still building")
        .expect("the progress is posted");
    assert!(!receipt.counted);
    assert_eq!(receipt.receipt, PROGRESS_UNREQUESTED);
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let kinds: Vec<String> = store
        .messages_in_thread(&thread)
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(kinds, vec!["info", "progress"]);
    let q = store.thread_quorum(&thread, NOW).unwrap().unwrap();
    assert!(!q.outstanding.is_empty(), "nothing discharged: {q:?}");
}

/// `Engine::session_ended_with` records the end and the cause it carried, where the coordinator's
/// escalation reads it.
#[test]
fn engine_session_ended_with_keeps_the_cause_on_the_session() {
    let (tmp, engine, worker) = engine();
    let (_thread, session) = commissioned(&engine, &worker);
    engine
        .session_ended_with(
            at(Some("coder"), Some(&session), NOW),
            &session,
            SessionEnd {
                cause: Some("told to stop (SIGTERM)"),
                attempts: Some(1),
            },
        )
        .expect("the end is accepted");
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    assert!(store.session_ended_at(&session).unwrap().is_some());
    assert_eq!(
        store.session_last_cause(&session).unwrap().as_deref(),
        Some("told to stop (SIGTERM)")
    );
}

// ---- a session inside one tool call (nxf 6j6v.j6yk) ---------------------------------------------

fn later(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(NOW, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

/// One transcript entry in the shape the sidecar flushes it.
fn entry(kind: &str, secs: i64, id: Option<&str>, data: serde_json::Value) -> TranscriptEntry {
    serde_json::from_value(serde_json::json!({
        "kind": kind,
        "at": later(secs),
        "toolUseId": id,
        "data": data,
    }))
    .expect("a transcript entry")
}

/// The coordinator's look at `thread` at `secs` — `nxc tick`, on the engine's workspace and worker.
fn look(tmp: &TempDir, engine: &Engine, worker: &Arc<Hand>, thread: &str, secs: i64) {
    let ws = Workspace::resolve(None, tmp.path()).unwrap();
    let db_path = ws.db_path_str().unwrap();
    let mut store = ws.open_chat_store().unwrap();
    let defs = engine.definitions().unwrap();
    let origin = engine.origin();
    let now = later(secs);
    let ctx = orchestration::Ctx {
        now: &now,
        origin: &origin,
        actor: "service",
        session: None,
        hop: 0,
        defs: &defs,
        worker: worker.as_ref(),
        timer: &DisabledTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id: thread }).expect("tick");
}

/// What `Engine::status` says of `thread`: (under recovery, its escalations).
fn seen(tmp: &TempDir, engine: &Engine, thread: &str, secs: i64) -> (bool, usize) {
    let report = engine
        .status(Reader::Owner, &later(secs), StatusScope::All(None))
        .expect("status reads");
    let row = report
        .operations
        .into_iter()
        .flat_map(|o| o.threads)
        .find(|t| t.thread_id == thread)
        .expect("a status row");
    let escalations = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap()
        .messages_in_thread(thread)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "escalation")
        .count();
    (row.recovery.is_some(), escalations)
}

/// A commissioned coder whose session runs and whose turn on the slot has begun.
fn working(tmp: &TempDir, engine: &Engine, worker: &Arc<Hand>) -> (String, String) {
    let (thread, session) = commissioned(engine, worker);
    worker.running.lock().unwrap().insert(session.clone());
    engine
        .transcript_append(
            &session,
            &[entry(
                "turn_start",
                1,
                None,
                serde_json::json!({ "thread": thread }),
            )],
        )
        .expect("the turn is recorded");
    let _ = tmp;
    (thread, session)
}

/// **A session in one tool call longer than three ask intervals is neither asked nor escalated** —
/// the open `tool_use` arrives through `Engine::transcript_append`, and `Engine::status` shows no
/// recovery under way after four ask intervals.
#[test]
fn engine_a_session_inside_one_long_tool_call_is_neither_asked_nor_escalated() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = working(&tmp, &engine, &worker);
    engine
        .transcript_append(
            &session,
            &[entry(
                "tool_use",
                60,
                Some("toolu_watch"),
                serde_json::json!({ "name": "Bash", "input": { "command": "gh run watch" } }),
            )],
        )
        .expect("the open call is recorded");
    let mut secs = 300;
    while secs <= 60 + 4 * SILENCE_SECS {
        look(&tmp, &engine, &worker, &thread, secs);
        secs += 300;
    }
    assert!(worker.asked.lock().unwrap().is_empty());
    assert_eq!(seen(&tmp, &engine, &thread, secs), (false, 0));
}

/// **A session that is really silent is still asked, and escalated after three** — its one call
/// returned, and nothing has been heard since.
#[test]
fn engine_a_really_silent_session_is_still_asked_and_escalated() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = working(&tmp, &engine, &worker);
    engine
        .transcript_append(
            &session,
            &[
                entry(
                    "tool_use",
                    60,
                    Some("toolu_read"),
                    serde_json::json!({ "name": "Read", "input": {} }),
                ),
                entry(
                    "tool_result",
                    70,
                    Some("toolu_read"),
                    serde_json::json!({ "content": "ok", "isError": false }),
                ),
            ],
        )
        .expect("the closed call is recorded");
    look(&tmp, &engine, &worker, &thread, 70 + SILENCE_SECS);
    assert_eq!(worker.asked.lock().unwrap().len(), 1, "asked");
    assert_eq!(seen(&tmp, &engine, &thread, 70 + SILENCE_SECS), (true, 0));
    let mut secs = 70 + SILENCE_SECS;
    while secs <= 7_200 {
        look(&tmp, &engine, &worker, &thread, secs);
        secs += 300;
    }
    assert_eq!(worker.asked.lock().unwrap().len(), 3);
    assert_eq!(seen(&tmp, &engine, &thread, secs).1, 1, "escalated once");
}

/// **A tool call that outlives its own bound is hung** — escalated once, with no ask spent on it.
#[test]
fn engine_a_tool_call_past_its_bound_is_escalated_as_hung() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = working(&tmp, &engine, &worker);
    engine
        .transcript_append(
            &session,
            &[entry(
                "tool_use",
                60,
                Some("toolu_watch"),
                serde_json::json!({ "name": "Bash", "input": {} }),
            )],
        )
        .expect("the open call is recorded");
    look(
        &tmp,
        &engine,
        &worker,
        &thread,
        60 + TOOL_CALL_BOUND_SECS - 1,
    );
    assert_eq!(
        seen(&tmp, &engine, &thread, 60 + TOOL_CALL_BOUND_SECS - 1).1,
        0
    );
    look(&tmp, &engine, &worker, &thread, 60 + TOOL_CALL_BOUND_SECS);
    assert_eq!(seen(&tmp, &engine, &thread, 60 + TOOL_CALL_BOUND_SECS).1, 1);
    assert!(worker.asked.lock().unwrap().is_empty());
}
