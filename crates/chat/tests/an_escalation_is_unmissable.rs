//! **An escalation must be unmissable where it ARRIVES** (nxf 6j6v.gk9j).
//!
//! The signal itself is not in doubt: `--escalate` is a declared bit, the supervisor branches on it,
//! and `nxc status` has carried `escalated: true` on the thread since 0.63.0. What the proving
//! ground (4jgn.g90w) measured is that none of that reaches the party that has to act. Two
//! escalations sat unanswered for over an hour while they held the working copy and stopped the
//! whole chain, because the recipient — a PM persona — did not recognise them: **an agent does not
//! run `nxc status`, it reads the message that woke it**, and in that message a hand-back looked
//! exactly like a result. It happened to the human reviewing that run too, with better tools in
//! hand: `state` and the process list were checked and `escalated` was missed.
//!
//! So this suite pins two things, which are the item's own acceptance:
//!
//! 1. the woken session meets the escalation in the text it reads FIRST — before the answer, not
//!    beside it — together with what it costs while it stands and what is expected in return;
//! 2. at the ROOT of an operation, "finished, go and read it" is distinguishable from "somebody is
//!    waiting on your decision". Both render as `awaiting you` on the root thread and always did.
//!
//! Driven through the library handle, because the message a resumed session is handed exists
//! nowhere else: it is the trigger's own text, and only a worker sees it.

mod common;

use std::sync::Mutex;

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::StatusScope;
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::sync::Arc;
use tempfile::TempDir;

const NOW: &str = "2026-08-24T10:00:00Z";

#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RecordingWorker {
    /// The message the LAST trigger handed its session — what the model actually reads.
    fn last_message(&self) -> String {
        self.seen
            .lock()
            .unwrap()
            .last()
            .map(|r| r.message.clone())
            .expect("at least one trigger")
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

/// A workspace with `pm` and `coder` declared, a live `pm` session in the map to act from, and the
/// handle over it.
fn team() -> (TempDir, Engine, Arc<RecordingWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("pm"), role("coder")], vec![]).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    {
        let mut store = Workspace::resolve(None, tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store");
        // The PM has to be a REAL session, not a human at a keyboard: the escalation travels back
        // through the thread's return address, and a human leaves none.
        store.create_pending_session("s-pm", "pm").unwrap();
        store.bind_session("s-pm", "real-pm").unwrap();
    }
    let worker = Arc::new(RecordingWorker::default());
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

fn as_session<'a>(session: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

/// The PM commissions the coder out of its own session; hand back (the coder's session, its thread).
fn commission(engine: &Engine, worker: &RecordingWorker) -> (String, String) {
    let receipt = engine
        .send_to(
            as_session("s-pm"),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "run the migration",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the coder is summoned");
    let _ = worker;
    (
        receipt.session.expect("a persona summon mints a session"),
        receipt.thread_id,
    )
}

// ---- (1) the text the woken session reads first ------------------------------------------------

#[test]
fn a_hand_back_announces_itself_before_the_answer_the_requester_is_woken_with() {
    let (_tmp, engine, worker) = team();
    let (coder_session, thread) = commission(&engine, &worker);

    engine
        .reply_thread(
            as_session(&coder_session),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "The migration needs a production credential I do not have.",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the escalation is posted");

    let woke_with = worker.last_message();
    assert!(
        woke_with.starts_with("ESCALATION"),
        "the requester must meet the hand-back in the first thing it reads, not infer it from \
         prose further down, got:\n{woke_with}"
    );
    assert!(
        woke_with.contains("working copy"),
        "…and be told what it costs while it stands — an unanswered escalation holds the \
         checkout, which is what made the measured hour expensive, got:\n{woke_with}"
    );
    assert!(
        woke_with.contains("Decide now"),
        "…and what is expected of it, got:\n{woke_with}"
    );
    assert!(
        woke_with.contains(&thread),
        "…and where to read what was actually said — this path hands over a POINTER rather than \
         the reply text, which is how the M2 quorum wake has always worked and is not this item's \
         to change, got:\n{woke_with}"
    );
    assert!(
        !woke_with.contains("act on the results"),
        "and NOT the ordinary wording, which tells the reader to act on results that do not \
         exist — a notice bolted onto a message that contradicts itself one sentence later would \
         be a worse read than the one this item is fixing, got:\n{woke_with}"
    );
}

#[test]
fn an_ordinary_answer_is_handed_over_exactly_as_it_was_before_this_item() {
    // The control, and the reason it matters: this notice rides on every 1:1 wake there is, so a
    // finished round must not grow a paragraph telling its requester that nothing is a result.
    let (_tmp, engine, worker) = team();
    let (coder_session, thread) = commission(&engine, &worker);

    engine
        .reply_thread(
            as_session(&coder_session),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "The migration ran; 412 rows moved.",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the answer is posted");

    let woke_with = worker.last_message();
    assert_eq!(
        woke_with,
        format!(
            "Review quorum for thread {thread} is complete (1/1 replied). Read the results via \
             `nxc threads show {thread}`, then act on the results and reply as appropriate."
        ),
        "byte-identical to what this path said before nxf 6j6v.gk9j — it closes every \
         `send --to <persona>` there is, so a finished round must not grow a paragraph"
    );
}

// ---- (2) the root of an operation --------------------------------------------------------------

#[test]
fn an_operation_with_an_unanswered_hand_back_under_it_says_a_decision_is_needed() {
    let (tmp, engine, worker) = team();
    let (coder_session, thread) = commission(&engine, &worker);
    engine
        .reply_thread(
            as_session(&coder_session),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "I cannot.",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap();

    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let one = [thread.as_str()];
    let report = nexus_chat::facade::status(
        &store,
        &nexus_chat::worker::DryWorker { log: None },
        NOW,
        StatusScope::Threads(&one),
    )
    .unwrap();
    let op = report.operations.first().expect("one operation");

    assert!(
        op.needs_decision,
        "the root of an operation with a standing hand-back under it must say so — otherwise it \
         reads exactly like a finished one"
    );
}

#[test]
fn a_finished_operation_needs_no_decision() {
    // The distinction is the whole point: without this the flag would be true of every operation
    // that ever ends, which is the same as being true of none.
    let (tmp, engine, worker) = team();
    let (coder_session, thread) = commission(&engine, &worker);
    engine
        .reply_thread(
            as_session(&coder_session),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap();

    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let one = [thread.as_str()];
    let report = nexus_chat::facade::status(
        &store,
        &nexus_chat::worker::DryWorker { log: None },
        NOW,
        StatusScope::Threads(&one),
    )
    .unwrap();
    let op = report.operations.first().expect("one operation");

    assert!(!op.needs_decision);
    assert!(
        !op.holds_working_tree,
        "nothing here declared it needs the checkout to itself"
    );
}
