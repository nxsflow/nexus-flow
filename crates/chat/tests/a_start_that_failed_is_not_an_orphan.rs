//! **A queued round whose start failed is reported, retried, and can be taken back** (nxf
//! 6j6v.br25) — and the tick that completes a channel asks whether the working copy may go.
//!
//! Measured in the foreign project `agents` on 2026-10-08, running 0.205.1: a working-copy chain
//! ended, the lease was released correctly, and the four rounds queued behind it were taken off the
//! queue to start. Every start failed on a transient timeout (`nxf prime --db …` exceeded the 30s
//! wall-clock timeout; a few minutes later it took 2.3 s), on a project on an external volume. What
//! was left:
//!
//! ```text
//! service.err.log   "could not be started: io: `nxf prime …` exceeded the 30s wall-clock timeout"
//! nxc status        threads at session_state 'unknown', stale, no process — and no reason
//! nxc resume        "nothing is on hold"
//! nxc withdraw      "nothing … waiting or running"
//! ```
//!
//! Four orphaned operations, re-sent by hand. The failure had reached exactly one place, the stderr
//! of the background service — and nothing retried it, nothing recorded it, nothing could discard it.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`), with a [`WorkerConfig::Custom`]
//! worker that answers which of its sessions still run and can be told to fail its next starts the
//! way the sidecar's did. The tick goes through the compute layer, because it is the background
//! service's verb rather than one on the handle.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::{ErrorKind, NxfError, Result as NxfResult};
use nexus_chat::facade::{StatusOperation, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, FailedConsequence, TickReceipt, TickRequest, MAX_START_ATTEMPTS,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{
    TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-08T16:39:00Z";

/// The persona session that commissions the first chain.
const PM_SESSION: &str = "s-pm";

/// The message the sidecar's start failed with in the field, verbatim apart from the path.
const PRIME_TIMEOUT: &str =
    "`nxf prime --db .nxs/db.sqlite` exceeded the 30s wall-clock timeout and was killed";

/// A worker that records every start it accepted, answers which sessions are still running, and —
/// once armed — fails the next `failing` starts of a CODER with an error of `kind`, counting each
/// attempt. Only the coder: the queued round's first step is the start under test, and the wake of
/// the requester that the same completion sends must not use up a failure meant for it.
#[derive(Default)]
struct FlakyWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    failing: Mutex<Option<(u32, ErrorKind)>>,
    refused: Mutex<u32>,
}

impl FlakyWorker {
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn started(&self, handle: &str) -> usize {
        self.handles().iter().filter(|h| *h == handle).count()
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

    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }

    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }

    /// Fail the next `n` starts with `kind` (`u32::MAX` for every one from here on).
    fn fail_next(&self, n: u32, kind: ErrorKind) {
        *self.failing.lock().unwrap() = Some((n, kind));
    }

    fn refusals(&self) -> u32 {
        *self.refused.lock().unwrap()
    }
}

impl Worker for FlakyWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let mut failing = self.failing.lock().unwrap();
        if let Some((left, kind)) = failing.as_mut() {
            if *left > 0 && req.role.handle == "coder" {
                *left -= 1;
                *self.refused.lock().unwrap() += 1;
                return Err(TriggerError::Failed(NxfError::new(*kind, PRIME_TIMEOUT)));
            }
        }
        drop(failing);
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }

    /// It does answer — from the set above, as a host worker answers from its processes — and says
    /// so, which is what lets the engine conclude "nothing runs here" rather than "nobody looked".
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

/// The measured incident's shape: an ordered flow over two steps that needs the working copy alone.
const EXCLUSIVE_CODING: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";

/// Both members at once, still claiming the working copy: one claim area per round, two entries in
/// it — so one member can start while the other's start fails.
const EXCLUSIVE_PARALLEL: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: parallel\nworking_tree: exclusive\n";

fn team() -> (TempDir, Engine, Arc<FlakyWorker>) {
    team_with(EXCLUSIVE_CODING)
}

