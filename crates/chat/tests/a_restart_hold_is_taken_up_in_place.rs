//! **A `service_restart` hold is taken up IN PLACE** (nxf 6j6v.wxz0, section C) — at the Engine
//! seam, where the hook records it (`engine-seam-test-rule`).
//!
//! A window hold may last a week, so its first firing secures the tree and hands the checkout on.
//! A planned restart's gap is 150 s, during which nobody else may take the checkout, and the same
//! session resumes in the same tree: so its hold arms no near firing, an early resume parks
//! nothing, and the take-up asks no drift question while the copy did not change hands.
//!
//! The worker here is a host worker that answers the liveness question; the two-process proofs are
//! in `a_restart_leaves_a_way_back.rs`.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::interruption::{SERVICE_RESTART_LIMIT, SERVICE_RESTART_MARGIN_SECS};
use nexus_chat::orchestration::{Caller, ResumeOutcome, ResumeRequest, ResumeScope};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T10:00:00Z";
/// The hook's start plus the grace plus the margin.
const UNTIL: &str = "2026-10-27T10:02:30Z";
const DETAIL: &str = "nxs self-update 0.221.0 -> 0.222.0 restarted the background service";

#[derive(Default)]
struct HostWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    /// The tree its sessions write to — the workspace root, as the sidecar's.
    dir: Mutex<Option<std::path::PathBuf>>,
}

impl HostWorker {
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
        true
    }
    fn working_copy(&self) -> Option<std::path::PathBuf> {
        self.dir.lock().unwrap().clone()
    }
}

fn role(handle: &str, exclusive: bool) -> RoleDecl {
    let tree = if exclusive {
        "working_tree: exclusive\n"
    } else {
        ""
    };
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n{tree}"
    ))
    .expect("test role parses")
}

fn workspace(worker: Arc<HostWorker>) -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    *worker.dir.lock().unwrap() = Some(tmp.path().to_path_buf());
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder", true), role("rival", true), role("pm", false)],
        vec![],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine)
}

