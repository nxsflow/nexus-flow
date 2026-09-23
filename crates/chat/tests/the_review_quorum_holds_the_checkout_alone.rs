//! **The shipped review quorum puts one session in the working copy at a time** — nxf 6j6v.2d00,
//! driven against the real declaration under `examples/role-runtime-v3/.nxs-personas/`.
//!
//! # The finding this answers
//!
//! Three of the four reviewers run the project's OWN quality gates, in their own session and on
//! purpose — `general` runs the gates the diff touched, `code-quality` "whatever of the project's
//! own quality gates your dimension needs", `test-quality` the test suite. `working_tree: exclusive`
//! protects that round from OTHER chains and never protected the members from each other: they are
//! one claim area, the first acquires and the rest inherit. So the declaration handed one working
//! copy and one `target/` to three concurrent `cargo` runs.
//!
//! Two of the same shape merely take cargo's own build lock and wait — correct, and three times as
//! slow. Two of DIFFERENT shapes overwrite each other's artifacts, and that is damage this repo has
//! MEASURED, not feared: its project memory records `no-concurrent-lean-build-during-tests`, where a
//! lean build overwrote `target/debug/nxs` and ~19 tests failed with `unrecognized subcommand
//! 'mcp'` — a shot artifact that reads like a code break. A quorum whose members each pick their own
//! gate selection produces exactly that divergence. And a reviewer that FIXES something (a format
//! run) writes into the checkout the other three are building in.
//!
//! # What was decided, and against what
//!
//! The ticket offered three ways. **`flow: sequential` on the quorum** is the one taken, and it is
//! the ticket's way (c) — re-cut the collection — without the price (c) was quoted at:
//!
//! * **(a) change nothing, name the price.** Refused. This collection is a TEMPLATE; the first
//!   thing anyone does with it is copy it. A template whose price is "three builds in one `target/`,
//!   and real damage the moment their gate selections diverge" teaches the damage.
//! * **(b) a new axis in `channels.yaml`** — make the unit of exclusion selectable, area or session.
//!   Refused HERE, not in general: the lease's queue is keyed by the claim area and the area is the
//!   operation (nxf 6j6v.8y6t), so serializing WITHIN one area needs a second lease level below it.
//!   That is a real engine change, and this finding does not pay for it while a declaration that
//!   already exists says exactly what the round needs.
//! * **(c) as the ticket phrased it** — one member runs the gates and hands the log to the other
//!   three. Refused: it buys serialization with the independence the collection explicitly wants
//!   ("judging real output rather than a gates log handed to you by someone else").
//!
//! Sequential costs neither. Every reviewer still runs the project's gates ITSELF, against the real
//! working copy, and forms its own verdict — and exactly one of them is in the checkout at a time,
//! because a `flow: sequential` channel opens step N+1 only once step N has settled AND its session
//! has ended (nxf 6j6v.10yb's liveness gate). Points 1, 2 and 3 above all go by construction.
//!
//! **What it does cost, said here because the declaration says it too**: wall-clock (four reviews in
//! series rather than four at once — though whenever their shapes matched, cargo's build lock was
//! already serializing them), and nxf 6j6v.ma7v's rule that on an ordered channel an ESCALATING
//! reply ends the chain instead of releasing the next step, so a reviewer that cannot run stops the
//! round rather than leaving three verdicts behind.
//!
//! # Why a test and not a comment
//!
//! `example_v3_valid.rs` loads and validates the shipped files; nothing there RUNS them. The
//! property at stake is not "the file says `flow: sequential`" — it is "no two of these sessions are
//! ever live at once", which is a fact about the engine driving that declaration. The worker below
//! does what the shipped `SidecarWorker` does and nothing else does: it DETACHES, so every session
//! it starts stays alive until something says otherwise. Under the dry worker — which reports no
//! session as running — this file could not fail.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-08-30T10:00:00Z";

fn v3_personas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/role-runtime-v3/.nxs-personas")
}

