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
use nexus_chat::error::{ErrorKind, NxfError};
use nexus_chat::facade::{StatusOperation, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, FailedConsequence, TickReceipt, TickRequest, MAX_START_ATTEMPTS,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
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

fn team() -> (TempDir, Engine, Arc<FlakyWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channel: ChannelDecl = serde_yaml::from_str(EXCLUSIVE_CODING).expect("channel parses");
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
        .expect("the reply is posted");
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
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(tmp.path()).expect("declarations");
    let ctx = orchestration::Ctx {
        now,
        origin: "local",
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
