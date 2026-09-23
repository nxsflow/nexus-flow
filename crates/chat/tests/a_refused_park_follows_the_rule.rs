//! ACCEPTANCE of nxf 6j6v.8bv9 / 6j6v.b9nf — **a refused park follows one rule**.
//!
//! A park can be refused, and until this item every refusal did the same thing: the working-tree
//! claim stayed where it was and the queue behind it kept waiting — with `nxc release` named as the
//! way out. The owner's decision of 2026-09-17 splits the refusals in two, by whether waiting can
//! change them:
//!
//! - **Fixable or transient** — a merge, rebase, cherry-pick, revert or bisect in progress
//!   (`mid_sequence`), a git command that failed (`git`): the claim STAYS, `nxc status` says so on
//!   the holding operation (`PARK REFUSED`), and the background service retries the park on every
//!   tick until it goes through.
//! - **Permanent** — the runtime names no working copy (`no_working_copy`), the directory is not a
//!   git repository (`not_a_repository`), no base branch was ever recorded for the operation
//!   (`no_base_recorded`): the working copy is HANDED ON WITHOUT PARKING, with a loud, named
//!   finding (`work_handed_on_unparked`), because nothing a tick can do will ever change the answer
//!   and a queue that waits for it waits for ever.
//!
//! **Everything here is real, as in the two acceptance runs this rule sits under**
//! (`an_unanswered_escalation_parks_and_comes_back.rs`, `an_interrupted_operation_hands_the_working_copy_on.rs`):
//! a real git repository, the real lease, the real queue, the real park. The engine handle
//! ([`Engine`]) carries every verb that has a seam — the commissions, the escalation, the
//! interruption, the resume and every status read — and [`orchestration::tick`] carries the tick,
//! which has no handle verb: it is what the background service runs. The only stand-in is the
//! worker, and only for what a worker does: [`ParkingWorker`] records its triggers and names where
//! sessions would run.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{StatusOperation, StatusReport, StatusScope};
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, Ctx, FailedConsequence, ParkOccasion, ResumeOutcome,
    ResumeReceipt, ResumeRequest, ResumeScope, TickReceipt, TickRequest,
};
use nexus_chat::park::ParkRefusal;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here and takes the working copy.
const T0: &str = "2026-09-05T09:00:00Z";
/// …escalates here, with nothing waiting yet.
const ESCALATED: &str = "2026-09-05T09:05:00Z";
/// …and `solo` queues behind it here: the thirty minutes begin.
const CONTENDED: &str = "2026-09-05T09:30:00Z";
/// One minute past the contention's own deadline, and well inside the coder's two-hour lease.
const PAST_THE_DEADLINE: &str = "2026-09-05T10:01:00Z";
/// One liveness re-check after [`PAST_THE_DEADLINE`] — the instant a refused park is retried at.
const RETRY: &str = "2026-09-05T10:02:00Z";
/// …and the two after that, for a refusal whose occasion keeps standing.
const SECOND_RETRY: &str = "2026-09-05T10:03:00Z";
const THIRD_RETRY: &str = "2026-09-05T10:04:00Z";
/// The human finally answers the escalation — the occasion that wanted the copy is over.
const ANSWERED: &str = "2026-09-05T10:05:00Z";
/// …and the next tick comes round with the tree still mid-merge.
const AFTER_THE_ANSWER: &str = "2026-09-05T10:06:00Z";
/// One liveness re-check after that: what a retry WOULD be armed at if one were still running.
const A_MINUTE_LATER: &str = "2026-09-05T10:07:00Z";

/// The availability-boundary run: the coder stops at a weekly window here…
const INTERRUPTED: &str = "2026-09-05T09:20:00Z";
/// …`solo` queues behind the copy it still holds…
const QUEUED: &str = "2026-09-05T09:20:05Z";
/// …the process grace passes and the scheduled resume finds the window still closed…
const GRACE: &str = "2026-09-05T09:20:10Z";
/// …a tick comes round while the tree is still mid-merge…
const STILL_MID_MERGE: &str = "2026-09-05T09:21:10Z";
/// …and another once it is not.
const MERGE_FINISHED: &str = "2026-09-05T09:22:10Z";
/// When the runtime says the window lifts — days away.
const UNTIL: &str = "2026-09-12T12:00:00Z";

/// The two-holds runs: a second operation is commissioned here and works without the copy…
const A_SECOND_OPERATION: &str = "2026-09-05T09:10:00Z";
/// …and stops at an availability boundary of its own here, holding nothing.
const THE_OTHER_HOLD: &str = "2026-09-05T09:15:00Z";
/// A window that lifts minutes rather than days away — whichever of the two holds is given it has
/// stopped being an occasion by [`AFTER_THE_WINDOW`].
const A_SHORT_WINDOW: &str = "2026-09-05T09:30:00Z";
/// The tick after that window has fallen, well inside the holder's own two-hour lease.
const AFTER_THE_WINDOW: &str = "2026-09-05T09:31:00Z";
/// One liveness re-check after it: the cadence a refused park comes back on, and the instant that
/// must NOT be armed once nobody is retrying one.
const A_MINUTE_AFTER_THE_WINDOW: &str = "2026-09-05T09:32:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records like [`DryWorker`] and names where sessions would run — or, for the one
/// case about a host that cannot say, names nothing.
struct ParkingWorker {
    log: PathBuf,
    working_copy: Option<PathBuf>,
}

