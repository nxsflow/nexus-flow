//! ACCEPTANCE of nxf 6j6v.xb24 — **a holder that dies inside its bound is noticed**.
//!
//! The two hours nobody was watching. `WORKING_TREE_LEASE_BOUND` is a backstop for a holder nobody
//! can ask about; a chain that dies HARD is one this device CAN ask about, and until this item
//! nothing asked until the backstop had elapsed. So a commission queued behind a corpse waited out
//! whatever was left of its two hours — on a machine where the whole question costs about two
//! milliseconds for twenty threads with twenty sessions
//! (`orchestration::tests::what_the_whole_dead_holder_check_costs`, the measurement this item was
//! told to take before it changed anything, end to end since fix round 1 of its review).
//!
//! What this file pins is the occasion and its guards, because the guards ARE the item. Taking a
//! working copy away writes into it, so acting on a holder that is merely quiet would be the
//! corruption the whole epic exists to prevent. All five have to hold:
//!
//! - the worker ANSWERS the liveness question — a fail-open `false` is not evidence;
//! - at least one thread of the claim area still owes an answer;
//! - nothing in the claim area has a live process;
//! - no owing thread's session reported its end — a reported end is a teardown that RAN;
//! - nothing in the claim area is interrupted at an availability boundary.
//!
//! …and once it does hold, it is the same park, the same refusal rule and the same retry as the
//! three occasions before it (`an_unanswered_escalation_parks_and_comes_back.rs`,
//! `an_interrupted_operation_hands_the_working_copy_on.rs`, `a_holder_past_its_bound_is_parked.rs`).
//!
//! Everything here is real: a real git repository, the real lease, the real queue, the real park.
//! The engine handle carries the commissions and every status read; `orchestration::tick` carries
//! the tick, which has no handle verb — it is what the background service runs. The only stand-in is
//! the worker, and only for what a worker does: it records its triggers, names where sessions run,
//! and answers which of them still have a process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{StatusOperation, StatusReport, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, Ctx, FailedConsequence, ParkOccasion, TickReceipt, TickRequest,
};
use nexus_chat::park::ParkRefusal;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here and takes the working copy. Nothing declares a `timeout:`, so its
/// lease gets the flat two-hour bound.
const T0: &str = "2026-09-08T09:00:00Z";
/// The coder escalates here — the one run that needs the stranded-escalation occasion instead.
const ESCALATED: &str = "2026-09-08T09:05:00Z";
/// `solo` queues behind the copy here.
const QUEUED: &str = "2026-09-08T09:10:00Z";
/// **Well inside the bound**: ninety minutes of the coder's lease still to run. Every tick in this
/// file fires here, because the whole item is what happens BEFORE the bound.
const INSIDE: &str = "2026-09-08T09:30:00Z";
/// One liveness re-check later — where the next look is armed while somebody waits.
const NEXT_LOOK: &str = "2026-09-08T09:31:00Z";
/// …and one after that, for a refusal whose occasion keeps standing.
const SECOND_LOOK: &str = "2026-09-08T09:32:00Z";
/// Past the thirty-minute contention deadline that runs from [`QUEUED`] — where the stranded
/// escalation, and not this occasion, takes the copy.
const PAST_THE_PARK_DEADLINE: &str = "2026-09-08T09:41:00Z";
/// The instant the coder's lease would have run out: [`T0`] plus `WORKING_TREE_LEASE_BOUND`.
const THE_BOUND: &str = "2026-09-08T11:00:00Z";
/// Where a holder's bound lands once a trigger of its own has ridden it forward again.
const RENEWED_UNTIL: &str = "2026-09-08T13:00:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records its triggers like [`DryWorker`], names where sessions run, and answers
/// which sessions still have a process — `a_holder_past_its_bound_is_parked.rs`'s
/// `LiveParkingWorker`, with the one thing this file needs on top: it can be made BLIND, so the
/// fail-open default of `session_is_running` can be told apart from a real `false`.
struct LiveParkingWorker {
    log: PathBuf,
    working_copy: Option<PathBuf>,
    running: Mutex<HashSet<String>>,
    /// `false` makes [`Worker::answers_liveness`] answer `false` while `session_is_running` keeps
    /// saying "nothing is running" — which is precisely the shape a worker that never looked has,
    /// and the one this occasion must never act on.
    looks: AtomicBool,
    /// **A lease renewal to run ONCE, from inside the park** — `(workspace root, the holder's
    /// thread, the bound it renews to)`.
    ///
    /// It hangs on the worker because `working_copy()` is the first thing `attempt_the_park` asks,
    /// which is the one moment that sits AFTER the sweep read the holder's `expires` and BEFORE the
    /// reclaim names it. That is the compare-and-swap's window, and it cannot be reached from
    /// outside the tick.
    renew: Mutex<Option<(PathBuf, String, String)>>,
}

