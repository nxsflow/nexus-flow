//! **An escalation answered above it lets the working copy go** (nxf 6j6v.ys54).
//!
//! Measured in the `agents` workspace on 2026-10-10, running 0.206.1. An operation delivered
//! cleanly — the finisher opened a PR, which was merged, and every session ended — and still held
//! the working-copy lease until its bound, with the next operation waiting behind it. The release
//! question was asked, at the finisher's reply and at its session end, and it said no without
//! saying why.
//!
//! The no came from one thread: a verify step of the ordered run had escalated. The escalation WAS
//! answered, in the normal way — the requester replied on the thread above it, and the run started
//! again in new sibling step threads, which delivered. Nothing was ever written again in the
//! escalating step's thread, so its escalation stayed its last reply, and the release rule read it
//! as unanswered for ever.
//!
//! The rule these tests pin: **a hand-back is answered once a thread above it was asked again
//! after it**, and an escalation that nothing above it answered still holds the copy, as before.
//! When the copy is held for a hand-back, `nxc status` says which thread holds it.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`), with a [`WorkerConfig::Custom`]
//! worker that records every start and answers which of its sessions still run, as
//! `a_queued_step_is_not_asked_until_it_starts.rs` does.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{StatusOperation, StatusScope};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-10T01:00:00Z";
/// The coder answers, and the verify step opens.
const BUILT: &str = "2026-10-10T01:05:00Z";
/// The verifier escalates: its tree collided with somebody else's.
const ESCALATED: &str = "2026-10-10T01:10:00Z";
/// The requester answers the escalation on the thread above it, and the run starts again.
const ANSWERED: &str = "2026-10-10T01:15:00Z";
/// The second coder answers.
const REBUILT: &str = "2026-10-10T01:20:00Z";
/// The second verifier answers: the operation has delivered.
const DELIVERED: &str = "2026-10-10T01:25:00Z";

/// The operation waiting for the working copy: one step.
const WAITING: &str =
    "name: holding\nmembers: [builder]\nflow: sequential\nworking_tree: exclusive\n";

/// The measured incident's shape: build, then verify, in one ordered run that needs the copy alone.
/// No window of its own, so its lease gets the two-hour fallback and no bound can move the copy
/// inside these tests.
const ORDERED: &str =
    "name: coding\nmembers: [coder, verifier]\nflow: sequential\nworking_tree: exclusive\n";

/// A worker that records every start it accepted and answers which sessions are still running.
#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl RecordingWorker {
    fn started(&self, handle: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .count()
    }

    fn last_for(&self, handle: &str) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone()
    }

    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        // A started session is running until the test says otherwise, as a real process would be.
        self.running
            .lock()
            .unwrap()
            .insert(req.internal_session.clone());
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }

    fn answers_liveness(&self) -> bool {
        true
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn team() -> (TempDir, Engine, Arc<RecordingWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [WAITING, ORDERED]
        .iter()
        .map(|y| serde_yaml::from_str(y).expect("channel parses"))
        .collect();
    let defs = Definitions::new(
        vec![role("builder"), role("coder"), role("verifier")],
        channels,
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
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

fn caller<'a>(actor: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(now),
    }
}

fn as_session<'a>(session: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(now),
    }
}