fn team_with(channel_yaml: &str) -> (TempDir, Engine, Arc<FlakyWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channel: ChannelDecl = serde_yaml::from_str(channel_yaml).expect("channel parses");
    let defs = Definitions::new(
        vec![role("pm"), role("coder"), role("finisher")],
        vec![channel],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    store(&tmp)
        .create_pending_session(PM_SESSION, "pm")
        .expect("the commissioning persona's session");
    let worker = Arc::new(FlakyWorker::default());
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

fn as_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

fn commission(engine: &Engine, who: Caller<'_>, body: &str) -> String {
    engine
        .send_to(
            who,
            SendToRequest {
                machine: None,
                to: "coding",
                body,
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission is accepted")
        .thread_id
}

fn answer(engine: &Engine, session: &str, thread: &str, body: &str) {
    answer_with_receipt(engine, session, thread, body);
}

fn answer_with_receipt(
    engine: &Engine,
    session: &str,
    thread: &str,
    body: &str,
) -> orchestration::ReplyReceipt {
    engine
        .reply_thread(
            as_session(session),
            ReplyThreadRequest {
                machine: None,
                thread,
                body,
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted")
}

/// The first chain — commissioned by the `pm` persona's session, as in the field, so its completion
/// has a requester to deliver to — holds the working copy; a second commission (by the human `pm2`)
/// is queued behind it; the coder has answered and ended, and the finisher has answered and is still
/// writing. Returns the second operation's root and the finisher's session — whose end is what
/// completes the chain.
fn a_finishing_chain_with_a_round_queued(
    engine: &Engine,
    worker: &FlakyWorker,
) -> (String, String) {
    commission(engine, as_session(PM_SESSION), "build T5");
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    worker.mark_running(&cs);
    let second = commission(engine, caller("pm2", NOW), "build T6");
    assert_eq!(
        worker.started("coder"),
        1,
        "the second round waits for the working copy"
    );

    answer(engine, &cs, &ct, "built");
    worker.mark_gone(&cs);
    engine.session_ended(caller("coder", NOW), &cs).unwrap();
    let finisher = worker.last_for("finisher");
    let (fs, ft) = (finisher.internal_session, finisher.reply_thread.unwrap());
    worker.mark_running(&fs);
    answer(engine, &fs, &ft, "finished, PR open");
    assert_eq!(
        worker.started("coder"),
        1,
        "the premise: the finisher is still writing, so nothing has handed the copy on yet"
    );
    (second, fs)
}

fn store(tmp: &TempDir) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn operation(engine: &Engine, now: &str, root: &str) -> StatusOperation {
    engine
        .status(now, StatusScope::Threads(&[root]))
        .expect("status")
        .operations
        .into_iter()
        .find(|op| op.root == root)
        .unwrap_or_else(|| panic!("operation {root} is in the report"))
}

/// The tick the background service runs, through the compute layer.
fn tick(tmp: &TempDir, worker: &FlakyWorker, now: &str, thread_id: &str) -> TickReceipt {
    tick_with(tmp, worker, &DryTimer, now, thread_id)
}

/// A timer that records what it was asked to arm: (thread, instant).
#[derive(Default)]
struct RecordingTimer {
    armed: Mutex<Vec<(String, String)>>,
}

impl Timer for RecordingTimer {
    fn schedule(&self, thread_id: &str, deadline: &str, _command: &str) -> NxfResult<TimerHandle> {
        self.armed
            .lock()
            .unwrap()
            .push((thread_id.to_string(), deadline.to_string()));
        Ok(TimerHandle(format!("recorded:{thread_id}")))
    }

    fn cancel(&self, _handle: &TimerHandle) -> NxfResult<()> {
        Ok(())
    }
}

/// The tick under the WORKSPACE's own origin — the one every address in its threads is qualified
/// with, and so the one the retry's "is it still owed" question has to be asked in.
fn tick_with(
    tmp: &TempDir,
    worker: &FlakyWorker,
    timer: &dyn Timer,
    now: &str,
    thread_id: &str,
) -> TickReceipt {
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
        timer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id }).expect("tick")
}

fn start_failures(warnings: &[FailedConsequence]) -> Vec<&FailedConsequence> {
    warnings
        .iter()
        .filter(|w| w.class == ConsequenceClass::StartFailed)
        .collect()
}

// ---- (1) the failure reaches the caller and the status view -------------------------------------

#[test]
fn a_queued_round_that_cannot_start_is_named_on_the_receipt_of_the_session_end_that_released() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Io);

    worker.mark_gone(&finisher);
    let receipt = engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    assert_eq!(
        worker.refusals(),
        1,
        "the premise: the release tried to start the queued coder"
    );
    let failed = start_failures(&receipt.warnings);
    assert_eq!(
        failed.len(),
        1,
        "the start that failed is on the receipt of the call that released, not only on stderr: \
         {:?}",
        receipt.warnings
    );
    let queued_thread = failed[0]
        .thread
        .clone()
        .expect("the finding names the thread the round owes its answer on");
    assert_eq!(
        store(&tmp).thread_root(&queued_thread).unwrap(),
        second,
        "…and that thread is the queued round's"
    );
    assert!(failed[0].session.is_some(), "and the session minted for it");
    assert!(
        failed[0]
            .detail
            .contains("exceeded the 30s wall-clock timeout"),
        "the worker's own words reach the caller: {}",
        failed[0].detail
    );
}

#[test]
fn the_status_view_shows_the_failed_start_on_its_operation_until_it_starts() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    let op = operation(&engine, NOW, &second);
    assert_eq!(
        op.start_failed.len(),
        1,
        "the operation that has nothing running and nothing queued says WHY: {op:?}"
    );
    let f = &op.start_failed[0];
    assert_eq!(f.role, "coder");
    assert_eq!(f.attempts, 1);
    assert!(f.error.contains("wall-clock timeout"), "{}", f.error);
    assert!(
        f.retry_at.is_some(),
        "an io failure is retried, and the record says when"
    );
    assert!(
        op.live,
        "an operation with a failed start stays in the default listing"
    );

    // Once it starts, the record goes with it.
    tick(&tmp, &worker, "2026-10-08T16:41:00Z", &second);
    assert_eq!(
        worker.started("coder"),
        2,
        "the premise: the retry started it"
    );
    assert!(
        operation(&engine, "2026-10-08T16:41:00Z", &second)
            .start_failed
            .is_empty(),
        "a start that happened is no longer a failed start"
    );
}

// ---- (3) a transient failure is retried, with a bound -------------------------------------------

#[test]
fn a_start_that_failed_on_a_timeout_is_started_by_the_next_tick_after_its_retry_instant() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    assert_eq!(
        worker.started("coder"),
        1,
        "the premise: the release's start failed"
    );

    // Too early: the retry has an instant, and a tick before it leaves the record alone.
    tick(&tmp, &worker, "2026-10-08T16:39:30Z", &second);
    assert_eq!(worker.started("coder"), 1, "not before its instant");

    let receipt = tick(&tmp, &worker, "2026-10-08T16:40:01Z", &second);
    assert_eq!(
        worker.started("coder"),
        2,
        "a start that failed on a timeout is started again by the tick, not left an orphan: {:?}",
        receipt.warnings
    );
    assert!(
        start_failures(&receipt.warnings).is_empty(),
        "{:?}",
        receipt.warnings
    );
    assert!(store(&tmp).failed_starts().unwrap().is_empty());
}

