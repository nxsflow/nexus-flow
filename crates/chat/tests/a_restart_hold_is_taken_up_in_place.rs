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
    /// Every start of these roles fails, as a start that cannot reach its runtime does.
    failing: Mutex<HashSet<String>>,
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
        if self.failing.lock().unwrap().contains(&req.role.handle) {
            return Err(nexus_chat::worker::TriggerError::Failed(
                nexus_chat::error::NxfError::io("the runtime could not be reached (stub)"),
            ));
        }
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
        vec![
            role("coder", true),
            role("rival", true),
            role("third", true),
            role("helper", true),
            role("pm", false),
        ],
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
    restart_hold_at(engine, NOW, session, until);
}

/// [`restart_hold`] recorded at `at`.
fn restart_hold_at(engine: &Engine, at: &str, session: &str, until: Option<&str>) {
    engine
        .session_interrupted(
            caller_at("coder", at),
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

// ---- C.4: the copy is pinned through the gap, at every reader -----------------------------------
//
// The coder holds the copy with no declared window, so its lease runs to the two-hour bound:
// commissioned at NOW, it expires at 12:00:00. Each leg below records the restart's hold, lets the
// session end the way its teardown does, and then reaches ONE reader of the guard in the gap.

/// The tick the background service runs, through the compute layer (it is no verb on the handle).
fn tick(
    root: &std::path::Path,
    worker: &HostWorker,
    now: &str,
    thread: &str,
) -> nexus_chat::orchestration::TickReceipt {
    use nexus_chat::workspace::ChatWorkspaceExt;
    let ws = nexus_chat::workspace::Workspace::resolve(None, root).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(root).expect("declarations");
    let ctx = nexus_chat::orchestration::Ctx {
        now,
        origin: &origin,
        actor: "service",
        session: None,
        hop: 0,
        defs: &defs,
        worker,
        timer: &nexus_chat::timer::DryTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
    };
    nexus_chat::orchestration::tick(
        &ctx,
        &mut store,
        nexus_chat::orchestration::TickRequest { thread_id: thread },
    )
    .expect("tick")
}

/// The restart's hold and the teardown's end, at `at`, with `until` 150 s later.
fn stopped_by_a_restart(engine: &Engine, at: &str, until: &str, session: &str) {
    restart_hold_at(engine, at, session, Some(until));
    engine
        .session_ended(caller_at("coder", at), session)
        .expect("its teardown announces the end");
}

/// Nothing moved: the work is in place, the lease row is the one given, nobody else started, and no
/// refused park is on record.
fn nothing_moved(
    root: &std::path::Path,
    worker: &HostWorker,
    lease: &(String, String),
    receipt: &nexus_chat::orchestration::TickReceipt,
) {
    the_work_is_in_place(root);
    assert_eq!(
        lease_row(root).as_ref(),
        Some(lease),
        "the lease row is unchanged"
    );
    assert!(worker.triggers_for("rival").is_empty(), "nothing handed on");
    assert!(
        receipt.parked.is_none() && receipt.handed_on.is_none() && receipt.working_tree.is_none(),
        "the tick moved nothing: {receipt:?}"
    );
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| format!("{:?}", w.class).contains("Park")
                || format!("{:?}", w.class).contains("Unparked")),
        "no park finding: {:?}",
        receipt.warnings
    );
    assert!(
        common::open_store(root)
            .connection()
            .query_row("SELECT COUNT(*) FROM park_refusals", [], |r| r
                .get::<_, i64>(0))
            .unwrap()
            == 0,
        "no PARK REFUSED row"
    );
}

/// **The tick's own park** (`park_step` → `park_an_interrupted_holder` → the funnel
/// `park_the_interrupted_tree`), on a lease still inside its bound: the round-2 High. A window hold
/// in this state is parked and handed on; a restart hold is not an occasion at all.
#[test]
fn a_tick_in_the_gap_does_not_park_the_holder_on_its_hold() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let lease = lease_row(root).unwrap();
    stopped_by_a_restart(
        &engine,
        "2026-10-27T10:01:00Z",
        "2026-10-27T10:03:30Z",
        &session,
    );

    let r = tick(root, &worker, "2026-10-27T10:02:00Z", &thread);

    nothing_moved(root, &worker, &lease, &r);
}

/// **The sweep** (`sweep_expired_working_tree`): the lease ran out inside the gap, and nothing has
/// a process — a sweep would reclaim it. Its warning names the hold instead of a process.
#[test]
fn a_sweep_in_the_gap_does_not_reclaim_an_expired_lease_on_a_restart_hold() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let lease = lease_row(root).unwrap();
    assert_eq!(
        lease.1, "2026-10-27T12:00:00Z",
        "the premise: the two-hour bound"
    );
    stopped_by_a_restart(
        &engine,
        "2026-10-27T11:59:00Z",
        "2026-10-27T12:01:30Z",
        &session,
    );

    let r = tick(root, &worker, "2026-10-27T12:00:30Z", &thread);

    nothing_moved(root, &worker, &lease, &r);
}