impl Worker for ParkingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        self.working_copy.clone()
    }
    fn resumes_sessions(&self) -> bool {
        true
    }
}

/// A timer that records every window it is asked to arm, in order — the retry cadence of a refused
/// park is a fact about WHAT IS ARMED, and nothing else can see it.
///
/// Arming replaces by key in the shipped service (`nxs_service::timers::arm`), so what a key will
/// actually fire at is the LAST instant recorded for it; [`Self::last_for`] reads exactly that.
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

/// A chat workspace that is ALSO a git repository on `release/1.2`, with two personas that each
/// want the working copy to themselves.
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
    // A persona that does NOT declare `working_tree: exclusive`. It runs beside whoever holds the
    // copy and never takes one — the second interrupted session of the two-holds runs below, which
    // exist to show that a hold somewhere else does not speak for the holder's own occasion.
    std::fs::write(
        roles.join("scribe.yaml"),
        "handle: scribe\njob_title: Writer\nsystem_prompt: You are scribe.\n",
    )
    .unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", "release/1.2"]);
    tmp
}

/// What a coding agent leaves behind in the tree it works in.
const HALF_FINISHED: &str = "as it was\nhalf finished\n";

struct Bench {
    tmp: TempDir,
    /// Where the worker says sessions run: the workspace root, a directory that is not a
    /// repository, or nothing at all.
    working_copy: Option<PathBuf>,
    /// Keeps a working copy that is NOT the workspace root alive for the bench's lifetime.
    _elsewhere: Option<TempDir>,
    timer: RecordingTimer,
}

impl Bench {
    fn new() -> Self {
        let tmp = workspace();
        let working_copy = Some(tmp.path().to_path_buf());
        Bench {
            tmp,
            working_copy,
            _elsewhere: None,
            timer: RecordingTimer::default(),
        }
    }

    /// The runtime names a directory, and that directory is not a git repository.
    fn with_a_working_copy_that_is_not_a_repository() -> Self {
        let elsewhere = TempDir::new().unwrap();
        let mut b = Bench::new();
        b.working_copy = Some(elsewhere.path().to_path_buf());
        b._elsewhere = Some(elsewhere);
        b
    }

