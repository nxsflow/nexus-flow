//! ACCEPTANCE of nxf 6j6v.8bv9 — **the sweep parks the work of a holder past its bound**.
//!
//! The last hand-off of the working copy that did not save anything first. A chain that died hard
//! without escalating is not contended by `park_the_stranded_holder`'s definition — nothing handed
//! its task back — so until this item `sweep_expired_working_tree` reclaimed its checkout with its
//! uncommitted work still sitting in the tree, and the next holder started in somebody else's
//! files. The owner's rule of 2026-09-17 is that EVERY hand-off caused by trouble parks first, and
//! this is the occasion that was left: **the lease ran out and nothing is running behind it**.
//!
//! It is the same park, the same refusal rule and the same retry as the two occasions before it
//! (`an_unanswered_escalation_parks_and_comes_back.rs`,
//! `an_interrupted_operation_hands_the_working_copy_on.rs`, `a_refused_park_follows_the_rule.rs`),
//! so what this file pins is what is NEW about running it from the sweep:
//!
//! - the work is on a branch and the tree is back on its base BEFORE the rival starts;
//! - a holder that is past its bound but still has a live process is not touched at all — the
//!   guard that predates this item and matters more now that a park writes into the tree;
//! - a refusal that can pass **keeps the copy**, which is what closes the hole the refusal rule
//!   shipped with: an expired lease used to let the sweep reclaim two lines after the very tick had
//!   said "the claim stays where it is";
//! - …and a refusal is only ever attempted, and only ever marked, while somebody is waiting for the
//!   copy and the bound is still past — the "a mark that says retrying is only true while something
//!   is retrying" rule, for this occasion's own stamp (`past_its_bound`).
//!
//! Everything here is real: a real git repository, the real lease, the real queue, the real park.
//! The engine handle carries the commissions and every status read; `orchestration::tick` carries
//! the tick, which has no handle verb — it is what the background service runs. The only stand-in
//! is the worker, and only for what a worker does: it records its triggers, names where sessions
//! run, and answers which of them still have a process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{StatusOperation, StatusReport, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, Ctx, FailedConsequence, TickReceipt, TickRequest,
};
use nexus_chat::park::ParkRefusal;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToReceipt, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here and takes the working copy. Nothing declares a `timeout:`, so its
/// lease gets the flat two-hour bound.
const T0: &str = "2026-09-08T09:00:00Z";
/// The coder escalates here — only the one run that needs a SECOND occasion standing beside this
/// one uses it.
const ESCALATED: &str = "2026-09-08T09:05:00Z";
/// `solo` queues behind the copy here.
const QUEUED: &str = "2026-09-08T09:10:00Z";
/// The instant the coder's lease runs out: [`T0`] plus `WORKING_TREE_LEASE_BOUND`.
const THE_BOUND: &str = "2026-09-08T11:00:00Z";
/// One second inside it — a tick here sweeps nothing.
const WITHIN_THE_BOUND: &str = "2026-09-08T10:59:59Z";
/// One second past it: from here on the copy is the queue's, if nothing is running for the holder.
const PAST_THE_BOUND: &str = "2026-09-08T11:00:01Z";
/// One liveness re-check after [`PAST_THE_BOUND`] — where a refused park's retry is armed.
const RETRY: &str = "2026-09-08T11:01:01Z";
/// …and one after that, for a refusal whose occasion keeps standing.
const SECOND_RETRY: &str = "2026-09-08T11:02:01Z";
/// Where a holder's bound lands once a trigger of its own has ridden it forward again.
const RENEWED_UNTIL: &str = "2026-09-08T13:00:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records its triggers like [`DryWorker`], names where sessions run, and — the part
/// this file needs and the sibling acceptance files do not — **answers which sessions still have a
/// process**, the way `a_way_out_of_a_held_lease.rs`'s `LiveSessionWorker` does.
///
/// A session it started is running until [`Self::mark_gone`] says otherwise, which is the shape of
/// the thing being tested: the sweep acts on a holder whose processes are gone, and on no other.
struct LiveParkingWorker {
    log: PathBuf,
    working_copy: Option<PathBuf>,
    running: Mutex<HashSet<String>>,
    /// **A lease renewal to run ONCE, from inside the park** — `(workspace root, the holder's
    /// thread, the instant it renews AT, the bound it renews TO)`.
    ///
    /// It hangs on the worker because `working_copy()` is the first thing the park asks, which is
    /// the one moment that sits AFTER the sweep read the holder's `expires` and BEFORE the reclaim
    /// names it. That is the compare-and-swap's window, and nothing outside the tick can reach it.
    /// Same hook, same reason, as the sibling file for the occasion inside the bound.
    renew: Mutex<Option<(PathBuf, String, String, String)>>,
}