/// **…and once `until + MARGIN` has passed with no resume, the ordinary reclaim applies**: the
/// service never came back, the copy goes to whoever waits, and the hold is still visible.
#[test]
fn past_the_holds_bound_the_copy_goes_to_whoever_waits_and_the_hold_stays_visible() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    stopped_by_a_restart(
        &engine,
        "2026-10-27T11:59:00Z",
        "2026-10-27T12:01:30Z",
        &session,
    );
    tick(root, &worker, "2026-10-27T12:00:30Z", &thread);
    assert!(
        worker.triggers_for("rival").is_empty(),
        "pinned inside the bound"
    );

    tick(root, &worker, "2026-10-27T12:02:01Z", &thread);

    assert_eq!(
        worker.triggers_for("rival").len(),
        1,
        "the rival has the copy now"
    );
    let standing = common::open_store(root)
        .current_interruption(&session)
        .unwrap()
        .expect("the hold still stands");
    assert_eq!(standing.limit, SERVICE_RESTART_LIMIT);
}

/// **A fresh commission arriving in the gap** (`expired_but_still_writing`, the acquire path): the
/// lease has expired and nothing runs, so its compare-and-swap would take it. It queues instead.
#[test]
fn a_commission_arriving_in_the_gap_queues_behind_the_held_copy() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, _thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let lease = lease_row(root).unwrap();
    stopped_by_a_restart(
        &engine,
        "2026-10-27T11:59:00Z",
        "2026-10-27T12:01:30Z",
        &session,
    );

    let receipt = engine
        .send_to(
            caller_at("pm", "2026-10-27T12:00:30Z"),
            SendToRequest {
                machine: None,
                to: "third",
                body: "a new piece of work",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("commissioned");

    assert!(
        worker.triggers_for("third").is_empty(),
        "not started into the held copy"
    );
    assert!(
        receipt.queued_behind.is_some(),
        "it waits behind the holder: {receipt:?}"
    );
    the_work_is_in_place(root);
    assert_eq!(lease_row(root), Some(lease));
}

/// **The stranded escalation's park** (`park_the_stranded_holder`): the holder handed its task back,
/// the rival waited past the park deadline — the occasion stands, and the hold keeps the copy.
#[test]
fn an_escalated_holder_on_a_restart_hold_is_not_parked_past_the_deadline() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    engine
        .reply_thread(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some("2026-10-27T10:05:00Z"),
            },
            nexus_chat::surface::ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "which API did you mean?",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the escalation is posted");
    // The first look notices the contention and starts its clock.
    tick(root, &worker, "2026-10-27T10:06:00Z", &thread);
    let lease = lease_row(root).unwrap();
    stopped_by_a_restart(
        &engine,
        "2026-10-27T10:35:00Z",
        "2026-10-27T10:37:30Z",
        &session,
    );

    let r = tick(root, &worker, "2026-10-27T10:36:30Z", &thread);

    nothing_moved(root, &worker, &lease, &r);
}

/// **A failed start's retry in the gap** (`hand_on_a_copy_nothing_runs_in`): a helper the holder's
/// operation commissioned failed to start, and its retry falls in the gap. It takes the holder's own
/// lease, fails again — and nothing runs in the area, so the copy would be handed on.
#[test]
fn a_failed_retry_in_the_gap_does_not_hand_the_held_copy_on() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let lease = lease_row(root).unwrap();
    // The coder commissions a helper inside its own operation, and the helper's start fails. Its
    // failed-start record is written as the release path writes one, due again at 10:02:00 — in
    // the gap.
    worker.failing.lock().unwrap().insert("helper".into());
    let sent = engine
        .send_to(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            SendToRequest {
                machine: None,
                to: "helper",
                body: "help",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the helper is commissioned");
    {
        let mut store = common::open_store(root);
        let helper: String = store
            .connection()
            .query_row(
                "SELECT internal_id FROM session_map WHERE role LIKE '%helper' ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("the helper's session was minted");
        let entry = nexus_chat::working_tree::QueuedTrigger {
            id: 0,
            scope_key: lease.0.clone(),
            role: "helper".into(),
            session: helper.clone(),
            thread: Some(sent.thread_id.clone()),
            message: "help".into(),
            model: None,
            depth: 1,
            priority: nexus_chat::model::Priority::Normal,
            enqueued_at: None,
        };
        store
            .record_failed_start(&entry, "the runtime could not be reached", NOW)
            .unwrap();
        store
            .schedule_failed_start_retry(&helper, "2026-10-27T10:02:00Z")
            .unwrap();
    }
    stopped_by_a_restart(
        &engine,
        "2026-10-27T10:01:00Z",
        "2026-10-27T10:03:30Z",
        &session,
    );

    let r = tick(root, &worker, "2026-10-27T10:02:00Z", &thread);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.detail.contains("a retry of a failed start was due")),
        "the premise: the retry ran in the gap and failed: {:?}",
        r.warnings
    );

    the_work_is_in_place(root);
    assert_eq!(
        lease_row(root).map(|l| l.0),
        Some(lease.0),
        "still the holder's"
    );
    assert!(worker.triggers_for("rival").is_empty(), "nothing handed on");
}

