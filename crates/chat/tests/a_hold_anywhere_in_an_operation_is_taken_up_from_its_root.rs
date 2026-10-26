//! **`nxc resume --thread <root>` takes up a hold standing anywhere in that operation** (nxf
//! 6j6v.cn4p).
//!
//! Measured on 2026-10-25 at ~21:29: a fresh coder session of a coding run hit the weekly limit on
//! its FIRST turn. `nxc status` showed the operation `interrupted: true`, and `nxc resume --thread
//! <root> --force` answered `nothing_on_hold` — even after the owner had logged in with another
//! account. The run was withdrawn and sent again.
//!
//! The cause was the lookup, not the hold: a thread resolved only through the sessions recorded ON
//! that thread, and a channel root has no session of its own — its steps run on threads of their
//! own beneath it. The root is the id `nxc status` shows a person, so the one id they had answered
//! nothing, while the board said something was on hold under it.
//!
//! The second half is a race of the same family: a resumed session that runs straight into the
//! limit again announces a NEW hold before the resume has stamped the old one. The store keeps one
//! standing hold per session, so the new announcement was swallowed, and the stamp then closed the
//! only hold there was — nothing on hold, nothing coming.
//!
//! Driven through the LIBRARY HANDLE (`engine-seam-test-rule`) with a [`WorkerConfig::Custom`]
//! worker that records every start and can run something while a start is in progress.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::{ErrorKind, NxfError};
use nexus_chat::facade::{Reader, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ResumeOutcome, ResumeReceipt, ResumeRequest, ResumeScope,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
use nexus_chat::worker::{
    TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-25T21:29:00Z";
/// The weekly window of the field case, still closed at [`NOW`].
const UNTIL: &str = "2026-10-28T17:00:00Z";
/// The window the resumed session runs into straight away.
const NEXT_UNTIL: &str = "2026-11-04T17:00:00Z";

/// The field case's shape: build, then verify, one ordered run.
const CODING: &str = "name: coding\nmembers: [coder, verifier]\nflow: sequential\n";
/// Two members asked at once — two sessions of one operation that can both be on hold.
const REVIEW: &str = "name: review\nmembers: [r1, r2]\nflow: parallel\n";

type Meanwhile = Box<dyn FnOnce(&TriggerRequest) + Send>;

/// A worker that records every start, says no session of its is running, continues conversations,
/// and runs [`Meanwhile`] once while a start is in progress.
#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    meanwhile: Mutex<Option<Meanwhile>>,
    /// Fail the next start with a transient error.
    fail_next: Mutex<bool>,
    /// Panic in the next start — a process that dies after the resume began and before the spawn.
    panic_next: std::sync::atomic::AtomicBool,
}

impl RecordingWorker {
    fn starts_of(&self, handle: &str) -> Vec<TriggerRequest> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .cloned()
            .collect()
    }

    fn first_of(&self, handle: &str) -> TriggerRequest {
        self.starts_of(handle)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no start of {handle}"))
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if self
            .panic_next
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            panic!("the process died before the session was spawned");
        }
        let meanwhile = self.meanwhile.lock().unwrap().take();
        if let Some(meanwhile) = meanwhile {
            meanwhile(&req);
        }
        if std::mem::take(&mut *self.fail_next.lock().unwrap()) {
            return Err(TriggerError::Failed(NxfError::new(
                ErrorKind::Io,
                "the runtime could not be started",
            )));
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, _internal_session: &str) -> bool {
        false
    }

    fn answers_liveness(&self) -> bool {
        true
    }

    fn resumes_sessions(&self) -> bool {
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
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [CODING, REVIEW]
        .into_iter()
        .map(|y| serde_yaml::from_str(y).expect("channel parses"))
        .collect();
    let roles = ["coder", "verifier", "r1", "r2"]
        .into_iter()
        .map(role)
        .collect();
    let defs = Definitions::new(roles, channels).expect("catalogue");
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

/// Commission a channel as the PM and hand back its root — the id `nxc status` shows.
fn commission(engine: &Engine, to: &str) -> String {
    engine
        .send_to(
            caller("pm", NOW),
            SendToRequest {
                machine: None,
                to,
                body: "build 6j6v.cn4p",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission is accepted")
        .thread_id
}

/// The runtime's two callbacks for a session the model went away from: the hold, then the end.
fn hit_the_limit(engine: &Engine, handle: &str, session: &str, until: &str) {
    engine
        .session_interrupted(
            caller(handle, NOW),
            session,
            "rate_limit",
            Some(until),
            "You've hit your weekly limit · resets Oct 28 at 5pm",
        )
        .expect("the hold is recorded");
    engine
        .session_ended(caller(handle, NOW), session)
        .expect("and the process is over either way");
}

fn resume(engine: &Engine, scope: ResumeScope<'_>, force: bool) -> ResumeReceipt {
    engine
        .resume_interrupted(caller("o/human", NOW), ResumeRequest { scope, force })
        .expect("the resume decides something")
}

fn operation_interrupted(engine: &Engine, root: &str) -> bool {
    let report = engine
        .status(
            Reader::Handle("o/human"),
            NOW,
            StatusScope::Threads(&[root]),
        )
        .expect("the board renders");
    let op = report
        .operations
        .iter()
        .find(|op| op.threads.iter().any(|t| t.thread_id == root))
        .expect("the operation is on the board");
    op.interrupted
}

/// Run `f` against the workspace from a connection of its own, as the background service or a
/// second terminal would — the way another process acts while the first is in the middle of a start.
fn in_another_process<R>(
    root: &Path,
    worker: &dyn Worker,
    now: &str,
    f: impl FnOnce(&orchestration::Ctx<'_>, &mut nexus_chat::store::ChatStore) -> R,
) -> R {
    let ws = Workspace::resolve(None, root).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(root).expect("declarations");
    let ctx = orchestration::Ctx {
        now,
        origin: &origin,
        actor: "o/human",
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
    f(&ctx, &mut store)
}

/// The resumed session's runtime announcing that the model went away again.
fn announce_from_another_process(root: &Path, session: &str, until: &str) {
    in_another_process(root, &RecordingWorker::default(), NOW, |ctx, store| {
        orchestration::session_interrupted(
            ctx,
            store,
            session,
            "rate_limit",
            Some(until),
            "You've hit your weekly limit · resets Nov 4 at 5pm",
        )
        .expect("the new hold is announced");
    });
}

fn standing_holds(root: &Path) -> Vec<nexus_chat::interruption::Interruption> {
    let ws = Workspace::resolve(None, root).expect("resolve workspace");
    ws.open_chat_store()
        .expect("open chat store")
        .standing_interruptions()
        .expect("the standing list reads")
}

#[test]
fn a_step_interrupted_on_its_first_turn_is_taken_up_by_the_root_status_shows() {
    let (_tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    let step = coder.reply_thread.clone().expect("the step owes an answer");
    assert_ne!(
        step, root,
        "the premise: the step runs on a thread of its own, and the root has no session"
    );
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    assert!(
        operation_interrupted(&engine, &root),
        "the premise: the board says something is on hold under the root"
    );

    // Without --force the window is still closed: the hold is FOUND, and it is not yet.
    let early = resume(&engine, ResumeScope::Thread(&root), false);
    assert_eq!(
        early.outcome,
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        },
        "the root finds the step's hold: {early:?}"
    );
    assert_eq!(
        early.session.as_deref(),
        Some(coder.internal_session.as_str())
    );
    assert_eq!(
        worker.starts_of("coder").len(),
        1,
        "nothing was started early"
    );

    // With --force a person who knows better (another account) takes it up.
    let forced = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(forced.outcome, ResumeOutcome::Resumed, "{forced:?}");
    assert_eq!(
        forced.session.as_deref(),
        Some(coder.internal_session.as_str())
    );
    assert_eq!(
        forced.thread.as_deref(),
        Some(step.as_str()),
        "the receipt names the thread the hold stood on"
    );
    let starts = worker.starts_of("coder");
    assert_eq!(starts.len(), 2, "the step session was started again");
    assert_eq!(
        starts[1].internal_session, coder.internal_session,
        "the SAME session is continued, not a new one minted"
    );

    // One predicate for both questions: once nothing is on hold, neither reader says otherwise.
    assert!(!operation_interrupted(&engine, &root));
    assert_eq!(
        resume(&engine, ResumeScope::Thread(&root), true).outcome,
        ResumeOutcome::NothingOnHold
    );
}

#[test]
fn of_several_holds_in_one_operation_the_oldest_is_taken_first_and_its_thread_named() {
    let (_tmp, engine, worker) = team();
    let root = commission(&engine, "review");
    let (r1, r2) = (worker.first_of("r1"), worker.first_of("r2"));
    // r2 went on hold FIRST.
    hit_the_limit(&engine, "r2", &r2.internal_session, UNTIL);
    hit_the_limit(&engine, "r1", &r1.internal_session, UNTIL);

    let first = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(first.outcome, ResumeOutcome::Resumed);
    assert_eq!(first.session.as_deref(), Some(r2.internal_session.as_str()));
    assert_eq!(first.thread, r2.reply_thread);
    assert!(
        operation_interrupted(&engine, &root),
        "r1 is still on hold, and the board still says so"
    );

    let second = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(second.outcome, ResumeOutcome::Resumed);
    assert_eq!(
        second.session.as_deref(),
        Some(r1.internal_session.as_str())
    );
    assert_eq!(second.thread, r1.reply_thread);
    assert!(!operation_interrupted(&engine, &root));
}

#[test]
fn the_session_scope_still_names_exactly_one_session() {
    let (tmp, engine, worker) = team();
    let _root = commission(&engine, "review");
    let (r1, r2) = (worker.first_of("r1"), worker.first_of("r2"));
    hit_the_limit(&engine, "r2", &r2.internal_session, UNTIL);
    hit_the_limit(&engine, "r1", &r1.internal_session, UNTIL);

    let by_session = resume(&engine, ResumeScope::Session(&r1.internal_session), true);
    assert_eq!(by_session.outcome, ResumeOutcome::Resumed);
    assert_eq!(
        by_session.session.as_deref(),
        Some(r1.internal_session.as_str()),
        "--session is not widened to the operation"
    );
    let standing = standing_holds(tmp.path());
    assert_eq!(
        standing
            .iter()
            .map(|h| h.session.as_str())
            .collect::<Vec<_>>(),
        [r2.internal_session.as_str()],
        "r1's hold is the one taken up, r2's — the OLDER one — still stands"
    );
}

#[test]
fn a_step_thread_names_its_whole_operation_too() {
    // Any thread of the operation, not only the root: a step thread resolves to the root and from
    // there to every hold beneath it — here the OTHER member's.
    let (_tmp, engine, worker) = team();
    let _root = commission(&engine, "review");
    let (r1, r2) = (worker.first_of("r1"), worker.first_of("r2"));
    hit_the_limit(&engine, "r2", &r2.internal_session, UNTIL);

    let r1_thread = r1.reply_thread.clone().expect("r1 owes an answer");
    let via_sibling = resume(&engine, ResumeScope::Thread(&r1_thread), true);
    assert_eq!(via_sibling.outcome, ResumeOutcome::Resumed);
    assert_eq!(
        via_sibling.session.as_deref(),
        Some(r2.internal_session.as_str())
    );
    assert_eq!(via_sibling.thread, r2.reply_thread);
}

#[test]
fn a_resumed_session_that_hits_the_limit_again_leaves_exactly_the_new_hold_standing() {
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    let old = standing_holds(tmp.path());
    assert_eq!(old.len(), 1, "the premise: one hold stands");

    // While the resume's start is in progress — before it has stamped the old hold — the resumed
    // session runs straight into the next window and says so.
    let path: PathBuf = tmp.path().to_path_buf();
    let session = coder.internal_session.clone();
    *worker.meanwhile.lock().unwrap() = Some(Box::new(move |req: &TriggerRequest| {
        assert_eq!(
            req.internal_session, session,
            "the premise: the coder is restarted"
        );
        announce_from_another_process(&path, &session, NEXT_UNTIL);
    }));

    let resumed = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(resumed.outcome, ResumeOutcome::Resumed, "{resumed:?}");
    assert!(
        worker.meanwhile.lock().unwrap().is_none(),
        "the premise: the new hold was announced during the start"
    );

    let standing = standing_holds(tmp.path());
    assert_eq!(
        standing.len(),
        1,
        "exactly one hold stands — neither none nor two: {standing:?}"
    );
    assert_ne!(standing[0].id, old[0].id, "the old hold is stamped");
    assert_eq!(
        standing[0].until.as_deref(),
        Some(NEXT_UNTIL),
        "and the one standing is the NEW one, not swallowed by the old"
    );
    assert!(operation_interrupted(&engine, &root));
}

#[test]
fn a_resume_whose_start_fails_leaves_the_hold_standing() {
    // The hold is stamped BEFORE the start now, so the start's failure has to put it back: a hold
    // closed over a session nothing was put back to work in is a hold nothing comes back to.
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    let before = standing_holds(tmp.path());
    *worker.fail_next.lock().unwrap() = true;

    engine
        .resume_interrupted(
            caller("o/human", NOW),
            ResumeRequest {
                scope: ResumeScope::Thread(&root),
                force: true,
            },
        )
        .expect_err("the failed start is reported");

    assert_eq!(
        standing_holds(tmp.path()),
        before,
        "the same hold stands again, untouched"
    );
    assert!(operation_interrupted(&engine, &root));
    assert_eq!(
        resume(&engine, ResumeScope::Thread(&root), true).outcome,
        ResumeOutcome::Resumed,
        "and the next resume takes it up"
    );
}

#[test]
fn a_resume_stays_inside_the_operation_its_thread_belongs_to() {
    let (_tmp, engine, worker) = team();
    let coding = commission(&engine, "coding");
    let review = commission(&engine, "review");
    let (coder, r1) = (worker.first_of("coder"), worker.first_of("r1"));

    // Only the review run is on hold.
    hit_the_limit(&engine, "r1", &r1.internal_session, UNTIL);
    assert!(operation_interrupted(&engine, &review));
    assert!(
        !operation_interrupted(&engine, &coding),
        "a hold in another operation does not mark this one"
    );
    let elsewhere = resume(&engine, ResumeScope::Thread(&coding), true);
    assert_eq!(
        elsewhere.outcome,
        ResumeOutcome::NothingOnHold,
        "the coding root does not reach into the review run: {elsewhere:?}"
    );
    assert_eq!(worker.starts_of("r1").len(), 1, "and nothing was started");

    // Both on hold: each root takes up its own.
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    let a = resume(&engine, ResumeScope::Thread(&coding), true);
    assert_eq!(a.session.as_deref(), Some(coder.internal_session.as_str()));
    assert_eq!(a.thread, coder.reply_thread);
    let b = resume(&engine, ResumeScope::Thread(&review), true);
    assert_eq!(b.session.as_deref(), Some(r1.internal_session.as_str()));
    assert_eq!(b.thread, r1.reply_thread);
}

#[test]
fn a_re_hit_during_a_start_that_then_fails_leaves_exactly_the_new_hold() {
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    let old = standing_holds(tmp.path());

    let path: PathBuf = tmp.path().to_path_buf();
    let session = coder.internal_session.clone();
    *worker.meanwhile.lock().unwrap() = Some(Box::new(move |_: &TriggerRequest| {
        announce_from_another_process(&path, &session, NEXT_UNTIL);
    }));
    *worker.fail_next.lock().unwrap() = true;
    engine
        .resume_interrupted(
            caller("o/human", NOW),
            ResumeRequest {
                scope: ResumeScope::Thread(&root),
                force: true,
            },
        )
        .expect_err("the start fails");

    let standing = standing_holds(tmp.path());
    assert_eq!(standing.len(), 1, "{standing:?}");
    assert_ne!(standing[0].id, old[0].id, "the old hold stays closed");
    assert_eq!(standing[0].until.as_deref(), Some(NEXT_UNTIL));
}

#[test]
fn a_second_resumer_during_the_first_ones_start_does_not_undo_it() {
    // Two ways back arrive at once — a person and the service. The first starts the session; the
    // second arrives inside that start, on a worker whose starts fail. Whatever the second decides
    // (today the slot's claim keeps it out: `BeingRecovered`), it must not undo the first: what
    // stands afterwards is the hold taken up and nothing on hold.
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);

    let path: PathBuf = tmp.path().to_path_buf();
    let second: Arc<Mutex<Option<String>>> = Arc::default();
    let seen = second.clone();
    let root_b = root.clone();
    *worker.meanwhile.lock().unwrap() = Some(Box::new(move |_: &TriggerRequest| {
        let failing = RecordingWorker::default();
        *failing.fail_next.lock().unwrap() = true;
        let failed = in_another_process(&path, &failing, NOW, |ctx, store| {
            orchestration::resume_interrupted(
                ctx,
                store,
                ResumeRequest {
                    scope: ResumeScope::Thread(&root_b),
                    force: true,
                },
            )
            .map(|r| r.outcome)
        });
        *seen.lock().unwrap() = Some(format!("{failed:?}"));
    }));

    let first = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(first.outcome, ResumeOutcome::Resumed);
    let second = second.lock().unwrap().clone();
    assert!(
        second
            .as_deref()
            .is_some_and(|o| !o.contains("NothingOnHold")),
        "the premise: the second resumer ran during the start and still found the hold: {second:?}"
    );
    assert_eq!(
        standing_holds(tmp.path()),
        vec![],
        "the first resumer's take-up stands"
    );
    assert!(!operation_interrupted(&engine, &root));
}

#[test]
fn a_resume_that_dies_before_the_spawn_leaves_the_hold_standing() {
    // A SIGINT, an OOM kill, a panic: the resume began and nothing was started. The hold must still
    // be there for the next resume — "nothing on hold" here is exactly the cn4p symptom.
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    let before = standing_holds(tmp.path());
    worker
        .panic_next
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resume(&engine, ResumeScope::Thread(&root), true)
    }));
    assert!(
        died.is_err(),
        "the premise: the resume died inside the start"
    );
    drop(engine);

    let after = standing_holds(tmp.path());
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before[0].id, "the same hold still stands");

    // …and the next process takes it up.
    let next = in_another_process(tmp.path(), worker.as_ref(), NOW, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Thread(&root),
                force: true,
            },
        )
        .expect("the resume decides")
    });
    assert_eq!(next.outcome, ResumeOutcome::Resumed);
    assert!(standing_holds(tmp.path()).is_empty());
}

