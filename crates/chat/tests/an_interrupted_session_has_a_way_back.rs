//! **A session that stopped because the model went away gets a defined way back** (nxf 6j6v.npy3).
//!
//! Three causes — an exhausted quota, an unreachable provider, a broken network — and one state:
//! *the model is not available to this session for a while, and nobody did anything wrong.* Before
//! this item the engine had no word for it, so the runtime's teardown posted an escalation in the
//! session's own name — *"I cannot carry this out"* — which is false, and the operation stopped on
//! `NEEDS DECISION` with nothing able to restart it.
//!
//! ## What was measured, and why the margin is the point
//!
//! 2026-09-12, a real planning run: of thirty sessions, exactly one hit the weekly window. The
//! engine's whole knowledge of it afterwards was `state: "ended"` — a clean, ordinary end — with its
//! thread reading `answered` and nothing anywhere `escalated`, `substituted` or `stale`. The entire
//! channel record of the workspace was searched and the interruption was not in it.
//!
//! It looked harmless only because the window closed **0.6 seconds AFTER** that session's `nxc
//! reply` landed. One second earlier and the result of three complete review rounds would have been
//! replaced by the sentence "I cannot carry this out", on a round that had in fact finished
//! thinking. And the probability is not evenly spread: a quota strikes the EXPENSIVE turns, and the
//! expensive turns are the judgements.
//!
//! ## The order these cases are in is the order the acceptance asks for
//!
//! *Erst sichtbar machen, dann fortsetzen.* Saying THAT it happened and WHEN it lifts comes first,
//! because a hold nobody can read is not a hold; continuing it comes second. So do these tests.
//!
//! Everything here drives the LIBRARY HANDLE — `engine-seam-test-rule`: the facts live on that seam,
//! and a worker's own answers (is a session running, can this runtime continue one) can only be
//! supplied there.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::StatusScope;
use nexus_chat::orchestration::{
    Caller, ResumeOutcome, ResumeRequest, ResumeScope, SessionScope, SessionState,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-09-12T09:53:52Z";
/// After the weekly window in the measured case lifts.
const LATER: &str = "2026-09-14T12:00:01Z";
const UNTIL: &str = "2026-09-14T12:00:00Z";

/// A worker that records its triggers and answers the two questions a host owns: whether a session
/// has a live process, and whether this runtime can CONTINUE a conversation it began.
#[derive(Default)]
struct HostWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    resumes: bool,
}

impl HostWorker {
    /// A worker in front of a runtime that continues its own conversations — the shipped sidecar's
    /// answer, and the one every case here but `a_runtime_that_cannot_continue…` runs on.
    fn continuing() -> HostWorker {
        HostWorker {
            resumes: true,
            ..HostWorker::default()
        }
    }

    fn only_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone()
    }

    fn triggers_for(&self, handle: &str) -> Vec<TriggerRequest> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .cloned()
            .collect()
    }
}

impl Worker for HostWorker {
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

    fn resumes_sessions(&self) -> bool {
        self.resumes
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn workspace(worker: Arc<dyn Worker>) -> (TempDir, Engine) {
    workspace_with(worker, TimerConfig::Disabled)
}

fn workspace_with(worker: Arc<dyn Worker>, timer: TimerConfig) -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine)
}

/// `NXC_TIMER_LOG` is process-global env, so every test here that reads it serializes against the
/// others — `a_clock_watches_the_liveness_gate.rs` takes exactly this precaution for exactly this
/// reason.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A `DryTimer` pointed at a fresh log, for the duration of one test.
struct TimerLog<'a> {
    #[allow(dead_code)]
    lock: MutexGuard<'a, ()>,
    path: std::path::PathBuf,
}

impl TimerLog<'_> {
    fn open(dir: &std::path::Path) -> TimerLog<'_> {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join("timer.log");
        std::env::set_var("NXC_TIMER_LOG", &path);
        TimerLog { lock, path }
    }

    /// `(deadline, command)` for every job armed, in the order they were armed.
    fn armed(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("schedule "))
            .filter_map(|l| {
                let deadline = l
                    .split_whitespace()
                    .find_map(|f| f.strip_prefix("deadline="))?
                    .to_string();
                let command = l.split_once(" command=")?.1.to_string();
                Some((deadline, command))
            })
            .collect()
    }
}