    /// The runtime names no directory at all — the default of every worker but the sidecar.
    fn without_a_working_copy() -> Self {
        let mut b = Bench::new();
        b.working_copy = None;
        b
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    /// The tree the operation's work is written into: the named working copy, or — for a host
    /// that names none — the workspace root, where its sessions would be writing all the same.
    fn tree(&self) -> PathBuf {
        self.working_copy
            .clone()
            .unwrap_or_else(|| self.root().to_path_buf())
    }

    fn worker(&self) -> ParkingWorker {
        ParkingWorker {
            log: self.root().join("dry.log"),
            working_copy: self.working_copy.clone(),
        }
    }

    /// A fresh handle per call, dropped after it — each verb opens the database cleanly, exactly as
    /// the CLI's processes do, and nothing cached in one handle outlives the call it served.
    fn engine(&self) -> Engine {
        Engine::open_with(
            None,
            self.root(),
            EngineConfig {
                worker: WorkerConfig::Custom(Arc::new(self.worker())),
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

    /// The persona hands its task back to the human.
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
        let worker = self.worker();
        let ctx = Ctx {
            now,
            origin: "local",
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &defs,
            worker: &worker,
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

    /// The operation's `--json` object on the default listing.
    fn operation_json(&self, now: &str, root: &str) -> serde_json::Value {
        let value = self.status(now).to_value();
        value["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .find(|op| op["root"] == root)
            .cloned()
            .unwrap_or_else(|| panic!("operation {root} is not in {value}"))
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

    /// The human answers the escalation: the coder owes a reply again, so the contention that
    /// wanted the working copy is over — see `an_answered_escalation_ends_the_contention_it_started`
    /// in the sibling acceptance file for that mechanism on its own.
    fn answer(&self, now: &str, thread: &str) {
        self.engine()
            .reply_thread(
                Caller {
                    session: None,
                    actor: Some("carsten"),
                    now: Some(now),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body: "the second one. Carry on.",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("the answer is posted");
    }

    /// Who holds the working copy at `now`, if anybody does.
    fn holder(&self, now: &str) -> Option<String> {
        self.store().working_tree_holder(now).expect("holder")
    }

    /// When the current holder's claim runs out — the instant a commission waiting behind it is
    /// armed at when nothing else is going on.
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
}

fn scope_of(thread: &str) -> String {
    WorkScope::Thread(thread.to_string()).key()
}

/// The coder takes the copy, leaves work in the tree and escalates; `solo` queues behind it and the
/// thirty minutes begin. Returns `(coder's thread, solo's thread)`.
fn a_stranded_holder_with_a_rival(b: &Bench) -> (String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.tree().join("src.txt"), HALF_FINISHED).unwrap();
    std::fs::write(b.tree().join("notes.md"), "my working notes\n").unwrap();
    b.escalate(ESCALATED, &session, &thread);
    let solo = b.commission(CONTENDED, "solo");
    assert!(!b.started("solo"), "the rival is queued, not started");
    assert_eq!(b.holder(CONTENDED), Some(scope_of(&thread)));
    (thread, solo)
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

/// Every text a scenario of this file put in front of a reader — EVERY class on the receipt, not
/// just the two park ones, so a sentence that arrives beside a refusal is held to the same rule as
/// the refusal itself.
///
/// No class is excluded any more. [`ConsequenceClass::TickUnscheduled`] used to be, because its
/// text (`unarmed_drain_finding` in `orchestration.rs`, reachable right after a refusal through
/// `arm_the_park_retry`) named `nxc release` and Task 8 had not yet rewritten it — that exclusion
/// would have hidden the one place the verb was still leaking through this file's own gate.
fn refusal_texts(warnings: &[FailedConsequence]) -> Vec<String> {
    warnings.iter().map(|w| w.detail.clone()).collect()
}

// ---- a transient refusal: the claim stays, status says so, the tick retries --------------------

fn a_merge_in_progress_is_held(b: &Bench) -> (String, String, Vec<String>) {
    let (thread, solo) = a_stranded_holder_with_a_rival(b);
    b.start_a_merge();

    let receipt = b.tick(PAST_THE_DEADLINE, &thread);

    assert!(receipt.parked.is_none(), "nothing was parked");
    assert!(
        receipt.handed_on.is_none(),
        "and nothing was handed on: {:?}",
        receipt.handed_on
    );
    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    assert_eq!(
        finding.thread.as_deref(),
        Some(thread.as_str()),
        "anchored at the claim root"
    );
    assert!(
        finding.detail.contains("a merge is in progress"),
        "it names the state: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("finish or abort the merge in")
            || finding.detail.contains("Finish or abort the merge in"),
        "it names what to fix: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("retries") && finding.detail.contains("every tick"),
        "and that it is retried: {}",
        finding.detail
    );

    // The claim stays, the queue waits, the tree is untouched.
    assert_eq!(b.holder(PAST_THE_DEADLINE), Some(scope_of(&thread)));
    assert!(!b.started("solo"), "the queue head has not started");
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

    // `nxc status` says so, on the holding operation.
    let op = b.operation(PAST_THE_DEADLINE, &thread);
    let note = op
        .park_refused
        .clone()
        .expect("the refusal is on the holding operation");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.since, PAST_THE_DEADLINE);
    assert_eq!(note.last_tried, PAST_THE_DEADLINE);
    assert!(note.detail.contains("a merge is in progress"), "{note:?}");
    let json = b.operation_json(PAST_THE_DEADLINE, &thread);
    assert_eq!(json["park_refused"]["refusal"], "mid_sequence", "{json}");
    assert_eq!(json["park_refused"]["since"], PAST_THE_DEADLINE, "{json}");

    // …and the retry is armed one liveness re-check out, keyed on the waiting commission — and
    // nothing armed after it in the same tick pushed it anywhere else.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(RETRY),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );

    let mut texts = refusal_texts(&receipt.warnings);
    texts.push(note.detail);
    (thread, solo, texts)
}

#[test]
fn a_merge_in_progress_holds_the_claim_and_says_so_on_status() {
    let b = Bench::new();
    a_merge_in_progress_is_held(&b);
}

#[test]
fn the_next_tick_parks_once_the_merge_is_resolved() {
    let b = Bench::new();
    let (thread, solo, _) = a_merge_in_progress_is_held(&b);
    b.finish_the_merge();

    let receipt = b.tick(RETRY, &thread);

    let parked = receipt
        .parked
        .expect("the retry parked the stranded operation");
    assert_eq!(parked.scope, scope_of(&thread));
    assert!(parked.parked.committed, "there was work to save");
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WorkNotParked),
        "{:?}",
        receipt.warnings
    );
    let rows = b.store().parked_work(&scope_of(&thread)).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].parked.branch, parked.parked.branch);

    // The copy went on, and the refusal is gone with the claim it was about.
    assert!(b.started("solo"), "the rival started");
    assert_eq!(b.holder(RETRY), Some(scope_of(&solo)));
    let op = b.operation(RETRY, &thread);
    assert!(op.park_refused.is_none(), "{:?}", op.park_refused);
    let json = b.operation_json(RETRY, &thread);
    assert!(
        json.get("park_refused").is_none(),
        "no refusal, no key: {json}"
    );
}

/// **A refusal is retried only as long as the OCCASION that wrote it stands** (fix round 1 of nxf
/// 6j6v.8bv9). The escalation is answered, the operation is at work again, and nothing is trying to
/// take its working copy any more — so nobody is retrying that park, and a `PARK REFUSED` mark that
/// stayed would send a reader to look at an operation nothing is waiting on.
///
/// The tree is still mid-merge throughout: what ended is the occasion, not the refusal's own
/// reason. That is exactly the window the first cut left open.
#[test]
fn a_held_refusal_whose_occasion_ends_leaves_no_mark_and_arms_the_liveness_cadence() {
    let b = Bench::new();
    let (thread, solo, _) = a_merge_in_progress_is_held(&b);

    b.answer(ANSWERED, &thread);
    assert!(
        b.store().working_tree_contended_since().unwrap().is_none(),
        "the premise: an answered escalation ends the contention"
    );
    assert!(
        b.merge_head().exists(),
        "and the refusal's own reason has NOT changed — only its occasion"
    );

    let receipt = b.tick(AFTER_THE_ANSWER, &thread);

    assert!(receipt.parked.is_none() && receipt.handed_on.is_none());
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WorkNotParked),
        "nothing is being parked, so nothing says it is not: {:?}",
        receipt.warnings
    );

    // No mark and no line on `nxc status`: both are rendered from this one field (the `cli` test
    // at the end of this file pins that rendering), and `--json` carries no key at all.
    let op = b.operation(AFTER_THE_ANSWER, &thread);
    assert!(
        op.park_refused.is_none(),
        "the occasion is over: {:?}",
        op.park_refused
    );
    let json = b.operation_json(AFTER_THE_ANSWER, &thread);
    assert!(json.get("park_refused").is_none(), "{json}");
    assert!(b
        .store()
        .park_refusal(&scope_of(&thread))
        .unwrap()
        .is_none());

    // …and the commission behind the claim still has a clock. This used to assert the holder's own
    // BOUND here — a refused park was the only thing that put a wait on the once-a-minute cadence —
    // and nxf 6j6v.xb24 made that false on purpose: while anybody is queued, every look comes back
    // on the cadence, because the holder can DIE inside its bound and waiting it out is the two
    // hours that item removes. What the cleared row still changes is the mark, which is asserted
    // above; what it no longer changes is this instant.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(A_MINUTE_LATER),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    assert_ne!(
        b.lease_expires().as_str(),
        A_MINUTE_LATER,
        "the premise of the assertion above: the bound is not where the cadence lands"
    );
}

/// **…and while the occasion DOES stand, the mark and its first instant do not move.** The
/// counter-test to the one above: the clearing may not be "a tick where nothing happened", and
/// "retrying since" may not be reset by the retries themselves.
#[test]
fn a_held_refusal_whose_occasion_still_stands_keeps_its_mark_and_its_first_instant() {
    let b = Bench::new();
    let (thread, solo, _) = a_merge_in_progress_is_held(&b);

    for (now, next_look) in [(RETRY, SECOND_RETRY), (SECOND_RETRY, THIRD_RETRY)] {
        let receipt = b.tick(now, &thread);

        let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
        assert!(
            finding.detail.contains("a merge is in progress"),
            "{}",
            finding.detail
        );
        assert!(receipt.parked.is_none() && receipt.handed_on.is_none());
        assert_eq!(b.holder(now), Some(scope_of(&thread)), "the claim stays");
        assert!(!b.started("solo"), "and the queue head keeps waiting");

        let note = b
            .operation(now, &thread)
            .park_refused
            .expect("the mark stands as long as its occasion does");
        assert_eq!(note.refusal, "mid_sequence");
        assert_eq!(
            note.since, PAST_THE_DEADLINE,
            "retrying since the FIRST refusal, not since this retry"
        );
        assert_eq!(note.last_tried, now, "and this one is the latest attempt");
        let json = b.operation_json(now, &thread);
        assert_eq!(
            json["park_refused"]["occasion"], "stranded_escalation",
            "the occasion that wrote the row is on it: {json}"
        );
        assert_eq!(json["park_refused"]["since"], PAST_THE_DEADLINE, "{json}");

        assert_eq!(
            b.timer.last_for(&solo).as_deref(),
            Some(next_look),
            "the retry keeps coming back on the liveness cadence — armed: {:?}",
            b.timer.armed.lock().unwrap()
        );
    }
}

// ---- a permanent refusal: the copy goes on, unparked, and says so -------------------------------

/// What every permanent refusal has in common, asserted once: the rival started, nothing was
/// parked, the finding is `work_handed_on_unparked`, and the operation's work is still in the tree.
fn handed_on_unparked(b: &Bench, kind: &str, says: &str) -> Vec<String> {
    let (thread, solo) = a_stranded_holder_with_a_rival(b);

    let receipt = b.tick(PAST_THE_DEADLINE, &thread);

    assert!(receipt.parked.is_none(), "nothing was parked");
    let handed = receipt
        .handed_on
        .clone()
        .expect("the copy was handed on, and the receipt says so");
    assert_eq!(handed.scope, scope_of(&thread));
    assert_eq!(handed.occasion, ParkOccasion::StrandedEscalation);
    assert!(handed.parked.is_none(), "{handed:?}");
    let refusal = handed.unparked.clone().expect("why it was not parked");
    assert!(refusal.is_permanent(), "{refusal:?}");
    assert_eq!(refusal.kind(), kind);
    assert!(
        handed.promotions.started.iter().any(|p| p.thread == solo),
        "the promotion is on the receipt: {handed:?}"
    );
    let json = serde_json::to_value(&receipt).expect("the receipt serializes");
    assert_eq!(json["handed_on"]["unparked"]["refusal"], kind, "{json}");
    assert_eq!(
        json["handed_on"]["occasion"], "stranded_escalation",
        "{json}"
    );

    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkHandedOnUnparked);
    assert_eq!(finding.thread.as_deref(), Some(thread.as_str()));
    assert!(finding.detail.contains(says), "{}", finding.detail);
    assert!(
        finding.detail.contains("WITHOUT parking"),
        "it says the copy moved unparked: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("stays in the tree"),
        "and where the uncommitted work is now: {}",
        finding.detail
    );