/// `NXC_TIMER_LOG` is process-global env: every test here serializes on it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    /// The deadlines of every resume armed, in order.
    fn resumes_armed(&self) -> Vec<String> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("schedule ") && l.contains("command=nxc resume"))
            .filter_map(|l| {
                l.split_whitespace()
                    .find_map(|f| f.strip_prefix("deadline="))
                    .map(str::to_string)
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

/// Commission `to` and hand back (its internal session, the thread it owes an answer on).
fn commission(engine: &Engine, worker: &HostWorker, to: &str, now: &str) -> (String, String) {
    engine
        .send_to(
            caller_at("pm", now),
            SendToRequest {
                machine: None,
                to,
                body: "build the restart hook",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the persona is commissioned");
    let t = worker
        .triggers_for(to)
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("no trigger for {to}"));
    (t.internal_session, t.reply_thread.expect("a reply thread"))
}

/// What the restart's hook records for one session, through the Engine seam.
fn restart_hold(engine: &Engine, session: &str, until: Option<&str>) {
    engine
        .session_interrupted(
            caller_at("coder", NOW),
            session,
            SERVICE_RESTART_LIMIT,
            until,
            DETAIL,
        )
        .expect("the hold is recorded");
}

fn resume(engine: &Engine, now: &str, session: &str) -> nexus_chat::orchestration::ResumeReceipt {
    engine
        .resume_interrupted(
            caller_at("pm", now),
            ResumeRequest {
                scope: ResumeScope::Session(session),
                force: false,
            },
        )
        .expect("the resume decides")
}

// ---- C.1: no near firing ------------------------------------------------------------------------

/// **DoD 3, the arming**: the hold arms `ChatResume` for `until` only — not at `RESUME_GRACE`, whose
/// firing would secure and park the tree.
#[test]
fn a_restart_hold_is_armed_for_until_and_never_for_the_near_firing() {
    let dir = TempDir::new().unwrap();
    let log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, _thread) = commission(&engine, &worker, "coder", NOW);

    let receipt = engine
        .session_interrupted(
            caller_at("coder", NOW),
            &session,
            SERVICE_RESTART_LIMIT,
            Some(UNTIL),
            DETAIL,
        )
        .expect("recorded");
    assert_eq!(receipt.armed_for.as_deref(), Some(UNTIL));
    assert!(receipt.warnings.is_empty(), "{:?}", receipt.warnings);
    assert_eq!(log.resumes_armed(), [UNTIL], "one firing, at until");
}

/// An in-place hold with no `until` arms nothing, and the receipt says so.
#[test]
fn a_restart_hold_with_no_until_arms_nothing_and_says_so() {
    let dir = TempDir::new().unwrap();
    let log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (_tmp, engine) = workspace(worker.clone());
    let (session, _thread) = commission(&engine, &worker, "coder", NOW);

    let receipt = engine
        .session_interrupted(
            caller_at("coder", NOW),
            &session,
            SERVICE_RESTART_LIMIT,
            None,
            DETAIL,
        )
        .expect("recorded");
    assert_eq!(receipt.armed_for, None);
    assert!(
        receipt
            .warnings
            .iter()
            .any(|w| w.detail.contains("no instant")),
        "the receipt names why nothing is armed: {:?}",
        receipt.warnings
    );
    assert!(log.resumes_armed().is_empty());
}

// ---- the working copy: a git checkout the coder holds, with a rival queued behind it ------------

fn git(root: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository with one base commit, the coder commissioned (it takes the lease), its own branch
/// with one commit and one uncommitted file, and a rival commissioned behind it (queued). Returns
/// (session, thread).
fn a_coder_at_work_with_a_rival_waiting(
    root: &std::path::Path,
    engine: &Engine,
    worker: &HostWorker,
) -> (String, String) {
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), ".nxs/\n*.log\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
    let (session, thread) = commission(engine, worker, "coder", NOW);
    git(root, &["switch", "-q", "-c", "fix/the-coders-branch"]);
    std::fs::write(root.join("done.rs"), "fn done() {}\n").unwrap();
    git(root, &["add", "done.rs"]);
    git(root, &["commit", "-q", "-m", "the coder's commit"]);
    std::fs::write(root.join("half-done.rs"), "fn half() {}\n").unwrap();
    engine
        .send_to(
            caller_at("pm", NOW),
            SendToRequest {
                machine: None,
                to: "rival",
                body: "rival work",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the rival is commissioned");
    // A rival that started would mean the coder held no lease — the setup's fault, not the code's.
    assert!(
        worker.triggers_for("rival").is_empty(),
        "the rival must be queued behind the coder, not started"
    );
    (session, thread)
}

fn lease_row(root: &std::path::Path) -> Option<(String, String)> {
    common::open_store(root).working_tree_lease_row().unwrap()
}

fn branches(root: &std::path::Path) -> Vec<String> {
    git(root, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// The coder's work is exactly where it left it: on its branch, its commit at HEAD, the uncommitted
/// file in the tree, and no park branch anywhere.
fn the_work_is_in_place(root: &std::path::Path) {
    assert_eq!(
        git(root, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "fix/the-coders-branch"
    );
    assert_eq!(
        git(root, &["log", "-1", "--format=%s"]),
        "the coder's commit"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("half-done.rs")).unwrap(),
        "fn half() {}\n",
        "the uncommitted file is still in the tree"
    );
    assert_eq!(
        branches(root),
        ["fix/the-coders-branch", "main"],
        "no park branch"
    );
}

// ---- C.2: an early resume parks nothing ---------------------------------------------------------

/// **DoD 3, the early resume**: `nxc resume` before `until` answers `NotYet`, re-arms for `until`,
/// and secures nothing — no park branch, no `work_not_parked` warning, the lease row unchanged.
#[test]
fn an_early_resume_of_a_restart_hold_answers_not_yet_and_parks_nothing() {
    let dir = TempDir::new().unwrap();
    let log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, _thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let lease = lease_row(root).expect("the coder holds the copy");
    restart_hold(&engine, &session, Some(UNTIL));
    engine
        .session_ended(caller_at("coder", NOW), &session)
        .expect("its teardown announces the end");

    let early = resume(&engine, "2026-10-27T10:00:30Z", &session);

    assert_eq!(
        early.outcome,
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        }
    );
    assert_eq!(
        early.armed_for.as_deref(),
        Some(UNTIL),
        "re-armed for until"
    );
    assert!(early.warnings.is_empty(), "{:?}", early.warnings);
    assert_eq!(log.resumes_armed(), [UNTIL, UNTIL]);
    the_work_is_in_place(root);
    assert_eq!(lease_row(root), Some(lease), "the lease row is unchanged");
    assert!(worker.triggers_for("rival").is_empty(), "nothing handed on");
}

/// The control: the same setup under a WINDOW hold is secured on the early resume — so the setup
/// can park, and the test above is not green for want of a park that could happen.
#[test]
fn the_same_early_resume_of_a_window_hold_parks_the_tree_and_hands_it_on() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, _thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    engine
        .session_interrupted(
            caller_at("coder", NOW),
            &session,
            "seven_day",
            Some("2026-11-01T00:00:00Z"),
            "weekly limit",
        )
        .unwrap();
    engine
        .session_ended(caller_at("coder", NOW), &session)
        .unwrap();

    resume(&engine, "2026-10-27T10:00:30Z", &session);

    assert_eq!(
        git(root, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main",
        "a window hold's tree is parked: the checkout is back on the base"
    );
    assert_eq!(
        worker.triggers_for("rival").len(),
        1,
        "and the copy went on"
    );
}
