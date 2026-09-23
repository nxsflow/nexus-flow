//! **A sequential step ends when its SESSION ends, not when its message arrives** (nxf 6j6v.10yb),
//! and the callback that says so (`Engine::session_ended` / `nxc session ended`).
//!
//! The defect, measured in the foreign project `watch-bundestag` on nxc 0.63.0 against a channel
//! declared `members: [coder, finisher]`, `flow: sequential`, `working_tree: exclusive`:
//!
//! ```text
//! 00:16:33  the coder REPLIES ("ZWISCHENSTAND — nicht der Abschluss")
//! 00:16:36  >>> the finisher's session starts <<<
//! 00:34:27  >>> the coder's session ends — 18 minutes after its answer <<<
//! ```
//!
//! Three sessions overlapped in ONE working copy, one of them for thirteen minutes, and one left an
//! unreverted mutation probe in the tree the next was about to commit. The coder did nothing wrong:
//! its declaration forbids it to wait on work it commissioned, and a session that ends without
//! answering gets a failure posted in its name — so answering mid-work was the only move it had.
//! Two meanings had collided on one verb, "I hand the turn back" and "this step is over", and only
//! the engine can tell them apart.
//!
//! Everything here is driven through the LIBRARY HANDLE, which is the seam this behaviour has to
//! hold on (`engine-seam-test-rule`) and the only one where the worker's own answer about a live
//! session can be supplied: [`WorkerConfig::Custom`] carries a worker that knows which of its
//! sessions are still running, exactly as a host worker driving a real runtime does.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-24T00:16:33Z";

/// A worker that records every trigger AND answers the one read the engine makes of it — which of
/// the sessions it started are still running.
///
/// This is the shape a real host worker has: it owns the processes, so it is the only thing that
/// can answer. `SidecarWorker` answers from the pid file its own session lock writes; this one
/// answers from a set a test moves, which is the same fact by a different route.
#[derive(Default)]
struct LiveSessionWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl LiveSessionWorker {
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    /// The one trigger for `handle`, which every case here expects to be unique.
    fn only_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        let mut it = seen.iter().filter(|r| r.role.handle == handle);
        let first = it
            .next()
            .unwrap_or_else(|| panic!("no trigger for {handle}, saw {:?}", self.handles()))
            .clone();
        assert!(
            it.next().is_none(),
            "expected exactly one trigger for {handle}"
        );
        first
    }

    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
}

impl Worker for LiveSessionWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
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

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// The `coding` channel of the measured incident: an ordered flow over two steps that needs the
/// working copy to itself.
const EXCLUSIVE_CODING: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";

/// The same flow WITHOUT the exclusive claim — the control. Nothing in this item may change it.
const SHARED_CODING: &str = "name: coding\nmembers: [coder, finisher]\nflow: sequential\n";

/// The SAME two members asked at once, still claiming the working copy — the shape nothing in the
/// suite combined until the review of PR #361 asked for it (Code Quality #2 / Test Quality #1).
const EXCLUSIVE_PARALLEL: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: parallel\nworking_tree: exclusive\n";

/// A workspace with the two personas and one channel declared, plus the handle over it and the
/// worker it drives.
fn team(channel_yaml: &str) -> (TempDir, Engine, Arc<LiveSessionWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder"), role("finisher")],
        vec![channel(channel_yaml)],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(LiveSessionWorker::default());
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

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// The coder answering from inside its own session — `actor` unset, so the identity is resolved
/// from the session map exactly as a spawned persona's `nxc reply` resolves it.
fn as_session<'a>(session: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

/// Open the round and hand back (the coder's internal session, the thread it owes an answer on).
fn open_round(engine: &Engine, worker: &LiveSessionWorker) -> (String, String) {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let coder = worker.only_for("coder");
    let slot = coder
        .reply_thread
        .clone()
        .expect("a commissioned step is told which thread it owes an answer on");
    (coder.internal_session.clone(), slot)
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

// ---- the defect ------------------------------------------------------------------------------

#[test]
fn the_next_step_does_not_start_while_the_previous_step_s_session_is_still_running() {
    let (_tmp, engine, worker) = team(EXCLUSIVE_CODING);
    let (coder_session, slot) = open_round(&engine, &worker);
    // 00:16:33 in the timeline: the coder is still writing, and says so.
    worker.mark_running(&coder_session);

    answer(
        &engine,
        &coder_session,
        &slot,
        "ZWISCHENSTAND — nicht der Abschluss.",
    );

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "the finisher must NOT have been started: the coder's answer settles a message board, and \
         its process is still writing into the working copy this channel declared it needs alone"
    );
}

#[test]
fn announcing_the_session_end_is_what_starts_the_next_step() {
    let (_tmp, engine, worker) = team(EXCLUSIVE_CODING);
    let (coder_session, slot) = open_round(&engine, &worker);
    worker.mark_running(&coder_session);
    answer(
        &engine,
        &coder_session,
        &slot,
        "ZWISCHENSTAND — nicht der Abschluss.",
    );
    // The premise this test's NAME rests on: nothing has started yet. Without it the assertion
    // below passes just as well when the reply already started the finisher, which is the state
    // this whole item exists to remove.
    assert_eq!(worker.handles(), vec!["coder".to_string()]);

    // 00:34:27: the process is over, and the only thing that can say so is the session itself.
    worker.running.lock().unwrap().remove(&coder_session);
    let receipt = engine
        .session_ended(caller("coder"), &coder_session)
        .expect("the announcement is accepted");

    assert_eq!(receipt.session, coder_session);
    assert_eq!(receipt.thread.as_deref(), Some(slot.as_str()));
    assert_eq!(
        receipt.reason, "considered",
        "the announcement reached a supervised step and re-asked its channel"
    );
    assert!(receipt.warnings.is_empty(), "{:?}", receipt.warnings);
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "the flow advances now — and only now"
    );
}