impl LiveParkingWorker {
    fn new(log: PathBuf, working_copy: Option<PathBuf>) -> Self {
        LiveParkingWorker {
            log,
            working_copy,
            running: Mutex::new(HashSet::new()),
            looks: AtomicBool::new(true),
            renew: Mutex::new(None),
        }
    }
    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
    /// This worker stops being able to answer the liveness question at all.
    fn stop_looking(&self) {
        self.looks.store(false, Ordering::SeqCst);
    }

    /// The next park of this workspace renews `thread`'s lease to `until` first — the holder's own
    /// trigger arriving in the middle of the park, staged where nothing else can stage it.
    fn renew_the_lease_once(&self, root: &Path, thread: &str, until: &str) {
        *self.renew.lock().unwrap() =
            Some((root.to_path_buf(), thread.to_string(), until.to_string()));
    }
}

impl Worker for LiveParkingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.running
            .lock()
            .unwrap()
            .insert(req.internal_session.clone());
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn answers_liveness(&self) -> bool {
        self.looks.load(Ordering::SeqCst)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        // The staged renewal, if one is armed — taken, so it happens exactly once and the retry a
        // minute later meets an ordinary park.
        if let Some((root, thread, until)) = self.renew.lock().unwrap().take() {
            let mut store = Workspace::resolve(None, &root)
                .expect("resolve workspace")
                .open_chat_store()
                .expect("open chat store");
            assert!(
                store
                    .acquire_working_tree(&WorkScope::Thread(thread), INSIDE, &until)
                    .expect("the holder's own trigger inherits its lease"),
                "the renewal has to land, or the race this stages never happens"
            );
        }
        self.working_copy.clone()
    }
    fn resumes_sessions(&self) -> bool {
        true
    }
}

/// A timer that records every window it is asked to arm, in order — the cadence a decline re-arms
/// at is a fact about WHAT IS ARMED, and nothing else can see it. Arming replaces by key in the
/// shipped service, so what a key will actually fire at is the LAST instant recorded for it.
#[derive(Default)]
struct RecordingTimer {
    armed: Mutex<Vec<(String, String)>>,
}

impl RecordingTimer {
    fn last_for(&self, key: &str) -> Option<String> {
        self.armed
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, at)| at.clone())
    }
}