impl Drop for TimerLog<'_> {
    fn drop(&mut self) {
        std::env::remove_var("NXC_TIMER_LOG");
    }
}

fn caller_at<'a>(actor: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(now),
    }
}

fn caller(actor: &str) -> Caller<'_> {
    caller_at(actor, NOW)
}

/// Commission the coder and hand back (its internal session, the thread it owes an answer on).
fn commission(engine: &Engine, worker: &HostWorker) -> (String, String) {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "review the three verdicts and consolidate them",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the persona is commissioned");
    let t = worker.only_for("coder");
    let thread = t
        .reply_thread
        .clone()
        .expect("a commission tells its role which thread it owes an answer on");
    (t.internal_session.clone(), thread)
}

/// The runtime's announcement: the model went away, here is which window and when it lifts.
fn announce(engine: &Engine, session: &str, until: Option<&str>) {
    engine
        .session_interrupted(
            caller("coder"),
            session,
            "seven_day",
            until,
            "You've hit your weekly limit · resets 2pm (Europe/Berlin)",
        )
        .expect("the hold is recorded");
    engine
        .session_ended(caller("coder"), session)
        .expect("and the process is over either way");
}

fn resume(engine: &Engine, now: &str, scope: ResumeScope<'_>, force: bool) -> ResumeOutcome {
    engine
        .resume_interrupted(caller_at("pm", now), ResumeRequest { scope, force })
        .expect("the resume decides something")
        .outcome
}

// ---- FIRST: it can be READ, which is the half that did not exist at all ----------------------

#[test]
fn the_operational_view_says_that_a_session_stopped_at_a_boundary_and_when_it_lifts() {
    // The measured gap, in one assertion: before this, every one of these reads said "a clean,
    // ordinary end" about a session that had run into a wall.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));

    let report = engine
        .session_state(SessionScope::Session(&session))
        .expect("a minted session can be asked about");
    let standing = &report.sessions[0];
    // The session HAS ended — that stays true, and every host matching on the enum keeps working.
    assert_eq!(standing.state, SessionState::Ended);
    // …and beside it, the fact that was readable nowhere.
    let hold = standing
        .interrupted
        .as_ref()
        .expect("the hold is on the session read");
    assert_eq!(hold.limit, "seven_day");
    assert_eq!(hold.until.as_deref(), Some(UNTIL));
    assert_eq!(hold.at, NOW);
    assert!(hold.on_hold());

    // And the same fact, spelled the same way, on the board-wide read — two readers of one question
    // must not qualify it differently.
    let status = engine
        .status(NOW, StatusScope::Threads(&[&thread]))
        .expect("the board renders");
    let op = &status.operations[0];
    assert!(op.interrupted, "the operation carries the mark: {op:?}");
    assert!(
        !op.needs_decision,
        "and is NOT calling a human to a chain with nothing wrong with it"
    );
    let row = op
        .threads
        .iter()
        .find(|t| t.thread_id == thread)
        .expect("the commissioned thread is on the board");
    assert_eq!(
        row.interrupted.as_ref().map(|h| h.limit.as_str()),
        Some("seven_day")
    );
}

#[test]
fn a_hold_whose_runtime_named_no_instant_is_still_recorded_and_says_the_instant_is_missing() {
    // Absent is a real answer and must never be filled in with a guess: what it costs is the
    // automatic way back, and inventing one would start a session at a moment nobody stated.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, _thread) = commission(&engine, &worker);
    announce(&engine, &session, None);

    let hold = engine
        .session_state(SessionScope::Session(&session))
        .expect("readable")
        .sessions[0]
        .interrupted
        .clone()
        .expect("the hold is recorded whether or not an instant came with it");
    assert_eq!(hold.until, None);
    assert!(
        !hold.is_due("2099-01-01T00:00:00Z"),
        "and it never falls due on its own, however long anybody waits"
    );
}

