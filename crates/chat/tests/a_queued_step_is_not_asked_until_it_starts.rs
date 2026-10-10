//! **A step's answer window starts when its session starts, not when it is queued** (nxf
//! 6j6v.1wep).
//!
//! Measured in the `agents` workspace on 2026-10-09/10, running 0.206.1. Two operations each opened
//! a coding flow. The first one's coder took the working-copy lease, and the second one's build
//! step was queued behind it. Six hours later the build step reached its declared window, still
//! queued and never started, and the tick treated that as silence:
//!
//! ```text
//! step "build" gave no answer inside its declared window; the flow advanced to step "verify" without it
//! ```
//!
//! A verify step was opened for a build that never ran. The rule these tests pin: **a step whose
//! trigger still waits in the working-copy queue has not been asked, so its window cannot lapse**.
//! It runs from the moment the step starts, and a step that started and then fell silent lapses
//! exactly as before. The lease the hand-off grants is bounded from the start, not from windows
//! that ran out in the queue.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`), with a [`WorkerConfig::Custom`]
//! worker that records every start and answers which of its sessions still run, as
//! `a_hand_off_starts_one_step_per_ordered_run.rs` does. The tick goes through the compute layer,
//! because it is the background service's verb rather than one on the handle.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::{ErrorKind, NxfError, Result as NxfResult};
use nexus_chat::orchestration::{self, Caller, ConsequenceClass, TickReceipt, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{
    TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-10T01:00:00Z";
/// Six hours on — the field's wait. The waiting round's one-minute window ran out long ago, if it
/// had been running.
const SIX_HOURS_LATER: &str = "2026-10-10T07:00:00Z";
/// The holder lets go, and the waiting round's first step starts.
const RELEASED: &str = "2026-10-10T07:00:30Z";
/// Thirty seconds into the started step's one-minute window.
const INSIDE_THE_FRESH_WINDOW: &str = "2026-10-10T07:01:00Z";
/// One minute after [`RELEASED`]: where the started step's window ends.
const FRESH_WINDOW_ENDS: &str = "2026-10-10T07:01:30Z";
/// A minute and a half after the start: the window that began then has run out.
const PAST_THE_FRESH_WINDOW: &str = "2026-10-10T07:02:00Z";

/// Forty-five seconds after [`RELEASED`]: past a thirty-second window that started then, and
/// before the retry of a start that failed then (one minute later).
const BEFORE_THE_RETRY: &str = "2026-10-10T07:01:15Z";
/// Where a thirty-second window that started at [`RELEASED`] would end.
const SHORT_WINDOW_FROM_RELEASED: &str = "2026-10-10T07:01:00Z";
/// Just past the retry instant of a start that failed at [`RELEASED`].
const AT_THE_RETRY: &str = "2026-10-10T07:01:31Z";
/// Nineteen seconds into the thirty-second window of a step started at [`AT_THE_RETRY`].
const INSIDE_THE_RETRIED_WINDOW: &str = "2026-10-10T07:01:50Z";
/// Past that window.
const PAST_THE_RETRIED_WINDOW: &str = "2026-10-10T07:02:10Z";

/// The pre-1wep shape of #38's tests: the queued coder is answered and the verifier opened at this
/// instant, with a one-minute window.
const VERIFIER_OPENED: &str = "2026-10-10T01:05:00Z";
/// The holder lets go inside its own two-hour lease, inside the verifier's window.
const HANDED_ON: &str = "2026-10-10T01:05:30Z";
/// Past the window the verifier was given when it was opened, while it waits behind the coder.
const PAST_THE_VERIFIERS_WINDOW: &str = "2026-10-10T01:07:00Z";

/// The reason a tick reports when it moved a flow on to its next step (`TICK_ADVANCED` in the crate).
const ADVANCED: &str = "advanced";

/// The operation that holds the working copy: one step, no window of its own.
const HOLDER: &str =
    "name: holding\nmembers: [builder]\nflow: sequential\nworking_tree: exclusive\n";

/// The measured incident's shape: build, then verify, in one ordered run that needs the copy alone.
const ORDERED: &str = "name: coding\nmembers: [coder, verifier]\nflow: sequential\nworking_tree: exclusive\ntimeout: 1m\n";

/// The same run with a window shorter than the spacing of a failed start's first retry (one
/// minute), so the window can pass while the retry is still due.
const SHORT: &str = "name: coding\nmembers: [coder, verifier]\nflow: sequential\nworking_tree: exclusive\ntimeout: 30s\n";

/// A worker that records every start it accepted and answers which sessions are still running.
#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    /// The error the next CODER start fails with, once.
    failing_coder: Mutex<Option<ErrorKind>>,
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

    fn fail_the_next_coder_start(&self, kind: ErrorKind) {
        *self.failing_coder.lock().unwrap() = Some(kind);
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if req.role.handle == "coder" {
            if let Some(kind) = self.failing_coder.lock().unwrap().take() {
                return Err(TriggerError::Failed(NxfError::new(
                    kind,
                    "the coder could not be started",
                )));
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
    team_with(ORDERED)
}

fn team_with(coding: &str) -> (TempDir, Engine, Arc<RecordingWorker>) {
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

/// A timer that records every job it was asked to arm, as `(thread, instant)`.
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

/// The tick the background service runs, through the compute layer.
fn tick(tmp: &TempDir, worker: &RecordingWorker, now: &str, thread_id: &str) -> TickReceipt {
    tick_with(tmp, worker, &DryTimer, now, thread_id)
}

/// [`tick`] with the timer the caller wants to look at afterwards.
fn tick_with(
    tmp: &TempDir,
    worker: &RecordingWorker,
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

fn lapse_findings(receipt: &TickReceipt) -> usize {
    findings(receipt, ConsequenceClass::StepUnanswered).len()
}

/// The threads of `receipt`'s findings of `class`.
fn findings(receipt: &TickReceipt, class: ConsequenceClass) -> Vec<Option<String>> {
    receipt
        .warnings
        .iter()
        .filter(|w| w.class == class)
        .map(|w| w.thread.clone())
        .collect()
}

/// The queue row of `role`, if it has one.
fn queue_row(tmp: &TempDir, role: &str) -> Option<nexus_chat::working_tree::QueuedTrigger> {
    store(tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .find(|q| q.role == role)
}

/// The first operation holds the working copy with its builder at work. Returns the builder's
/// session and the thread it answers on.
fn a_holder_at_work(engine: &Engine, worker: &RecordingWorker) -> (String, String) {
    commission(engine, caller("pm", NOW), "holding", "build T5");
    let builder = worker.last_for("builder");
    (builder.internal_session, builder.reply_thread.unwrap())
}

/// The holder finishes at `at`: the builder answers and its session ends, which lets the copy go.
fn the_holder_lets_go(
    engine: &Engine,
    worker: &RecordingWorker,
    session: &str,
    thread: &str,
    at: &str,
) {
    answer(engine, session, thread, at, "built");
    worker.mark_gone(session);
    engine
        .session_ended(caller("builder", at), session)
        .expect("the session end is accepted");
}

/// The holder finishes at [`RELEASED`], and the copy goes to the waiting round. Returns the jobs
/// the hand-off armed.
///
/// Six hours in, the holder's own lease ran out long ago (it declares no window, so it got the
/// two-hour fallback) and only its live builder kept the copy where it was. A lease that has run
/// out is not released by the session end; the tick's sweep reclaims it once nothing runs behind
/// it. That is the field's route too: the waiting round was started by the tick's hand-off.
///
/// The tick is asked about the HOLDER's thread, as the service would be by the holder's own job, so
/// a job armed for the waiting round's channel can only have come from the hand-off.
fn the_copy_is_handed_on(
    tmp: &TempDir,
    engine: &Engine,
    worker: &RecordingWorker,
    holder: (&str, &str),
) -> Vec<(String, String)> {
    the_holder_lets_go(engine, worker, holder.0, holder.1, RELEASED);
    let timer = RecordingTimer::default();
    tick_with(tmp, worker, &timer, RELEASED, holder.1);
    timer.armed.into_inner().unwrap()
}

/// The ordered round, commissioned while the holder works: its coder waits for the working copy.
/// Returns the round's root and the channel thread its steps hang under.
fn an_ordered_round_queued_behind_the_holder(tmp: &TempDir, engine: &Engine) -> (String, String) {
    let root = commission(engine, caller("pm2", NOW), "coding", "build T6");
    assert_eq!(
        queued_for(tmp, &root),
        vec!["coder".to_string()],
        "the premise: the round's first step waits for the working copy"
    );
    let coder_slot = store(tmp)
        .list_working_tree_queue()
        .unwrap()
        .into_iter()
        .find(|q| q.role == "coder")
        .and_then(|q| q.thread)
        .expect("the queued coder names its slot");
    let channel_thread = store(tmp)
        .thread_parent(&coder_slot)
        .unwrap()
        .expect("the slot hangs under its channel thread");
    (root, channel_thread)
}

// ---- the incident ----------------------------------------------------------------------------

#[test]
fn a_step_queued_behind_the_working_copy_does_not_lapse_and_its_window_starts_when_it_does() {
    let (tmp, engine, worker) = team();
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);

    // Six hours in the queue. The builder is still at work, so the copy stays where it is.
    let receipt = tick(&tmp, &worker, SIX_HOURS_LATER, &channel_thread);
    assert_ne!(
        receipt.reason, ADVANCED,
        "a step that never started was never asked; the flow must not move past it: {receipt:?}"
    );
    assert_eq!(
        lapse_findings(&receipt),
        0,
        "and nothing may report it as silent: {receipt:?}"
    );
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["coder".to_string()],
        "the coder is still waiting, and no verify step was opened behind it"
    );
    assert_eq!(worker.started("coder") + worker.started("verifier"), 0);

    // The holder lets go and the coder starts. Its window runs from now, and something watches it.
    let armed = the_copy_is_handed_on(&tmp, &engine, &worker, (&builder, &builder_thread));
    assert_eq!(worker.started("coder"), 1, "the hand-off starts the coder");
    assert!(
        armed.contains(&(channel_thread.clone(), FRESH_WINDOW_ENDS.to_string())),
        "the hand-off arms a look at the end of the window the coder got when it started: \
         {armed:?}"
    );

    let receipt = tick(&tmp, &worker, INSIDE_THE_FRESH_WINDOW, &channel_thread);
    assert_ne!(
        receipt.reason, ADVANCED,
        "thirty seconds into a one-minute window is not a lapse: {receipt:?}"
    );
    assert_eq!(worker.started("verifier"), 0);
    assert!(queued_for(&tmp, &round).is_empty());

    // The coder says nothing for the whole window it got when it started: THAT is a lapse.
    let receipt = tick(&tmp, &worker, PAST_THE_FRESH_WINDOW, &channel_thread);
    assert_eq!(
        receipt.reason, ADVANCED,
        "a started step that stayed silent for its window still advances the flow: {receipt:?}"
    );
    assert_eq!(lapse_findings(&receipt), 1, "{receipt:?}");
    assert_eq!(
        worker.started("verifier") + queued_for(&tmp, &round).len(),
        1,
        "the verify step was opened once"
    );
}

// ---- what must NOT change: the declared `timeout:` of a started step ----------------------------

#[test]
fn a_started_step_that_falls_silent_still_lapses_on_its_window() {
    let (tmp, engine, worker) = team();
    // Nobody holds the copy: the coder starts at once.
    let round = commission(&engine, caller("pm2", NOW), "coding", "build T6");
    assert_eq!(worker.started("coder"), 1, "the premise: the coder started");
    let coder_slot = worker.last_for("coder").reply_thread.unwrap();
    let channel_thread = store(&tmp)
        .thread_parent(&coder_slot)
        .unwrap()
        .expect("the slot hangs under its channel thread");

    let receipt = tick(&tmp, &worker, "2026-10-10T01:00:30Z", &channel_thread);
    assert_ne!(receipt.reason, ADVANCED, "{receipt:?}");

    let receipt = tick(&tmp, &worker, "2026-10-10T01:02:00Z", &channel_thread);
    assert_eq!(
        receipt.reason, ADVANCED,
        "silent for its whole window: the flow goes on without it, as declared: {receipt:?}"
    );
    assert_eq!(lapse_findings(&receipt), 1, "{receipt:?}");
    assert_eq!(
        worker.started("verifier") + queued_for(&tmp, &round).len(),
        1,
        "the verify step was opened once"
    );
}

// ---- the lease the hand-off grants -------------------------------------------------------------

#[test]
fn the_lease_handed_on_after_a_long_wait_is_not_already_expired() {
    let (tmp, engine, worker) = team();
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);
    tick(&tmp, &worker, SIX_HOURS_LATER, &channel_thread);

    the_copy_is_handed_on(&tmp, &engine, &worker, (&builder, &builder_thread));
    assert_eq!(worker.started("coder"), 1, "the premise: the coder started");

    let (holder, expires) = store(&tmp)
        .working_tree_lease_row()
        .unwrap()
        .expect("the copy was handed on, not freed");
    assert!(
        holder.ends_with(&round),
        "the working copy is the waiting round's now: {holder}"
    );
    assert!(
        expires.as_str() >= FRESH_WINDOW_ENDS,
        "the lease is bounded from the start, not by windows that ran out in the queue: it runs at \
         least to the end of the window the coder got when it started ({FRESH_WINDOW_ENDS}), and it \
         expires at {expires}"
    );
    assert_eq!(
        store(&tmp)
            .working_tree_holder(RELEASED)
            .unwrap()
            .as_deref(),
        Some(holder.as_str()),
        "the copy is held, not free for the next comer to take"
    );
}

// ---- a start that failed and will be retried has not been asked either ---------------------------

/// The coder's start fails on something waiting can fix, and the tick will retry it a minute
/// later. Its thirty-second window passes before the retry is due. The step has still not been
/// asked: the flow must not open the verifier, or the retry would start the coder beside it.
#[test]
fn a_failed_start_with_a_retry_due_does_not_lapse_and_its_retry_runs_alone() {
    let (tmp, engine, worker) = team_with(SHORT);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);
    tick(&tmp, &worker, SIX_HOURS_LATER, &channel_thread);
    worker.fail_the_next_coder_start(ErrorKind::Io);

    let armed = the_copy_is_handed_on(&tmp, &engine, &worker, (&builder, &builder_thread));
    assert_eq!(worker.started("coder"), 0, "the premise: the start failed");
    let failed = store(&tmp).failed_starts().unwrap();
    assert!(
        failed.len() == 1 && failed[0].retry_at.is_some(),
        "the premise: the failed start is on record with a retry due: {failed:?}"
    );
    assert!(
        !armed
            .iter()
            .any(|(thread, at)| thread == &channel_thread && at == SHORT_WINDOW_FROM_RELEASED),
        "a start that did not happen arms no look at a window that is not running: {armed:?}"
    );

    let receipt = tick(&tmp, &worker, BEFORE_THE_RETRY, &channel_thread);
    assert_ne!(
        receipt.reason, ADVANCED,
        "a start that failed and will be retried has not been asked: {receipt:?}"
    );
    assert_eq!(lapse_findings(&receipt), 0, "{receipt:?}");
    assert_eq!(worker.started("verifier"), 0);
    assert!(
        queued_for(&tmp, &round).is_empty(),
        "no verifier was opened"
    );

    tick(&tmp, &worker, AT_THE_RETRY, &channel_thread);
    assert_eq!(worker.started("coder"), 1, "the retry started the coder");
    assert_eq!(
        worker.started("verifier"),
        0,
        "and nothing of the run runs beside it"
    );

    // Its window runs from the retry that started it, and a started step that stays silent lapses.
    let receipt = tick(&tmp, &worker, INSIDE_THE_RETRIED_WINDOW, &channel_thread);
    assert_ne!(receipt.reason, ADVANCED, "{receipt:?}");
    let receipt = tick(&tmp, &worker, PAST_THE_RETRIED_WINDOW, &channel_thread);
    assert_eq!(receipt.reason, ADVANCED, "{receipt:?}");
    assert_eq!(lapse_findings(&receipt), 1, "{receipt:?}");
}

/// A start that failed on something waiting cannot fix is not retried. Nothing will ever start
/// it, so its window lapses as before and the flow moves on: holding it would stall the run with
/// nothing able to end the wait but a withdraw.
#[test]
fn a_failed_start_that_will_not_be_retried_still_lapses() {
    let (tmp, engine, worker) = team_with(SHORT);
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (_, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);
    tick(&tmp, &worker, SIX_HOURS_LATER, &channel_thread);
    worker.fail_the_next_coder_start(ErrorKind::Validation);

    the_copy_is_handed_on(&tmp, &engine, &worker, (&builder, &builder_thread));
    let failed = store(&tmp).failed_starts().unwrap();
    assert!(
        failed.len() == 1 && failed[0].retry_at.is_none(),
        "the premise: the failed start is on record and will not be retried: {failed:?}"
    );

    let receipt = tick(&tmp, &worker, BEFORE_THE_RETRY, &channel_thread);
    assert_eq!(
        receipt.reason, ADVANCED,
        "a start nothing will retry lapses on its window: {receipt:?}"
    );
}

// ---- a wait that outlasts the window is reported, not silent ------------------------------------

/// The queued step does not lapse, and the tick says why the flow is not moving: the step's window
/// has passed while its start still waits. Without it the receipt read `not_due` and nothing else,
/// however long the wait.
#[test]
fn a_tick_reports_a_step_that_still_waits_past_its_window() {
    let (tmp, engine, worker) = team();
    let (_builder, _) = a_holder_at_work(&engine, &worker);
    let (_, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);
    let coder_slot = queue_row(&tmp, "coder").and_then(|q| q.thread).unwrap();

    let receipt = tick(&tmp, &worker, "2026-10-10T01:00:30Z", &channel_thread);
    assert!(
        findings(&receipt, ConsequenceClass::StepStillWaiting).is_empty(),
        "inside its window, nothing to say: {receipt:?}"
    );

    let receipt = tick(&tmp, &worker, SIX_HOURS_LATER, &channel_thread);
    assert_eq!(
        findings(&receipt, ConsequenceClass::StepStillWaiting),
        vec![Some(coder_slot)],
        "past its window, the tick names the step that still waits: {receipt:?}"
    );
    assert_eq!(lapse_findings(&receipt), 0, "and does not call it silent");
}

// ---- a later step put back in line by the hand-off has not been asked either ---------------------

/// #38's shape: both steps of the run in line, the hand-off starts the coder and puts the verifier
/// back in line (`Promotions::deferred`) until the coder is over. The coder then falls silent past
/// its own window with its process still in the copy, and the verifier's window passes while it
/// waits. The coder's lapse would move the flow on to the verifier — which is already opened and
/// waiting. The verifier has not been asked, so it must not count as lapsed too: with both counted
/// as lapsed, the run would be consolidated past a step that never ran.
#[test]
fn a_later_step_put_back_in_line_does_not_lapse_while_it_waits() {
    let (tmp, engine, worker) = team();
    let (builder, builder_thread) = a_holder_at_work(&engine, &worker);
    let (round, channel_thread) = an_ordered_round_queued_behind_the_holder(&tmp, &engine);
    // Both steps in line, as a build before 1wep left them: the coder's row is out of the queue
    // while its window lapses, and goes back in its old place.
    let coder = queue_row(&tmp, "coder").unwrap();
    assert!(store(&tmp)
        .remove_working_tree_queue_entry(coder.id)
        .unwrap());
    tick(&tmp, &worker, VERIFIER_OPENED, &channel_thread);
    store(&tmp)
        .enqueue_working_tree(&coder, VERIFIER_OPENED)
        .unwrap();
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["coder".to_string(), "verifier".to_string()],
        "the premise: both steps of the run are in line"
    );
    let verifier_slot = queue_row(&tmp, "verifier").and_then(|q| q.thread).unwrap();

    the_holder_lets_go(&engine, &worker, &builder, &builder_thread, HANDED_ON);
    assert_eq!(worker.started("coder"), 1, "the premise: the coder started");
    assert_eq!(
        queued_for(&tmp, &round),
        vec!["verifier".to_string()],
        "the premise: the verifier waits for it"
    );

    // Past the coder's window (restarted when it started) and the verifier's (from when it was
    // opened). The coder's process is still in the copy, so the verifier keeps waiting.
    let receipt = tick(&tmp, &worker, PAST_THE_VERIFIERS_WINDOW, &channel_thread);
    // `not_due` is the tick's WAITING answer. Counting the verifier as lapsed too made the set
    // settled with no step after it, and the tick went for the consolidation instead (here
    // `no_return_address`, because the round was commissioned by hand).
    assert_eq!(
        receipt.reason, "not_due",
        "the run is not consolidated past a step that never ran: {receipt:?}"
    );
    assert_eq!(
        findings(&receipt, ConsequenceClass::StepStillWaiting),
        vec![Some(verifier_slot.clone())],
        "and the tick names the step that still waits: {receipt:?}"
    );
    assert_eq!(
        worker.started("verifier"),
        0,
        "the coder is still in the copy"
    );

    // The coder's process ends: the verifier that waited starts, in its own slot.
    let coder = worker.last_for("coder");
    worker.mark_gone(&coder.internal_session);
    engine
        .session_ended(
            caller("coder", PAST_THE_VERIFIERS_WINDOW),
            &coder.internal_session,
        )
        .expect("the session end is accepted");
    assert_eq!(worker.started("verifier"), 1, "the waiting verifier starts");
    assert_eq!(
        worker.last_for("verifier").reply_thread.as_deref(),
        Some(verifier_slot.as_str())
    );
}