#[test]
fn a_hold_belongs_to_the_operation_its_session_is_recorded_on_for_both_readers() {
    // The shape where the board's per-row marks and the operation flag could part: a session whose
    // recorded thread lies in ANOTHER operation than the thread it was commissioned on (a session
    // resumed elsewhere). The pinned answer: the hold belongs to the operation of the thread the
    // session is recorded on — the board marks THAT operation, and its root's resume takes it up;
    // the other operation is neither marked nor reaches it.
    let (tmp, engine, worker) = team();
    let coding = commission(&engine, "coding");
    let review = commission(&engine, "review");
    let coder = worker.first_of("coder");
    let r1 = worker.first_of("r1");
    let coder_thread = coder.reply_thread.clone().expect("the step owes an answer");
    {
        let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
        let mut store = ws.open_chat_store().expect("open chat store");
        store
            .record_session_thread(&r1.internal_session, Some(&coder_thread))
            .expect("the session now stands in the coding run");
    }
    hit_the_limit(&engine, "r1", &r1.internal_session, UNTIL);

    assert!(operation_interrupted(&engine, &coding));
    assert!(!operation_interrupted(&engine, &review));
    assert_eq!(
        resume(&engine, ResumeScope::Thread(&review), true).outcome,
        ResumeOutcome::NothingOnHold
    );
    let taken = resume(&engine, ResumeScope::Thread(&coding), true);
    assert_eq!(taken.session.as_deref(), Some(r1.internal_session.as_str()));
    assert_eq!(taken.thread.as_deref(), Some(coder_thread.as_str()));
}