#[test]
fn a_start_that_keeps_failing_is_tried_a_bounded_number_of_times_and_then_left_for_withdraw() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(u32::MAX, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    // Ticks far enough apart that every retry is due.
    for now in [
        "2026-10-08T17:00:00Z",
        "2026-10-08T18:00:00Z",
        "2026-10-08T19:00:00Z",
        "2026-10-08T20:00:00Z",
    ] {
        tick(&tmp, &worker, now, &second);
    }

    assert_eq!(
        worker.refusals(),
        MAX_START_ATTEMPTS,
        "the release's own attempt plus the retries, and not one more"
    );
    let op = operation(&engine, "2026-10-08T20:00:00Z", &second);
    assert_eq!(op.start_failed.len(), 1, "{op:?}");
    assert_eq!(op.start_failed[0].attempts, MAX_START_ATTEMPTS);
    assert_eq!(
        op.start_failed[0].retry_at, None,
        "past the bound nothing is due, and the record says so"
    );
}

#[test]
fn a_start_refused_for_a_reason_waiting_cannot_fix_is_not_retried() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Validation);
    worker.mark_gone(&finisher);
    let receipt = engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    assert_eq!(start_failures(&receipt.warnings).len(), 1);

    tick(&tmp, &worker, "2026-10-08T18:00:00Z", &second);
    assert_eq!(
        worker.started("coder"),
        1,
        "the same call a minute later meets the same answer, so nothing retries it"
    );
    let op = operation(&engine, "2026-10-08T18:00:00Z", &second);
    assert_eq!(op.start_failed.len(), 1);
    assert_eq!(op.start_failed[0].retry_at, None);
}