    assert!(b.started("solo"), "the rival started");
    assert_eq!(b.holder(PAST_THE_DEADLINE), Some(scope_of(&solo)));
    assert!(b
        .store()
        .parked_work(&scope_of(&thread))
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(b.tree().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "the uncommitted work stays in the tree for the next holder"
    );
    assert!(b.tree().join("notes.md").exists());
    assert_eq!(
        git(b.root(), &["branch", "--list", "nxs/park/*"]),
        "",
        "no park branch was made"
    );
    let op = b.operation(PAST_THE_DEADLINE, &thread);
    assert!(
        op.park_refused.is_none(),
        "a permanent refusal is not retried, so it is not shown as one: {:?}",
        op.park_refused
    );

    refusal_texts(&receipt.warnings)
}

#[test]
fn a_workspace_that_is_not_a_repository_hands_the_copy_on_unparked() {
    let b = Bench::with_a_working_copy_that_is_not_a_repository();
    handed_on_unparked(&b, "not_a_repository", "is not a git repository");
}

fn without_a_base(b: &Bench) -> Vec<String> {
    // The base is recorded on the operation's first acquire; take it away before the tick.
    let thread = b.commission(T0, "coder");
    let deleted = b
        .store()
        .connection()
        .execute(
            "DELETE FROM operation_base WHERE scope_key = ?1",
            [scope_of(&thread)],
        )
        .expect("delete the recorded base");
    assert_eq!(deleted, 1, "a base had been recorded on the first acquire");
    // …and run the rest of the scene on the SAME operation.
    let session = b.session_of("coder");
    std::fs::write(b.tree().join("src.txt"), HALF_FINISHED).unwrap();
    std::fs::write(b.tree().join("notes.md"), "my working notes\n").unwrap();
    b.escalate(ESCALATED, &session, &thread);
    let solo = b.commission(CONTENDED, "solo");

    let receipt = b.tick(PAST_THE_DEADLINE, &thread);

    let handed = receipt.handed_on.clone().expect("handed on");
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
        finding.detail.contains("no base branch was ever recorded"),
        "{}",
        finding.detail
    );
    assert!(
        finding.detail.contains("WITHOUT parking"),
        "{}",
        finding.detail
    );
    assert!(b.started("solo"));
    assert_eq!(b.holder(PAST_THE_DEADLINE), Some(scope_of(&solo)));
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "nothing was committed: the work is still in the tree"
    );
    assert_ne!(git(b.root(), &["status", "--porcelain"]), "");
    assert_eq!(git(b.root(), &["branch", "--list", "nxs/park/*"]), "");
    refusal_texts(&receipt.warnings)
}