impl LiveParkingWorker {
    fn new(log: PathBuf, working_copy: Option<PathBuf>) -> Self {
        LiveParkingWorker {
            log,
            working_copy,
            running: Mutex::new(HashSet::new()),
            renew: Mutex::new(None),
        }
    }
    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
    /// …and back again, for the one run that stages a lease RENEWAL by hand: a renewal is a trigger,
    /// and a trigger is a chain with a process (nxf 6j6v.xb24).
    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
    /// The next park of this workspace renews `thread`'s lease to `until` first — the holder's own
    /// trigger arriving in the middle of the park, staged where nothing else can stage it.
    fn renew_the_lease_once(&self, root: &Path, thread: &str, now: &str, until: &str) {
        *self.renew.lock().unwrap() = Some((
            root.to_path_buf(),
            thread.to_string(),
            now.to_string(),
            until.to_string(),
        ));
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
        true
    }
    fn working_copy(&self) -> Option<PathBuf> {
        // The staged renewal, if one is armed — taken, so it happens exactly once.
        if let Some((root, thread, now, until)) = self.renew.lock().unwrap().take() {
            let mut store = Workspace::resolve(None, &root)
                .expect("resolve workspace")
                .open_chat_store()
                .expect("open chat store");
            assert!(
                store
                    .acquire_working_tree(&WorkScope::Thread(thread), &now, &until)
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

/// A timer that records every window it is asked to arm, in order — the retry cadence of a refused
/// park is a fact about WHAT IS ARMED, and nothing else can see it. Arming replaces by key in the
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

/// A chat workspace that is ALSO a git repository on [`BASE`], with three personas that each want
/// the working copy to themselves.
///
/// The third is the NEWCOMER of fix round 1: the runs that go through the acquire path need one
/// operation holding the copy, a second already in line for it, and a third whose own trigger is
/// what arrives — three claim areas, because the whole question there is who overtakes whom.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "solo", "newcomer"] {
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

    /// The human commissions a persona. Returns the whole receipt — the runs that go through the
    /// ACQUIRE path read `queued_behind` and `warnings` off it, because on that path the commission
    /// itself is what hands the copy on and its receipt is the only one there is.
    fn commission_receipt(&self, now: &str, to: &str) -> SendToReceipt {
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
    }

    /// The human commissions a persona. Returns the thread it opened.
    fn commission(&self, now: &str, to: &str) -> String {
        self.commission_receipt(now, to).thread_id
    }

    /// The persona hands its task back to the human — the second occasion, for the one run that
    /// needs two standing at once.
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

    /// The operation rooted at `root` on the DEFAULT listing — absent there is a failure of its
    /// own, which is why this panics rather than answering `None`.
    fn operation(&self, now: &str, root: &str) -> StatusOperation {
        let report = self.status(now);
        report
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

    /// Who the lease ROW names, expired or not — `working_tree_holder` answers `None` over an
    /// expired row, and "the claim stays with the holder past its bound" is exactly what several of
    /// these runs are about.
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

    /// Finish it the way `git merge --quit` does: the sequencer state goes, the tree stays.
    fn finish_the_merge(&self) {
        std::fs::remove_file(self.merge_head()).unwrap();
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

/// The shape every run here starts from: the coder takes the copy, leaves work in the tree, and
/// `solo` queues behind it. Returns `(coder's thread, coder's session, solo's thread)`.
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
        "the flat two-hour bound, which every instant here is measured against"
    );
    (thread, session, solo)
}

/// The same shape with **nobody in line** — the ordinary single-agent day: one operation took the
/// copy, left work in the tree and died, and the next thing that happens on this device is somebody
/// asking for the copy. Returns `(the holder's thread, its session)`.
fn a_holder_with_nobody_waiting(b: &Bench) -> (String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    std::fs::write(b.root().join("notes.md"), NOTES).unwrap();
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert_eq!(b.lease_expires(), THE_BOUND);
    assert!(
        b.store().peek_working_tree_queue().unwrap().is_none(),
        "nobody is waiting for the copy, which is the whole point of this shape"
    );
    (thread, session)
}

// ---- the occasion itself -------------------------------------------------------------------------

/// **The whole of this item.** A holder whose processes are gone and whose lease has run out loses
/// the working copy — and its work goes onto a branch first, tracked change and untracked file
/// alike, with the tree back on the branch the operation started from before the rival is started.
#[test]
fn a_dead_holder_past_its_bound_has_its_work_parked_before_the_copy_goes_on() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    let swept = receipt
        .working_tree
        .clone()
        .expect("the sweep took the copy");
    assert_eq!(swept.scope, scope_of(&thread));
    assert_eq!(swept.expired, THE_BOUND);
    let parked = swept.parked.clone().expect("its work was parked first");
    assert!(parked.committed, "there was work to save");
    assert!(parked.created_branch, "on a branch of its own");
    assert!(swept.unparked.is_none(), "{swept:?}");
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

    // …and the row that lets the operation be told where it is when it comes back.
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
        swept.promotions.started.len(),
        1,
        "{:?}",
        swept.promotions.started
    );
    assert!(b.started("solo"));
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));