#[test]
fn the_recorded_end_alone_releases_the_step_even_while_the_process_is_reported_alive() {
    // The reason `session_map.ended` exists beside `Worker::session_is_running` rather than instead
    // of it: the announcement is made BY the ending session, whose process is necessarily still
    // alive while it speaks. A design that asked only the worker would decline at exactly the moment
    // it is being told the truth, and nothing would ever ask again.
    let (_tmp, engine, worker) = team(EXCLUSIVE_CODING);
    let (coder_session, slot) = open_round(&engine, &worker);
    worker.mark_running(&coder_session);
    answer(&engine, &coder_session, &slot, "done");
    assert_eq!(worker.handles(), vec!["coder".to_string()]);

    // Deliberately NOT removed from `running`: the worker keeps answering "alive".
    engine
        .session_ended(caller("coder"), &coder_session)
        .expect("the announcement is accepted");

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "the session's own word about its end outranks a liveness read taken while it speaks"
    );
}

#[test]
fn a_second_announcement_for_the_same_session_starts_nothing_a_second_time() {
    let (_tmp, engine, worker) = team(EXCLUSIVE_CODING);
    let (coder_session, slot) = open_round(&engine, &worker);
    worker.mark_running(&coder_session);
    answer(&engine, &coder_session, &slot, "done");
    assert_eq!(worker.handles(), vec!["coder".to_string()], "the gate held");
    engine
        .session_ended(caller("coder"), &coder_session)
        .unwrap();
    let before = worker.handles();
    assert_eq!(
        before,
        vec!["coder".to_string(), "finisher".to_string()],
        "the first announcement did open the next step — otherwise there is no repeat to test"
    );

    engine
        .session_ended(caller("coder"), &coder_session)
        .expect("a repeat is a clean no-op, not an error");

    assert_eq!(
        worker.handles(),
        before,
        "a retried teardown must not open the next step twice"
    );
}

// ---- what must NOT change --------------------------------------------------------------------

#[test]
fn a_channel_that_does_not_claim_the_working_copy_still_advances_on_the_answer_alone() {
    // The gate is scoped to the declaration that says "this work must not share the tree". On a
    // `shared` channel two live sessions are not a defect — the damage 10yb measured is two writers
    // in one checkout — so this path is untouched, down to the trigger order.
    let (_tmp, engine, worker) = team(SHARED_CODING);
    let (coder_session, slot) = open_round(&engine, &worker);
    worker.mark_running(&coder_session);

    answer(&engine, &coder_session, &slot, "done");

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "nothing about a shared channel changes"
    );
}

#[test]
fn a_worker_that_cannot_answer_the_liveness_question_leaves_the_flow_exactly_as_it_was() {
    // `Worker::session_is_running`'s DEFAULT is `false`, and the direction is the whole reason it is
    // safe to add a method to a trait every host implements: a runtime that cannot tell answers
    // "not running", and its channels advance on the answer exactly as they always did. `true` by
    // default would let one unknowable session stall a channel with nothing able to release it.
    //
    // This worker overrides only `trigger`, so what runs below IS the default body.
    #[derive(Default)]
    struct SilentWorker(Mutex<Vec<TriggerRequest>>);
    impl Worker for SilentWorker {
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.0.lock().unwrap().push(req);
            Ok(TriggerOutcome::Accepted)
        }
    }

    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder"), role("finisher")],
        vec![channel(EXCLUSIVE_CODING)],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(SilentWorker::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .unwrap();

    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .unwrap();
    let (session, slot) = {
        let seen = worker.0.lock().unwrap();
        let coder = seen.first().expect("the first step was started").clone();
        (
            coder.internal_session,
            coder.reply_thread.expect("the step is told its thread"),
        )
    };

    answer(&engine, &session, &slot, "done");

    let handles: Vec<String> = worker
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.role.handle.clone())
        .collect();
    assert_eq!(
        handles,
        vec!["coder".to_string(), "finisher".to_string()],
        "an unanswerable liveness question must not stall the flow"
    );
}