#[test]
fn an_operation_without_a_recorded_base_hands_the_copy_on_unparked() {
    let b = Bench::new();
    without_a_base(&b);
}

#[test]
fn a_worker_that_names_no_working_copy_hands_the_copy_on_unparked() {
    let b = Bench::without_a_working_copy();
    handed_on_unparked(
        &b,
        "no_working_copy",
        "does not run sessions in a directory",
    );
}

// ---- the availability-boundary occasion, retried by the tick ------------------------------------

fn resume(b: &Bench, now: &str, session: &str) -> ResumeReceipt {
    b.engine()
        .resume_interrupted(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(now),
            },
            ResumeRequest {
                scope: ResumeScope::Session(session),
                force: false,
            },
        )
        .expect("the resume decides something")
}

/// The setup both availability-boundary runs share: the coder stops at a weekly window with work
/// in the tree, `solo` queues behind the copy it still holds, the tree goes mid-merge, and the
/// scheduled resume at the process grace finds the park refused and HOLDS the claim.
///
/// Returns `(the coder's thread, the coder's session, solo's thread, the texts so far)`.
fn an_interrupted_holder_with_a_held_refusal(b: &Bench) -> (String, String, String, Vec<String>) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    let engine = b.engine();
    let as_coder = Caller {
        session: Some(&session),
        actor: Some("coder"),
        now: Some(INTERRUPTED),
    };
    engine
        .session_interrupted(
            as_coder,
            &session,
            "seven_day",
            Some(UNTIL),
            "You've hit your weekly limit",
        )
        .expect("the hold is recorded");
    engine
        .session_ended(as_coder, &session)
        .expect("and the process is over");
    drop(engine);
    let solo = b.commission(QUEUED, "solo");
    assert!(!b.started("solo"));
    b.start_a_merge();

    // The scheduled resume at the process grace: the window is a week away, the copy should go on,
    // and the tree is mid-merge — so it is HELD, and said so.
    let out = resume(b, GRACE, &session);
    assert_eq!(
        out.outcome,
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        }
    );
    let finding = the_finding(&out.warnings, ConsequenceClass::WorkNotParked);
    assert!(
        finding.detail.contains("a merge is in progress"),
        "{}",
        finding.detail
    );
    let texts = refusal_texts(&out.warnings);
    assert_eq!(b.holder(GRACE), Some(scope_of(&thread)), "the claim stays");
    assert!(!b.started("solo"));
    let op = b.operation(GRACE, &thread);
    assert!(op.interrupted, "the hold still stands");
    let note = op.park_refused.expect("the refusal is on status");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.since, GRACE);
    assert_eq!(
        b.operation_json(GRACE, &thread)["park_refused"]["occasion"],
        "availability_boundary",
        "stamped with the occasion that refused it"
    );

    (thread, session, solo, texts)
}