    // `--json` carries the park on the sweep it belongs to.
    let json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        json["working_tree"]["parked"]["branch"], parked.branch,
        "{json}"
    );
    assert_eq!(json["working_tree"]["scope"], scope_of(&thread), "{json}");
}

/// **A bound running out is not enough to take the copy** — the guard that predates this item and
/// matters more now that acting on it WRITES into the tree. A `git checkout` against a tree a live
/// session is building in is the corruption this whole epic exists to prevent.
#[test]
fn a_live_holder_past_its_bound_is_left_alone() {
    let b = Bench::new();
    let (thread, _session, solo) = a_holder_with_a_rival(&b);
    // …and nothing marks the coder's session gone: it is still writing.

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    assert!(receipt.working_tree.is_none(), "{:?}", receipt.working_tree);
    no_park_finding(&receipt);
    assert_eq!(b.park_branches(), "", "nothing was committed anywhere");
    assert!(b
        .store()
        .parked_work(&scope_of(&thread))
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "the live session's work is exactly where it left it"
    );
    assert!(b.root().join("notes.md").exists());
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"), "the rival waits");
    assert!(
        b.operation(PAST_THE_BOUND, &thread).park_refused.is_none(),
        "nothing was attempted, so nothing is being retried"
    );
    // The look comes back on the liveness cadence rather than at a bound that has already passed.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
}

/// **A tick inside the bound does not sweep a LIVE holder**, park or no park — the direction that
/// would be a disaster to get wrong.
///
/// It used to mark the holder's session gone and claim the same thing, and that claim was made
/// false on purpose by nxf 6j6v.xb24: a holder inside its bound whose sessions are provably dead IS
/// parked now, which is the whole of that item and is covered by
/// `a_holder_that_dies_inside_its_bound_is_noticed.rs`. What this run keeps pinning is the part
/// that did not change and is the ordinary state of every working operation there is — a holder
/// with a live process keeps its checkout, bound or no bound.
#[test]
fn a_live_holder_inside_its_bound_is_not_parked_by_the_sweep() {
    let b = Bench::new();
    let (thread, _session, _solo) = a_holder_with_a_rival(&b);
    // …and nothing marks the coder's session gone: it is still writing.

    let receipt = b.tick(WITHIN_THE_BOUND, &thread);

    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    assert_eq!(b.park_branches(), "");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
}