#[test]
fn a_mark_that_cannot_be_written_is_reported_and_the_resume_goes_on() {
    // The store refuses the mark (injected with a trigger, as a full disk or a locked file would).
    // The resume still takes the hold up — the mark only protects a re-hit during the start — and
    // says on its receipt what it could not record.
    let (tmp, engine, worker) = team();
    let root = commission(&engine, "coding");
    let coder = worker.first_of("coder");
    hit_the_limit(&engine, "coder", &coder.internal_session, UNTIL);
    {
        let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
        let store = ws.open_chat_store().expect("open chat store");
        store
            .connection()
            .execute_batch(
                "CREATE TRIGGER refuse_the_mark BEFORE UPDATE OF taking_up_at
                   ON session_interruption
                 BEGIN SELECT RAISE(ABORT, 'injected: the mark cannot be written'); END;",
            )
            .expect("the fault is injected");
    }

    let resumed = resume(&engine, ResumeScope::Thread(&root), true);
    assert_eq!(resumed.outcome, ResumeOutcome::Resumed, "{resumed:?}");
    assert_eq!(
        worker.starts_of("coder").len(),
        2,
        "the session was started"
    );
    assert!(
        resumed
            .warnings
            .iter()
            .any(|w| w.detail.contains("could not be marked as being taken up")),
        "the receipt names the mark it could not write: {:?}",
        resumed.warnings
    );
    assert!(
        standing_holds(tmp.path()).is_empty(),
        "and the hold is taken up"
    );
}