impl Timer for RecordingTimer {
    fn schedule(
        &self,
        thread_id: &str,
        deadline: &str,
        _command: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        self.armed
            .lock()
            .unwrap()
            .push((thread_id.to_string(), deadline.to_string()));
        Ok(TimerHandle(format!("recorded:{thread_id}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> nexus_chat::error::Result<()> {
        Ok(())
    }
}

/// Every git call here runs with a **pinned `$HOME`**, for the sibling acceptance files' two
/// reasons: the source gate that flags an unpinned `init`, and determinism — unpinned, these
/// commands read the developer's own `~/.gitconfig`.
fn git(root: &Path, args: &[&str]) -> String {
    use nxs_test_support::PinHome;
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .pin_home(nxs_test_support::pinned_home())
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(root: &Path, msg: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            msg,
        ],
    );
}

/// The branch the operation started on, and the one the tree must be standing on again once its
/// work has been parked.
const BASE: &str = "release/1.2";

/// A chat workspace that is ALSO a git repository on [`BASE`], with two personas that each want the
/// working copy to themselves.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "solo"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\njob_title: Implementer\nsystem_prompt: You are {handle}.\n\
                 working_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", BASE]);
    tmp
}

/// What a coding agent leaves behind in the tree it works in.
const HALF_FINISHED: &str = "as it was\nhalf finished\n";
/// …and beside it, a file git has never been told about.
const NOTES: &str = "my working notes\n";

struct Bench {
    tmp: TempDir,
    worker: Arc<LiveParkingWorker>,
    timer: RecordingTimer,
}

impl Bench {
    fn new() -> Self {
        let tmp = workspace();
        let worker = Arc::new(LiveParkingWorker::new(
            tmp.path().join("dry.log"),
            Some(tmp.path().to_path_buf()),
        ));
        Bench {
            tmp,
            worker,
            timer: RecordingTimer::default(),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    /// A fresh handle per call, dropped after it — each verb opens the database cleanly, exactly as
    /// the CLI's processes do. The WORKER is shared, because which sessions are alive is a fact
    /// about the host and outlives any one call.
    fn engine(&self) -> Engine {
        Engine::open_with(
            None,
            self.root(),
            EngineConfig {
                worker: WorkerConfig::Custom(self.worker.clone()),
                timer: TimerConfig::Disabled,
                ..EngineConfig::default()
            },
        )
        .expect("open engine")
    }

    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.root())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    /// The human commissions a persona. Returns the thread it opened.
    fn commission(&self, now: &str, to: &str) -> String {
        self.engine()
            .send_to(
                Caller {
                    session: None,
                    actor: Some("carsten"),
                    now: Some(now),
                },
                SendToRequest {
                    machine: None,
                    to,
                    body: "do the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("commission")
            .thread_id
    }

    /// The persona hands its task back to the human — the occasion that owns a holder whose
    /// sessions all ended normally.
    fn escalate(&self, now: &str, session: &str, thread: &str) {
        self.engine()
            .reply_thread(
                Caller {
                    session: Some(session),
                    actor: Some("coder"),
                    now: Some(now),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body: "I cannot go on: which of the two APIs did you mean?",
                    escalate: true,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("the escalation is posted");
    }

    /// The tick the background service runs — through the compute layer, because it is not a verb
    /// on the handle; with [`RecordingTimer`], so what it arms can be read back.
    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = self.worker.clone();
        let ctx = Ctx {
            now,
            origin: "local",
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &defs,
            worker: worker.as_ref(),
            timer: &self.timer,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        };
        orchestration::tick(&ctx, &mut store, TickRequest { thread_id: thread }).expect("tick")
    }

    fn status(&self, now: &str) -> StatusReport {
        self.engine()
            .status(now, StatusScope::Workspace)
            .expect("status reads")
    }

    fn operation(&self, now: &str, root: &str) -> StatusOperation {
        self.status(now)
            .operations
            .iter()
            .find(|op| op.root == root)
            .cloned()
            .unwrap_or_else(|| panic!("operation {root} is not on the default listing"))
    }

    fn triggers(&self) -> Vec<String> {
        let raw = std::fs::read_to_string(self.root().join("dry.log")).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        for line in raw.lines() {
            if line.starts_with("trigger role=") {
                out.push(line.to_string());
            } else if let Some(last) = out.last_mut() {
                last.push('\n');
                last.push_str(line);
            }
        }
        out
    }

    fn started(&self, handle: &str) -> bool {
        self.triggers()
            .iter()
            .any(|l| l.starts_with(&format!("trigger role={handle} ")))
    }

    fn session_of(&self, handle: &str) -> String {
        let line = self
            .triggers()
            .into_iter()
            .find(|l| l.starts_with(&format!("trigger role={handle} ")))
            .unwrap_or_else(|| panic!("no trigger for {handle} in {:?}", self.triggers()));
        line.split(" session=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("session field")
            .to_string()
    }

    /// Who the lease ROW names, expired or not.
    fn lease_holder(&self) -> Option<String> {
        self.store()
            .working_tree_lease_row()
            .expect("the lease row reads")
            .map(|(holder, _)| holder)
    }

    /// When the current holder's claim runs out.
    fn lease_expires(&self) -> String {
        self.store()
            .working_tree_lease_row()
            .expect("the lease row reads")
            .expect("somebody holds the working copy")
            .1
    }

    /// Put the tree in the middle of a merge. `MERGE_HEAD` is the marker `WorkingCopy::mid_sequence`
    /// asks git for, and it is all a tree needs to be refused as mid-flight; writing it rather than
    /// provoking a real conflict keeps the operation's uncommitted work exactly where it is.
    fn start_a_merge(&self) {
        let head = git(self.root(), &["rev-parse", "HEAD"]);
        std::fs::write(self.merge_head(), format!("{head}\n")).unwrap();
    }

    fn merge_head(&self) -> PathBuf {
        self.root().join(".git").join("MERGE_HEAD")
    }

    /// The park branches this repository has, as git lists them.
    fn park_branches(&self) -> String {
        git(self.root(), &["branch", "--list", "nxs/park/*"])
    }

    fn head_branch(&self) -> String {
        git(self.root(), &["rev-parse", "--abbrev-ref", "HEAD"])
    }
}

fn scope_of(thread: &str) -> String {
    WorkScope::Thread(thread.to_string()).key()
}

/// The one finding of `class` on a receipt — and none of the other park class beside it, so a
/// refusal cannot be reported as both held and handed on.
fn the_finding(warnings: &[FailedConsequence], class: ConsequenceClass) -> FailedConsequence {
    let other = match class {
        ConsequenceClass::WorkNotParked => ConsequenceClass::WorkHandedOnUnparked,
        _ => ConsequenceClass::WorkNotParked,
    };
    assert!(
        !warnings.iter().any(|w| w.class == other),
        "a refusal is held OR handed on, never both: {warnings:?}"
    );
    let found: Vec<&FailedConsequence> = warnings.iter().filter(|w| w.class == class).collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one {class:?} finding: {warnings:?}"
    );
    found[0].clone()
}

fn no_park_finding(receipt: &TickReceipt) {
    assert!(
        !receipt.warnings.iter().any(|w| matches!(
            w.class,
            ConsequenceClass::WorkNotParked | ConsequenceClass::WorkHandedOnUnparked
        )),
        "nothing was refused: {:?}",
        receipt.warnings
    );
}

/// Nothing of this operation's moved: no branch, no row, the tree exactly as the session left it,
/// and the claim still the holder's with the rival still waiting.
fn the_copy_stayed(b: &Bench, thread: &str) {
    assert_eq!(b.park_branches(), "", "nothing was committed anywhere");
    assert!(b.store().parked_work(&scope_of(thread)).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "the work is exactly where the session left it"
    );
    assert!(b.root().join("notes.md").exists());
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(thread).as_str()));
    assert!(!b.started("solo"), "the rival waits");
}

/// The shape every run here starts from: the coder takes the copy INSIDE a two-hour bound, leaves
/// work in the tree, and `solo` queues behind it. Returns `(coder's thread, coder's session,
/// solo's thread)`.
fn a_holder_with_a_rival(b: &Bench) -> (String, String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    std::fs::write(b.root().join("notes.md"), NOTES).unwrap();
    let solo = b.commission(QUEUED, "solo");
    assert!(!b.started("solo"), "the rival is queued, not started");
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert_eq!(
        b.lease_expires(),
        THE_BOUND,
        "the flat two-hour bound — every tick here fires ninety minutes before it"
    );
    (thread, session, solo)
}

// ---- the occasion itself -------------------------------------------------------------------------

/// **The whole of this item.** The coder's process is gone, it still owes the answer it was
/// commissioned for, and it never announced an end — so it did not finish, it died. Its work goes
/// onto a branch and the queue moves, ninety minutes before the bound that used to be the only
/// thing watching.
#[test]
fn a_holder_killed_inside_its_bound_is_parked_and_the_queue_moves() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);