/// **Task 2's rule, reached through the sweep** (nxf 6j6v.7hqc): a clean tree already standing on
/// the operation's own base is parked without minting a branch the engine will never delete. The
/// copy still goes on, and the operation is still told where it stood.
#[test]
fn a_dead_holder_with_a_clean_tree_is_handed_on_without_a_new_branch() {
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    let solo = b.commission(QUEUED, "solo");
    assert_eq!(
        git(b.root(), &["status", "--porcelain"]),
        "",
        "the premise: this operation left nothing behind"
    );
    b.worker.mark_gone(&session);

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    let swept = receipt.working_tree.clone().expect("the copy went on");
    let parked = swept.parked.clone().expect("a park still happened");
    assert!(!parked.committed, "there was nothing to commit: {parked:?}");
    assert!(!parked.created_branch, "and no branch to make: {parked:?}");
    assert_eq!(parked.branch, BASE);
    assert_eq!(b.park_branches(), "", "no park branch was minted");
    no_park_finding(&receipt);

    assert_eq!(b.head_branch(), BASE);
    assert!(b.started("solo"), "the rival started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));
}

/// **A permanent refusal hands the copy on unparked** — requirement 4, through the sweep. An
/// operation with no recorded base has nowhere to put the tree back to, and no tick will ever
/// change that, so holding the queue for it would hold it for ever.
#[test]
fn a_dead_holder_that_never_recorded_a_base_is_handed_on_unparked() {
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    // The base is recorded on the operation's first acquire; take it away before the tick.
    let deleted = b
        .store()
        .connection()
        .execute(
            "DELETE FROM operation_base WHERE scope_key = ?1",
            [scope_of(&thread)],
        )
        .expect("delete the recorded base");
    assert_eq!(deleted, 1, "a base had been recorded on the first acquire");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    let solo = b.commission(QUEUED, "solo");
    b.worker.mark_gone(&session);

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    let swept = receipt.working_tree.clone().expect("the copy went on");
    assert!(swept.parked.is_none(), "{swept:?}");
    assert!(
        matches!(swept.unparked, Some(ParkRefusal::NoBaseRecorded(_))),
        "{swept:?}"
    );
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["working_tree"]["unparked"]["refusal"],
        "no_base_recorded"
    );
    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkHandedOnUnparked);
    assert!(
        finding.detail.contains("past its bound"),
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
    assert!(
        b.operation(PAST_THE_BOUND, &thread).park_refused.is_none(),
        "a permanent refusal is not retried, so it is not shown as one"
    );
}

// ---- a refusal that can pass keeps the copy ------------------------------------------------------

/// The held state both of the runs below start from: the holder is dead, past its bound, somebody
/// is waiting — and the tree is mid-merge, so the park is refused for a reason a person can fix.
fn a_dead_holder_mid_merge_is_held(b: &Bench) -> (String, String) {
    let (thread, session, solo) = a_holder_with_a_rival(b);
    b.worker.mark_gone(&session);
    b.start_a_merge();

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    assert!(
        receipt.working_tree.is_none(),
        "the sweep declined: {:?}",
        receipt.working_tree
    );
    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    assert_eq!(
        finding.thread.as_deref(),
        Some(thread.as_str()),
        "anchored at the claim root"
    );
    assert!(
        finding.detail.contains("past its bound"),
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
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"), "the queue head has not started");
    assert_eq!(b.park_branches(), "");
    assert!(b
        .store()
        .parked_work(&scope_of(&thread))
        .unwrap()
        .is_empty());
    assert!(b.merge_head().exists(), "the merge state is left alone");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
    assert!(b.root().join("notes.md").exists());

    // `nxc status` says so on the holding operation, stamped with THIS occasion.
    let op = b.operation(PAST_THE_BOUND, &thread);
    let note = op.park_refused.clone().expect("the refusal is shown");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.occasion, "past_its_bound");
    assert_eq!(note.since, PAST_THE_BOUND);

    // …and the retry is armed one liveness re-check out, keyed on the waiting commission.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    (thread, solo)
}

/// **The hole the refusal rule shipped with, closed.** An expired lease used to let the sweep
/// reclaim the copy two lines after the same tick had said "the claim stays where it is and
/// whatever is queued keeps waiting" — so the one refusal a person can act on was also the one that
/// was silently overruled. The copy stays until the merge is finished, and then goes by itself.
#[test]
fn a_dead_holder_mid_merge_keeps_the_copy_until_the_merge_is_resolved() {
    let b = Bench::new();
    let (thread, solo) = a_dead_holder_mid_merge_is_held(&b);

    b.finish_the_merge();
    let receipt = b.tick(RETRY, &thread);

    let swept = receipt.working_tree.clone().expect("the retry swept");
    let parked = swept.parked.clone().expect("and parked first");
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
    let op = b.operation(RETRY, &thread);
    assert!(
        op.park_refused.is_none(),
        "the refusal went with the claim it was about: {:?}",
        op.park_refused
    );
}

/// **A refusal is retried, and marked, only while its occasion stands** (fix rounds 1 and 2 of nxf
/// 6j6v.8bv9, for this occasion's own stamp). The sweep moves the copy for somebody who is waiting
/// for it; withdraw that commission and nobody is retrying anything, though the holder is still
/// dead, still past its bound and the tree is still mid-merge.
#[test]
fn a_held_sweep_refusal_goes_when_nobody_waits_for_the_copy_any_more() {
    let b = Bench::new();
    let (thread, solo) = a_dead_holder_mid_merge_is_held(&b);

    b.engine()
        .withdraw(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(RETRY),
            },
            &solo,
        )
        .expect("the queued commission comes back");

    let receipt = b.tick(RETRY, &thread);

    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    let op = b.operation(RETRY, &thread);
    assert!(
        op.park_refused.is_none(),
        "nobody is retrying it: {:?}",
        op.park_refused
    );
    assert!(
        b.store()
            .park_refusal(&scope_of(&thread))
            .unwrap()
            .is_none(),
        "and no row is left behind"
    );
    // What did NOT change: the claim, the reason, and the work.
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(b.merge_head().exists());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
}