/// **The funnel itself** (`park_the_interrupted_tree`): a second session of the holder's operation
/// is on a WINDOW hold — a park waiting to happen — while the coder is on the restart's hold. The
/// window hold is an occasion, and the park it reaches must still see the coder coming back.
#[test]
fn a_window_hold_beside_a_restart_hold_does_not_park_the_shared_copy() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    engine
        .send_to(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            SendToRequest {
                machine: None,
                to: "helper",
                body: "help",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the helper is commissioned inside the coder's operation");
    let helper = worker.triggers_for("helper")[0].internal_session.clone();
    engine
        .session_interrupted(
            caller_at("helper", NOW),
            &helper,
            "seven_day",
            Some("2026-11-01T00:00:00Z"),
            "weekly limit",
        )
        .unwrap();
    engine
        .session_ended(caller_at("helper", NOW), &helper)
        .unwrap();
    let lease = lease_row(root).unwrap();
    stopped_by_a_restart(
        &engine,
        "2026-10-27T10:01:00Z",
        "2026-10-27T10:03:30Z",
        &session,
    );

    let r = tick(root, &worker, "2026-10-27T10:02:00Z", &thread);

    nothing_moved(root, &worker, &lease, &r);
}

/// **An in-place hold is not an occasion of the tick's park** (`park_an_interrupted_holder`'s
/// filter): a refusal that occasion recorded earlier is forgotten once only a restart hold stands —
/// a mark that says "retrying" is only true while something is retrying.
#[test]
fn a_restart_hold_does_not_keep_an_old_refused_park_retrying() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    let (holder, _) = lease_row(root).unwrap();
    common::open_store(root)
        .note_park_refusal(
            &holder,
            &nexus_chat::park::ParkRefusal::MidSequence("a rebase was in progress".into()),
            nexus_chat::orchestration::ParkOccasion::AvailabilityBoundary,
            NOW,
        )
        .unwrap();
    stopped_by_a_restart(
        &engine,
        "2026-10-27T10:01:00Z",
        "2026-10-27T10:03:30Z",
        &session,
    );

    tick(root, &worker, "2026-10-27T10:02:00Z", &thread);

    assert_eq!(
        common::open_store(root).park_refusal(&holder).unwrap(),
        None,
        "no availability-boundary park is waiting to happen, so its refusal is not kept"
    );
    the_work_is_in_place(root);
}

/// **`nxc status` names the hold that pins the copy** (round-3 IR #4, must-fix 9): in the gap the
/// lease may have expired and nothing runs, so without it the holder reads like an operation that
/// holds nothing — while the copy is kept for it. Read from the same guard the readers ask.
#[test]
fn status_names_the_restart_hold_that_pins_the_copy_and_drops_it_past_the_bound() {
    let dir = TempDir::new().unwrap();
    let _log = TimerLog::open(dir.path());
    let worker = Arc::new(HostWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let root = tmp.path();
    let (session, thread) = a_coder_at_work_with_a_rival_waiting(root, &engine, &worker);
    stopped_by_a_restart(
        &engine,
        "2026-10-27T11:59:00Z",
        "2026-10-27T12:01:30Z",
        &session,
    );
    let op_root = common::open_store(root).thread_root(&thread).unwrap();
    let pinned = |now: &str| {
        engine
            .status(
                nexus_chat::facade::Reader::Handle("o/human"),
                now,
                nexus_chat::facade::StatusScope::Workspace,
            )
            .unwrap()
            .operations
            .into_iter()
            .find(|op| op.root == op_root)
            .expect("the coder's operation is listed")
            .pinned_by_restart_hold
    };

    assert_eq!(
        pinned("2026-10-27T12:00:30Z").as_deref(),
        Some("2026-10-27T12:01:30Z")
    );
    assert_eq!(
        pinned("2026-10-27T12:02:01Z"),
        None,
        "past the bound it pins nothing"
    );
}
