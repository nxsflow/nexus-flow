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
//! How two steps of one run come to be queued together at all is a different bug (nxf 6j6v.1wep):
//! the first step's answer window runs while it waits for the copy, it lapses, and the flow moves
//! on to the second step, which queues behind the same lease. These tests reproduce that shape on
//! purpose, because it is the shape the field produced; what they pin is what the HAND-OFF does
//! with it.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`), with a [`WorkerConfig::Custom`]
//! worker that records every start, answers which of its sessions still run (or, for one test,
//! declines to), and can be told to fail a coder's starts. The tick goes through the compute
//! layer, because it is the background service's verb rather than one on the handle.

mod common;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::{ErrorKind, NxfError};
use nexus_chat::orchestration::{self, Caller, Promotions, TickReceipt, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
use nexus_chat::worker::{
    TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-10T01:00:00Z";
/// Past the one-minute window the waiting round's first step was given.
const LAPSED: &str = "2026-10-10T01:05:00Z";
/// The holder lets go, inside the verifier's own window (opened at [`LAPSED`] with one minute), so
/// the only lapse in play is the coder's.
const LATER: &str = "2026-10-10T01:05:30Z";
/// Still inside both windows: the verifier's, and the coder's, restarted when it started.
const LATER_STILL: &str = "2026-10-10T01:05:50Z";
/// Past the coder's restarted window (it started at [`LATER`] with one minute), and past the retry
/// instant of a start that failed at [`LATER`].
const CODER_LAPSED: &str = "2026-10-10T01:06:31Z";
/// Past the holder's own two-hour bound: it declares no window, so its lease runs to the fallback.
const HOLDER_EXPIRED: &str = "2026-10-10T03:01:00Z";
/// Past the window the coder was given when the sweep started it at [`HOLDER_EXPIRED`].
const AFTER_THE_SWEPT_CODER: &str = "2026-10-10T03:03:00Z";

/// The operation that holds the working copy: one step, no window of its own.
const HOLDER: &str =
    "name: holding\nmembers: [builder]\nflow: sequential\nworking_tree: exclusive\n";

/// The measured incident's shape: build, then verify, in one ordered run that needs the copy alone.
const ORDERED: &str = "name: coding\nmembers: [coder, verifier]\nflow: sequential\nworking_tree: exclusive\ntimeout: 1m\n";

/// The same order, declared as a state machine (`steps:`) rather than as `flow: sequential`.
const STEPPED: &str = "name: coding\nmembers: [coder, verifier]\nworking_tree: exclusive\ntimeout: 1m\nsteps:\n  - id: build\n    target: coder\n    next: verify\n  - id: verify\n    target: verifier\n";

/// An ordered run whose second step is a whole parallel round.
const ORDERED_WITH_A_ROUND: &str = "name: coding\nmembers: [coder, review]\nflow: sequential\nworking_tree: exclusive\ntimeout: 1m\n";
const ROUND: &str = "name: review\nmembers: [r1, r2]\nworking_tree: exclusive\n";

/// The same two roles asked at once — a declared parallel round, still claiming the copy.
const PARALLEL: &str =
    "name: coding\nmembers: [coder, verifier]\nflow: parallel\nworking_tree: exclusive\n";

/// A worker that records every start it accepted and answers which sessions are still running.
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    /// Whether it answers the liveness question at all. One that does not says "not running" for
    /// everything, which is the trait's default.
    answers: bool,
    /// How many more starts of a CODER fail with a transient (`io`) error.
    failing_coders: Mutex<u32>,
    /// Run once while a coder's start is IN PROGRESS: after the engine asked for it and before its
    /// process exists — the window in which another process can act on the same workspace.
    meanwhile: Mutex<Option<Meanwhile>>,
}

type Meanwhile = Box<dyn FnOnce(&RecordingWorker) + Send>;

impl RecordingWorker {
    fn new(answers: bool) -> Self {
        RecordingWorker {
            seen: Mutex::new(Vec::new()),
            running: Mutex::new(HashSet::new()),
            answers,
            failing_coders: Mutex::new(0),
            meanwhile: Mutex::new(None),
        }
    }

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

    fn fail_the_next_coder_start(&self) {
        *self.failing_coders.lock().unwrap() = 1;
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let mut failing = self.failing_coders.lock().unwrap();
        if *failing > 0 && req.role.handle == "coder" {
            *failing -= 1;
            return Err(TriggerError::Failed(NxfError::new(
                ErrorKind::Io,
                "`nxf prime` exceeded the 30s wall-clock timeout",
            )));
        }
        drop(failing);
        if req.role.handle == "coder" {
            let meanwhile = self.meanwhile.lock().unwrap().take();
            if let Some(meanwhile) = meanwhile {
                meanwhile(self);
            }
        }
        // A started session is running until the test says otherwise, as a real process would be.
        self.running
            .lock()
            .unwrap()
            .insert(req.internal_session.clone());
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.answers && self.running.lock().unwrap().contains(internal_session)
    }

    fn answers_liveness(&self) -> bool {
        self.answers
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn team(coding: &str) -> (TempDir, Engine, Arc<RecordingWorker>) {
    team_with(&[coding], true)
}

fn team_with(channels: &[&str], answers: bool) -> (TempDir, Engine, Arc<RecordingWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = std::iter::once(&HOLDER)
        .chain(channels)
        .map(|y| serde_yaml::from_str(y).expect("channel parses"))
        .collect();
    let roles = ["builder", "coder", "verifier", "r1", "r2"]
        .into_iter()
        .map(role)
        .collect();
    let defs = Definitions::new(roles, channels).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(RecordingWorker::new(answers));
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

fn end(engine: &Engine, worker: &RecordingWorker, actor: &str, session: &str, now: &str) {
    worker.mark_gone(session);
    engine
        .session_ended(caller(actor, now), session)
        .expect("the session end is accepted");
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

/// The queue row of `role`: its slot and the instant it first joined the queue.
fn queued_row(tmp: &TempDir, role: &str) -> (String, String) {
    store(tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .find(|q| q.role == role)
        .map(|q| (q.thread.unwrap(), q.enqueued_at.unwrap()))
        .unwrap_or_else(|| panic!("{role} is queued"))
}

/// The tick the background service runs, through the compute layer.
fn tick(tmp: &TempDir, worker: &RecordingWorker, now: &str, thread_id: &str) -> TickReceipt {
    tick_in(tmp.path(), worker, now, thread_id)
}

/// [`tick`] by the workspace's path, for a tick run from inside the worker (a second process).
fn tick_in(root: &Path, worker: &RecordingWorker, now: &str, thread_id: &str) -> TickReceipt {
    let ws = Workspace::resolve(None, root).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(root).expect("declarations");
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

/// What a tick's hand-off of the working copy promoted, whichever movement carried it.
fn tick_promotions(receipt: &TickReceipt) -> Option<&Promotions> {
    receipt
        .working_tree
        .as_ref()
        .map(|s| &s.promotions)
        .or(receipt.handed_on.as_ref().map(|h| &h.promotions))
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
    end(engine, worker, "builder", session, LATER);
}

// ---- the incident: two steps of one ordered run queued together ---------------------------------

/// The ordered round queued behind the holder with BOTH of its steps waiting — the field's shape.
/// `waiting` is what the queue holds for the round once the flow has moved past its lapsed first
/// step. Returns the round's root.
fn a_round_queued_past_its_lapsed_first_step(
    tmp: &TempDir,
    worker: &RecordingWorker,
    engine: &Engine,
    waiting: &[&str],
) -> String {
    let root = commission(engine, caller("pm2", NOW), "coding", "build T6");
    assert_eq!(
        queued_for(tmp, &root),
        vec!["coder".to_string()],
        "the premise: the round's first step waits for the working copy"
    );
    // The coder's window lapses while it waits (nxf 6j6v.1wep), and the flow moves on to the next
    // step, which queues behind the same lease.
    let (coder_slot, _) = queued_row(tmp, "coder");
    let channel_thread = store(tmp)
        .thread_parent(&coder_slot)
        .unwrap()
        .expect("the slot hangs under its channel thread");
    tick(tmp, worker, LAPSED, &channel_thread);
    assert_eq!(
        queued_for(tmp, &root),
        waiting.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "the premise: every step that has been opened is waiting in the queue"
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 1, "only the holder runs");
    root
}

fn an_ordered_round_queued_with_both_steps(
    tmp: &TempDir,
    engine: &Engine,
    worker: &RecordingWorker,
) -> String {
    a_round_queued_past_its_lapsed_first_step(tmp, worker, engine, &["coder", "verifier"])
}

#[test]
fn the_hand_off_starts_the_first_step_of_an_ordered_run_and_not_the_one_after_it() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    let (_, waited_since) = queued_row(&tmp, "verifier");

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
    assert_eq!(
        queued_row(&tmp, "verifier").1,
        waited_since,
        "and it keeps the place in line it had, rather than starting over at the back"
    );
    // The lease ROW, not `working_tree_holder`: the bound this area derived may already be past,
    // because the windows of its steps ran while they waited (nxf 6j6v.1wep). What this test pins
    // is WHO the copy was handed to.
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
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());

    // Something asks while the coder is still at work: the verifier must still wait.
    tick(&tmp, &worker, LATER_STILL, &ct);
    assert_eq!(worker.started("verifier"), 0, "the coder is at work");
    answer(&engine, &cs, &ct, LATER_STILL, "built T6");
    assert_eq!(
        worker.started("verifier"),
        0,
        "an answer is a message; the coder's process is still in the working copy"
    );

    end(&engine, &worker, "coder", &cs, LATER_STILL);

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

/// A coder whose process died without a word, and without an answer. The flow itself does not move
/// past such a step until its window lapses, and neither does the waiting step.
#[test]
fn a_step_whose_session_died_without_a_word_holds_the_next_one_until_its_window_lapses() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());

    worker.mark_gone(&cs);
    tick(&tmp, &worker, LATER_STILL, &ct);
    assert_eq!(
        worker.started("verifier"),
        0,
        "the coder still owes its answer inside its window"
    );

    tick(&tmp, &worker, CODER_LAPSED, &ct);
    assert_eq!(
        worker.started("verifier"),
        1,
        "once its window lapsed, the tick starts the step after it"
    );
}

/// The coder answers, into the slot the flow had counted as lapsed, and then ends. The step that
/// starts is the verifier that was waiting, once: the answer does not open a second verifier slot
/// beside it, and nothing starts it again later.
///
/// (Had the verifier's window ALSO lapsed in the queue, the flow would open the verifier step a
/// second time on that answer. That is nxf 6j6v.1wep's to settle; this item only makes sure that
/// the two never run side by side.)
#[test]
fn an_answered_earlier_step_starts_the_waiting_step_once() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    let (waiting_slot, _) = queued_row(&tmp, "verifier");
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());

    answer(&engine, &cs, &ct, LATER_STILL, "built T6");
    end(&engine, &worker, "coder", &cs, LATER_STILL);

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

    tick(&tmp, &worker, LATER_STILL, &waiting_slot);
    assert_eq!(
        worker.started("verifier"),
        1,
        "and a later question starts nothing a second time"
    );
}

// ---- review of PR #38: the holes in "queued or running" ----------------------------------------

/// **The race inside the hand-off** (review of PR #38, Integrity & Robustness #1). While the coder's
/// start is in progress (its prompt composed, its process not yet there), another process runs a
/// tick on the same workspace. If the verifier were already back in line at that moment, the
/// coder would be neither queued nor running, and that tick would start the verifier first.
#[test]
fn a_tick_from_another_process_during_the_earlier_steps_start_does_not_start_the_later_step() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    let (verifier_slot, _) = queued_row(&tmp, "verifier");
    let root = tmp.path().to_path_buf();
    *worker.meanwhile.lock().unwrap() = Some(Box::new(move |w: &RecordingWorker| {
        tick_in(&root, w, LATER, &verifier_slot);
    }));

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);

    assert!(
        worker.meanwhile.lock().unwrap().is_none(),
        "the premise: the other process's tick ran during the coder's start"
    );
    assert_eq!(worker.started("coder"), 1);
    assert_eq!(
        worker.started("verifier"),
        0,
        "the tick that ran during the coder's start did not start the step after it"
    );
}