fn an_interrupted_holder_is_retried(b: &Bench) -> Vec<String> {
    let (thread, _session, solo, mut texts) = an_interrupted_holder_with_a_held_refusal(b);

    // A plain tick while the merge is still in progress: retried, refused again, and the refusal
    // keeps the instant it was FIRST refused at — "retrying since" means since then.
    let again = b.tick(STILL_MID_MERGE, &thread);
    let finding = the_finding(&again.warnings, ConsequenceClass::WorkNotParked);
    texts.extend(refusal_texts(&again.warnings));
    assert!(finding.detail.contains("a merge is in progress"));
    assert!(again.handed_on.is_none() && again.parked.is_none());
    let note = b
        .operation(STILL_MID_MERGE, &thread)
        .park_refused
        .expect("still refused");
    assert_eq!(note.since, GRACE, "first refused at the resume");
    assert_eq!(note.last_tried, STILL_MID_MERGE, "tried again by the tick");
    assert!(!b.started("solo"));

    // The merge is finished; a plain tick — not a resume — parks and hands on.
    b.finish_the_merge();
    let receipt = b.tick(MERGE_FINISHED, &thread);
    assert!(
        receipt.parked.is_none(),
        "`parked` is the stranded escalation's"
    );
    let handed = receipt
        .handed_on
        .clone()
        .expect("the tick secured the interrupted operation");
    assert_eq!(handed.occasion, ParkOccasion::AvailabilityBoundary);
    assert_eq!(handed.scope, scope_of(&thread));
    assert!(handed.unparked.is_none(), "{handed:?}");
    let parked = handed.parked.clone().expect("its work was parked");
    assert!(parked.committed);
    assert!(
        handed.promotions.started.iter().any(|p| p.thread == solo),
        "{handed:?}"
    );
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["handed_on"]["occasion"],
        "availability_boundary"
    );
    assert!(b.started("solo"));
    assert_eq!(b.holder(MERGE_FINISHED), Some(scope_of(&solo)));
    assert_eq!(
        b.store().parked_work(&scope_of(&thread)).unwrap().len(),
        1,
        "one park"
    );
    assert_eq!(
        git(b.root(), &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        "release/1.2",
        "the tree is back on the operation's base"
    );
    let op = b.operation(MERGE_FINISHED, &thread);
    assert!(op.park_refused.is_none(), "{:?}", op.park_refused);
    assert!(op.interrupted, "the hold still stands: nothing took it up");
    texts
}

#[test]
fn an_interrupted_holder_whose_park_was_refused_is_retried_by_the_tick() {
    let b = Bench::new();
    an_interrupted_holder_is_retried(&b);
}

/// **The other occasion ends the same way** (fix round 1 of nxf 6j6v.8bv9): the availability
/// boundary moves the working copy only for somebody who is waiting for it, so the commission
/// behind it being withdrawn ends the occasion — and the mark that says the park is being retried
/// goes with it, though the hold still stands and the tree is still mid-merge.
#[test]
fn a_held_refusal_of_an_interrupted_holder_goes_when_nobody_waits_for_the_copy_any_more() {
    let b = Bench::new();
    let (thread, _session, solo, _) = an_interrupted_holder_with_a_held_refusal(&b);

    b.engine()
        .withdraw(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(STILL_MID_MERGE),
            },
            &solo,
        )
        .expect("the queued commission comes back");

    let receipt = b.tick(STILL_MID_MERGE, &thread);

    assert!(receipt.parked.is_none() && receipt.handed_on.is_none());
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WorkNotParked),
        "nothing is attempting that park: {:?}",
        receipt.warnings
    );
    let op = b.operation(STILL_MID_MERGE, &thread);
    assert!(op.interrupted, "the hold itself still stands");
    assert!(
        op.park_refused.is_none(),
        "but nobody is retrying its park: {:?}",
        op.park_refused
    );
    assert!(b
        .store()
        .park_refusal(&scope_of(&thread))
        .unwrap()
        .is_none());

    // …and nothing else moved: the claim, the merge state and the work are where they were.
    assert_eq!(b.holder(STILL_MID_MERGE), Some(scope_of(&thread)));
    assert!(b.merge_head().exists());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
}

/// **Two sessions interrupted at once, and only one of them holding the working copy** — the shape
/// the holder's own occasion has to be told apart from (fix round 2 of nxf 6j6v.8bv9).
///
/// The scribe declares no `working_tree: exclusive`, so it runs beside the coder and never holds
/// anything; both stop at an availability boundary. Only the coder's hold can ever park THIS
/// checkout, so only the coder's hold can keep the coder's refusal "retrying".
///
/// The two windows are the parameters, because which of the two holds is still an occasion at the
/// tick each caller runs is the whole question. Returns `(the coder's thread, the scribe's session,
/// the scribe's thread, solo's thread)`.
fn two_interrupted_sessions(
    b: &Bench,
    the_holders_window: &str,
    the_others_window: &str,
) -> (String, String, String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();

    // The other operation: started, running against nothing of its own, and interrupted where it
    // stands. It never touches the lease.
    let scribe = b.commission(A_SECOND_OPERATION, "scribe");
    let scribe_session = b.session_of("scribe");
    assert_eq!(
        b.holder(A_SECOND_OPERATION),
        Some(scope_of(&thread)),
        "the second operation needs no working copy and takes none"
    );
    b.engine()
        .session_interrupted(
            Caller {
                session: Some(&scribe_session),
                actor: Some("scribe"),
                now: Some(THE_OTHER_HOLD),
            },
            &scribe_session,
            "seven_day",
            Some(the_others_window),
            "You've hit your weekly limit",
        )
        .expect("the other hold is recorded");

    let engine = b.engine();
    let as_coder = Caller {
        session: Some(&session),
        actor: Some("coder"),
        now: Some(INTERRUPTED),
    };
    engine
        .session_interrupted(
            as_coder,
            &session,
            "seven_day",
            Some(the_holders_window),
            "You've hit your weekly limit",
        )
        .expect("the holder's hold is recorded");
    engine
        .session_ended(as_coder, &session)
        .expect("and the process is over");
    drop(engine);

    let solo = b.commission(QUEUED, "solo");
    assert!(!b.started("solo"));
    b.start_a_merge();

    // The scheduled resume at the process grace: mid-merge, so the park is refused and HELD, and
    // the row is stamped with the occasion that refused it.
    let out = resume(b, GRACE, &session);
    assert_eq!(
        out.outcome,
        ResumeOutcome::NotYet {
            until: Some(the_holders_window.to_string())
        }
    );
    the_finding(&out.warnings, ConsequenceClass::WorkNotParked);
    let note = b
        .operation(GRACE, &thread)
        .park_refused
        .expect("the holder's park is refused and held");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.since, GRACE);
    assert_eq!(
        b.operation_json(GRACE, &thread)["park_refused"]["occasion"],
        "availability_boundary"
    );
    assert!(
        b.store()
            .current_interruption(&scribe_session)
            .unwrap()
            .expect("the other session is on hold too")
            .on_hold(),
        "the premise: two holds stand at once"
    );

    (thread, scribe_session, scribe, solo)
}