/// **The other way this occasion ends: the lease is not past its bound any more.** Every trigger a
/// chain makes rides its bound forward, and an inherit by the SAME holder deliberately keeps its
/// refusal row — so it is this function that has to notice that nothing is past anything any more.
/// A mark left standing sends a person to look at an operation nobody is waiting on.
///
/// **What this run no longer claims** (nxf 6j6v.xb24): that clearing the row also puts the waiting
/// commission back on the holder's own BOUND. Since that item every wait comes back on the liveness
/// cadence while somebody is queued — that is what lets a holder which dies inside its bound be
/// noticed in a minute — so the armed instant is the same either way and is no longer evidence
/// about the row. The row itself is, and it is what this asserts.
#[test]
fn a_held_sweep_refusal_goes_when_the_lease_is_no_longer_past_its_bound() {
    let b = Bench::new();
    let (thread, solo) = a_dead_holder_mid_merge_is_held(&b);

    // **The session is writing again first** (nxf 6j6v.xb24). A renewal is what a TRIGGER does, and
    // a trigger is a chain with a live process — so staging the renewal without it would stage a
    // state that cannot occur, and one that since xb24 means something else entirely: a holder that
    // is provably dead inside its bound is an occasion of its own, and this run would be watching
    // that occasion take over the retry rather than watching the row be forgotten.
    b.worker.mark_running(&b.session_of("coder"));
    // The statement every trigger in the claim area runs — `ChatStore::acquire_working_tree` on the
    // key that already holds the lease, which is an INHERIT: it moves `expires` and leaves the
    // refusal row exactly where it was.
    assert!(b
        .store()
        .acquire_working_tree(&WorkScope::Thread(thread.clone()), RETRY, RENEWED_UNTIL)
        .expect("the holder's own trigger inherits its lease"));
    assert!(
        b.store()
            .park_refusal(&scope_of(&thread))
            .unwrap()
            .is_some(),
        "the premise: an inherit does not clear the holder's own refusal"
    );

    let receipt = b.tick(RETRY, &thread);

    assert!(receipt.working_tree.is_none());
    no_park_finding(&receipt);
    let op = b.operation(RETRY, &thread);
    assert!(
        op.park_refused.is_none(),
        "the lease is not past anything, so nobody is retrying that park: {:?}",
        op.park_refused
    );
    // …and somebody still comes back for the copy, on the cadence every queued wait now uses.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(SECOND_RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(b.merge_head().exists(), "the reason itself is untouched");
}

/// The counter-case, so the clearing above cannot be over-eager: the occasion still stands, so the
/// mark stays, `retrying since` keeps its FIRST instant, and the cadence is armed again.
#[test]
fn a_held_sweep_refusal_keeps_its_mark_while_the_copy_is_still_wanted() {
    let b = Bench::new();
    let (thread, solo) = a_dead_holder_mid_merge_is_held(&b);

    let receipt = b.tick(RETRY, &thread);

    assert!(receipt.working_tree.is_none());
    the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    let note = b
        .operation(RETRY, &thread)
        .park_refused
        .expect("still refused");
    assert_eq!(note.occasion, "past_its_bound");
    assert_eq!(note.since, PAST_THE_BOUND, "not this retry's own instant");
    assert_eq!(note.last_tried, RETRY, "…which is what moves");
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(SECOND_RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"));
}

/// **Two occasions, one park, one tick.** A stranded escalation whose lease has ALSO run out is
/// acted on by the park step first; when that park is refused and HOLDS the claim, the sweep behind
/// it must not attempt the same park again in the same breath — nor undo the hold by reclaiming.
/// One refusal, one finding, one stamp.
#[test]
fn a_park_held_by_another_occasion_is_not_attempted_again_by_the_sweep() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.escalate(ESCALATED, &session, &thread);
    b.worker.mark_gone(&session);
    b.start_a_merge();
    assert!(
        b.store().working_tree_contended_since().unwrap().is_some(),
        "the premise: the stranded-escalation occasion stands too"
    );

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    // Exactly one refusal, and it belongs to the occasion that acted first.
    the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    let note = b
        .operation(PAST_THE_BOUND, &thread)
        .park_refused
        .expect("refused");
    assert_eq!(
        note.occasion, "stranded_escalation",
        "the occasion that acted owns the row"
    );
    // …and the sweep did not undo it.
    assert!(receipt.working_tree.is_none(), "{:?}", receipt.working_tree);
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"));
    assert_eq!(b.park_branches(), "");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
    // **And somebody is still coming back for it** (fix round 1). The declining `return` skips the
    // sweep's own re-arm, so what keeps the waiting commission on a clock is the retry the park step
    // armed a moment earlier — one liveness re-check out, keyed on the queue head. Without this the
    // run above passes over a workspace where nothing would ever look again.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
}

