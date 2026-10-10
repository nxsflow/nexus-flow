//! **A hand-off starts one step per ordered run, and every member of a parallel round** (nxf
//! 6j6v.9g8j).
//!
//! Measured in the `agents` workspace on 2026-10-10 at 01:09:28Z, running 0.206.1. Another
//! operation's working-copy lease ran out, and the hand-off started BOTH queued entries of the
//! waiting operation at once:
//!
//! ```text
//! started coder on thread m-01M4GVQA9QA1…
//! started verifier on thread m-01M4HGAGF2Q8WJA5N8X3GNVKV0
//! ```
//!
//! Those were two SEQUENTIAL steps of one ordered coding channel — build, then verify. The coder
//! switched branches mid-run and the verifier's gate run was spoiled. The hand-off took the whole
//! claim area off the queue and fired it, which is right for the members of a parallel fan-out and
//! wrong for the steps of an ordered run.
//!
//! How two steps of one run came to be queued together in the field is a different bug (nxf
//! 6j6v.1wep): the first step's answer window ran while it waited for the copy, it lapsed, and the
//! flow moved on to the second step, which queued behind the same lease. Since 1wep a queued step
//! no longer lapses (`a_queued_step_is_not_asked_until_it_starts.rs`), so these tests make the same
//! shape another way: an answer is posted under the queued coder's session before it ever started,
//! which settles the step and opens the next. What they pin is what the HAND-OFF does with two
//! steps of one run in line together, whichever way they got there.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`), with a [`WorkerConfig::Custom`]
//! worker that records every start and answers which of its sessions still run. The tick goes
//! through the compute layer, because it is the background service's verb rather than one on the
//! handle.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{self, Caller, TickReceipt, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-10T01:00:00Z";
/// The queued coder's step is answered, and the verifier is opened behind it.
const LAPSED: &str = "2026-10-10T01:05:00Z";
/// The holder lets go, inside the verifier's own window (opened at [`LAPSED`] with one minute).
const LATER: &str = "2026-10-10T01:05:30Z";
/// Still inside both windows: the verifier's, and the coder's, restarted when it started.
const LATER_STILL: &str = "2026-10-10T01:05:50Z";

/// The operation that holds the working copy: one step, no window of its own.
const HOLDER: &str =
    "name: holding\nmembers: [builder]\nflow: sequential\nworking_tree: exclusive\n";

/// The measured incident's shape: build, then verify, in one ordered run that needs the copy alone.
const ORDERED: &str = "name: coding\nmembers: [coder, verifier]\nflow: sequential\nworking_tree: exclusive\ntimeout: 1m\n";

/// The same two roles asked at once — a declared parallel round, still claiming the copy.
const PARALLEL: &str =
    "name: coding\nmembers: [coder, verifier]\nflow: parallel\nworking_tree: exclusive\n";

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

fn team(coding: &str) -> (TempDir, Engine, Arc<RecordingWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [HOLDER, coding]
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

fn answer(engine: &Engine, session: &str, thread: &str, now: &str, body: &str) {
    engine
        .reply_thread(
            as_session(session, now),
            ReplyThreadRequest {
                machine: None,
                thread,
                body,
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
}

fn store(tmp: &TempDir) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The roles waiting in the queue for operation `root`, in take order.
fn queued_for(tmp: &TempDir, root: &str) -> Vec<String> {
    let s = store(tmp);
    s.list_working_tree_queue()
        .unwrap()
        .into_iter()
        .filter(|q| {
            q.thread
                .as_deref()
                .is_some_and(|t| s.thread_root(t).unwrap() == root)
        })
        .map(|q| q.role)
        .collect()
}

/// The tick the background service runs, through the compute layer.
fn tick(tmp: &TempDir, worker: &RecordingWorker, now: &str, thread_id: &str) -> TickReceipt {
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(tmp.path()).expect("declarations");
    let ctx = orchestration::Ctx {
        now,
        origin: &origin,
        actor: "service",
        session: None,
        hop: 0,
        defs: &defs,
        worker,
        timer: &DryTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id }).expect("tick")
}

/// The first operation holds the working copy with its builder at work. Returns the builder's
/// session and the thread it answers on.
fn a_holder_at_work(engine: &Engine, worker: &RecordingWorker) -> (String, String) {
    commission(engine, caller("pm", NOW), "holding", "build T5");
    let builder = worker.last_for("builder");
    (builder.internal_session, builder.reply_thread.unwrap())
}

/// The holder finishes: the builder answers and its session ends, which is what lets the copy go.
fn the_holder_lets_go(engine: &Engine, worker: &RecordingWorker, session: &str, thread: &str) {
    answer(engine, session, thread, LATER, "built");
    worker.mark_gone(session);
    engine
        .session_ended(caller("builder", LATER), session)
        .expect("the session end is accepted");
}

// ---- the incident: two steps of one ordered run queued together ---------------------------------

/// The ordered round queued behind the holder with BOTH of its steps waiting — the field's shape.
/// Returns the round's root and the coder's slot.
fn an_ordered_round_queued_with_both_steps(
    tmp: &TempDir,
    engine: &Engine,
    worker: &RecordingWorker,
) -> (String, String) {
    let root = commission(engine, caller("pm2", NOW), "coding", "build T6");
    assert_eq!(
        queued_for(tmp, &root),
        vec!["coder".to_string()],
        "the premise: the round's first step waits for the working copy"
    );
    // The coder's step is settled while its trigger still waits, and the flow moves on to the
    // verifier — which queues behind the same lease. In the field that was the coder's window
    // lapsing in the queue (nxf 6j6v.1wep). A queued step no longer lapses, so the shape is made
    // here by an answer posted under the queued coder's session before it ever started.
    let coder = store(tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .find(|q| q.role == "coder")
        .expect("the coder is queued");
    let coder_slot = coder.thread.expect("the queued coder names its slot");
    answer(
        engine,
        &coder.session,
        &coder_slot,
        LAPSED,
        "built T6 by hand",
    );
    assert_eq!(
        queued_for(tmp, &root),
        vec!["coder".to_string(), "verifier".to_string()],
        "the premise: both steps of the one run are waiting in the queue"
    );
    assert_eq!(worker.started("coder") + worker.started("verifier"), 0);
    (root, coder_slot)
}

#[test]
fn the_hand_off_starts_the_first_step_of_an_ordered_run_and_not_the_one_after_it() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, _) = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);

    assert_eq!(
        worker.started("coder"),
        1,
        "the hand-off starts the run's first step"
    );
    assert_eq!(
        worker.started("verifier"),
        0,
        "and NOT the step after it: two sessions of one ordered run in one working copy is the \
         collision the lease exists to prevent"
    );
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["verifier".to_string()],
        "the later step is still waiting, not dropped"
    );
    // The lease ROW, not `working_tree_holder`: what this test pins is WHO the copy was handed to.
    // That the bound it was handed with has not already passed is
    // `a_queued_step_is_not_asked_until_it_starts.rs`'s to pin (nxf 6j6v.1wep).
    let (holder, _) = store(&tmp)
        .working_tree_lease_row()
        .unwrap()
        .expect("the copy was handed on, not freed");
    assert!(
        holder.ends_with(&round),
        "the working copy is the waiting round's now: {holder}"
    );
}

