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

/// What the background service would do, from its own connection — the way a second process acts
/// on the workspace while the first is in the middle of a start.
fn announce_from_another_process(root: &Path, session: &str, until: &str) {
    let ws = Workspace::resolve(None, root).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(root).expect("declarations");
    let ctx = orchestration::Ctx {
        now: NOW,
        origin: &origin,
        actor: "o/coder",
        session: None,
        hop: 0,
        defs: &defs,
        worker: &RecordingWorker::default(),
        timer: &DryTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
    };
    orchestration::session_interrupted(
        &ctx,
        &mut store,
        session,
        "rate_limit",
        Some(until),
        "You've hit your weekly limit · resets Nov 4 at 5pm",
    )
    .expect("the new hold is announced");
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
    let (_tmp, engine, worker) = team();
    let _root = commission(&engine, "review");
    let (r1, r2) = (worker.first_of("r1"), worker.first_of("r2"));
    hit_the_limit(&engine, "r2", &r2.internal_session, UNTIL);
    hit_the_limit(&engine, "r1", &r1.internal_session, UNTIL);

    let by_session = resume(&engine, ResumeScope::Session(&r1.internal_session), true);
    assert_eq!(
        by_session.session.as_deref(),
        Some(r1.internal_session.as_str()),
        "--session is not widened to the operation"
    );
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