/// The coder's start fails on a transient error at the hand-off. It is on record for a retry, which
/// is as much "in line" as a queue row: the verifier must not start before it (review of PR #38,
/// Code Quality #2, Integrity & Robustness #2).
#[test]
fn a_failed_start_of_the_earlier_step_holds_the_later_one_until_its_retry_has_run() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    worker.fail_the_next_coder_start();

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    assert_eq!(
        worker.started("coder"),
        0,
        "the premise: the coder's start failed"
    );
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["verifier".to_string()],
        "the verifier waits in line"
    );

    let (verifier_slot, _) = queued_row(&tmp, "verifier");
    tick(&tmp, &worker, LATER_STILL, &verifier_slot);
    assert_eq!(
        worker.started("verifier"),
        0,
        "the coder's retry is still pending, so the verifier still waits"
    );

    tick(&tmp, &worker, CODER_LAPSED, &verifier_slot);
    assert_eq!(worker.started("coder"), 1, "the retry started the coder");
    assert_eq!(
        worker.started("verifier"),
        0,
        "and the verifier does not start beside it"
    );
}

/// A worker that does not answer liveness says "not running" for a coder that is still writing.
/// Its `false` means "I never looked", so every session that has not announced its end counts as
/// present (review of PR #38, Code Quality #1, Integrity & Robustness #3).
#[test]
fn with_a_worker_that_does_not_answer_liveness_an_unended_session_holds_the_next_step() {
    let (tmp, engine, worker) = team_with(&[ORDERED], false);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());

    answer(&engine, &cs, &ct, LATER_STILL, "built T6");
    tick(&tmp, &worker, LATER_STILL, &ct);
    assert_eq!(
        worker.started("verifier"),
        0,
        "the coder never announced its end, and nothing could look whether it is still writing"
    );

    end(&engine, &worker, "coder", &cs, LATER_STILL);
    assert_eq!(
        worker.started("verifier"),
        1,
        "its announced end starts the step after it"
    );
}