// ---- the same occasion, reached by a newcomer's own trigger (fix round 1) ------------------------

/// **The second door onto this occasion, and it used to be unparked** (fix round 1 of nxf
/// 6j6v.8bv9). A newcomer's own trigger runs no tick: `the_queue_goes_before_a_newcomer` reclaims
/// the expired lease on the ACQUIRE path, so that whoever is already in line goes before whoever
/// happens to ask (nxf 6j6v.yd4w). It had the sweep's guards minus the park — so whenever a
/// newcomer got there before a tick did, the dead holder's work was handed on still sitting in the
/// tree, which is exactly what this item exists to stop.
#[test]
fn a_newcomers_own_trigger_parks_the_dead_holders_work_before_the_copy_goes_on() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);

    // No tick anywhere: a third operation simply asks for the working copy.
    let newcomer = b.commission_receipt(PAST_THE_BOUND, "newcomer");

    // The dead holder's work went onto a branch — tracked change and untracked file alike.
    let rows = b.store().parked_work(&scope_of(&thread)).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let branch = rows[0].parked.branch.clone();
    assert_eq!(rows[0].parked.base_branch, BASE);
    assert_eq!(
        git(b.root(), &["show", &format!("{branch}:src.txt")]),
        HALF_FINISHED.trim()
    );
    assert_eq!(
        git(b.root(), &["show", &format!("{branch}:notes.md")]),
        NOTES.trim()
    );

    // …and only THEN did the copy move — to the one who was already waiting, not to the newcomer.
    assert_eq!(b.head_branch(), BASE);
    assert_eq!(git(b.root(), &["status", "--porcelain"]), "");
    assert!(!b.root().join("notes.md").exists());
    assert!(b.started("solo"), "the queue went first");
    assert!(!b.started("newcomer"), "and the newcomer took its place");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));
    assert_eq!(newcomer.queue_position, Some(1), "{newcomer:?}");
    assert!(
        !newcomer.warnings.iter().any(|w| matches!(
            w.class,
            ConsequenceClass::WorkNotParked | ConsequenceClass::WorkHandedOnUnparked
        )),
        "the park went through, so there is nothing to report: {:?}",
        newcomer.warnings
    );
}

/// **A refusal that can pass keeps the copy on this path too**, or the acquire path would overrule
/// the very rule the tick beside it obeys: the holder's work is still uncommitted in the tree, and
/// handing an agent a checkout mid-merge is worse than making it wait.
#[test]
fn a_newcomer_does_not_take_a_copy_whose_park_was_refused() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    b.start_a_merge();

    let newcomer = b.commission_receipt(PAST_THE_BOUND, "newcomer");

    // Nothing moved: the claim, the queue, the tree and its half-finished merge.
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"), "the queue head was not overtaken");
    assert!(!b.started("newcomer"));
    assert_eq!(b.park_branches(), "");
    assert!(b
        .store()
        .parked_work(&scope_of(&thread))
        .unwrap()
        .is_empty());
    assert!(b.merge_head().exists());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // The newcomer queued behind the holder it did not overtake, exactly as it would have while the
    // lease was live — and its receipt says the park was refused rather than leaving it to stderr.
    assert_eq!(
        newcomer.queued_behind.as_deref(),
        Some(scope_of(&thread).as_str()),
        "{newcomer:?}"
    );
    assert_eq!(newcomer.queue_position, Some(2), "{newcomer:?}");
    let finding = the_finding(&newcomer.warnings, ConsequenceClass::WorkNotParked);
    assert!(
        finding.detail.contains("past its bound")
            && finding.detail.contains("a merge is in progress"),
        "it names the occasion and what to fix: {}",
        finding.detail
    );

    // …and `nxc status` carries it on the holding operation, stamped with THIS occasion, so the
    // service's own retry is the one that eventually moves the copy.
    let note = b
        .operation(PAST_THE_BOUND, &thread)
        .park_refused
        .expect("the refusal is shown");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.occasion, "past_its_bound");

    // The way out: the copy goes by itself once the merge is done.
    b.finish_the_merge();
    let receipt = b.tick(RETRY, &thread);
    assert!(receipt.working_tree.is_some(), "{receipt:?}");
    assert!(b.started("solo"));
}

