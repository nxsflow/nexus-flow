//! **Deterministic hurdles at the channel supervisor** (nxf 6j6v.n92p) — the frame, both classes of
//! hurdle in it, and each of the four decisions the ticket asks the design to make.
//!
//! Everything here is driven through [`Engine`], which is the seam `manufakt.io` and `nexflow.it`
//! speak over — the rule that a new library seam is not finished without a test AT that seam, and
//! the one that has been the heaviest review finding three times running. The EXECUTOR behind the
//! declared half ([`nexus_chat::worker::SidecarWorker::run_precondition`], which runs a real `sh`
//! against a real working directory) is unit-tested in `worker.rs` against real processes; what is
//! tested here is what the supervisor DOES with its answer.
//!
//! The worker below records every question it is asked — hurdles and triggers, in one ordered log —
//! which is what makes decision 2 ("before the trigger, not after") assertable rather than merely
//! stated.

mod common;

use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::precondition::{PreconditionOutcome, PREVIOUS_STEP_STILL_WRITING};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-08-29T10:00:00Z";

/// One thing the engine asked this worker to do, in the order it asked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Asked {
    Hurdle(String),
    Trigger(String),
}

/// A worker whose hurdle answers are SCRIPTED, and which records the order of every question.
///
/// It stands in for a real executor deliberately: what the gate does with a verdict is the
/// behaviour under test, and a real `sh` would make the tests depend on the machine they run on for
/// the one thing this file is not about.
#[derive(Default)]
struct ScriptedWorker {
    asked: Mutex<Vec<Asked>>,
    /// Commands that must answer NO. Anything not named here exits zero and prints what
    /// [`ScriptedWorker::prints`] says (nothing, unless a test said otherwise).
    refuse: Mutex<Vec<String>>,
    /// `(command, nth)` — commands that answer NO only from their `nth` ask onward. See
    /// [`ScriptedWorker::refuse_from_ask`].
    refuse_from: Mutex<Vec<(String, usize)>>,
    /// What a PASSING command writes on stdout, per command — so a hurdle declaring
    /// `expect: "0"` can be made to pass without the worker having to know the declaration.
    prints: Mutex<Vec<(String, String)>>,
    /// Commands the worker declares itself unable to run at all.
    unrunnable: Mutex<Vec<String>>,
    running: Mutex<Vec<String>>,
    /// When set, every session this worker starts reports as RUNNING from the instant it is
    /// started — which is what the shipped [`nexus_chat::worker::SidecarWorker`] does, because it
    /// spawns a DETACHED `node` and answers `session_is_running` from that process's own pid file.
    ///
    /// It is off by default so every test written before it reads unchanged, and the tests that
    /// need it are the ones about a step opened while another one is live.
    detached: Mutex<bool>,
    /// The trigger requests themselves, kept beside the ordered log because one test needs a whole
    /// request (the slot thread a step was told to answer on).
    seen: Mutex<Vec<TriggerRequest>>,
}

impl ScriptedWorker {
    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    fn triggered(&self) -> Vec<String> {
        self.asked()
            .into_iter()
            .filter_map(|a| match a {
                Asked::Trigger(h) => Some(h),
                Asked::Hurdle(_) => None,
            })
            .collect()
    }

    fn refuse(&self, command: &str) {
        self.refuse.lock().unwrap().push(command.to_string());
    }

    /// Refuse `command` only from its `nth` ask onward (1-based) — what it takes to build a scene in
    /// which an EARLIER member cleared the same hurdle and started, and a LATER one is refused by it
    /// while that member is still alive. Refusing by command alone cannot express that: the same
    /// command is asked once per member, so a plain [`refuse`](Self::refuse) fires at member one and
    /// nothing is ever started.
    fn refuse_from_ask(&self, command: &str, nth: usize) {
        self.refuse_from
            .lock()
            .unwrap()
            .push((command.to_string(), nth));
    }

    fn cannot_run(&self, command: &str) {
        self.unrunnable.lock().unwrap().push(command.to_string());
    }

    fn prints(&self, command: &str, stdout: &str) {
        self.prints
            .lock()
            .unwrap()
            .push((command.to_string(), stdout.to_string()));
    }

    fn trigger_for(&self, handle: &str) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}: {:?}", self.asked()))
            .clone()
    }
}

