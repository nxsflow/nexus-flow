//! **The ladder's calls at the `Engine` seam** — `Engine::confirm_rest` (`nxc confirm --rest`, nxf
//! 6j6v.1r61) and `Engine::session_ended_with` (`nxc session ended --cause --attempts`, nxf
//! 6j6v.558h). The products speak `Engine::*`, so the CLI and the compute-layer tests in
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
use nexus_chat::orchestration::{self, Caller, ConsequenceClass, SessionEnd, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
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
    stopped: Mutex<Vec<String>>,
    /// When set, every withdrawal of a session's asks answers this error.
    withdraw_fails: Mutex<Option<&'static str>>,
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
    fn stop_session(&self, internal_session: &str) -> Result<SessionStop, String> {
        self.stopped
            .lock()
            .unwrap()
            .push(internal_session.to_string());
        Ok(SessionStop::Requested)
    }
    fn ask_session(&self, ask: &SessionAsk) -> AskOutcome {
        if !self.session_is_running(&ask.session) {
            return AskOutcome::NoProcess;
        }
        self.asked.lock().unwrap().push(ask.clone());
        AskOutcome::Asked
    }
    fn withdraw_asks(&self, _internal_session: &str) -> Result<(), String> {
        match *self.withdraw_fails.lock().unwrap() {
            None => Ok(()),
            Some(e) => Err(e.to_string()),
        }
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

/// **`Engine::confirm_rest` with no nudge pending is refused, and nothing is recorded** (nxf
/// 6j6v.1r61): the slot's row and its log are as they were, and nothing is posted.
#[test]
fn engine_confirm_rest_with_no_nudge_pending_is_refused_and_records_nothing() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = commissioned(&engine, &worker);
    let refused = engine
        .confirm_rest(at(None, Some(&session), NOW), &thread, 5)
        .expect_err("nothing was asked");
    assert_eq!(refused.kind, nxs_foundation::error::ErrorKind::Validation);
    assert!(
        refused.msg.contains("nothing is asked of you"),
        "{}",
        refused.msg
    );
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let rows: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM recovery_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(store.messages_in_thread(&thread).unwrap().len(), 1);
}

/// **`Engine::confirm_rest` answers a nudge with a rest** (nxf 6j6v.1r61): a session silent on its
/// slot is nudged at the look ten minutes after its last entry; its confirmation of 20 minutes is
/// granted until the confirmation's instant plus 20 minutes, and `Engine::status` shows the rest.
#[test]
fn engine_confirm_rest_grants_a_rest_after_a_nudge() {
    let (tmp, engine, worker) = engine();
    let (thread, session) = working(&tmp, &engine, &worker);
    look(&tmp, &engine, &worker, &thread, 1 + SILENCE_SECS);
    assert_eq!(worker.asked.lock().unwrap().len(), 1, "nudged");
    let receipt = engine
        .confirm_rest(
            at(None, Some(&session), &later(1 + SILENCE_SECS + 30)),
            &thread,
            20,
        )
        .expect("the rest is granted");
    assert_eq!(receipt.granted_minutes, 20);
    assert_eq!(receipt.until, later(1 + SILENCE_SECS + 30 + 1_200));
    assert!(!receipt.clamped);
    let report = engine
        .status(
            Reader::Owner,
            &later(1 + SILENCE_SECS + 31),
            StatusScope::All(None),
        )
        .expect("status reads");
    let recovery = report
        .operations
        .into_iter()
        .flat_map(|o| o.threads)
        .find(|t| t.thread_id == thread)
        .and_then(|t| t.recovery)
        .expect("under recovery");
    assert_eq!(
        recovery.confirmed_until.as_deref(),
        Some(receipt.until.as_str())
    );
    assert_eq!(
        (recovery.confirmations, recovery.rest_total_secs),
        (1, 1_200)
    );
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
        liveness: Default::default(),
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

/// **A session that is really silent is nudged, stopped, and escalated when it will not stop** (nxf
/// 6j6v.1r61) — its one call returned and nothing has been heard since; the evidence arrives through
/// `Engine::transcript_append` and the outcome is read through `Engine::status`. This host's
/// process ignores both SIGTERMs, so the coordinator escalates at twice the stop's grace.
#[test]
fn engine_a_really_silent_session_is_nudged_stopped_and_escalated() {
    use nexus_chat::ladder::{NUDGE_WAIT_SECS, STOP_GRACE_SECS};
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
    let nudge = 70 + SILENCE_SECS;
    look(&tmp, &engine, &worker, &thread, nudge - 1);
    assert!(worker.asked.lock().unwrap().is_empty(), "not yet");
    look(&tmp, &engine, &worker, &thread, nudge);
    assert_eq!(worker.asked.lock().unwrap().len(), 1, "nudged");
    assert_eq!(seen(&tmp, &engine, &thread, nudge), (true, 0));
    let stop = nudge + NUDGE_WAIT_SECS;
    look(&tmp, &engine, &worker, &thread, stop);
    assert_eq!(worker.stopped.lock().unwrap().len(), 1, "stopped");
    look(&tmp, &engine, &worker, &thread, stop + STOP_GRACE_SECS);
    assert_eq!(
        worker.stopped.lock().unwrap().len(),
        2,
        "the one further SIGTERM"
    );
    look(&tmp, &engine, &worker, &thread, stop + 2 * STOP_GRACE_SECS);
    assert_eq!(
        seen(&tmp, &engine, &thread, stop + 2 * STOP_GRACE_SECS).1,
        1,
        "escalated once"
    );
    assert_eq!(worker.asked.lock().unwrap().len(), 1, "one nudge");
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

/// **`Engine::reply_thread` when the nudges cannot be withdrawn** (review of pass 2, TQ #1): one
/// session serves two slots, both nudged, and the worker cannot unlink its ask files. The reply on
/// the first is posted all the same and its owed turn is over; the receipt carries the failed
/// withdrawal as a `RecoveryFailed` warning, and the other slot's nudge row is cleared.
#[test]
fn engine_a_reply_whose_withdrawal_fails_is_posted_warns_and_clears_the_other_nudge() {
    let (tmp, engine, worker) = engine();
    let (x, session) = working(&tmp, &engine, &worker);
    let y = engine
        .send_to(
            at(Some("carsten"), None, NOW),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "And a --until flag.",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the second commission")
        .thread_id;
    let store = || {
        Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap()
    };
    store()
        .connection()
        .execute(
            "UPDATE owed_starts SET session = ?1 WHERE thread = ?2",
            [&session, &y],
        )
        .unwrap();
    let asked_at = |thread: &str| -> Option<String> {
        store()
            .connection()
            .query_row(
                "SELECT asked_at FROM owed_starts WHERE thread = ?1",
                [thread],
                |r| r.get(0),
            )
            .unwrap()
    };
    let nudge = 1 + SILENCE_SECS;
    look(&tmp, &engine, &worker, &x, nudge);
    look(&tmp, &engine, &worker, &y, nudge);
    assert_eq!(worker.asked.lock().unwrap().len(), 2, "both slots nudged");
    assert!(asked_at(&x).is_some() && asked_at(&y).is_some());
    *worker.withdraw_fails.lock().unwrap() = Some("the ask file could not be unlinked (injected)");

    let receipt = engine
        .reply_thread(
            at(None, Some(&session), &later(nudge + 30)),
            ReplyThreadRequest {
                machine: None,
                thread: &x,
                body: "done: --since is in",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is admitted");

    assert!(receipt.posted, "the reply is posted");
    let warning = receipt
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::RecoveryFailed)
        .unwrap_or_else(|| panic!("a RecoveryFailed warning: {:?}", receipt.warnings));
    assert_eq!(warning.thread.as_deref(), Some(x.as_str()));
    assert!(
        warning.detail.contains("could not be unlinked (injected)"),
        "{}",
        warning.detail
    );
    assert_eq!(asked_at(&x), None, "the first slot's owed turn is over");
    assert_eq!(asked_at(&y), None, "the other slot's nudge row is cleared");
}