/// **The holder's own hold is what keeps the holder's refusal "retrying"** (fix round 2 of nxf
/// 6j6v.8bv9). A session interrupted somewhere else has nothing to do with this claim: it never
/// held the copy, no tick will ever park it here, and it may not keep a `PARK REFUSED` mark alive
/// on an operation whose own occasion is over.
///
/// The holder's window falls, so its hold is the resume's to take up and not a park waiting to
/// happen; the other session stays interrupted for days. One tick: the mark, the row and the
/// once-a-minute retry are gone, and the other operation's hold is untouched.
#[test]
fn a_hold_somewhere_else_does_not_keep_the_holders_refusal_alive() {
    let b = Bench::new();
    let (thread, scribe_session, scribe, solo) =
        two_interrupted_sessions(&b, A_SHORT_WINDOW, UNTIL);

    let receipt = b.tick(AFTER_THE_WINDOW, &thread);

    assert!(receipt.parked.is_none() && receipt.handed_on.is_none());
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WorkNotParked),
        "the holder's own occasion is over, so nothing is attempting that park: {:?}",
        receipt.warnings
    );

    let op = b.operation(AFTER_THE_WINDOW, &thread);
    assert!(
        op.park_refused.is_none(),
        "nobody is retrying it: {:?}",
        op.park_refused
    );
    let json = b.operation_json(AFTER_THE_WINDOW, &thread);
    assert!(json.get("park_refused").is_none(), "{json}");
    assert!(b
        .store()
        .park_refusal(&scope_of(&thread))
        .unwrap()
        .is_none());

    // …and the commission behind the claim still has a clock — on the cadence every queued wait
    // uses since nxf 6j6v.xb24, rather than the holder's own bound this used to assert. See the
    // sibling run above for why that stopped being evidence about the row.
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(A_MINUTE_AFTER_THE_WINDOW),
        "armed: {:?}",
        b.timer.armed.lock().unwrap()
    );
    assert_ne!(b.lease_expires().as_str(), A_MINUTE_AFTER_THE_WINDOW);

    // The other operation is exactly where it was: still on hold, and nothing of this decided
    // anything about it.
    let hold = b
        .store()
        .current_interruption(&scribe_session)
        .unwrap()
        .expect("the other hold stands");
    assert!(hold.on_hold() && hold.resumed_at.is_none());
    assert_eq!(hold.until.as_deref(), Some(UNTIL));
    assert!(b.operation(AFTER_THE_WINDOW, &scribe).interrupted);

    // Nothing moved in the workspace either: the claim, the merge state and the work stand.
    assert_eq!(b.holder(AFTER_THE_WINDOW), Some(scope_of(&thread)));
    assert!(!b.started("solo"));
    assert!(b.merge_head().exists());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
}

/// **…and the other way round the mark stays.** The counter-test: the unrelated session's window
/// falls while the holder's own hold stands, so the park is still waiting to happen — the mark, its
/// first instant and the once-a-minute retry all keep going.
#[test]
fn the_holders_own_hold_standing_keeps_its_mark_when_another_hold_ends() {
    let b = Bench::new();
    let (thread, scribe_session, _scribe, solo) =
        two_interrupted_sessions(&b, UNTIL, A_SHORT_WINDOW);

    let receipt = b.tick(AFTER_THE_WINDOW, &thread);

    let finding = the_finding(&receipt.warnings, ConsequenceClass::WorkNotParked);
    assert!(
        finding.detail.contains("a merge is in progress"),
        "{}",
        finding.detail
    );
    assert!(receipt.parked.is_none() && receipt.handed_on.is_none());

    let note = b
        .operation(AFTER_THE_WINDOW, &thread)
        .park_refused
        .expect("the holder's own occasion still stands");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(note.since, GRACE, "retrying since the first refusal");
    assert_eq!(note.last_tried, AFTER_THE_WINDOW);
    assert_eq!(
        b.timer.last_for(&solo).as_deref(),
        Some(A_MINUTE_AFTER_THE_WINDOW),
        "still on the liveness cadence — armed: {:?}",
        b.timer.armed.lock().unwrap()
    );

    assert_eq!(b.holder(AFTER_THE_WINDOW), Some(scope_of(&thread)));
    assert!(!b.started("solo"));
    let hold = b
        .store()
        .current_interruption(&scribe_session)
        .unwrap()
        .expect("the other row stays either way");
    assert!(hold.on_hold(), "and nothing took it up");
}