#[test]
fn an_unminted_session_is_not_found_rather_than_silently_recorded() {
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let err = engine
        .session_interrupted(caller("coder"), "m-nobody", "seven_day", None, "x")
        .expect_err("a callback naming a session this workspace never minted is a mistake");
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn announcing_the_same_hold_twice_leaves_one() {
    // A retried teardown must not make one closed window look like two.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, _thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));
    announce(&engine, &session, Some(UNTIL));

    let hold = engine
        .session_state(SessionScope::Session(&session))
        .expect("readable")
        .sessions[0]
        .interrupted
        .clone()
        .expect("still exactly one hold");
    assert_eq!(hold.at, NOW, "the FIRST announcement's terms stand");
}

// ---- THEN: the way back, and every check it makes before starting anything --------------------

#[test]
fn nothing_on_hold_is_a_clean_no_op_so_the_verb_is_safe_to_run_twice() {
    // The background job and a human can both arrive; neither may fail because the other was first.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, _thread) = commission(&engine, &worker);
    assert_eq!(
        resume(&engine, LATER, ResumeScope::Session(&session), false),
        ResumeOutcome::NothingOnHold
    );
}

#[test]
fn a_round_that_was_answered_before_the_window_closed_is_not_run_again() {
    // **THE MEASURED CASE.** The quota struck 0.6 s after this session's own reply landed: the
    // judgement was durable and the obligation discharged. A resume that had not looked would have
    // paid for the whole round a second time and posted a second verdict over the first.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    engine
        .reply_thread(
            caller("coder"),
            nexus_chat::surface::ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "all three reviews agree: needs_rework",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the answer lands");
    announce(&engine, &session, Some(UNTIL));

    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::AlreadyAnswered
    );
    assert_eq!(
        worker.triggers_for("coder").len(),
        1,
        "nothing was started a second time"
    );
    // …and the hold is closed, so nothing comes back for it again.
    assert!(engine
        .session_state(SessionScope::Session(&session))
        .expect("readable")
        .sessions[0]
        .interrupted
        .is_none());
}

#[test]
fn a_session_that_is_already_running_is_left_alone() {
    // The single-process guard's fact, read rather than re-implemented (nxf 6j6v.7qtf). It is what
    // keeps a human resuming early from colliding with the background job arriving at the reset
    // time — the two processes that rule exists to keep out of one working copy.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));
    worker.running.lock().unwrap().insert(session.clone());

    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::AlreadyRunning
    );
    assert_eq!(worker.triggers_for("coder").len(), 1);
    // The hold STANDS: nothing took it up, so it must not look as though something had.
    assert!(engine
        .session_state(SessionScope::Session(&session))
        .expect("readable")
        .sessions[0]
        .interrupted
        .is_some());
}

#[test]
fn before_the_window_lifts_nothing_is_started_and_force_is_what_overrides_it() {
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));

    assert_eq!(
        resume(&engine, NOW, ResumeScope::Thread(&thread), false),
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        }
    );
    assert_eq!(worker.triggers_for("coder").len(), 1, "nothing started");

    // **The owner's case**: *"Vielleicht hat der Nutzer noch einen anderen Account und will direkt
    // weiter machen."* The record knows when the window lifts for the account that hit it, and not
    // what the person at the terminal has arranged since — so this is a report to a human, never a
    // refusal of one.
    assert_eq!(
        resume(&engine, NOW, ResumeScope::Thread(&thread), true),
        ResumeOutcome::Resumed
    );
    assert_eq!(worker.triggers_for("coder").len(), 2);
}