/// **A newcomer does not attempt a park another occasion is already holding** — the acquire end of
/// `park_step`'s "at most ONE occasion acts", and the reason it matters is the STAMP: a second
/// attempt would meet the identical refusal and overwrite the row of the occasion that is actually
/// retrying, restarting its "retrying since" from the newcomer's own instant.
#[test]
fn a_newcomer_does_not_restamp_a_refusal_another_occasion_owns() {
    let b = Bench::new();
    let (thread, session, _solo) = a_holder_with_a_rival(&b);
    b.escalate(ESCALATED, &session, &thread);
    b.worker.mark_gone(&session);
    b.start_a_merge();
    // The tick's park step refuses under the stranded-escalation occasion and holds the claim.
    b.tick(PAST_THE_BOUND, &thread);
    let before = b
        .store()
        .park_refusal(&scope_of(&thread))
        .unwrap()
        .expect("held");
    assert_eq!(before.occasion, "stranded_escalation");

    let newcomer = b.commission_receipt(RETRY, "newcomer");

    let after = b
        .store()
        .park_refusal(&scope_of(&thread))
        .unwrap()
        .expect("still held");
    assert_eq!(
        after.occasion, "stranded_escalation",
        "the occasion that is retrying still owns its row"
    );
    assert_eq!(
        after.last_tried, before.last_tried,
        "…and nothing tried that park again: {after:?}"
    );
    // …and the copy stayed where the held refusal left it.
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("solo"));
    assert!(!b.started("newcomer"));
    assert_eq!(b.park_branches(), "");
    assert_eq!(newcomer.queue_position, Some(2), "{newcomer:?}");
    assert!(
        !newcomer.warnings.iter().any(|w| matches!(
            w.class,
            ConsequenceClass::WorkNotParked | ConsequenceClass::WorkHandedOnUnparked
        )),
        "nothing was attempted, so nothing is reported twice: {:?}",
        newcomer.warnings
    );
}

/// **The ordinary single-agent shape, and it was the last door still handing the copy on unparked**
/// (final review of this branch, Finding 1). One agent works alone, crashes with uncommitted work,
/// nobody else is running, the bound passes, and the next `nxc send` takes the copy.
///
/// Nothing in the queue is not a reason not to park. The fairness gate exists to decide WHO gets the
/// copy next, and with an empty queue there is nothing to decide — but the copy still LEAVES a
/// holder that came to grief, which is the owner's whole rule of 2026-09-17: every hand-off caused
/// by trouble parks first. The tick cannot stand in for this one either: its sweep returns on an
/// empty queue by design, and nothing there is handed on.
#[test]
fn a_newcomers_own_trigger_parks_a_dead_holder_nobody_was_waiting_for() {
    let b = Bench::new();
    let (thread, session) = a_holder_with_nobody_waiting(&b);
    b.worker.mark_gone(&session);

    // No tick, no queue, no rival: the next commission simply asks for the working copy.
    let newcomer = b.commission_receipt(PAST_THE_BOUND, "newcomer");

    // The dead holder's work went onto a branch — tracked change and untracked file alike.
    let rows = b.store().parked_work(&scope_of(&thread)).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let branch = rows[0].parked.branch.clone();
    assert_eq!(rows[0].parked.base_branch, BASE);
    assert_eq!(
        git(b.root(), &["show", &format!("{branch}:src.txt")]),
        HALF_FINISHED.trim()
    );
    assert_eq!(
        git(b.root(), &["show", &format!("{branch}:notes.md")]),
        NOTES.trim()
    );

    // …and the newcomer — who is the hand-off here, there being nobody else — started in a CLEAN
    // tree standing on the branch the dead operation had started from.
    assert_eq!(b.head_branch(), BASE);
    assert_eq!(git(b.root(), &["status", "--porcelain"]), "");
    assert!(!b.root().join("notes.md").exists());
    assert!(b.started("newcomer"), "it took the copy: {newcomer:?}");
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&newcomer.thread_id).as_str())
    );
    assert_eq!(newcomer.queue_position, None, "{newcomer:?}");
    assert!(
        !newcomer.warnings.iter().any(|w| matches!(
            w.class,
            ConsequenceClass::WorkNotParked | ConsequenceClass::WorkHandedOnUnparked
        )),
        "the park went through, so there is nothing to report: {:?}",
        newcomer.warnings
    );
}