/// A worker that behaves like the shipped one in the ONE respect this file is about: it detaches, so
/// a session it started reports as running until the engine is told that session ended.
///
/// It also keeps the HIGH-WATER MARK of how many were live at once — which is the measurement, not
/// a derived convenience: asserting on the final state could not tell "one at a time" from "all four
/// at once and three already finished".
#[derive(Default)]
struct DetachedWorker {
    started: Mutex<Vec<TriggerRequest>>,
    live: Mutex<BTreeSet<String>>,
    most_live_at_once: Mutex<usize>,
}

impl DetachedWorker {
    fn started_handles(&self) -> Vec<String> {
        self.started
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn request_for(&self, handle: &str) -> TriggerRequest {
        self.started
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no session for {handle}: {:?}", self.started_handles()))
            .clone()
    }

    /// The session ends: the process is gone, so it no longer reports as running.
    fn ends(&self, internal_session: &str) {
        self.live.lock().unwrap().remove(internal_session);
    }
}

impl Worker for DetachedWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let mut live = self.live.lock().unwrap();
        live.insert(req.internal_session.clone());
        let mut peak = self.most_live_at_once.lock().unwrap();
        *peak = (*peak).max(live.len());
        drop(peak);
        drop(live);
        self.started.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.live.lock().unwrap().contains(internal_session)
    }
}