    let receipt = b.tick(INSIDE, &thread);

    let handed = receipt
        .handed_on
        .clone()
        .expect("the copy was taken from the dead holder");
    assert_eq!(handed.scope, scope_of(&thread));
    assert_eq!(handed.occasion, ParkOccasion::DiedInsideItsBound);
    assert!(
        receipt.working_tree.is_none(),
        "not a sweep: the lease had not run out — {:?}",
        receipt.working_tree
    );
    let parked = handed.parked.clone().expect("its work was parked first");
    assert!(parked.committed, "there was work to save");
    assert!(parked.created_branch, "on a branch of its own");
    assert!(handed.unparked.is_none(), "{handed:?}");
    no_park_finding(&receipt);

    // The branch holds BOTH files — the tracked change and the one git had never been told about.
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim()
    );
    assert_eq!(
        git(b.root(), &["show", &format!("{}:notes.md", parked.branch)]),
        NOTES.trim()
    );

    // …and the row that lets the operation be told where it is if it ever comes back.
    let rows = b.store().parked_work(&scope_of(&thread)).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].parked.branch, parked.branch);
    assert_eq!(rows[0].parked.base_branch, BASE);

    // The tree the rival starts in is the base branch, clean.
    assert_eq!(b.head_branch(), BASE);
    assert_eq!(git(b.root(), &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\n"
    );
    assert!(!b.root().join("notes.md").exists());

    // And the rival really started, holding the copy.
    assert_eq!(
        handed.promotions.started.len(),
        1,
        "{:?}",
        handed.promotions
    );
    assert!(b.started("solo"));
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));

    // `--json` carries the occasion by name, on the field the hand-off belongs to.
    let json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(json["handed_on"]["occasion"], "died_inside_its_bound");
    assert_eq!(json["handed_on"]["parked"]["branch"], parked.branch);
    assert!(json.get("working_tree").is_none(), "{json}");
}

/// **A holder that is still WRITING keeps the copy** — the guard no park may ever be without, and
/// the one this occasion could most easily have got wrong: inside the bound, a live session is the
/// ordinary state of every working operation in the world.
#[test]
fn a_holder_still_running_inside_its_bound_keeps_the_copy() {
    let b = Bench::new();
    let (thread, _session, _solo) = a_holder_with_a_rival(&b);
    // …and nothing marks the coder's session gone: it is still writing.

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
    assert!(
        b.operation(INSIDE, &thread).park_refused.is_none(),
        "nothing was attempted, so nothing is being retried"
    );
}

