//! **The operation view says what IS, not what was** (nxf 6j6v.1vxs, nxf 6j6v.qmy6).
//!
//! `nxc status` is documented as *everything still going on here*, and after a day of real use it
//! was the opposite. Measured in the proving ground (4jgn.g90w) on 2026-09-06, with `nxc 0.84.0`
//! and the whole log that run produced:
//!
//! ```text
//! nxc status          424 lines
//! operations           13, every one of them listed
//! threads             411
//! nothing open in      9 of the 13
//! ```
//!
//! Every one of the thirteen stood on the root's `awaiting_human` alone, and that flag is derived
//! from two facts that never stop being true — the root asked, and the root was answered. Nothing
//! could ever take an operation out of that view again, so a reader asking *is anything hanging?*
//! got thirteen yeses, of which four were about something a person could act on. The reader who
//! measured it built a watchdog around `--json` instead, which is a view abdicating its own
//! question.
//!
//! Two changes, one command, and they are one unit because the second writes into the line the
//! first just re-decided:
//!
//! 1. **`live` stops counting `awaiting_human`** (6j6v.1vxs). What is left is what a reader can DO
//!    something about — an open thread, an unanswered hand-back, this device's working copy — and a
//!    finished operation is one `--all` away instead of permanently in the way.
//! 2. **each thread says what became of the session working on it** (6j6v.qmy6). `nxc session
//!    state` has answered that for ONE session since 6j6v.h383; it took a second call, which is the
//!    gap that item named and left open. Now the fact that separates *waiting for a human* from
//!    *hung* stands beside the session id it is about.
//!
//! Driven through the LIBRARY HANDLE, because the process half exists nowhere else: a worker's own
//! answer about a live session can only be supplied to an `Engine` (`engine-seam-test-rule`). The
//! terminal half is `tests/golden/messaging.trycmd`.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{StatusOperation, StatusScope, ThreadState};
use nexus_chat::orchestration::{Caller, SessionState};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-09-06T10:00:00Z";

/// A host worker that owns processes: it records what it was asked to start and answers the one
/// read the engine makes of it, which is the shape every real runtime has.
#[derive(Default)]
struct LiveSessionWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl LiveSessionWorker {
    fn only_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        let mut it = seen.iter().filter(|r| r.role.handle == handle);
        let first = it
            .next()
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone();
        assert!(it.next().is_none(), "expected exactly one trigger");
        first
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

    fn answers_liveness(&self) -> bool {
        true
    }
}

/// A worker resting on [`Worker::session_is_running`]'s DEFAULT — what every host that has not
/// implemented the method has, and the case nxf 6j6v.qmy6 had to decide what to hand back to.
#[derive(Default)]
struct SilentWorker;

impl Worker for SilentWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Ok(TriggerOutcome::Accepted)
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

const CODING: &str = "name: coding\nmembers: [coder]\nflow: sequential\nworking_tree: exclusive\n";
const REVIEW: &str = "name: review\nmembers: [reviewer]\nflow: parallel\n";

/// A workspace with `pm`, `coder` and `reviewer` declared, two channels, a live `pm` session to act
/// from, and the handle over it.
fn team(worker: Arc<dyn Worker>) -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("pm"), role("coder"), role("reviewer")],
        vec![channel(CODING), channel(REVIEW)],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    {
        let mut store = Workspace::resolve(None, tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store");
        // The PM has to be a REAL session: an answer travels back through the thread's return
        // address, and a human at a keyboard leaves none.
        store.create_pending_session("s-pm", "pm").unwrap();
        store.bind_session("s-pm", "real-pm").unwrap();
        // A SECOND pm session, and it is not decoration: the parent edge of a `send_to` is stamped
        // from the AMBIENT session's recorded position, so a second commission out of `s-pm` after
        // it already stands on a thread becomes a CHILD of the first operation rather than an
        // operation of its own. A test about two operations therefore needs two callers.
        store.create_pending_session("s-pm2", "pm").unwrap();
        store.bind_session("s-pm2", "real-pm2").unwrap();
    }
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine)
}

fn as_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

/// The PM opens a round in `to`; hand back (the member's internal session, the thread it owes an
/// answer on).
fn commission(
    engine: &Engine,
    worker: &LiveSessionWorker,
    to: &str,
    member: &str,
) -> (String, String) {
    commission_from(engine, worker, "s-pm", to, member)
}