/// The workspace, seeded with the REAL shipped declaration — not a fixture shaped like it.
fn shipped_example(tmp: &TempDir) -> (Engine, Arc<DetachedWorker>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = nexus_chat::role::load_all_roles(&v3_personas_dir()).expect("the shipped roles");
    let channels =
        nexus_chat::channel::load_all_channels(&v3_personas_dir()).expect("the shipped channels");
    let defs = Definitions::new(roles, channels).expect("the shipped catalogue is sound");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(DetachedWorker::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (engine, worker)
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// One reviewer does what the declaration tells it to: it answers, and its session ends. Both, in
/// that order, because the flow waits for both — an answer alone leaves the process writing.
fn reviewer_finishes(engine: &Engine, worker: &DetachedWorker, handle: &str) {
    let req = worker.request_for(handle);
    let thread = req
        .reply_thread
        .clone()
        .unwrap_or_else(|| panic!("{handle} was told which thread it owes an answer on"));
    engine
        .reply_thread(
            Caller {
                session: Some(&req.internal_session),
                actor: None,
                now: Some(NOW),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "Gates run, findings below.\n\nReady to merge? yes",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap_or_else(|e| panic!("{handle}'s reply is posted: {e:?}"));
    worker.ends(&req.internal_session);
    engine
        .session_ended(caller(handle), &req.internal_session)
        .unwrap_or_else(|e| panic!("{handle}'s session end is announced: {e:?}"));
}

#[test]
fn the_four_reviewers_run_one_at_a_time_in_the_shipped_declaration() {
    // THE measurement nxf 6j6v.2d00's DoD asks for: never two gate runs in one `target/` at once.
    // It is taken at the only place that can decide it — how many sessions the runtime is holding
    // open in the checkout — because what each of them then does with `cargo` is the project's
    // business and not the engine's.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = shipped_example(&tmp);

    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "review",
                body: "review branch feat/x",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the quorum opens");

    assert_eq!(
        worker.started_handles(),
        vec!["general".to_string()],
        "opening the round starts the FIRST reviewer and nobody else"
    );

    for handle in ["general", "code-quality", "test-quality", "integrity"] {
        reviewer_finishes(&engine, &worker, handle);
    }

    let mut asked = worker.started_handles();
    asked.retain(|h| h != nexus_chat::channel::SYNTHESIS_HANDLE);
    assert_eq!(
        asked,
        vec![
            "general".to_string(),
            "code-quality".to_string(),
            "test-quality".to_string(),
            "integrity".to_string(),
        ],
        "…and every reviewer is still asked, in the order the declaration lists them — the round \
         is serialized, not shortened"
    );
    assert_eq!(
        *worker.most_live_at_once.lock().unwrap(),
        1,
        "and at no instant were two of them live in the same working copy: {:?}",
        worker.started_handles()
    );
}

/// A reviewer that CANNOT carry its dimension out, said the one way the engine reads.
fn reviewer_escalates(engine: &Engine, worker: &DetachedWorker, handle: &str) {
    let req = worker.request_for(handle);
    let thread = req.reply_thread.clone().expect("the step owes an answer");
    engine
        .reply_thread(
            Caller {
                session: Some(&req.internal_session),
                actor: None,
                now: Some(NOW),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "The build is broken before my dimension even starts — I cannot judge this.",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap_or_else(|e| panic!("{handle}'s escalation is posted: {e:?}"));
    worker.ends(&req.internal_session);
    engine
        .session_ended(caller(handle), &req.internal_session)
        .unwrap_or_else(|e| panic!("{handle}'s session end is announced: {e:?}"));
}

#[test]
fn a_reviewer_that_cannot_run_ends_the_round_instead_of_folding_a_verdict_out_of_one_opinion() {
    // **The combination the ORDERING introduced, on the shape the file now really has** (review of
    // PR #397, Test Quality). Two rules meet here for the first time in this collection, and only
    // here: nxf 6j6v.ma7v — on an ordered channel an escalating reply ends the chain instead of
    // releasing the next step — and nxf 6j6v.e9qj — an escalating set is never folded by a
    // `summarize` consolidator. Each is tested elsewhere; neither was tested TOGETHER, and
    // `channel_flow.rs`'s ordered-escalation case runs on a channel that declares no `summarize` at
    // all, so it cannot see the interaction.
    //
    // What makes the gap worth closing rather than noting: this quorum's consolidator is a MODEL,
    // told by the declaration to aggregate four verdicts into one merge-or-not line. If an escalating
    // set were folded anyway, an `opus` synthesizer would be handed one reviewer's "I cannot" and
    // asked for a verdict — and the requester would read `Ready to merge?` off a round in which
    // nobody reviewed anything. That is a wrong ANSWER, not a missing one.
    //
    // The price this asserts is the one `channels.yaml` names out loud: a reviewer that cannot run
    // stops the round rather than leaving the other three's verdicts behind. It is deliberate, and a
    // change of mind about it has to turn this red.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = shipped_example(&tmp);
    let board = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "review",
                body: "review branch feat/x",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the quorum opens")
        .thread_id;

    reviewer_escalates(&engine, &worker, "general");

    assert_eq!(
        worker.started_handles(),
        vec!["general".to_string()],
        "the chain ENDS: no further reviewer is commissioned, and no consolidation is spawned over \
         the one opinion that exists — {:?}",
        worker.started_handles()
    );
    let report = engine
        .status(NOW, nexus_chat::facade::StatusScope::Threads(&[&board]))
        .expect("the requester reads the round");
    let row = report
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .find(|t| t.thread_id == board)
        .expect("the board is in the report");
    assert!(
        row.escalated,
        "…and the round comes back as HANDED BACK, so the requester branches on a field instead of \
         reading a summary that was never written: {row:?}"
    );
}

#[test]
fn the_round_still_ends_in_one_synthesized_verdict_over_all_four() {
    // Serializing must not cost the thing the quorum is FOR. The declared consolidator still runs,
    // once, at the end — and it is handed every reviewer's own reply, so the verdict is folded from
    // four independent judgements exactly as it was when they ran at once.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = shipped_example(&tmp);
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "review",
                body: "review branch feat/x",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the quorum opens");
    for handle in ["general", "code-quality", "test-quality", "integrity"] {
        reviewer_finishes(&engine, &worker, handle);
    }

    let synth = worker.request_for(nexus_chat::channel::SYNTHESIS_HANDLE);
    for handle in ["general", "code-quality", "test-quality", "integrity"] {
        assert!(
            synth.message.contains(handle),
            "the fold is handed {handle}'s answer: {}",
            synth.message
        );
    }
    assert_eq!(
        worker
            .started_handles()
            .iter()
            .filter(|h| *h == nexus_chat::channel::SYNTHESIS_HANDLE)
            .count(),
        1,
        "exactly one consolidation, at the end"
    );
}