// ---- (4) an orphan can be discarded ---------------------------------------------------------------

#[test]
fn withdraw_finds_a_round_whose_start_failed_and_discards_it() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    let receipt = engine
        .withdraw(caller("pm2", NOW), &second)
        .expect("a commission whose start failed is something to withdraw, not `nothing is there`");

    assert_eq!(receipt.withdrawn.len(), 1, "{receipt:?}");
    assert_eq!(receipt.withdrawn[0].role, "coder");
    assert!(
        store(&tmp).failed_starts().unwrap().is_empty(),
        "its record is gone"
    );
    let op = operation(&engine, NOW, &second);
    assert!(op.start_failed.is_empty(), "{op:?}");

    // …and the retry it had been given does not bring it back.
    tick(&tmp, &worker, "2026-10-08T18:00:00Z", &second);
    assert_eq!(
        worker.started("coder"),
        1,
        "a withdrawn commission is not started"
    );
}

// ---- (2) the tick that completes a channel asks the release question ----------------------------

#[test]
fn a_tick_that_completes_the_chain_hands_the_working_copy_to_the_queued_round() {
    let (tmp, engine, worker) = team();
    let (_second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    let holder = |tmp: &TempDir| store(tmp).working_tree_holder(NOW).unwrap();
    let first_chain = holder(&tmp).expect("the first chain holds the working copy");
    let finisher_thread = worker.last_for("finisher").reply_thread.unwrap();
    let channel_thread = store(&tmp)
        .thread_parent(&finisher_thread)
        .unwrap()
        .expect("the channel thread");

    // The finisher's process dies hard: it never announces its end, so nothing but the tick will
    // notice that the chain is over.
    worker.mark_gone(&finisher);
    let receipt = tick(&tmp, &worker, NOW, &channel_thread);

    assert!(
        receipt.acted,
        "the premise: this tick is what completed the channel: {receipt:?}"
    );
    assert_ne!(
        holder(&tmp),
        Some(first_chain),
        "the chain the tick completed let the working copy go"
    );
    assert_eq!(
        worker.started("coder"),
        2,
        "and the queued round has it now, not at the lease's bound"
    );
}

// ---- review of PR #32: the lease a failed start can leave behind (Integrity #1) -----------------

/// The thread the queued round's coder owes its answer on, read from its failed-start record.
fn failed_thread(tmp: &TempDir) -> String {
    store(tmp).failed_starts().unwrap()[0]
        .thread
        .clone()
        .expect("a failed start names its thread")
}

#[test]
fn a_retry_that_takes_the_copy_and_fails_again_hands_it_on() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(2, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    assert_eq!(
        store(&tmp).working_tree_holder(NOW).unwrap(),
        None,
        "the premise: nothing started at the release, so the copy was handed on to nobody"
    );

    // The retry asks for the copy, gets it — it is free — and its start fails again.
    let later = "2026-10-08T16:40:30Z";
    tick(&tmp, &worker, later, &second);
    assert_eq!(
        worker.refusals(),
        2,
        "the premise: the retry ran and failed"
    );
    assert_eq!(
        store(&tmp).working_tree_holder(later).unwrap(),
        None,
        "a retry that took the copy and started nothing must not keep it"
    );

    // So the next commission gets it at once instead of queueing behind an empty claim.
    commission(&engine, caller("pm3", later), "build T7");
    assert_eq!(
        worker.started("coder"),
        2,
        "the third round starts: nobody holds the copy"
    );
}

#[test]
fn a_queued_third_round_starts_once_the_failed_start_holding_the_copy_is_withdrawn() {
    let (tmp, engine, worker) = team_with(EXCLUSIVE_PARALLEL);
    commission(&engine, as_session(PM_SESSION), "both of you, now");
    let (cs, ct) = {
        let c = worker.last_for("coder");
        (c.internal_session, c.reply_thread.unwrap())
    };
    let (fs, ft) = {
        let f = worker.last_for("finisher");
        (f.internal_session, f.reply_thread.unwrap())
    };
    worker.mark_running(&cs);
    worker.mark_running(&fs);
    let second = commission(&engine, caller("pm2", NOW), "the second round");
    answer(&engine, &cs, &ct, "coder done");
    answer(&engine, &fs, &ft, "finisher done");
    worker.mark_gone(&cs);
    engine.session_ended(caller("coder", NOW), &cs).unwrap();

    // The second round is released into: its finisher starts, its coder cannot — for a reason
    // waiting does not fix, so no retry is involved.
    worker.fail_next(1, ErrorKind::Validation);
    worker.mark_gone(&fs);
    engine.session_ended(caller("finisher", NOW), &fs).unwrap();
    assert_eq!(
        worker.started("finisher"),
        2,
        "the premise: one member started"
    );
    assert_eq!(worker.started("coder"), 1, "the premise: the other did not");
    let second_holds = store(&tmp)
        .working_tree_holder(NOW)
        .unwrap()
        .expect("the second round holds the copy");

    // A third round queues behind it; the second round's finisher answers and ends. Its coder's
    // thread still owes, so the copy stays — with nothing running in that operation.
    commission(&engine, caller("pm3", NOW), "the third round");
    let f2 = worker.last_for("finisher");
    let (f2s, f2t) = (f2.internal_session, f2.reply_thread.unwrap());
    answer(&engine, &f2s, &f2t, "finisher done again");
    engine.session_ended(caller("finisher", NOW), &f2s).unwrap();
    assert_eq!(
        store(&tmp).working_tree_holder(NOW).unwrap().as_deref(),
        Some(second_holds.as_str()),
        "the premise: the failed start's thread still owes, so the copy is kept for it"
    );
    assert_eq!(
        worker.started("coder"),
        1,
        "the premise: the third round waits"
    );

    let receipt = engine
        .withdraw(caller("pm2", NOW), &second)
        .expect("the failed start is withdrawn");
    assert_eq!(receipt.withdrawn.len(), 1, "{receipt:?}");

    assert_ne!(
        store(&tmp).working_tree_holder(NOW).unwrap().as_deref(),
        Some(second_holds.as_str()),
        "with its last obligation discharged, the withdrawn operation lets the copy go"
    );
    assert_eq!(
        worker.started("coder"),
        2,
        "and the queued third round starts, not at the dead-holder sweep or the lease's bound"
    );
}

// ---- review of PR #32: the retry claim (Integrity #2, #3; Test #4, #8) --------------------------

/// A failed start whose retry is due at 16:40:00, and the handle and worker it was made with.
fn a_failed_start_due_for_a_retry() -> (TempDir, Engine, Arc<FlakyWorker>, String) {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    assert_eq!(store(&tmp).failed_starts().unwrap().len(), 1, "the premise");
    (tmp, engine, worker, second)
}

#[test]
fn two_overlapping_claims_on_one_due_retry_hand_it_out_once() {
    let (tmp, _engine, _worker, _second) = a_failed_start_due_for_a_retry();
    let mut s = store(&tmp);
    let first = s
        .claim_due_failed_starts("2026-10-08T16:41:00Z", "2026-10-08T16:51:00Z")
        .unwrap();
    let second = s
        .claim_due_failed_starts("2026-10-08T16:41:00Z", "2026-10-08T16:51:00Z")
        .unwrap();
    assert_eq!(first.len(), 1, "the due retry is claimed");
    assert!(
        second.is_empty(),
        "a second tick overlapping the first must not start the same session again: {second:?}"
    );
}

#[test]
fn a_withdraw_while_a_retry_is_starting_the_session_leaves_it_alone() {
    let (tmp, engine, _worker, second) = a_failed_start_due_for_a_retry();
    let thread = failed_thread(&tmp);
    // A tick has claimed the retry and is starting the session right now.
    store(&tmp)
        .claim_due_failed_starts("2026-10-08T16:40:00Z", "2026-10-08T16:50:00Z")
        .unwrap();

    let receipt = engine
        .withdraw(caller("pm2", "2026-10-08T16:40:05Z"), &second)
        .expect("withdraw answers");

    assert!(
        receipt.withdrawn.is_empty(),
        "a commission being started is not discarded under that start: {receipt:?}"
    );
    assert_eq!(
        receipt.started_meanwhile,
        vec![thread],
        "it is reported as started meanwhile, so the caller knows to look again"
    );
    assert_eq!(
        store(&tmp).failed_starts().unwrap().len(),
        1,
        "and its record is left for the retry that holds it"
    );
}

#[test]
fn a_retry_whose_process_died_after_the_claim_falls_due_again() {
    let (tmp, _engine, worker, second) = a_failed_start_due_for_a_retry();
    // A tick claims the retry and dies before it fires or records anything.
    store(&tmp)
        .claim_due_failed_starts("2026-10-08T16:40:00Z", "2026-10-08T16:50:00Z")
        .unwrap();
    let rows = store(&tmp).failed_starts().unwrap();
    assert!(
        rows[0].retry_at.is_some(),
        "a claimed row is never left looking like `not retried`: {rows:?}"
    );

    tick(&tmp, &worker, "2026-10-08T16:50:01Z", &second);
    assert_eq!(
        worker.started("coder"),
        2,
        "once the claim's hold has passed, the next tick starts it"
    );
}

// ---- review of PR #32: a requeue is reported, and its record goes (decision b) -----------------

#[test]
fn a_promoted_round_that_goes_straight_back_into_the_queue_is_named_on_the_release_receipt() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    // Re-parenting, staged on the row: the queued entry was parked under a claim area that is no
    // longer the one its thread resolves to. The release grants the copy to the area on the row,
    // and the fired trigger — which re-derives its own — loses the acquire and queues again.
    store(&tmp)
        .connection()
        .execute(
            "UPDATE working_tree_queue SET scope_key = 'thread:m-re-parented'",
            [],
        )
        .unwrap();

    worker.mark_gone(&finisher);
    let receipt = engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    let requeued: Vec<_> = receipt
        .warnings
        .iter()
        .filter(|w| w.class == ConsequenceClass::StartRequeued)
        .collect();
    assert_eq!(
        requeued.len(),
        1,
        "the release was to start it and did not, and the receipt says so: {:?}",
        receipt.warnings
    );
    let thread = requeued[0].thread.clone().expect("it names the thread");
    assert_eq!(store(&tmp).thread_root(&thread).unwrap(), second);
    assert_eq!(worker.started("coder"), 1, "nothing was started for it");
    assert!(
        store(&tmp)
            .list_working_tree_queue()
            .unwrap()
            .iter()
            .any(|q| q.thread.as_deref() == Some(thread.as_str())),
        "it is back in the queue"
    );
}