impl Worker for ScriptedWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.asked
            .lock()
            .unwrap()
            .push(Asked::Trigger(req.role.handle.clone()));
        if *self.detached.lock().unwrap() {
            self.running
                .lock()
                .unwrap()
                .push(req.internal_session.clone());
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == internal_session)
    }

    fn run_precondition(&self, command: &str) -> PreconditionOutcome {
        self.asked
            .lock()
            .unwrap()
            .push(Asked::Hurdle(command.to_string()));
        if let Some(why) = self
            .unrunnable
            .lock()
            .unwrap()
            .iter()
            .find(|c| *c == command)
        {
            return PreconditionOutcome::Unavailable(format!("no way to run {why}"));
        }
        // How many times THIS command has been asked, counted off the ordered log this call has
        // already been appended to — so the first ask is 1.
        let asked_so_far = self
            .asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| matches!(a, Asked::Hurdle(c) if c == command))
            .count();
        let refused = self.refuse.lock().unwrap().iter().any(|c| c == command)
            || self
                .refuse_from
                .lock()
                .unwrap()
                .iter()
                .any(|(c, nth)| c == command && asked_so_far >= *nth);
        let stdout = match refused {
            true => " M src/lib.rs".to_string(),
            false => self
                .prints
                .lock()
                .unwrap()
                .iter()
                .find(|(c, _)| c == command)
                .map(|(_, out)| out.clone())
                .unwrap_or_default(),
        };
        PreconditionOutcome::Ran {
            status: Some(if refused { 1 } else { 0 }),
            stdout,
            stderr: String::new(),
        }
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

/// The ordered `coding` channel with two declared hurdles, in the shape the ticket sketches.
const HURDLED: &str = "\
name: coding
members: [coder, finisher]
flow: sequential
preconditions:
  - name: remote-not-ahead
    run: git rev-list --count HEAD..@{u}
    expect: \"0\"
  - name: tree-clean
    run: git status --porcelain
    expect: \"\"
";

/// [`HURDLED`] as a QUORUM — the same two declared hurdles with no `flow:`, so the members are
/// asked at once. Used to show that the brought-along exclusion above does not take the DECLARED
/// hurdles with it.
const HURDLED_QUORUM: &str = "\
name: coding
members: [coder, finisher]
preconditions:
  - name: remote-not-ahead
    run: git rev-list --count HEAD..@{u}
    expect: \"0\"
  - name: tree-clean
    run: git status --porcelain
    expect: \"\"
";

/// The same flow with NO hurdle declared — the control for the cost claim, and the channel the
/// brought-along hurdle is asserted on (it needs the working copy to itself, and nothing else).
const BARE: &str = "name: coding\nmembers: [coder, finisher]\nflow: sequential\n";
const EXCLUSIVE_BARE: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";
/// The same two members as a QUORUM — no `flow:`, so `Flow::Parallel`: one pass, every member
/// asked at once. The shape the shipped `review` channel has, and the one the brought-along hurdle
/// has to leave alone (nxf 6j6v.2d00's own finding).
const EXCLUSIVE_QUORUM: &str =
    "name: coding\nmembers: [coder, finisher]\nworking_tree: exclusive\n";