fn commission(engine: &Engine, who: Caller<'_>, to: &str, body: &str) -> String {
    engine
        .send_to(
            who,
            SendToRequest {
                machine: None,
                to,
                body,
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission is accepted")
        .thread_id
}

fn post(engine: &Engine, who: Caller<'_>, thread: &str, body: &str, escalate: bool) {
    engine
        .reply_thread(
            who,
            ReplyThreadRequest {
                machine: None,
                thread,
                body,
                escalate,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
}

/// The latest step of `handle` answers at `at` — escalating or not — and its session ends, as a
/// finished agent's does. Returns the thread the step answered on.
fn the_step_ends(
    engine: &Engine,
    worker: &RecordingWorker,
    handle: &str,
    at: &str,
    body: &str,
    escalate: bool,
) -> String {
    let step = worker.last_for(handle);
    let thread = step.reply_thread.expect("the step answers on a thread");
    post(
        engine,
        as_session(&step.internal_session, at),
        &thread,
        body,
        escalate,
    );
    worker.mark_gone(&step.internal_session);
    engine
        .session_ended(caller(handle, at), &step.internal_session)
        .expect("the session end is accepted");
    thread
}

fn store(tmp: &TempDir) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn holder(tmp: &TempDir) -> Option<String> {
    store(tmp)
        .working_tree_lease_row()
        .unwrap()
        .map(|(holder, _)| holder)
}

fn operation(engine: &Engine, root: &str, at: &str) -> StatusOperation {
    let ids = [root];
    engine
        .status(at, StatusScope::Threads(&ids))
        .expect("status")
        .operations
        .into_iter()
        .find(|op| op.root == root)
        .expect("the operation is reported")
}

/// The ordered run starts on the free copy, and a second operation queues behind it. Returns the
/// run's root.
fn a_run_with_a_round_waiting_behind_it(engine: &Engine, worker: &RecordingWorker) -> String {
    let root = commission(engine, caller("pm", NOW), "coding", "build T6");
    assert_eq!(worker.started("coder"), 1, "the premise: the coder started");
    commission(engine, caller("pm2", NOW), "holding", "build T5");
    assert_eq!(
        worker.started("builder"),
        0,
        "the premise: the second operation waits for the copy"
    );
    root
}

/// The coder builds, and the verifier escalates. Returns the escalating thread.
fn the_verifier_escalates(engine: &Engine, worker: &RecordingWorker) -> String {
    the_step_ends(engine, worker, "coder", BUILT, "built", false);
    assert_eq!(worker.started("verifier"), 1, "the verify step opened");
    the_step_ends(
        engine,
        worker,
        "verifier",
        ESCALATED,
        "collision: somebody else's checkout is in my tree",
        true,
    )
}

// ---- the incident ----------------------------------------------------------------------------

#[test]
fn an_escalation_answered_on_the_thread_above_it_lets_the_copy_go_once_the_redo_delivers() {
    let (tmp, engine, worker) = team();
    let root = a_run_with_a_round_waiting_behind_it(&engine, &worker);
    let escalating = the_verifier_escalates(&engine, &worker);
    assert_eq!(
        worker.started("builder"),
        0,
        "the premise: an unanswered escalation holds the copy"
    );

    // The answer, the normal way: the requester replies on the thread above the escalation, and
    // the run starts again in new step threads.
    post(&engine, caller("pm", ANSWERED), &root, "redo it", false);
    assert_eq!(worker.started("coder"), 2, "the run started again");
    the_step_ends(&engine, &worker, "coder", REBUILT, "built again", false);
    let redone = the_step_ends(&engine, &worker, "verifier", DELIVERED, "verified", false);
    assert_ne!(
        redone, escalating,
        "the premise: the redo ran in a new thread, so nothing was written in the escalating one"
    );
    assert_eq!(
        store(&tmp).last_reply_kind(&escalating).unwrap().as_deref(),
        Some("escalation"),
        "the premise: the escalation is still the last reply of its own thread"
    );

    assert_eq!(
        worker.started("builder"),
        1,
        "the operation delivered and every session ended: the copy goes to the round waiting \
         behind it"
    );
    let now_held_by = holder(&tmp).expect("the copy was handed on, not freed");
    assert!(
        !now_held_by.ends_with(&root),
        "the delivered operation no longer holds the copy: {now_held_by}"
    );
    let op = operation(&engine, &root, DELIVERED);
    assert!(!op.holds_working_tree, "{op:?}");
    assert_eq!(op.held_by_hand_back, None, "{op:?}");
}

// ---- what must NOT change: an escalation nobody answered -----------------------------------------

#[test]
fn an_escalation_with_nothing_after_it_still_holds_the_copy() {
    let (tmp, engine, worker) = team();
    let root = a_run_with_a_round_waiting_behind_it(&engine, &worker);
    the_verifier_escalates(&engine, &worker);

    assert_eq!(
        worker.started("builder"),
        0,
        "nobody answered the escalation: the task is mid-flight and the copy stays with it"
    );
    assert!(
        holder(&tmp).is_some_and(|h| h.ends_with(&root)),
        "the escalating operation holds the copy: {:?}",
        holder(&tmp)
    );
}

// ---- the reason for the no ----------------------------------------------------------------------

#[test]
fn status_names_the_hand_back_that_keeps_the_copy() {
    let (_tmp, engine, worker) = team();
    let root = a_run_with_a_round_waiting_behind_it(&engine, &worker);

    // At work, the copy is held for the running step, and no hand-back is named.
    let op = operation(&engine, &root, NOW);
    assert!(op.holds_working_tree, "{op:?}");
    assert_eq!(op.held_by_hand_back, None, "{op:?}");

    // The escalation stands in the step's thread and, passed through, on the root above it. Either
    // is the reason; the read names the first it meets in the tree's order, which is the root.
    let escalating = the_verifier_escalates(&engine, &worker);
    let op = operation(&engine, &root, ESCALATED);
    assert!(op.holds_working_tree, "{op:?}");
    assert_eq!(
        op.open, 0,
        "the premise: nothing in the operation is open: {op:?}"
    );
    let named = op
        .held_by_hand_back
        .clone()
        .expect("a copy held with nothing open says which hand-back holds it");
    assert!(
        [root.as_str(), escalating.as_str()].contains(&named.as_str()),
        "it names a thread the escalation stands in: {named} ({op:?})"
    );
    assert!(
        op.threads
            .iter()
            .any(|t| t.thread_id == named && t.escalated),
        "and that thread shows the escalation: {op:?}"
    );
    let json = serde_json::to_value(&op).unwrap();
    assert_eq!(
        json["held_by_hand_back"].as_str(),
        Some(named.as_str()),
        "and `--json` carries it: {json}"
    );

    // Answered, redone and delivered: the copy goes, and nothing is named any more.
    post(&engine, caller("pm", ANSWERED), &root, "redo it", false);
    let op = operation(&engine, &root, ANSWERED);
    assert_eq!(
        op.held_by_hand_back, None,
        "a new turn is running; the copy is held for it, not for the hand-back: {op:?}"
    );
}