/// [`commission`] out of a named caller session — see `team` for why a second operation needs one.
fn commission_from(
    engine: &Engine,
    worker: &LiveSessionWorker,
    from: &str,
    to: &str,
    member: &str,
) -> (String, String) {
    engine
        .send_to(
            as_session(from),
            SendToRequest {
                machine: None,
                to,
                body: "do the thing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let started = worker.only_for(member);
    let slot = started
        .reply_thread
        .clone()
        .expect("a commissioned step is told which thread it owes an answer on");
    (started.internal_session.clone(), slot)
}

fn answer(engine: &Engine, session: &str, thread: &str, escalate: bool) {
    engine
        .reply_thread(
            as_session(session),
            ReplyThreadRequest {
                machine: None,
                thread,
                body: "…",
                escalate,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the member answers");
}

fn listed(engine: &Engine, scope: StatusScope<'_>) -> Vec<StatusOperation> {
    engine.status(NOW, scope).expect("status reads").operations
}

// ---- (1) what the default listing is about (nxf 6j6v.1vxs) -------------------------------------

#[test]
fn a_finished_operation_leaves_the_default_listing_and_is_one_flag_away() {
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "review", "reviewer");

    // While the round is open the operation is exactly where it belongs.
    let open = listed(&engine, StatusScope::Workspace);
    assert_eq!(
        open.len(),
        1,
        "a round nobody has answered is going on: {open:?}"
    );
    assert!(open[0].live);
    let root = open[0].root.clone();

    answer(&engine, &coder, &slot, false);

    // Answered: the root now carries `awaiting_human`, which is the FLAG the old rule listed on…
    let all = listed(&engine, StatusScope::All(None));
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(
        all[0].threads[0].awaiting_human,
        "premise — this is the flag that used to keep it listed for ever: {all:?}"
    );
    assert!(
        !all[0].live,
        "…and nothing is expected of anybody any more, so it is not going on: {all:?}"
    );

    // …and the default listing is quiet.
    assert!(
        listed(&engine, StatusScope::Workspace).is_empty(),
        "a finished operation is not an answer to \"is anything still running here?\""
    );

    // Reachable, whole, and still saying it is finished — both ways in, without knowing the id for
    // one of them, which is what makes this a separation rather than a loss.
    assert_eq!(all[0].root, root);
    let by_id = listed(&engine, StatusScope::Threads(&[slot.as_str()]));
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].root, root);
    assert!(!by_id[0].live, "{by_id:?}");
}

#[test]
fn an_unanswered_hand_back_keeps_its_operation_listed_although_nothing_is_open() {
    // The case the new rule exists to keep: an escalation DISCHARGES the expectation, so the tree
    // has no open thread and the root reads `awaiting_human` exactly as a finished one does. It is
    // the opposite of finished — the chain has stopped and a human has to decide — and an escalation
    // holds the checkout while it stands.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "review", "reviewer");
    answer(&engine, &coder, &slot, true);

    let listing = listed(&engine, StatusScope::Workspace);
    assert_eq!(
        listing.len(),
        1,
        "an operation waiting on a decision must not fall out with the finished ones: {listing:?}"
    );
    assert_eq!(
        listing[0].open, 0,
        "nothing is owed — that is the trap: {listing:?}"
    );
    assert!(listing[0].needs_decision, "{listing:?}");
    assert!(listing[0].live, "{listing:?}");
    assert!(
        listing[0].threads[0].awaiting_human,
        "and it looks like a finished one at the root, which is why `live` cannot rest on that \
         flag: {listing:?}"
    );
}

#[test]
fn an_operation_still_holding_the_working_copy_keeps_its_operation_listed() {
    // The third term, and the one whose absence would be worst: a chain that finished without
    // giving the checkout back stops the NEXT one, and the default view is where a person looks for
    // that. `working_tree: exclusive` on the `coding` channel is what puts the claim there.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "coding", "coder");
    answer(&engine, &coder, &slot, true);

    let listing = listed(&engine, StatusScope::Workspace);
    assert_eq!(listing.len(), 1, "{listing:?}");
    assert!(
        listing[0].holds_working_tree,
        "the escalation parked the checkout, and that is a fact about the MACHINE, not only about \
         this round: {listing:?}"
    );
    assert!(listing[0].live, "{listing:?}");
}

#[test]
fn an_operation_whose_only_signal_is_a_failed_dead_end_stays_listed() {
    // The fourth term, and the one this rule was first written WITHOUT. `ThreadState::Orphaned` is
    // the derived place a failed consequence surfaces: the answer arrived, nothing was opened out
    // of it, and the parent that asked never moved on. Nothing is open, nothing was handed back,
    // no checkout is held — so on the first three terms alone the operation vanished from the
    // default view, silently, which is the class of thing this item exists to stop.
    //
    // It is rare on purpose: the state errs towards missing a failure rather than raising a false
    // alarm, and in the 411-thread workspace this item was measured on there was not one.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());

    // The pm asks the coder, and the coder answers: the ROOT is discharged.
    let (coder, root) = commission(&engine, &worker, "coder", "coder");
    answer(&engine, &coder, &root, false);

    // Out of that answered thread the coder asks the reviewer, and the reviewer answers. Nothing
    // upstream ever looked at THAT answer — the root was already discharged and never moved again.
    let branch = engine
        .send_to(
            as_session(&coder),
            SendToRequest {
                machine: None,
                to: "reviewer",
                body: "check it",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the branch opens");
    let reviewer = worker.only_for("reviewer").internal_session;
    answer(&engine, &reviewer, &branch.thread_id, false);

    let listing = listed(&engine, StatusScope::Workspace);
    assert_eq!(
        listing.len(),
        1,
        "a failed dead end is not \"still running\" and IS the shape of hanging this view is for: \
         {listing:?}"
    );
    assert_eq!(listing[0].open, 0, "{listing:?}");
    assert!(!listing[0].needs_decision, "{listing:?}");
    assert!(!listing[0].holds_working_tree, "{listing:?}");
    assert!(
        listing[0]
            .threads
            .iter()
            .any(|t| t.state == ThreadState::Orphaned),
        "premise — the dead end is the ONLY reason this operation is here: {listing:?}"
    );
    assert!(listing[0].live, "{listing:?}");
}

#[test]
fn the_channel_narrows_the_all_form_exactly_as_it_narrows_the_live_one() {
    // `--all` is the same SELECTION with the liveness filter off, which is why it takes the channel
    // rather than replacing it. Two finished operations in two channels; asking for one channel's
    // history must not hand back the other's.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (reviewer, review_slot) = commission(&engine, &worker, "review", "reviewer");
    answer(&engine, &reviewer, &review_slot, false);
    let (coder, coding_slot) = commission_from(&engine, &worker, "s-pm2", "coding", "coder");
    answer(&engine, &coder, &coding_slot, false);

    assert!(
        listed(&engine, StatusScope::Workspace).is_empty(),
        "both are finished"
    );
    assert_eq!(
        listed(&engine, StatusScope::All(None)).len(),
        2,
        "and both are reachable"
    );

    let one = listed(&engine, StatusScope::All(Some("review")));
    assert_eq!(one.len(), 1, "one channel, one operation: {one:?}");
    assert!(
        one[0].threads.iter().any(|t| t.thread_id == review_slot),
        "{one:?}"
    );
    assert!(
        listed(&engine, StatusScope::Channel("review")).is_empty(),
        "and the live form of the same narrowing stays quiet, because it is finished"
    );
}

// ---- (2) what became of the session working here (nxf 6j6v.qmy6) -------------------------------

#[test]
fn an_open_thread_says_whether_the_session_that_owes_it_is_still_running() {
    // The whole item in one call: `session` says whose transcript to follow, `session_state` says
    // whether anybody is still writing it. Before this the second fact cost a `session state` call
    // per thread.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "coding", "coder");
    worker.running.lock().unwrap().insert(coder.clone());

    let thinking = thread_of(&engine, &slot);
    assert_eq!(thinking.state, ThreadState::Open);
    assert_eq!(thinking.session.as_deref(), Some(coder.as_str()));
    assert_eq!(
        thinking.session_state,
        Some(SessionState::Running),
        "a live process behind the assignee session is what tells \"still thinking\" from \
         \"hung\": {thinking:?}"
    );

    // Killed hard: nothing announced an end, and no process answers. `unknown`, not `ended` — this
    // read states no fact nobody established, exactly as `nxc session state` does not.
    worker.running.lock().unwrap().remove(&coder);
    assert_eq!(
        thread_of(&engine, &slot).session_state,
        Some(SessionState::Unknown),
        "a hard kill is not an announcement"
    );

    // And the session's own word, which outranks the process question.
    engine
        .session_ended(as_session(&coder), &coder)
        .expect("the announcement is accepted");
    let hung = thread_of(&engine, &slot);
    assert_eq!(hung.state, ThreadState::Open, "nobody answered the thread");
    assert_eq!(
        hung.session_state,
        Some(SessionState::Ended),
        "an OPEN thread whose session is over is the definition of hung, and it now takes one \
         call to see: {hung:?}"
    );
}

#[test]
fn the_state_is_present_exactly_where_the_session_it_is_about_is() {
    // The pairing IS the contract: the state reported is the state OF the id printed beside it. So
    // a thread with no session must carry no state either — `unknown` there would mean "asked and
    // nobody answered", and nobody was asked about anything.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "coding", "coder");
    answer(&engine, &coder, &slot, false);

    let threads: Vec<_> = listed(&engine, StatusScope::All(None))
        .into_iter()
        .flat_map(|o| o.threads)
        .collect();
    assert!(!threads.is_empty(), "premise: there is a tree to look at");
    for t in &threads {
        assert_eq!(
            t.session.is_some(),
            t.session_state.is_some(),
            "a state without a session, or a session without a state: {t:?}"
        );
    }
    assert!(
        threads.iter().any(|t| t.session.is_none()),
        "premise: at least one thread of this tree was never put to work on — otherwise the loop \
         above proves only the easy half: {threads:?}"
    );
}

#[test]
fn a_report_says_whether_its_worker_could_answer_the_process_question_at_all() {
    // The decision nxf 6j6v.qmy6 had to make about hosts that cannot answer: a field reading
    // `unknown` for half the world is one a reader learns to ignore — unless the same call says
    // WHY. It is the same bit, spelled the same way, as on `SessionStateReport`, because two reads
    // answering one question must not qualify it differently.
    let live = Arc::new(LiveSessionWorker::default());
    let (_tmp_live, engine) = team(live.clone());
    let (coder, _slot) = commission(&engine, &live, "coding", "coder");
    live.running.lock().unwrap().insert(coder);
    assert!(
        engine
            .status(NOW, StatusScope::Workspace)
            .unwrap()
            .worker_answers_liveness,
        "this worker looks at processes, so its `unknown` would be evidence"
    );

    let (_tmp_silent, silent) = team(Arc::new(SilentWorker));
    silent
        .send_to(
            as_session("s-pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "do the thing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let report = silent.status(NOW, StatusScope::Workspace).unwrap();
    assert!(
        !report.worker_answers_liveness,
        "and one resting on the trait default admits it: {report:?}"
    );
    assert!(
        report
            .operations
            .iter()
            .flat_map(|o| &o.threads)
            .filter(|t| t.session.is_some())
            .all(|t| t.session_state == Some(SessionState::Unknown)),
        "…which is what makes every `unknown` beneath it honest rather than alarming: {report:?}"
    );
}

// ---- (3) and the rows are trimmed the same way the operations are (nxf 6j6v.1vxs) -------------

/// The chain the trim is about: `depth` hand-offs, each answered, with an ESCALATION at the far
/// end. Hand back the root's thread id.
///
/// This is the shape the proving ground actually produced — an operation is alive because of ONE
/// thread far down it, and everything above that thread is finished.
fn a_long_chain(engine: &Engine, worker: &LiveSessionWorker, depth: usize) -> String {
    let mut caller_session = "s-pm".to_string();
    let mut root: Option<String> = None;
    for step in 0..depth {
        // Alternating handles, because a session handing work to ITSELF is a different question
        // than the one this test asks — the shape here is a chain of hand-offs, which is what the
        // proving ground produced.
        let to = match step % 2 {
            0 => "coder",
            _ => "reviewer",
        };
        let receipt = engine
            .send_to(
                as_session(&caller_session),
                SendToRequest {
                    machine: None,
                    to,
                    body: "carry it on",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the hand-off posts");
        root.get_or_insert_with(|| receipt.thread_id.clone());
        let started = worker
            .seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a trigger");
        // Every step answers its own thread before handing on, so the whole chain is discharged —
        // except the deepest, which is HANDED BACK. That is the shape the proving ground produced
        // and the one the trim is about: an operation alive because of one thread at the far end,
        // with everything above it finished.
        answer(
            engine,
            &started.internal_session,
            &receipt.thread_id,
            step + 1 == depth,
        );
        caller_session = started.internal_session;
    }
    root.expect("a root")
}

#[test]
fn a_listing_prints_the_threads_that_say_why_and_counts_the_rest() {
    let worker = Arc::new(LiveSessionWorker::default());
    let (tmp, engine) = team(worker.clone());
    let root = a_long_chain(&engine, &worker, 6);

    let listing = listed(&engine, StatusScope::Workspace);
    assert_eq!(listing.len(), 1, "{listing:?}");
    assert_eq!(
        listing[0].threads.len(),
        6,
        "the whole chain is in the record: {listing:?}"
    );
    assert_eq!(
        listing[0].open, 0,
        "every step answered its own thread: {listing:?}"
    );
    assert!(
        listing[0].needs_decision,
        "and it is alive because of the far end: {listing:?}"
    );

    // The RECORD is untouched — every thread, every field. Only the terminal trims, so a tool that
    // reads `--json` sees exactly what it saw before.
    let text = String::from_utf8_lossy(
        &nxs_test_support::cargo_bin("nxc")
            .current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .arg("status")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();

    assert!(
        text.contains(&format!("operation {root}")) && text.contains("6 thread(s), 0 open"),
        "the operation line still carries the whole size: {text}"
    );
    assert!(
        text.contains("handed back") || text.contains("HANDED BACK"),
        "the thread the operation is alive for is one of the rows, not one of a hundred: {text}"
    );
    assert!(
        text.contains("answered thread(s) not shown"),
        "and the finished middle is counted rather than printed: {text}"
    );
    assert!(
        text.contains("4 answered thread(s) not shown"),
        "and it counts them rather than printing them: {text}"
    );
    assert_eq!(
        text.lines()
            .filter(|l| l.trim_start().starts_with("m-"))
            .count(),
        2,
        "exactly two thread rows — the root and the hand-back: {text}"
    );

    // `--all` is asked for every operation and gets every ROW of them too (review of PR #444, Test
    // Quality #3): the same six-deep chain, untrimmed. This used to be checked only against a
    // one-thread fixture, where "untrimmed" and "trimmed" produce the same output and the claim
    // proves nothing.
    let everything = String::from_utf8_lossy(
        &nxs_test_support::cargo_bin("nxc")
            .current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .args(["status", "--all"])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert_eq!(
        everything
            .lines()
            .filter(|l| l.trim_start().starts_with("m-"))
            .count(),
        6,
        "every thread of the tree under `--all` as well: {everything}"
    );
    assert!(
        !everything.contains("not shown"),
        "and nothing is summarised away there either: {everything}"
    );

    // `--thread` was asked for a TREE and gets all of it, which is what makes the trim a summary
    // rather than a loss.
    let whole = String::from_utf8_lossy(
        &nxs_test_support::cargo_bin("nxc")
            .current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .args(["status", "--thread", &root])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert_eq!(
        whole
            .lines()
            .filter(|l| l.trim_start().starts_with("m-"))
            .count(),
        6,
        "every thread of the tree: {whole}"
    );
    assert!(
        !whole.contains("not shown"),
        "and nothing is summarised away there: {whole}"
    );
}

/// An `nxc` invocation in this test's workspace, with the dry runtime pinned — no read here starts
/// anything, and a resolved sidecar would make the assertions a function of the machine.
fn nxc(tmp: &TempDir) -> assert_cmd::Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry");
    c
}

#[test]
fn a_substituted_reply_is_never_folded_into_the_count() {
    // **The gap the review of PR #444 found in `says_why`.** A reply the RUNTIME posted standing in
    // for an agent that never spoke is DISCHARGED and not escalated — so `state != Answered` misses
    // it and `escalated` misses it, and the first version of the row trim folded it into "N
    // answered thread(s) not shown". That is precisely the mistake nxf 6j6v.kffm exists to prevent,
    // made by a filter instead of by a reader: a round that looks answered and in which the agent
    // never spoke.
    let worker = Arc::new(LiveSessionWorker::default());
    let (tmp, engine) = team(worker.clone());
    let root = a_long_chain(&engine, &worker, 6);

    // Turn one of the hidden middle threads into a substituted one, the way the sidecar's teardown
    // does: the same discharged reply, carrying the marker.
    let middle = listed(&engine, StatusScope::Workspace)
        .remove(0)
        .threads
        .into_iter()
        .find(|t| t.depth > 0 && t.state == ThreadState::Answered && !t.escalated)
        .expect("the chain has an answered middle");
    {
        let mut store = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        store.set_wall_clock(NOW);
        store.post_message(&nexus_chat::model::MessageEnvelope {
            origin: "o".into(),
            channel_id: middle.channel_id.clone().unwrap_or_default(),
            sender: middle.expects.first().cloned().unwrap_or_default(),
            kind: nexus_chat::model::MessageKind::Report,
            priority: nexus_chat::model::Priority::Normal,
            disposition: nexus_chat::model::Disposition::InTurn,
            thread_id: Some(middle.thread_id.clone()),
            refs: nexus_chat::model::Refs {
                substituted: true,
                ..nexus_chat::model::Refs::default()
            },
            body: "sidecar: the session ended without answering".into(),
        });
    }
    let again = listed(&engine, StatusScope::Workspace).remove(0);
    let row = again
        .threads
        .iter()
        .find(|t| t.thread_id == middle.thread_id)
        .expect("still in the tree");
    assert!(
        row.substituted && !row.escalated && row.state == ThreadState::Answered,
        "premise — the exact combination neither of the other two reasons catches: {row:?}"
    );

    let text = String::from_utf8_lossy(
        &nxc(&tmp)
            .arg("status")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert!(
        text.contains(&middle.thread_id),
        "a reply the RUNTIME wrote is never one of the threads counted away: {text}"
    );
    assert!(
        text.contains("posted BY THE RUNTIME"),
        "…and the row still says so where a reader meets it: {text}"
    );
    let _ = root;
}

#[test]
fn the_two_forms_that_already_show_a_finished_tree_refuse_each_other() {
    // `--thread` shows one operation whether it runs or not, and `--all` shows every operation
    // whether it runs or not. Accepting both together would have to mean one of them silently
    // wins, so the parser refuses — in either order, because a conflict declared on one side only
    // is a conflict half the time.
    let worker = Arc::new(LiveSessionWorker::default());
    let (tmp, engine) = team(worker.clone());
    let (_coder, slot) = commission(&engine, &worker, "coding", "coder");

    for args in [
        vec!["status", "--thread", slot.as_str(), "--all"],
        vec!["status", "--all", "--thread", slot.as_str()],
    ] {
        let err = nxc(&tmp).args(&args).assert().failure();
        let text = String::from_utf8_lossy(&err.get_output().stderr).into_owned();
        assert!(
            text.contains("cannot be used with"),
            "{args:?} must be refused by the parser, not silently resolved: {text}"
        );
    }
}

#[test]
fn the_two_empty_answers_are_different_sentences() {
    // An empty LISTING is an answer to a narrow question and says where the rest went; an empty
    // `--all` is the whole answer and must not point anywhere, because there is nowhere to point.
    let worker = Arc::new(LiveSessionWorker::default());
    let (tmp, engine) = team(worker.clone());
    let (coder, slot) = commission(&engine, &worker, "coding", "coder");

    let quiet = String::from_utf8_lossy(
        &nxc(&tmp)
            .arg("status")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert!(
        !quiet.contains("nothing"),
        "premise: while the round is open the listing has something to say: {quiet}"
    );

    answer(&engine, &coder, &slot, false);
    let listing = String::from_utf8_lossy(
        &nxc(&tmp)
            .arg("status")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert!(
        listing.contains("nothing open") && listing.contains("--all"),
        "the filtered form says where the finished ones are: {listing}"
    );

    // …and on a workspace that really holds nothing, `--all` says so without sending anybody on.
    let (empty_tmp, _empty) = team(Arc::new(LiveSessionWorker::default()));
    let all = String::from_utf8_lossy(
        &nxc(&empty_tmp)
            .args(["status", "--all"])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert_eq!(
        all.trim(),
        "nothing here",
        "the unfiltered form has nowhere to point: {all}"
    );
}

/// One thread of the operation `slot` belongs to, read fresh through the handle.
fn thread_of(engine: &Engine, slot: &str) -> nexus_chat::facade::StatusThread {
    engine
        .status(NOW, StatusScope::Threads(&[slot]))
        .expect("status reads")
        .operations
        .into_iter()
        .flat_map(|o| o.threads)
        .find(|t| t.thread_id == slot)
        .unwrap_or_else(|| panic!("no thread {slot} in its own operation"))
}