fn team(tmp: &TempDir, channel_yaml: &str) -> (Engine, Arc<ScriptedWorker>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder"), role("finisher")],
        vec![channel(channel_yaml)],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(ScriptedWorker::default());
    // The `remote-not-ahead` hurdle declares `expect: "0"`, so a PASS from it prints `0`.
    worker.prints("git rev-list --count HEAD..@{u}", "0");
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

fn as_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

fn open(engine: &Engine) -> nexus_chat::surface::SendToReceipt {
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
        .expect("the channel opens")
}

fn answer(engine: &Engine, session: &str, thread: &str) -> nexus_chat::orchestration::ReplyReceipt {
    engine
        .reply_thread(
            as_session(session),
            ReplyThreadRequest {
                machine: None,
                thread,
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted")
}

// ---- decision 2: BEFORE the trigger --------------------------------------------------------

#[test]
fn every_declared_hurdle_is_asked_before_the_step_is_triggered() {
    // The ordering IS the decision: after the trigger the session is up and the damage is up with
    // it. Both hurdles are asked, in the order they are declared, and only then is the step started.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    open(&engine);
    assert_eq!(
        worker.asked(),
        vec![
            Asked::Hurdle("git rev-list --count HEAD..@{u}".into()),
            Asked::Hurdle("git status --porcelain".into()),
            Asked::Trigger("coder".into()),
        ],
        "the hurdles are asked in declaration order, and the step starts only after both cleared"
    );
}

#[test]
fn a_channel_that_declares_no_hurdle_starts_no_extra_process() {
    // The cost claim's other half (decision 4): a channel that declares nothing pays nothing. The
    // brought-along hurdle is a store read plus a pid-file read, and starts no process either.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, BARE);
    open(&engine);
    assert_eq!(worker.asked(), vec![Asked::Trigger("coder".into())]);
}

// ---- decision 1: fail-closed ---------------------------------------------------------------

#[test]
fn a_hurdle_that_refuses_stops_the_channel_from_opening_and_starts_nobody() {
    // The fan-out arm: a live requester is waiting on this call, so a channel that started nobody
    // must not answer like one that did.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    worker.refuse("git status --porcelain");

    let err = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect_err("a refused hurdle refuses the send");
    assert_eq!(
        err.kind,
        nxs_foundation::error::ErrorKind::Forbidden,
        "not `validation`: nothing about the call or the declaration is malformed — a rule looked \
         at the world and said not yet: {err:?}"
    );
    assert!(
        err.msg.contains("tree-clean"),
        "it names the hurdle: {err:?}"
    );
    assert!(
        err.msg.contains("M src/lib.rs"),
        "and shows what the hurdle produced, so the requester can act on it: {err:?}"
    );
    assert!(
        worker.triggered().is_empty(),
        "and nobody was started: {:?}",
        worker.asked()
    );
}

#[test]
fn a_hurdle_that_cannot_be_run_stops_the_step_rather_than_letting_it_through() {
    // Decision 1's whole point: a hurdle that lets a doubt through is not a hurdle. The command did
    // not refuse — it could not be asked at all — and the step is stopped either way.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    worker.cannot_run("git rev-list --count HEAD..@{u}");

    let err = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect_err("an unaskable hurdle does not pass");
    assert!(
        err.msg.contains("remote-not-ahead"),
        "it names the hurdle that could not be checked: {err:?}"
    );
    assert!(worker.triggered().is_empty(), "{:?}", worker.asked());
}

#[test]
fn a_host_worker_that_declines_the_seam_refuses_every_declared_hurdle() {
    // The trait DEFAULT, at the seam an embedding host actually reaches. A host that has not
    // implemented `run_precondition` loses exactly the feature it did not implement — loudly, in
    // the answer to its own call, rather than by running the step anyway.
    struct Bare;
    impl Worker for Bare {
        fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
            Ok(TriggerOutcome::Accepted)
        }
    }
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder"), role("finisher")],
        vec![channel(HURDLED)],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(Bare)),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let err = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect_err("a hurdle nobody can check does not pass");
    assert!(
        err.msg.contains("remote-not-ahead"),
        "the first hurdle is the one reported: {err:?}"
    );
}

#[test]
fn the_first_refusal_wins_and_nothing_after_it_is_asked() {
    // One refusal names one hurdle, and the cost of a refused round is bounded by the hurdle that
    // refused rather than by the length of the list.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    worker.refuse("git rev-list --count HEAD..@{u}");
    worker.refuse("git status --porcelain");

    let err = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect_err("refused");
    assert!(err.msg.contains("remote-not-ahead"), "{err:?}");
    assert!(
        !err.msg.contains("tree-clean"),
        "the second hurdle was never asked, so it cannot be in the answer: {err:?}"
    );
    assert_eq!(
        worker.asked(),
        vec![Asked::Hurdle("git rev-list --count HEAD..@{u}".into())],
        "and it was never asked at all"
    );
}

// ---- decision 3: a DECLARED CLASS, not prose ------------------------------------------------

#[test]
fn a_refused_advance_reports_its_own_class_with_the_hurdle_and_its_output() {
    // The other arm of the fork: the reply that got here is durable, so the refusal rides that
    // reply's own receipt. `precondition_refused` is deliberately not `step_skipped` — one says the
    // plumbing failed, the other says a rule said not yet, and they call for opposite responses.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    let board = open(&engine).thread_id;
    let coder = worker.trigger_for("coder");
    let slot = coder.reply_thread.clone().expect("the step owes an answer");

    worker.refuse("git status --porcelain");
    let receipt = answer(&engine, &coder.internal_session, &slot);

    let refusal = receipt
        .warnings
        .iter()
        .find(|w| w.class == nexus_chat::orchestration::ConsequenceClass::PreconditionRefused)
        .unwrap_or_else(|| panic!("the refusal is reported: {:?}", receipt.warnings));
    let named = refusal
        .precondition
        .as_ref()
        .expect("the structured half is present on this class");
    assert_eq!(named.hurdle, "tree-clean");
    assert_eq!(
        named.kind,
        nexus_chat::precondition::RefusalKind::Verdict,
        "the project's own rule said no — as opposed to not being askable"
    );
    assert!(named.output.contains("M src/lib.rs"), "{named:?}");
    assert_eq!(
        refusal.thread.as_deref(),
        Some(board.as_str()),
        "the finding names the CHANNEL thread: nothing was minted for the step, and that absence \
         is the honest signal"
    );
    assert_eq!(
        worker.triggered(),
        vec!["coder".to_string()],
        "the second step never started: {:?}",
        worker.asked()
    );
}

#[test]
fn a_refusal_carries_no_wake_reason_because_nothing_was_being_started() {
    // The class's own contract, asserted rather than assumed: `reason` answers "why was a session
    // not put in motion", and here no session was being put in motion at all — the step was stopped
    // ahead of the trigger, which is the whole point.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    open(&engine);
    let coder = worker.trigger_for("coder");
    let slot = coder.reply_thread.clone().unwrap();
    worker.refuse("git status --porcelain");
    let receipt = answer(&engine, &coder.internal_session, &slot);
    let refusal = receipt
        .warnings
        .iter()
        .find(|w| w.precondition.is_some())
        .expect("reported");
    assert!(refusal.reason.is_none(), "{refusal:?}");
    assert!(refusal.session.is_none(), "{refusal:?}");
}

// ---- the BROUGHT-ALONG class -----------------------------------------------------------------

#[test]
fn the_brought_along_hurdle_holds_with_nothing_declared() {
    // The class's whole reason for existing: it is a property of the building block, so it applies
    // to a channel that declares no hurdle at all — and it closes the entrance nxf 6j6v.10yb could
    // not see, a requester's FOLLOW-UP starting a fresh pass while the previous one is still
    // writing. Nothing settles on that path, so none of the checks a settled set runs are reached.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, EXCLUSIVE_BARE);
    let board = open(&engine).thread_id;
    let coder = worker.trigger_for("coder");

    // The coder answers and its process keeps running — the measured shape of 10yb.
    worker
        .running
        .lock()
        .unwrap()
        .push(coder.internal_session.clone());
    answer(
        &engine,
        &coder.internal_session,
        &coder.reply_thread.clone().unwrap(),
    );

    // The requester asks again. This starts a NEW pass and settles nothing.
    let receipt = engine
        .reply_thread(
            caller("pm"),
            ReplyThreadRequest {
                machine: None,
                thread: &board,
                body: "another round please",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the follow-up is posted");

    let refusal = receipt
        .warnings
        .iter()
        .find_map(|w| w.precondition.as_ref())
        .unwrap_or_else(|| panic!("the brought-along hurdle refused: {:?}", receipt.warnings));
    assert_eq!(refusal.hurdle, PREVIOUS_STEP_STILL_WRITING);
    assert_eq!(refusal.kind, nexus_chat::precondition::RefusalKind::BuiltIn);
    assert_eq!(
        worker.triggered(),
        vec!["coder".to_string()],
        "and no second session was put into the same checkout: {:?}",
        worker.asked()
    );
}

#[test]
fn the_members_of_one_quorum_are_not_each_others_previous_step() {
    // **nxf 6j6v.2d00's own finding, and a regression this hurdle introduced when it moved to the
    // step-opening seam.** `previous-step-still-writing` asks whether an EARLIER STEP of this
    // channel thread is still writing. A `parallel` channel has exactly one pass — its members ARE
    // that step — so at a fan-out there is no earlier step to be writing, and the sessions the loop
    // is starting must not be read as one.
    //
    // Read as one, they were fatal rather than merely wrong: the fan-out opens the members in a
    // single call, so member 1 is already spawned and alive when member 2's gate runs, and a
    // refusal there fails the whole `send --to <channel>` with `Forbidden`. The shipped example's
    // four-member `review` quorum — `working_tree: exclusive`, every member running the project's
    // gates — could not be opened AT ALL against a real worker: one reviewer started, the round
    // died, and nothing was consolidated.
    //
    // Invisible to every other test in this file because they all declare `flow: sequential`, and
    // invisible to every channel test because the dry worker reports no session as running. The
    // worker here does what the shipped sidecar does: it detaches, so what it started is alive.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, EXCLUSIVE_QUORUM);
    *worker.detached.lock().unwrap() = true;

    open(&engine);

    assert_eq!(
        worker.triggered(),
        vec!["coder".to_string(), "finisher".to_string()],
        "every member of the quorum is asked: {:?}",
        worker.asked()
    );
}

#[test]
fn a_quorum_member_started_while_a_sibling_is_live_still_asks_the_declared_hurdles() {
    // The exclusion is scoped to the BROUGHT-ALONG hurdle and to THIS pass's own members — it is not
    // a general "skip the gate on a fan-out". A project's declared rules are asked for every member,
    // exactly as decision 2 says, and one that refuses still stops the round.
    //
    // **This test asserted none of that until the review of PR #397 said so** (Code Quality and Test
    // Quality, independently). It refused the hurdle outright, so the refusal fired at member ONE,
    // nothing was ever started, and no sibling was ever alive — the one condition its name is about.
    // Mutating the fix it was meant to guard left it green. That is `a-test-name-is-a-claim` in this
    // very branch, one day after the branch wrote a ticket note about the rule.
    //
    // The scene the name promises needs a hurdle that CLEARS for the first member and refuses for
    // the second, which is what `refuse_from_ask` is for. Then member one really is started and
    // really is alive (the worker detaches) when member two's gate runs — and what is asserted is
    // that the declared hurdle was still asked there, and still stopped the round.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &format!("{HURDLED_QUORUM}working_tree: exclusive\n"));
    *worker.detached.lock().unwrap() = true;
    worker.refuse_from_ask("git status --porcelain", 2);

    let err = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect_err("a refused hurdle refuses the send");
    assert!(err.msg.contains("tree-clean"), "{err:?}");
    assert_eq!(
        worker.triggered(),
        vec!["coder".to_string()],
        "THE PREMISE, and what the old shape never had: the first member cleared the same hurdle \
         and IS running — so the second member's gate ran beside a live sibling: {:?}",
        worker.asked()
    );
    assert_eq!(
        worker.asked(),
        vec![
            Asked::Hurdle("git rev-list --count HEAD..@{u}".into()),
            Asked::Hurdle("git status --porcelain".into()),
            Asked::Trigger("coder".into()),
            Asked::Hurdle("git rev-list --count HEAD..@{u}".into()),
            Asked::Hurdle("git status --porcelain".into()),
        ],
        "…and BOTH declared hurdles were asked again for it, in declaration order, before it was \
         refused — the exclusion took the brought-along hurdle out of the way and nothing else"
    );
}

#[test]
fn a_brought_along_refusal_asks_no_declared_hurdle_at_all() {
    // The ORDER, as an interaction rather than as two separate facts (PR #391 review, Test Quality
    // #3). The brought-along class is what holds when nobody declared anything, so a declaration
    // can never be what decides whether it is asked — and asking it first is also what stops a
    // channel whose previous step is still writing from spending processes to find that out.
    //
    // Only observable where BOTH could fire: a channel that claims the working copy AND declares
    // hurdles, with a session still alive. Nothing in the two tests either side of this one can see
    // it, which is exactly why it needs its own.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &format!("{HURDLED}working_tree: exclusive\n"));
    let board = open(&engine).thread_id;
    let coder = worker.trigger_for("coder");
    worker
        .running
        .lock()
        .unwrap()
        .push(coder.internal_session.clone());
    answer(
        &engine,
        &coder.internal_session,
        &coder.reply_thread.clone().unwrap(),
    );

    // Both hurdles would refuse if they were asked — so if either had been, the refusal would name
    // it instead of the brought-along one, and the assertion below on the log would see the call.
    worker.refuse("git rev-list --count HEAD..@{u}");
    worker.refuse("git status --porcelain");
    let asked_before = worker.asked().len();

    let receipt = engine
        .reply_thread(
            caller("pm"),
            ReplyThreadRequest {
                machine: None,
                thread: &board,
                body: "another round please",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the follow-up is posted");

    let refusal = receipt
        .warnings
        .iter()
        .find_map(|w| w.precondition.as_ref())
        .unwrap_or_else(|| panic!("something refused: {:?}", receipt.warnings));
    assert_eq!(refusal.hurdle, PREVIOUS_STEP_STILL_WRITING);
    assert_eq!(
        worker.asked().len(),
        asked_before,
        "and NOT ONE declared hurdle was asked — the brought-along class answered first and \
         short-circuited the list: {:?}",
        worker.asked()
    );
}

#[test]
fn the_brought_along_hurdle_leaves_a_shared_channel_alone() {
    // The same control 10yb draws: two live sessions in a `shared` channel are not a defect — the
    // damage is two WRITERS in one checkout — so a channel that claims nothing is untouched.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, BARE);
    let board = open(&engine).thread_id;
    let coder = worker.trigger_for("coder");
    worker
        .running
        .lock()
        .unwrap()
        .push(coder.internal_session.clone());
    answer(
        &engine,
        &coder.internal_session,
        &coder.reply_thread.clone().unwrap(),
    );

    let receipt = engine
        .reply_thread(
            caller("pm"),
            ReplyThreadRequest {
                machine: None,
                thread: &board,
                body: "another round please",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the follow-up is posted");
    assert!(
        receipt.warnings.iter().all(|w| w.precondition.is_none()),
        "a shared channel is not gated on liveness: {:?}",
        receipt.warnings
    );
}

// ---- the property that moved here from `channel_flow.rs` -------------------------------------

#[test]
fn a_pass_whose_first_step_a_hurdle_refused_delivers_nothing_rather_than_the_pass_before_it() {
    // **The stale answer the hq71 review traced, at the one shape that still reaches it.** A step
    // that is stopped before it mints a thread leaves the new pass with no slot of its own, and
    // `current_pass` would then anchor on the PREVIOUS pass's step-one slot — settled, complete,
    // about a request nobody made now. What stops it being delivered is the ORDERING in
    // `supervisor_hand_out_next_turn`: the channel thread is re-armed only once step one has
    // actually started, so a pass that never began leaves the register where the last consolidation
    // put it and `supervisor_consider_set` returns early.
    //
    // It used to be driven by editing a declaration between the two passes; since nxf 6j6v.n92p an
    // operation is bound to the declarations it opened under, so a refused HURDLE is the cause that
    // remains — and it is the better one, because it is a supported state rather than a broken file.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, HURDLED);
    let board = open(&engine).thread_id;

    // Pass one, both steps.
    let coder = worker.trigger_for("coder");
    answer(
        &engine,
        &coder.internal_session,
        &coder.reply_thread.clone().unwrap(),
    );
    let finisher = worker.trigger_for("finisher");
    let closing = answer(
        &engine,
        &finisher.internal_session,
        &finisher.reply_thread.clone().unwrap(),
    );
    assert!(
        closing.completed.is_some(),
        "pass one delivered: {closing:?}"
    );

    // The requester asks again, and step one is refused.
    worker.refuse("git status --porcelain");
    let again = engine
        .reply_thread(
            caller("pm"),
            ReplyThreadRequest {
                machine: None,
                thread: &board,
                body: "another round please",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the follow-up is posted");

    assert!(
        again.warnings.iter().any(|w| w.precondition.is_some()),
        "the refusal is reported on the receipt of the call that caused it: {:?}",
        again.warnings
    );
    assert!(
        again.completed.is_none(),
        "and the previous pass is NOT handed back as the answer to this request: {again:?}"
    );
    assert_eq!(
        worker.triggered(),
        vec!["coder".to_string(), "finisher".to_string()],
        "nothing new was started: {:?}",
        worker.asked()
    );
}