#[test]
fn a_retry_that_finds_the_copy_taken_goes_back_into_the_queue_in_its_old_place() {
    let (tmp, engine, worker, second) = a_failed_start_due_for_a_retry();
    let thread = failed_thread(&tmp);
    // Meanwhile another commission takes the free copy.
    commission(&engine, caller("pm3", NOW), "build T7");
    assert_eq!(
        worker.started("coder"),
        2,
        "the premise: the third round holds it"
    );

    tick(&tmp, &worker, "2026-10-08T16:40:30Z", &second);

    assert!(
        store(&tmp).failed_starts().unwrap().is_empty(),
        "a failed start that is waiting in line again is no longer a failed start"
    );
    let queued: Vec<_> = store(&tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .filter(|q| q.thread.as_deref() == Some(thread.as_str()))
        .collect();
    assert_eq!(queued.len(), 1, "it is in the queue");
    assert_eq!(
        queued[0].enqueued_at.as_deref(),
        Some(NOW),
        "with the place it first queued at, not behind everything that came since"
    );
}

// ---- review of PR #32: every path reports, and says so when it cannot decide (Test #5) ----------

#[test]
fn a_reply_that_releases_names_the_start_that_failed() {
    let (_tmp, engine, worker) = team();
    commission(&engine, as_session(PM_SESSION), "build T5");
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    worker.mark_running(&cs);
    commission(&engine, caller("pm2", NOW), "build T6");
    answer(&engine, &cs, &ct, "built");
    worker.mark_gone(&cs);
    engine.session_ended(caller("coder", NOW), &cs).unwrap();
    let finisher = worker.last_for("finisher");
    let (fs, ft) = (finisher.internal_session, finisher.reply_thread.unwrap());

    // The finisher's process is already gone when its answer lands, so the REPLY completes the
    // chain and releases.
    worker.fail_next(1, ErrorKind::Io);
    let receipt = answer_with_receipt(&engine, &fs, &ft, "finished, PR open");

    assert_eq!(
        worker.refusals(),
        1,
        "the premise: the reply's release tried the start"
    );
    assert_eq!(
        start_failures(&receipt.warnings).len(),
        1,
        "the reply's receipt names it: {:?}",
        receipt.warnings
    );
}

#[test]
fn the_tick_that_completes_the_chain_names_the_start_that_failed() {
    let (tmp, engine, worker) = team();
    let (_second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    let finisher_thread = worker.last_for("finisher").reply_thread.unwrap();
    let channel_thread = store(&tmp)
        .thread_parent(&finisher_thread)
        .unwrap()
        .expect("the channel thread");
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);

    let receipt = tick(&tmp, &worker, NOW, &channel_thread);

    assert!(receipt.acted, "the premise: {receipt:?}");
    assert_eq!(
        start_failures(&receipt.warnings).len(),
        1,
        "the tick's receipt names it: {:?}",
        receipt.warnings
    );
}

/// Make every release of the lease fail inside the store — the one way to stage a release question
/// that cannot be answered without reaching into the engine.
fn the_lease_cannot_be_released(tmp: &TempDir) {
    store(tmp)
        .connection()
        .execute_batch(
            "CREATE TRIGGER br25_injected BEFORE DELETE ON working_tree_lease
             BEGIN SELECT RAISE(ABORT, 'injected for the test'); END;",
        )
        .unwrap();
}

fn lease_undecided(warnings: &[FailedConsequence]) -> usize {
    warnings
        .iter()
        .filter(|w| w.class == ConsequenceClass::LeaseUndecided)
        .count()
}

#[test]
fn a_session_end_whose_release_cannot_be_decided_says_so() {
    let (tmp, engine, worker) = team();
    let (_second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    the_lease_cannot_be_released(&tmp);
    worker.mark_gone(&finisher);
    let receipt = engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    assert_eq!(
        lease_undecided(&receipt.warnings),
        1,
        "{:?}",
        receipt.warnings
    );
}

#[test]
fn a_reply_whose_release_cannot_be_decided_says_so() {
    let (tmp, engine, worker) = team();
    commission(&engine, as_session(PM_SESSION), "build T5");
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    worker.mark_running(&cs);
    commission(&engine, caller("pm2", NOW), "build T6");
    answer(&engine, &cs, &ct, "built");
    worker.mark_gone(&cs);
    engine.session_ended(caller("coder", NOW), &cs).unwrap();
    let finisher = worker.last_for("finisher");
    let (fs, ft) = (finisher.internal_session, finisher.reply_thread.unwrap());
    the_lease_cannot_be_released(&tmp);

    let receipt = answer_with_receipt(&engine, &fs, &ft, "finished, PR open");

    assert_eq!(
        lease_undecided(&receipt.warnings),
        1,
        "the reply is posted, and its receipt says the copy's release could not be decided: {:?}",
        receipt.warnings
    );
}

#[test]
fn a_tick_whose_release_cannot_be_decided_says_so() {
    let (tmp, engine, worker) = team();
    let (_second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    let finisher_thread = worker.last_for("finisher").reply_thread.unwrap();
    let channel_thread = store(&tmp)
        .thread_parent(&finisher_thread)
        .unwrap()
        .expect("the channel thread");
    the_lease_cannot_be_released(&tmp);
    worker.mark_gone(&finisher);
    let receipt = tick(&tmp, &worker, NOW, &channel_thread);
    assert!(receipt.acted, "the premise: {receipt:?}");
    assert_eq!(
        lease_undecided(&receipt.warnings),
        1,
        "{:?}",
        receipt.warnings
    );
}

// ---- review of PR #32: the tick's Advance branch does not release (decision c) -----------------

#[test]
fn a_tick_that_moves_a_flow_on_to_its_next_step_keeps_the_copy() {
    let (tmp, engine, worker) = team();
    commission(&engine, as_session(PM_SESSION), "build T5");
    let coder = worker.last_for("coder");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    worker.mark_running(&cs);
    commission(&engine, caller("pm2", NOW), "build T6");
    answer(&engine, &cs, &ct, "built");
    let holder = store(&tmp)
        .working_tree_holder(NOW)
        .unwrap()
        .expect("the first chain holds the copy");
    let channel_thread = store(&tmp)
        .thread_parent(&ct)
        .unwrap()
        .expect("the channel thread");

    // The coder's process dies without announcing it; only the tick can move the flow on.
    worker.mark_gone(&cs);
    let receipt = tick(&tmp, &worker, NOW, &channel_thread);

    assert_eq!(receipt.reason, "advanced", "the premise: {receipt:?}");
    assert_eq!(
        worker.started("finisher"),
        1,
        "the premise: the next step started"
    );
    assert_eq!(
        store(&tmp).working_tree_holder(NOW).unwrap().as_deref(),
        Some(holder.as_str()),
        "the chain goes on in the copy it holds"
    );
    assert_eq!(
        worker.started("coder"),
        1,
        "and the queued round still waits"
    );
}

// ---- review of PR #32: what the record says (Test #6, #7; Integrity #4) ------------------------

#[test]
fn the_retries_are_spaced_further_apart_and_the_tick_is_armed_for_each() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(2, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    let thread = failed_thread(&tmp);
    let retry_at = |tmp: &TempDir| store(tmp).failed_starts().unwrap()[0].retry_at.clone();
    assert_eq!(
        retry_at(&tmp).as_deref(),
        Some("2026-10-08T16:40:00Z"),
        "the first failure is retried a minute later"
    );

    let timer = RecordingTimer::default();
    tick_with(&tmp, &worker, &timer, "2026-10-08T16:40:30Z", &second);
    assert_eq!(
        retry_at(&tmp).as_deref(),
        Some("2026-10-08T16:42:30Z"),
        "the second is retried two minutes after it failed"
    );
    assert!(
        timer
            .armed
            .lock()
            .unwrap()
            .contains(&(thread, "2026-10-08T16:42:30Z".to_string())),
        "and the tick is armed for that instant on the thread it owes: {:?}",
        timer.armed.lock().unwrap()
    );
}

#[test]
fn the_status_json_carries_start_failed_only_on_an_operation_that_has_one() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    let first_root = store(&tmp)
        .thread_root(&worker.last_for("finisher").reply_thread.unwrap())
        .unwrap();
    worker.fail_next(1, ErrorKind::Io);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();

    let failed = serde_json::to_value(operation(&engine, NOW, &second)).unwrap();
    let entry = &failed["start_failed"][0];
    assert_eq!(entry["role"], "coder", "{failed}");
    assert_eq!(entry["attempts"], 1, "{failed}");
    assert_eq!(entry["retry_at"], "2026-10-08T16:40:00Z", "{failed}");
    let clean = serde_json::to_value(operation(&engine, NOW, &first_root)).unwrap();
    assert!(
        clean.get("start_failed").is_none(),
        "an operation where no start failed says nothing about it: {clean}"
    );
}

#[test]
fn a_withdrawn_failed_start_puts_no_worker_text_into_the_synced_thread() {
    let (tmp, engine, worker) = team();
    let (second, finisher) = a_finishing_chain_with_a_round_queued(&engine, &worker);
    worker.fail_next(1, ErrorKind::Validation);
    worker.mark_gone(&finisher);
    engine
        .session_ended(caller("finisher", NOW), &finisher)
        .unwrap();
    let thread = failed_thread(&tmp);

    engine.withdraw(caller("pm2", NOW), &second).unwrap();

    let bodies: Vec<String> = {
        let s = store(&tmp);
        let mut st = s
            .connection()
            .prepare("SELECT body FROM messages WHERE thread_id = ?1")
            .unwrap();
        st.query_map([thread.as_str()], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let discharge = bodies
        .iter()
        .find(|b| b.contains("could not be started"))
        .unwrap_or_else(|| panic!("the discharge is posted: {bodies:?}"));
    assert!(
        !discharge.contains("nxf prime") && !discharge.contains(".nxs/db.sqlite"),
        "the worker's own text — a command line, a path on this machine — stays out of the synced \
         thread: {discharge}"
    );
}

// ---- review of PR #32: a retry nobody waits for any more (Integrity #5) -------------------------

#[test]
fn a_retry_whose_thread_was_answered_meanwhile_is_not_started() {
    let (tmp, engine, worker, second) = a_failed_start_due_for_a_retry();
    let thread = failed_thread(&tmp);
    // The answer arrives by another route — here, posted under the coder's own name by hand.
    engine
        .reply_thread(
            caller("coder", NOW),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "done by hand",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the answer is posted");

    tick(&tmp, &worker, "2026-10-08T16:40:30Z", &second);

    assert_eq!(
        worker.started("coder"),
        1,
        "a session is not started for an answer that has already been given"
    );
    assert!(
        store(&tmp).failed_starts().unwrap().is_empty(),
        "and the record goes with it"
    );
}