#[test]
fn a_runtime_that_cannot_continue_a_conversation_refuses_by_name() {
    // A `false` here is a DECLINE, and the refusal is the whole value of it: continuing on such a
    // runtime would start a session from NOTHING under a name that has a transcript, so the
    // continued session would not see the work it already did and would do it again. The transcript
    // is the protection against exactly that, and this is the state where it is silently absent.
    let worker = Arc::new(HostWorker::default()); // resumes_sessions() == false, the trait default
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    // It got as far as a real runtime session — so there IS a conversation whose loss would matter.
    engine
        .bind_runtime_session(&session, "sdk-abc")
        .expect("bound");
    announce(&engine, &session, Some(UNTIL));

    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::ProviderCannotResume
    );
    assert_eq!(worker.triggers_for("coder").len(), 1);

    // **AND THE HOLD IS CLOSED RATHER THAN LEFT STANDING FOREVER** (independent review of PR #476,
    // Integrity & Robustness #2). Whether a runtime continues its own conversations is a FIXED fact
    // about this workspace, so a hold left open on it would be refused in identical words by every
    // future resume, human and scheduled alike — a dead end wearing an "ON HOLD" label, which reads
    // like something is coming. Nothing is.
    assert!(
        engine
            .session_state(SessionScope::Session(&session))
            .expect("readable")
            .sessions[0]
            .interrupted
            .is_none(),
        "the hold must be closed: nothing will ever be able to take it up"
    );
    // …and a person is told, because the decision that is left — take the work up by hand, or
    // commission it again — is theirs and not the engine's. `tell_the_root_thread` is best-effort
    // and records a `FailedConsequence` when the message does not land, so an empty `warnings` IS
    // the assertion that it did. (What the message SAYS is pinned next door, in
    // `an_interrupted_operation_hands_the_working_copy_on.rs`, where the store can be read
    // directly — this seam's own thread read is membership-gated.)
    let receipt = engine
        .resume_interrupted(
            caller_at("pm", LATER),
            ResumeRequest {
                scope: ResumeScope::Thread(&thread),
                force: false,
            },
        )
        .expect("decides");
    assert_eq!(
        receipt.outcome,
        ResumeOutcome::NothingOnHold,
        "and the closed hold means a second attempt has nothing left to refuse"
    );
    assert!(receipt.warnings.is_empty(), "{:?}", receipt.warnings);
}

#[test]
fn a_session_that_never_reached_a_runtime_is_started_fresh_rather_than_refused() {
    // The other side of that line, and it is not a special case but the honest reading of it: a
    // session with no bound runtime session never produced anything, so there is no history to lose
    // and starting it is exactly right. Refusing here would strand a hold nothing could ever take
    // up, on the very worker default most hosts have.
    let worker = Arc::new(HostWorker::default());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));

    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::Resumed
    );
}

#[test]
fn a_resumed_session_continues_with_its_own_runtime_conversation() {
    // **Continuing, not repeating.** `TurnTerms::runtime_retries` states the reason in its own
    // words — *the files, the messages, the pull requests all happen a second time* — which is why
    // that retry only ever covers a run the model never touched. Continuing is SAFER than
    // restarting, because the transcript carries every tool call WITH its result: a session that
    // sees its own `nxf create` and the id it returned does not create the ticket again.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    engine
        .bind_runtime_session(&session, "sdk-abc")
        .expect("bound");
    announce(&engine, &session, Some(UNTIL));

    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::Resumed
    );
    let again = worker.triggers_for("coder");
    assert_eq!(again.len(), 2);
    assert_eq!(
        again[1].resume_real.as_deref(),
        Some("sdk-abc"),
        "the trigger continues the conversation it began, rather than opening a new one"
    );
    assert_eq!(
        again[1].internal_session, session,
        "…under the same internal session, so the transcript, the depth budget and the thread \
         position all stay one thing"
    );
    // The text it is handed says the three things it needs, and says them the same way every time.
    let told = &again[1].message;
    assert!(told.contains("interrupted"), "{told}");
    assert!(
        told.contains("CHECK THE WORLD"),
        "the one thing the transcript cannot protect it from — a call whose result never arrived: \
         {told}"
    );
    assert!(
        told.contains("not starting over"),
        "and that its own history is in front of it: {told}"
    );

    // Taken up — so the board stops reporting a hold nobody is acting on.
    assert!(engine
        .session_state(SessionScope::Session(&session))
        .expect("readable")
        .sessions[0]
        .interrupted
        .is_none());

    // **And a second arm finds nothing left to do** (independent review of PR #476, Test Quality
    // #3) — the end-to-end shape of the race this verb has to survive: a human resumes, and the
    // background job's own firing arrives a moment later. It must be a clean no-op, not a second
    // start.
    assert_eq!(
        resume(&engine, LATER, ResumeScope::Thread(&thread), false),
        ResumeOutcome::NothingOnHold
    );
    assert_eq!(
        worker.triggers_for("coder").len(),
        2,
        "the second arm started nothing"
    );
}