/// The same order, declared as `steps:` rather than as `flow: sequential` (review of PR #38, Test
/// Quality #1).
#[test]
fn a_stepped_channel_is_an_ordered_run_too() {
    let (tmp, engine, worker) = team(STEPPED);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);

    assert_eq!(
        (worker.started("coder"), worker.started("verifier")),
        (1, 0),
        "the stepped flow's first step starts alone"
    );
    assert_eq!(queued_for(&tmp, &round), vec!["verifier".to_string()]);
}

/// A parallel round that is ONE step of an ordered run: its members wait for the step before them,
/// and then start together (review of PR #38, Test Quality #1).
#[test]
fn a_parallel_round_nested_as_one_step_waits_as_one_and_starts_together() {
    let (tmp, engine, worker) = team_with(&[ORDERED_WITH_A_ROUND, ROUND], true);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let round =
        a_round_queued_past_its_lapsed_first_step(&tmp, &worker, &engine, &["coder", "r1", "r2"]);

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread);
    assert_eq!(
        (
            worker.started("coder"),
            worker.started("r1"),
            worker.started("r2")
        ),
        (1, 0, 0),
        "the round's members wait for the step before them"
    );
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["r1".to_string(), "r2".to_string()]
    );

    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    answer(&engine, &cs, &ct, LATER_STILL, "built T6");
    end(&engine, &worker, "coder", &cs, LATER_STILL);

    assert_eq!(
        (worker.started("r1"), worker.started("r2")),
        (1, 1),
        "and then the whole round starts together, as it was declared"
    );
}