// ---- the texts ----------------------------------------------------------------------------------

/// **No refusal sends its reader to `nxc release`** — the verb left the surface (nxf 6j6v.b9nf),
/// and a refusal that named it would be a map to a door that is no longer there. Every refusal
/// this file can produce is collected here, held and handed on alike.
#[test]
fn no_refusal_text_names_the_release_verb() {
    let mut texts = Vec::new();
    texts.extend(a_merge_in_progress_is_held(&Bench::new()).2);
    texts.extend(handed_on_unparked(
        &Bench::with_a_working_copy_that_is_not_a_repository(),
        "not_a_repository",
        "is not a git repository",
    ));
    texts.extend(without_a_base(&Bench::new()));
    texts.extend(handed_on_unparked(
        &Bench::without_a_working_copy(),
        "no_working_copy",
        "does not run sessions in a directory",
    ));
    texts.extend(an_interrupted_holder_is_retried(&Bench::new()));

    assert!(
        texts.len() >= 7,
        "every scenario produced its text: {texts:#?}"
    );
    for text in &texts {
        assert!(
            !text.contains("nxc release") && !text.contains("release --thread"),
            "a refusal names the release verb: {text}"
        );
    }
}

// ---- the listing rule, and the terminal ---------------------------------------------------------

/// **An operation with a refused park stays on the default listing** even when nothing else about
/// it would keep it there (requirement 7 of this item): a refusal row is a thing somebody has to go
/// and fix, and a view that hides it once the threads are discharged hides the one fact that says
/// why the copy is not moving.
#[test]
fn an_operation_with_a_refused_park_and_nothing_else_open_is_still_listed_by_default() {
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    // The coder answers for real: nothing is open, nothing is escalated, the copy is given back.
    b.engine()
        .reply_thread(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(ESCALATED),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("answered");
    assert_eq!(
        b.holder(ESCALATED),
        None,
        "a finished operation gives the copy back"
    );
    assert!(
        !b.status(ESCALATED)
            .operations
            .iter()
            .any(|op| op.root == thread),
        "the premise: without a refusal the operation is finished and not listed"
    );

    b.store()
        .note_park_refusal(
            &scope_of(&thread),
            &ParkRefusal::Git("`git checkout` failed: the index is locked".to_string()),
            ParkOccasion::StrandedEscalation,
            CONTENDED,
        )
        .expect("note the refusal");

    let op = b.operation(CONTENDED, &thread);
    assert!(op.live, "a refused park keeps the operation live");
    assert_eq!(op.open, 0);
    assert_eq!(op.park_refused.map(|n| n.refusal).as_deref(), Some("git"));
}

mod cli {
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    use nexus_chat::park::ParkRefusal;
    use nexus_chat::working_tree::WorkScope;
    use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};

    const NOW: &str = "2026-09-17T10:00:00Z";

    fn workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        std::fs::write(
            roles.join("coder.yaml"),
            "handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n",
        )
        .unwrap();
        tmp
    }

    fn nxc(tmp: &TempDir) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
        c
    }

    fn json_of(cmd: &mut Command) -> Value {
        let out = cmd.assert().success();
        serde_json::from_slice(&out.get_output().stdout).expect("valid json")
    }

    /// **`nxc status` marks the holding operation and says what to fix**, and `--json` carries the
    /// same record. The refusal is written straight into the store: HOW a refusal comes to exist is
    /// the rest of this file, and the dry worker a CLI test runs under names no working copy, so it
    /// could only ever produce the permanent kind.
    #[test]
    fn the_status_line_marks_a_refused_park_and_says_what_to_fix() {
        let tmp = workspace();
        let thread = json_of(nxc(&tmp).args([
            "--json",
            "send",
            "--to",
            "coder",
            "--no-ref",
            "build the thing",
        ]))["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
        let detail = "the working copy at /work is mid-flight — a merge is in progress \
                      (MERGE_HEAD is present) — Finish or abort the merge in /work";
        Workspace::resolve(None, tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
            .note_park_refusal(
                &WorkScope::Thread(thread.clone()).key(),
                &ParkRefusal::MidSequence(detail.to_string()),
                nexus_chat::orchestration::ParkOccasion::StrandedEscalation,
                "2026-09-17T09:30:00Z",
            )
            .expect("note the refusal");

        let out = nxc(&tmp).args(["status"]).assert().success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        let header = stdout
            .lines()
            .find(|l| l.starts_with(&format!("operation {thread}")))
            .unwrap_or_else(|| panic!("no operation line: {stdout}"));
        assert!(header.contains("PARK REFUSED"), "{header}");
        assert!(
            stdout.contains(&format!(
                "  park refused (mid_sequence): {detail} — retrying since 2026-09-17T09:30:00Z"
            )),
            "{stdout}"
        );

        let report = json_of(nxc(&tmp).args(["--json", "status"]));
        let op = &report["operations"][0];
        assert_eq!(op["root"], thread.as_str());
        assert_eq!(op["park_refused"]["refusal"], "mid_sequence");
        assert_eq!(op["park_refused"]["detail"], detail);
        assert_eq!(op["park_refused"]["since"], "2026-09-17T09:30:00Z");
        assert_eq!(op["park_refused"]["last_tried"], "2026-09-17T09:30:00Z");
    }
}