/// **A holder whose sessions ended normally and handed the task back is not DEAD, it is STRANDED**
/// — and that occasion has its own thirty-minute contention rule, because a human may still be
/// about to answer. Taking the copy early here would commit somebody's work out from under an
/// escalation they are in the middle of reading.
///
/// Both halves, so this is a claim about which occasion owns it rather than about nothing
/// happening: at [`INSIDE`] nothing moves, and once the contention deadline passes the copy does go
/// — under `stranded_escalation`.
#[test]
fn a_holder_that_handed_back_is_left_to_the_escalation_rule() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.escalate(ESCALATED, &session, &thread);
    b.worker.mark_gone(&session);
    assert!(
        b.store().working_tree_contended_since().unwrap().is_some(),
        "the premise: the escalation's own occasion is the one standing here"
    );

    let early = b.tick(INSIDE, &thread);

    assert!(early.handed_on.is_none(), "{:?}", early.handed_on);
    assert!(early.working_tree.is_none());
    no_park_finding(&early);
    the_copy_stayed(&b, &thread);

    // …and the rule that DOES own it still gets it, at its own deadline.
    let late = b.tick(PAST_THE_PARK_DEADLINE, &thread);
    let parked = late.parked.expect("the stranded escalation parked it");
    assert_eq!(parked.scope, scope_of(&thread));
    assert!(b.started("solo"));
}

/// **A worker that cannot answer the liveness question never takes a copy early.**
///
/// [`Worker::session_is_running`] has a fail-OPEN default of `false`, and every other caller reads
/// it as permission to release a claim whose clock has already run out. This one would read it as
/// evidence that a chain is DEAD, and "I never looked" is not evidence of anything — so the copy of
/// an operation that is very possibly still working would be committed and taken away on every host
/// that does not implement the probe.
#[test]
fn a_worker_that_cannot_answer_liveness_never_takes_a_copy_early() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    // Exactly the shape of a worker that never looked: "nothing is running" and no way to know.
    b.worker.mark_gone(&session);
    b.worker.stop_looking();
    assert!(
        !b.status(INSIDE).worker_answers_liveness,
        "the premise, as a host reads it"
    );

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
}

/// **A session that REPORTED its end is not a hard death.** The teardown ran — the process shut
/// itself down and said so — whatever the register then says about an unanswered obligation. That
/// is a session that ended without replying, and acting on it as if it were a crash would be
/// reading a tidy shutdown as a corpse.
#[test]
fn a_session_that_reported_its_end_is_not_taken_early() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    // The one difference from the run at the top of this file.
    b.store()
        .mark_session_ended(&session, "2026-09-08T09:20:00Z")
        .expect("the session announces its end");

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
}

/// **A thread that owes an answer and never ran a session is waiting for a PERSON, not dying.**
/// Nothing there has a process to have lost, so "no process is running" says nothing at all about
/// it — and committing an operation's work because a human has not replied yet would be the most
/// obviously wrong reading this rule could have.
///
/// Staged by clearing the session's `thread` column, which is exactly the shape the store has
/// whenever the party that owes an answer is one this device never ran a session for — a human, or
/// a session another device ran. (`record_session_thread(.., None)` is a documented no-op, so the
/// column is cleared directly.)
#[test]
fn a_thread_that_owes_an_answer_with_no_session_of_its_own_is_not_a_death() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    let moved = b
        .store()
        .connection()
        .execute(
            "UPDATE session_map SET thread = NULL WHERE internal_id = ?1",
            [&session],
        )
        .expect("the session is no longer recorded on that thread");
    assert_eq!(moved, 1, "the premise: that thread now has no session");

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
}

/// **An interrupted holder is not taken early by THIS rule** — the availability boundary has its
/// own path, its own window, its own "is the round already answered" question and its own resume.
///
/// The hold here is deliberately one that has FALLEN DUE, which is what makes this a test of this
/// rule rather than of the one next door: a due hold is the resume's to take up, so
/// `park_an_interrupted_holder` steps over it and the decision lands squarely on
/// `holder_is_provably_dead` — whose fifth clause is deliberately wider than the park step's. What
/// declining costs is a minute; what acting would cost is parking an operation somebody is at that
/// moment resuming.
#[test]
fn an_interrupted_holder_is_not_taken_early_by_this_rule() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    b.store()
        .record_interruption(
            &session,
            "five_hour",
            Some("2026-09-08T09:20:00Z"), // already due at INSIDE
            "You've hit your 5-hour limit",
            "2026-09-08T09:15:00Z",
        )
        .expect("the hold is recorded");

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
}