/// The sweep hands the expired holder's copy to the waiting round, and its receipt says where the
/// round's later step went (review of PR #38, Test Quality #6).
#[test]
fn the_sweeps_receipt_names_the_step_that_was_put_back_in_line() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, _) = a_holder_at_work(&engine, &worker);
    let round = an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    // The holder dies hard and its bound runs out: only the sweep will move the copy.
    worker.mark_gone(&builder);
    let (verifier_slot, _) = queued_row(&tmp, "verifier");

    let receipt = tick(&tmp, &worker, HOLDER_EXPIRED, &verifier_slot);

    let promotions = tick_promotions(&receipt).expect("the tick handed the copy on");
    assert_eq!(
        promotions
            .started
            .iter()
            .map(|p| p.role.as_str())
            .collect::<Vec<_>>(),
        vec!["coder"]
    );
    assert_eq!(
        promotions
            .deferred
            .iter()
            .map(|p| p.role.as_str())
            .collect::<Vec<_>>(),
        vec!["verifier"],
        "the receipt names the step that waits for the one before it"
    );
    assert_eq!(queued_for(&tmp, &round), vec!["verifier".to_string()]);
}

/// The holder's own waiting step is not somebody waiting for the copy. A sweep past the holder's
/// bound, with nothing but that step in line, must not take the copy from the operation to hand it
/// straight back to the same operation (review of PR #38, Test Quality #4).
#[test]
fn an_operation_is_not_swept_for_its_own_next_step() {
    let (tmp, engine, worker) = team(ORDERED);
    let (builder, _) = a_holder_at_work(&engine, &worker);
    an_ordered_round_queued_with_both_steps(&tmp, &engine, &worker);
    worker.mark_gone(&builder);
    let (verifier_slot, _) = queued_row(&tmp, "verifier");
    tick(&tmp, &worker, HOLDER_EXPIRED, &verifier_slot);
    let coder = worker.last_for("coder");
    // The swept-in coder dies hard as well; its operation's own lease runs out behind it.
    worker.mark_gone(&coder.internal_session);

    let receipt = tick(&tmp, &worker, AFTER_THE_SWEPT_CODER, &verifier_slot);

    assert!(
        receipt.working_tree.is_none() && receipt.handed_on.is_none(),
        "nobody else is waiting, so nothing reclaims the copy: {:?} {:?}",
        receipt.working_tree,
        receipt.handed_on
    );
    assert_eq!(
        worker.started("verifier"),
        1,
        "the operation's next step starts in the copy it already holds"
    );
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