#[test]
fn a_hold_is_addressable_by_thread_and_by_session_and_both_find_the_same_one() {
    // A person types the thread (it is what `nxc status` shows them); the background job carries
    // the session (the hold is keyed on it). One verb, one record.
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, thread) = commission(&engine, &worker);
    announce(&engine, &session, Some(UNTIL));

    let by_thread = engine
        .resume_interrupted(
            caller_at("pm", NOW),
            ResumeRequest {
                scope: ResumeScope::Thread(&thread),
                force: false,
            },
        )
        .expect("decides");
    let by_session = engine
        .resume_interrupted(
            caller_at("pm", NOW),
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides");
    assert_eq!(by_thread.session, Some(session.clone()));
    assert_eq!(by_session.session, Some(session));
    assert_eq!(by_thread.outcome, by_session.outcome);
}

// ---- the AUTOMATIC half: something has to come back on its own -------------------------------

#[test]
fn the_way_back_is_armed_twice_over_one_hold_and_the_first_firing_is_soon() {
    // **The half a human never runs.** The owner asked for both — *"Ich finde die Idee gut, dass
    // der Hintergrunddienst die Arbeit wieder aufnimmt ... Ein manuelles Starten sollte auch
    // unterstuetzt werden"* — and the service is the only clock in the system.
    //
    // TWO firings over one hold, and that is not a detail: the first comes at the process grace
    // and secures the working copy, because a weekly window is up to a week and holding this
    // workspace's only checkout that long would stop everything else. The second is the instant
    // the runtime named.
    let tmp_for_log = TempDir::new().unwrap();
    let log = TimerLog::open(tmp_for_log.path());
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace_with(worker.clone(), TimerConfig::Dry);
    let (session, thread) = commission(&engine, &worker);

    let receipt = engine
        .session_interrupted(caller("coder"), &session, "seven_day", Some(UNTIL), "quota")
        .expect("the hold is recorded");
    assert!(
        receipt.warnings.is_empty(),
        "arming must not fail quietly: {:?}",
        receipt.warnings
    );
    // The FIRST firing: soon, and not at `until`.
    assert_eq!(receipt.armed_for.as_deref(), Some("2026-09-12T09:54:02Z"));
    let first = log.armed();
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(first[0].0, "2026-09-12T09:54:02Z");
    assert_eq!(
        first[0].1,
        format!("nxc resume --session {session}"),
        "the job names the SESSION — the hold is keyed on it, and a person types the thread"
    );

    // That firing finds the window still closed, secures what it can, and re-arms for the instant
    // the runtime named. `--force` is deliberately absent from the job's own argv.
    let second = engine
        .resume_interrupted(
            caller_at("pm", NOW),
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides");
    assert_eq!(
        second.outcome,
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        }
    );
    assert_eq!(second.armed_for.as_deref(), Some(UNTIL));
    let both = log.armed();
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(both[1].0, UNTIL);
    assert_eq!(both[1].1, format!("nxc resume --session {session}"));
    assert_eq!(thread, second.thread.unwrap());
}

#[test]
fn a_hold_with_no_instant_is_armed_once_and_then_waits_for_a_person() {
    // Nothing may be armed for a moment nobody stated. The near firing still happens — the working
    // copy has to be handed on either way — and after it nothing is scheduled, which is exactly
    // what the receipt and the terminal line both say out loud.
    let tmp_for_log = TempDir::new().unwrap();
    let log = TimerLog::open(tmp_for_log.path());
    let worker = Arc::new(HostWorker::continuing());
    let (_tmp, engine) = workspace_with(worker.clone(), TimerConfig::Dry);
    let (session, _thread) = commission(&engine, &worker);
    engine
        .session_interrupted(caller("coder"), &session, "rate_limit", None, "quota")
        .expect("recorded");
    assert_eq!(log.armed().len(), 1, "the near firing is armed either way");

    let r = engine
        .resume_interrupted(
            caller_at("pm", NOW),
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides");
    assert_eq!(r.outcome, ResumeOutcome::NotYet { until: None });
    assert_eq!(r.armed_for, None);
    assert_eq!(
        log.armed().len(),
        1,
        "nothing more is armed: there is no moment to arm it for"
    );
}