/// **A copy nobody is waiting for is never taken.** The whole occasion exists to move a working
/// copy for a commission that is standing still because of it; with an empty queue there is no
/// standing still, and a dead operation's tree is left exactly as it is for whoever comes to look
/// at it.
#[test]
fn a_dead_holder_nobody_is_waiting_for_keeps_its_tree() {
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    std::fs::write(b.root().join("notes.md"), NOTES).unwrap();
    b.worker.mark_gone(&session);

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    no_park_finding(&receipt);
    the_copy_stayed(&b, &thread);
}

// ---- the cadence ---------------------------------------------------------------------------------

/// **The sweep comes back in a MINUTE, not at the bound** (requirement 4) — which is the other half
/// of the item, because a rule nothing runs is not a rule. A holder alive at 09:30 can be dead at
/// 09:31, and a wait re-armed at its two-hour bound would find out at 11:00.
///
/// Asserted on what is ARMED, which is the only place this is visible: `arm_the_lease_bound` files
/// one window per thread and REPLACES it, so the instant recorded last for the waiting commission
/// is the instant something will actually come back at. The holder here is ALIVE — this is the
/// decline that happens on every tick of every ordinary operation, and the one that used to send
/// the waiter away for two hours.
#[test]
fn the_sweep_rechecks_at_the_liveness_cadence_while_someone_waits() {
    let b = Bench::new();
    let (thread, _session, solo) = a_holder_with_a_rival(&b);

    b.tick(INSIDE, &thread);

    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(NEXT_LOOK),
        "a decline inside the bound comes back one liveness re-check out: {:?}",
        b.timer.armed.lock().unwrap()
    );
    assert_ne!(
        b.timer.last_for(&solo).as_deref(),
        Some(THE_BOUND),
        "…and emphatically not at the holder's bound, which is the wait this item removes"
    );

    // **And it KEEPS re-arming**, which is the half a single look would not give: however long the
    // holder lives, the next look is always a minute away, so the tick that finds it dead is at
    // most a minute after it died.
    b.tick(NEXT_LOOK, &thread);
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(SECOND_LOOK),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
}

// ---- the refusal rule, inherited -----------------------------------------------------------------

/// The held state both of the runs below start from: the holder is dead inside its bound, somebody
/// is waiting — and the tree is mid-merge, so the park is refused for a reason a person can fix.
fn a_dead_holder_mid_merge_is_held(b: &Bench) -> (String, String, String) {
    let (thread, session, solo) = a_holder_with_a_rival(b);
    b.worker.mark_gone(&session);
    b.start_a_merge();

    let receipt = b.tick(INSIDE, &thread);

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    assert_eq!(
        finding.thread.as_deref(),
        Some(thread.as_str()),
        "anchored at the claim root"
    );
    assert!(
        finding.detail.contains("inside its bound"),
        "it names the occasion: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("a merge is in progress"),
        "and the state that refused it: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("retries") && finding.detail.contains("every tick"),
        "and that somebody is retrying it: {}",
        finding.detail
    );
    // The rule this occasion inherits along with the park (requirement 2 of nxf 6j6v.b9nf): no
    // refusal text offers a verb that has left the surface.
    for text in &receipt.warnings {
        assert!(
            !text.detail.contains("nxc release"),
            "a refusal text names the release verb: {}",
            text.detail
        );
    }

    // The claim stays, the queue waits, the tree is untouched — merge state included.
    the_copy_stayed(b, &thread);
    assert!(b.merge_head().exists(), "the merge state is left alone");

    // `nxc status` says so on the holding operation, stamped with THIS occasion.
    let note = b
        .operation(INSIDE, &thread)
        .park_refused
        .expect("the refusal is shown");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.occasion, "died_inside_its_bound");
    assert_eq!(note.since, INSIDE);

    // …and the retry is armed one liveness re-check out, keyed on the waiting commission.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(NEXT_LOOK),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    (thread, session, solo)
}

/// **A refusal that can pass keeps the copy here too**, and the copy goes by itself once the reason
/// is gone. Handing an agent a checkout that is mid-merge is worse than making it wait.
#[test]
fn a_dead_holder_mid_merge_keeps_the_copy_until_the_merge_is_resolved() {
    let b = Bench::new();
    let (thread, _session, solo) = a_dead_holder_mid_merge_is_held(&b);

    std::fs::remove_file(b.merge_head()).unwrap(); // `git merge --quit`
    let receipt = b.tick(NEXT_LOOK, &thread);

    let handed = receipt.handed_on.clone().expect("the retry took the copy");
    assert_eq!(handed.occasion, ParkOccasion::DiedInsideItsBound);
    let parked = handed.parked.clone().expect("and parked first");
    assert!(parked.committed);
    no_park_finding(&receipt);
    assert_eq!(
        git(b.root(), &["show", &format!("{}:notes.md", parked.branch)]),
        NOTES.trim()
    );
    assert_eq!(b.head_branch(), BASE);
    assert_eq!(git(b.root(), &["status", "--porcelain"]), "");
    assert!(b.started("solo"));
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));
    assert!(
        b.operation(NEXT_LOOK, &thread).park_refused.is_none(),
        "the refusal went with the claim it was about"
    );
}