/// **…and the refusal rule is the same with nobody in line.** A park that can pass keeps the copy
/// where it is, so the newcomer waits rather than walking into a half-finished merge over somebody
/// else's uncommitted files — exactly what it does when a rival is already queued.
#[test]
fn a_newcomer_with_nobody_in_line_does_not_take_a_copy_whose_park_was_refused() {
    let b = Bench::new();
    let (thread, session) = a_holder_with_nobody_waiting(&b);
    b.worker.mark_gone(&session);
    b.start_a_merge();

    let newcomer = b.commission_receipt(PAST_THE_BOUND, "newcomer");

    // Nothing moved: the claim, the tree and its half-finished merge.
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );
    assert!(!b.started("newcomer"));
    assert_eq!(b.park_branches(), "");
    assert!(b
        .store()
        .parked_work(&scope_of(&thread))
        .unwrap()
        .is_empty());
    assert!(b.merge_head().exists());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // The newcomer queued behind the holder instead of reclaiming it, and its own receipt says why.
    assert_eq!(
        newcomer.queued_behind.as_deref(),
        Some(scope_of(&thread).as_str()),
        "{newcomer:?}"
    );
    assert_eq!(newcomer.queue_position, Some(1), "{newcomer:?}");
    let finding = the_finding(&newcomer.warnings, ConsequenceClass::WorkNotParked);
    assert!(
        finding.detail.contains("past its bound")
            && finding.detail.contains("a merge is in progress"),
        "it names the occasion and what to fix: {}",
        finding.detail
    );
    let note = b
        .operation(PAST_THE_BOUND, &thread)
        .park_refused
        .expect("the refusal is shown");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.occasion, "past_its_bound");

    // The way out is the one every other refusal has: the newcomer's own wait put it in the queue,
    // so the service's retry now has somebody to hand the copy to once the merge is finished.
    b.finish_the_merge();
    let receipt = b.tick(RETRY, &thread);
    assert!(receipt.working_tree.is_some(), "{receipt:?}");
    assert!(b.started("newcomer"));
}

/// **A reclaim that loses the race still arms the waiter's next look** (fix round 1 of nxf
/// 6j6v.xb24's review, Finding 2 — the same defect, on this occasion's own branch).
///
/// The holder's own trigger rides its bound forward while its work is being committed, so the park
/// goes through and the reclaim behind it — which names the exact `expires` this sweep read — matches
/// nothing. All of that is right: the work is safe on a branch and the copy stays with a chain that
/// just proved it is alive.
///
/// What was wrong is what came next. This tick is the one an earlier arming scheduled and it is
/// one-shot, so returning here without arming anything left the rival with no clock at all — back to
/// "waits until some other chain releases", which is the wedge nxf 6j6v.fabb exists to remove. The
/// rule is one rule for both branches of the sweep: a tick that leaves the copy where it was arms
/// the next look.
#[test]
fn a_reclaim_that_loses_the_race_still_arms_the_waiters_next_look() {
    let b = Bench::new();
    let (thread, session, solo) = a_holder_with_a_rival(&b);
    b.worker.mark_gone(&session);
    // Armed BEFORE the tick, fired from inside the park: see `LiveParkingWorker::renew`.
    b.worker
        .renew_the_lease_once(b.root(), &thread, PAST_THE_BOUND, RENEWED_UNTIL);
    assert!(
        b.timer.last_for(&solo).is_none(),
        "every window THIS timer records is one the tick below asked for"
    );

    let receipt = b.tick(PAST_THE_BOUND, &thread);

    // The park happened and is reported — the work IS on a branch, whatever the reclaim then did.
    let swept = receipt.working_tree.clone().expect("the work was parked");
    assert!(swept.parked.is_some(), "{swept:?}");
    no_park_finding(&receipt);
    assert!(
        swept.promotions.started.is_empty() && swept.promotions.requeued.is_empty(),
        "the compare-and-swap lost, so nothing was promoted: {:?}",
        swept.promotions
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
        Some(RETRY),
        "a tick that took nothing arms the next look, on the cadence: {:?}",
        b.timer.armed.lock().unwrap()
    );
}