#[test]
fn the_later_step_waits_while_the_earlier_one_is_still_writing_and_starts_when_it_ends() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, coder_slot) = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    // Its answer is already in its slot, so the start carries no obligation to reply.
    let (cs, ct) = (coder.internal_session, coder_slot);

    // Something asks while the coder is still at work: the verifier must still wait.
    tick(&tmp, &worker, LATER_STILL, &ct);
    assert_eq!(
        worker.started("verifier"),
        0,
        "the coder's process is still in the working copy"
    );

    worker.mark_gone(&cs);
    engine
        .session_ended(caller("coder", LATER_STILL), &cs)
        .expect("the session end is accepted");

    assert_eq!(
        worker.started("verifier"),
        1,
        "once the earlier step's session is over, the waiting step starts: it is not stranded"
    );
    assert!(
        queued_for(&tmp, &round).is_empty(),
        "and nothing of the round is left in the queue"
    );
}

#[test]
fn a_tick_starts_the_later_step_when_the_earlier_one_died_without_announcing_its_end() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (_, coder_slot) = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    // Its answer is already in its slot, so the start carries no obligation to reply.
    let (cs, ct) = (coder.internal_session, coder_slot);

    // The coder's process dies hard: no `session ended`, only the tick can notice.
    worker.mark_gone(&cs);
    tick(&tmp, &worker, LATER_STILL, &ct);

    assert_eq!(
        worker.started("verifier"),
        1,
        "the tick asks what the session end would have asked"
    );
}

/// The coder answers again, into the slot the flow had already counted as settled, and then ends.
/// The step that starts is the verifier that was waiting, once: the answer does not open a second
/// verifier slot beside it.
///
/// (In the field the verifier's window could ALSO lapse in the queue, and the flow would then open
/// the verifier step a second time on that answer. Since nxf 6j6v.1wep a queued step does not
/// lapse, so that second slot is no longer opened that way.)
#[test]
fn an_answered_earlier_step_starts_the_waiting_step_once() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, coder_slot) = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    let waiting_slot = store(&tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .find(|q| q.role == "verifier")
        .and_then(|q| q.thread)
        .expect("the queued verifier names its slot");
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    // Its answer is already in its slot, so the start carries no obligation to reply.
    let (cs, ct) = (coder.internal_session, coder_slot);

    answer(&engine, &cs, &ct, LATER_STILL, "built T6");
    assert_eq!(
        worker.started("verifier"),
        0,
        "an answer is a message; the coder's process is still in the working copy"
    );
    worker.mark_gone(&cs);
    engine
        .session_ended(caller("coder", LATER_STILL), &cs)
        .expect("the session end is accepted");

    assert_eq!(
        worker.started("verifier"),
        1,
        "exactly one verifier runs once the coder is over"
    );
    assert_eq!(
        worker.last_for("verifier").reply_thread.as_deref(),
        Some(waiting_slot.as_str()),
        "and it is the step that was waiting in line"
    );
    assert!(queued_for(&tmp, &round).is_empty());
}

// ---- what must NOT change: a declared parallel round starts together ----------------------------

#[test]
fn the_hand_off_still_starts_every_member_of_a_parallel_round_together() {
    let (tmp, engine, worker) = team(PARALLEL);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round = commission(&engine, caller("pm2", NOW), "coding", "review T6");
    assert_eq!(
        queued_for(&tmp, &round).len(),
        2,
        "the premise: both members of the round wait for the working copy"
    );

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);

    assert_eq!(
        (worker.started("coder"), worker.started("verifier")),
        (1, 1),
        "a parallel round was declared to run side by side, and the hand-off honours that"
    );
    assert!(queued_for(&tmp, &round).is_empty());
}