/// **A refusal is retried, and marked, only while its occasion stands** — the rule two fix rounds of
/// nxf 6j6v.8bv9 established, for this occasion's own stamp. A session that starts answering again
/// is a holder that is not dead, so nobody is retrying that park any more, though the tree is still
/// mid-merge and somebody is still waiting.
#[test]
fn a_held_refusal_goes_when_the_holder_turns_out_to_be_alive() {
    let b = Bench::new();
    let (thread, session, solo) = a_dead_holder_mid_merge_is_held(&b);

    b.worker.mark_running(&session);
    let receipt = b.tick(NEXT_LOOK, &thread);

    assert!(receipt.handed_on.is_none());
    no_park_finding(&receipt);
    assert!(
        b.operation(NEXT_LOOK, &thread).park_refused.is_none(),
        "nobody is retrying it: {:?}",
        b.operation(NEXT_LOOK, &thread).park_refused
    );
    assert!(
        b.store()
            .park_refusal(&scope_of(&thread))
            .unwrap()
            .is_none(),
        "and no row is left behind"
    );
    // What did NOT change: the claim, the reason, and the work.
    the_copy_stayed(&b, &thread);
    assert!(b.merge_head().exists());
    // …and something still comes back, because the queue is still a queue.
    assert_eq!(b.timer.last_for(&solo).as_deref(), Some(SECOND_LOOK));
}

/// The counter-case, so the clearing above cannot be over-eager: the occasion still stands, so the
/// mark stays, `retrying since` keeps its FIRST instant, and the cadence is armed again.
#[test]
fn a_held_refusal_keeps_its_mark_while_the_holder_is_still_dead() {
    let b = Bench::new();
    let (thread, _session, solo) = a_dead_holder_mid_merge_is_held(&b);

    let receipt = b.tick(NEXT_LOOK, &thread);

    assert!(receipt.handed_on.is_none());
    the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    let note = b
        .operation(NEXT_LOOK, &thread)
        .park_refused
        .expect("still refused");
    assert_eq!(note.occasion, "died_inside_its_bound");
    assert_eq!(note.since, INSIDE, "not this retry's own instant");
    assert_eq!(note.last_tried, NEXT_LOOK, "…which is what moves");
    assert_eq!(b.timer.last_for(&solo).as_deref(), Some(SECOND_LOOK));
    the_copy_stayed(&b, &thread);
}

/// **A PERMANENT refusal hands the copy on unparked** — the other half of the rule, inherited whole.
/// An operation with no recorded base has nowhere to put the tree back to, and no tick will ever
/// change that, so holding the queue for it would hold it for ever.
#[test]
fn a_dead_holder_that_never_recorded_a_base_is_handed_on_unparked() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    let deleted = b
        .store()
        .connection()
        .execute(
            "DELETE FROM operation_base WHERE scope_key = ?1",
            [scope_of(&thread)],
        )
        .expect("delete the recorded base");
    assert_eq!(deleted, 1, "a base had been recorded on the first acquire");
    b.worker.mark_gone(&session);

    let receipt = b.tick(INSIDE, &thread);

    let handed = receipt.handed_on.clone().expect("the copy went on");
    assert_eq!(handed.occasion, ParkOccasion::DiedInsideItsBound);
    assert!(handed.parked.is_none(), "{handed:?}");
    assert!(
        matches!(handed.unparked, Some(ParkRefusal::NoBaseRecorded(_))),
        "{handed:?}"
    );
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["handed_on"]["unparked"]["refusal"],
        "no_base_recorded"
    );
    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkHandedOnUnparked);
    assert!(
        finding.detail.contains("inside its bound"),
        "it says why the copy was being taken: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("WITHOUT parking"),
        "…and what happened to the work instead: {}",
        finding.detail
    );

    assert_eq!(b.park_branches(), "");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "the uncommitted work stays in the tree for the next holder"
    );
    assert!(b.started("solo"));
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));
}

// ---- the reclaim is still a compare-and-swap ------------------------------------------------------

