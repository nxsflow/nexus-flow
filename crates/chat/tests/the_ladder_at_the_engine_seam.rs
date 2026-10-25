//! **The ladder's two new calls, at the `Engine` seam** (nxf 6j6v.558h) — `Engine::reply_progress`
//! (`nxc reply --progress`) and `Engine::session_ended_with` (`nxc session ended --cause
//! --attempts`). The products speak `Engine::*`, so the CLI and the compute-layer tests in
//! `the_ladder.rs` do not count for these two.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{Caller, SessionEnd};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest, PROGRESS_UNREQUESTED};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{
    SessionStop, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-25T09:00:00Z";

#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
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