// ---- the parallel shape, which the item left open and the review asked to pin ------------------

#[test]
fn a_parallel_channel_that_claims_the_working_copy_is_gated_too_and_that_is_deliberate() {
    // nxf 6j6v.10yb asks out loud whether this rule should hold for a parallel quorum and does not
    // decide it; the review of PR #361 confirmed that in the built code it DOES — the gate sits in
    // `supervisor_consider_set` before the `Flow::Sequential` branch, and `channel_needs_working_tree`
    // does not consult `flow` at all — and that nothing pinned it in either direction. This is that
    // pin, plus the reasoning it was missing:
    //
    // **Concurrency is what a parallel channel is FOR, and the gate does not take it away.** Both
    // members are commissioned at once and both run at once; what waits is the CONSOLIDATION. That
    // is the right place for it, because consolidating is what wakes the requester back into this
    // checkout — the same collision, one level up, and exactly the class 6j6v.10yb measured. The
    // cost is a delivery that arrives when the last member's process exits rather than when its
    // message lands; the alternative is a requester resumed into a tree a slow member is still
    // writing.
    //
    // If a future item decides the other way, this test is where the decision has to be changed —
    // which is the point of pinning a behaviour that was reached by construction.
    let (_tmp, engine, worker) = team(EXCLUSIVE_PARALLEL);
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "both of you, now",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "BOTH members start at once — the gate must not have turned a parallel fan-out into a queue"
    );

    let coder = worker.only_for("coder");
    let finisher = worker.only_for("finisher");
    let (cs, ct) = (coder.internal_session, coder.reply_thread.unwrap());
    let (fs, ft) = (finisher.internal_session, finisher.reply_thread.unwrap());
    worker.mark_running(&cs);
    worker.mark_running(&fs);

    answer(&engine, &cs, &ct, "coder done");
    answer(&engine, &fs, &ft, "finisher done");

    // THE ASSERTION, and it has to be about the BOARD rather than about the worker: a `pass_through`
    // consolidation on this shape triggers nothing at all (the requester is a human and has no
    // session to wake), so a trigger count is green whether or not the gate held and would pin
    // nothing. The channel thread's own register is what moves when the round consolidates.
    let channel_thread = {
        let store = seed_store(&_tmp);
        store
            .thread_parent(&ct)
            .expect("the slot has a parent")
            .expect("the channel thread")
    };
    let complete = |tmp: &TempDir| {
        seed_store(tmp)
            .thread_quorum(&channel_thread, NOW)
            .unwrap()
            .expect("a board")
            .complete
    };
    assert!(
        !complete(&_tmp),
        "both members answered and BOTH are still writing — the round must not have consolidated \
         and woken its requester back into the checkout they are still in"
    );

    engine.session_ended(caller("coder"), &cs).unwrap();
    assert!(
        !complete(&_tmp),
        "one of the two is still writing, so the round is still not over"
    );

    engine.session_ended(caller("finisher"), &fs).unwrap();
    assert!(
        complete(&_tmp),
        "…and once BOTH processes are over, it consolidates"
    );
}

/// A store opened separately from the handle — for the board state a worker cannot report.
fn seed_store(dir: &TempDir) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, dir.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

// ---- the callback's own contract ---------------------------------------------------------------

#[test]
fn an_internal_session_this_workspace_never_minted_is_not_found() {
    // `session bind`'s contract, for `session bind`'s reason: the sidecar's callbacks are a
    // contract, and one that silently succeeded about a session nobody knows would hide a spec or
    // environment mismatch instead of reporting it.
    let (_tmp, engine, _worker) = team(EXCLUSIVE_CODING);
    let err = engine
        .session_ended(caller("coder"), "s-never-minted")
        .expect_err("an unknown session is refused");
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(
        err.to_string().contains("s-never-minted"),
        "the refusal names what was looked for, got: {err}"
    );
}

#[test]
fn a_session_that_stood_in_no_supervised_step_is_a_clean_no_op_that_says_so() {
    // A persona summoned directly owes its answer to one named party; there is no set whose settling
    // this could change. It still records the end — the row is the fact — and reports which branch
    // it took, because "nothing happened" and "nothing was there" are different answers.
    let (_tmp, engine, worker) = team(EXCLUSIVE_CODING);
    let receipt = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "change the button label",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the persona is summoned");
    let session = receipt.session.expect("a persona summon mints a session");
    assert_eq!(worker.handles(), vec!["coder".to_string()]);

    let ended = engine
        .session_ended(caller("coder"), &session)
        .expect("the announcement is accepted");

    assert_eq!(ended.reason, "not_a_channel_step");
    assert!(
        ended.thread.is_some(),
        "it still names where the session stood"
    );
}