/// **A holder that renews between the read and the write keeps its lease** — the race the controller
/// asked to be tested, and the reason this occasion may drop the `expires <= now` clause and nothing
/// else.
///
/// The park runs between the two: `park_and_hand_on` commits the work and only THEN reclaims, which
/// is the whole point. So a trigger the claim area makes in that window rides its bound forward, and
/// the delete — which names the exact `expires` that was read — matches nothing. Here that renewal
/// also disproves the evidence the reclaim rested on: a chain that triggers is a chain with a
/// process.
#[test]
fn a_holder_that_renews_while_its_work_is_parked_keeps_its_lease() {
    let b = Bench::new();
    let (thread, _session, _solo) = a_holder_with_a_rival(&b);
    let read_a_moment_ago = b.lease_expires();
    assert_eq!(read_a_moment_ago, THE_BOUND);

    // The statement every trigger in the claim area runs: an inherit on the key that already holds
    // the lease, which moves `expires` forward.
    let mut store = b.store();
    assert!(store
        .acquire_working_tree(&WorkScope::Thread(thread.clone()), INSIDE, RENEWED_UNTIL)
        .expect("the holder's own trigger inherits its lease"));

    let taken = store
        .reclaim_a_dead_holders_working_tree_and_take_next(
            &scope_of(&thread),
            &read_a_moment_ago,
            INSIDE,
            THE_BOUND,
        )
        .expect("the reclaim runs");

    assert!(
        taken.is_empty(),
        "nothing was handed on: the row this decided about is not the row that is there now"
    );
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str()),
        "the holder keeps the working copy"
    );
    assert_eq!(b.lease_expires(), RENEWED_UNTIL, "with its renewed bound");
    assert!(!b.started("solo"), "and the queue is still a queue");

    // …and the same call against the row that IS there does hand the copy on, so the assertion
    // above is about the compare-and-swap and not about the method refusing everything.
    let taken = store
        .reclaim_a_dead_holders_working_tree_and_take_next(
            &scope_of(&thread),
            RENEWED_UNTIL,
            INSIDE,
            THE_BOUND,
        )
        .expect("the reclaim runs");
    assert_eq!(taken.len(), 1, "{taken:?}");
}

/// **…and the commission waiting behind that race still gets its next look** (fix round 1, Finding
/// 2) — the half the store-level test above structurally cannot see.
///
/// The renewal happens for real here, inside a real tick: the holder's own trigger rides its bound
/// forward while its work is being committed, so the park goes through and the reclaim that follows
/// matches nothing. Everything about that is right — the work is safe on a branch, the copy stays
/// with a holder that just proved it is alive.
///
/// What was wrong is what came next: the tick reported the hand-off and returned, and the ONE alarm
/// the waiting commission had was the one this tick had just spent. Nothing was scheduled to look
/// again, so the rival's wait fell back to "until some other chain releases" — the wedge this whole
/// item removes, reached through the one door that skipped the re-arm. A tick that moves nothing
/// arms the next look, and that is true of this tick too.
#[test]
fn a_reclaim_that_loses_the_race_still_arms_the_waiters_next_look() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    // Armed BEFORE the tick, fired from inside the park: see `LiveParkingWorker::renew`.
    b.worker
        .renew_the_lease_once(b.root(), &thread, RENEWED_UNTIL);
    assert!(
        b.timer.last_for(&solo).is_none(),
        "the rival's own queueing armed its alarm through the handle's timer; every window THIS \
         timer records is one the tick below asked for"
    );

    let receipt = b.tick(INSIDE, &thread);

    // The park happened and is reported — the work IS on a branch, whatever the reclaim then did.
    let handed = receipt.handed_on.clone().expect("the work was parked");
    assert_eq!(handed.occasion, ParkOccasion::DiedInsideItsBound);
    assert!(handed.parked.is_some(), "{handed:?}");
    no_park_finding(&receipt);

    // …and the copy did NOT move: the delete named the `expires` this tick read, and that row is
    // not the row that is there now.
    assert!(
        handed.promotions.started.is_empty() && handed.promotions.requeued.is_empty(),
        "the compare-and-swap lost, so nothing was promoted: {:?}",
        handed.promotions
    );
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str()),
        "the holder keeps the working copy"
    );
    assert_eq!(b.lease_expires(), RENEWED_UNTIL, "with its renewed bound");
    assert!(!b.started("solo"), "and the queue is still a queue");

    // The claim under test: the rival's alarm was spent by this tick, so this tick owes it another.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(NEXT_LOOK),
        "a tick that took nothing arms the next look, on the cadence: {:?}",
        b.timer.armed.lock().unwrap()
    );
}
