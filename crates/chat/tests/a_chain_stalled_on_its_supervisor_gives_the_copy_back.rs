//! ACCEPTANCE of nxf 6j6v.3rrv — **a chain that stalled on its own supervisor gives the working copy
//! back**, and of the one door of 6j6v.2pnn it shares: a copy held with nothing running in it can
//! be withdrawn.
//!
//! # The shape, as it was measured on 2026-11-02
//!
//! Operation `m-01M6DFYKSF94479AWGXC4AKJXB` in this repository's own workspace, a `coding` run
//! (`build -> verify -> judge -> finish`, `working_tree: exclusive`, `timeout: 6h`):
//!
//! 1. The coder answered its third rework pass while its process was still alive, so the advance to
//!    `verify` was refused by the brought-along hurdle `previous-step-still-writing` — correctly.
//! 2. The sidecar then ran `nxc session ended`. That call recorded the end (`session_map.ended` is
//!    17:06:21.824456Z), took the advance it had been waiting for, OPENED the `verify` slot thread
//!    and stamped its deadline — and was then killed by the sidecar's own 30-second call timeout
//!    (`nxc session ended failed: spawnSync nxc ETIMEDOUT` in the agent log) before the commission
//!    wrote the slot's register or posted its task. What was left is a slot thread with a deadline,
//!    no `expects_reply_from` and no message, created at the very instant the end was recorded.
//! 3. From there NOTHING moved it: the root waited on `__channel__`, every step was answered, no
//!    session was alive, and nothing was queued in the operation —
//!    - a tick on the root answered `not_due`, because the empty slot is neither `complete` (it
//!      expects nobody) nor `stale` (it owes nobody), so the set is never settled;
//!    - the dead-holder sweep declined, because the only thread still owing — the root, waiting on
//!      the engine's own `__channel__` — has no session of its own, which that rule reads as "a
//!      human is being waited for";
//!    - `nxc withdraw` refused, because nothing was queued, failed or running;
//!    - and the lease's bound was not the root's long-past deadline but the one fixed at the last
//!      renewal (21:09:31Z, six hours after the last rework slot opened).
//!
//! This file pins the way out on each of those doors, and a guard against every way the new rules
//! could take a copy they must not. Everything is real — a git repository, the lease, the queue,
//! the park; the engine handle carries every commission and the withdrawal, and
//! `orchestration::tick` carries the tick, which is what the background service runs. The worker is
//! the only stand-in, and only for what a worker does.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::orchestration::{self, Caller, Ctx, ParkOccasion, TickReceipt, TickRequest};
use nexus_chat::precondition::PreconditionOutcome;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::{QueuedTrigger, WorkScope};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The run is commissioned and the coder takes the working copy.
const T0: &str = "2026-11-02T09:00:00Z";
/// The coder answers while its process still runs — the advance is refused, correctly.
const ANSWERED: &str = "2026-11-02T09:20:00Z";
/// A rival commission queues behind the copy.
const QUEUED: &str = "2026-11-02T09:21:00Z";
/// The coder's end is recorded — the last thing that happens in the operation.
const ENDED: &str = "2026-11-02T09:22:00Z";
/// One minute after it: too early to call the chain stalled.
const JUST_AFTER: &str = "2026-11-02T09:23:00Z";
/// Five minutes after the empty slot opened at [`ENDED`] — when the tick first counts it out.
const SLOT_COUNTS_OUT: &str = "2026-11-02T09:27:00Z";
/// Inside the revival's window: past [`SLOT_COUNTS_OUT`], short of it plus one liveness cadence.
const REVIVING: &str = "2026-11-02T09:27:30Z";
/// One second short of five minutes of quiet.
const ONE_SECOND_SHORT: &str = "2026-11-02T09:26:59Z";
/// Exactly five minutes of quiet.
const FIVE_MINUTES: &str = "2026-11-02T09:27:00Z";
/// Well past the quiet the engine waits out before it calls a chain stalled.
const QUIET: &str = "2026-11-02T09:40:00Z";
/// The lease's bound: six hours (the channel's `timeout:`) after the coder's slot opened.
const THE_BOUND: &str = "2026-11-02T15:00:00Z";
/// Past it.
const PAST_THE_BOUND: &str = "2026-11-02T15:05:00Z";

const CHANNELS: &str = "- name: coding\n  members: []\n  timeout: 6h\n  \
                        working_tree: exclusive\n  steps:\n    \
                        - id: build\n      target: coder\n      next: verify\n    \
                        - id: verify\n      target: verifier\n";

/// [`CHANNELS`] with one DECLARED hurdle — the project's own rule, the kind a person fixes by hand.
const CHANNELS_WITH_A_HURDLE: &str = "- name: coding\n  members: []\n  timeout: 6h\n  \
                                      working_tree: exclusive\n  preconditions:\n    \
                                      - name: tree-clean\n      run: check the tree\n  steps:\n    \
                                      - id: build\n      target: coder\n      next: verify\n    \
                                      - id: verify\n      target: verifier\n";

// ---- harness ------------------------------------------------------------------------------------

/// Records its triggers like [`DryWorker`], names where sessions run, answers which of them still
/// have a process, and answers a declared hurdle — refusing it while `blocked`.
struct LiveWorker {
    log: PathBuf,
    working_copy: PathBuf,
    running: Mutex<HashSet<String>>,
    looks: AtomicBool,
    blocked: AtomicBool,
}

impl Worker for LiveWorker {
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
        Some(self.working_copy.clone())
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn resumes_sessions(&self) -> bool {
        true
    }
    fn run_precondition(&self, _command: &str) -> PreconditionOutcome {
        PreconditionOutcome::Ran {
            status: Some(if self.blocked.load(Ordering::SeqCst) {
                1
            } else {
                0
            }),
            stdout: String::new(),
            stderr: "the tree is not clean".into(),
        }
    }
}

/// Records every window it is asked to arm — the one place an armed look is visible. A thread's
/// window (`schedule`, keyed by the thread) and a look (keyed by its slot) are recorded apart, as the
/// service files them apart.
#[derive(Default)]
struct RecordingTimer {
    armed: Mutex<Vec<(String, String)>>,
    looks: Mutex<Vec<(String, String, String)>>,
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
    fn schedule_stranded_look(
        &self,
        slot: &str,
        thread: &str,
        deadline: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        self.looks.lock().unwrap().push((
            slot.to_string(),
            thread.to_string(),
            deadline.to_string(),
        ));
        Ok(TimerHandle(format!("recorded:stranded-look-{slot}")))
    }
}

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

/// The branch the operation started on.
const BASE: &str = "feat/liveness";
/// What the coder left uncommitted in the tree.
const HALF_FINISHED: &str = "as it was\nhalf finished\n";

fn workspace(channels: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "verifier"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("solo.yaml"),
        "handle: solo\nsystem_prompt: You are solo.\nworking_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(roles.join("channels.yaml"), channels).unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\ntimer.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    git(
        tmp.path(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            "first",
        ],
    );
    git(tmp.path(), &["checkout", "-q", "-b", BASE]);
    tmp
}

struct Bench {
    tmp: TempDir,
    worker: Arc<LiveWorker>,
    timer: RecordingTimer,
    /// The workspace's own replica prefix — the `<origin>` of every qualified handle minted here,
    /// the supervisor's included.
    origin: String,
}

impl Bench {
    fn new() -> Self {
        Self::with(CHANNELS)
    }

    fn with(channels: &str) -> Self {
        let tmp = workspace(channels);
        let worker = Arc::new(LiveWorker {
            log: tmp.path().join("dry.log"),
            working_copy: tmp.path().to_path_buf(),
            running: Mutex::new(HashSet::new()),
            looks: AtomicBool::new(true),
            blocked: AtomicBool::new(false),
        });
        let mut b = Bench {
            tmp,
            worker,
            timer: RecordingTimer::default(),
            origin: String::new(),
        };
        b.origin = b.engine().origin();
        b
    }

    fn handle(&self, bare: &str) -> String {
        format!("{}/{bare}", self.origin)
    }

    fn supervisor(&self) -> String {
        self.handle("__channel__")
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn engine(&self) -> Engine {
        Engine::open_with(
            None,
            self.root(),
            EngineConfig {
                worker: WorkerConfig::Custom(self.worker.clone()),
                timer: nexus_chat::timer::TimerConfig::Disabled,
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

    fn human(now: &str) -> Caller<'_> {
        Caller {
            session: None,
            actor: Some("carsten"),
            now: Some(now),
        }
    }

    fn commission(&self, now: &str, to: &str) -> String {
        self.engine()
            .send_to(
                Self::human(now),
                SendToRequest {
                    machine: None,
                    to,
                    body: "build the liveness report",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("commission")
            .thread_id
    }

    fn reply(&self, now: &str, session: &str, actor: &str, thread: &str, escalate: bool) {
        self.engine()
            .reply_thread(
                Caller {
                    session: Some(session),
                    actor: Some(actor),
                    now: Some(now),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body: "built, tests green",
                    escalate,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("the reply is posted");
    }

    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = self.worker.clone();
        let ctx = Ctx {
            now,
            origin: &self.origin,
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
            peers: None,
            liveness: Default::default(),
        };
        orchestration::tick(&ctx, &mut store, TickRequest { thread_id: thread }).expect("tick")
    }

    fn triggers(&self) -> Vec<String> {
        std::fs::read_to_string(self.root().join("dry.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("trigger role="))
            .map(str::to_string)
            .collect()
    }

    fn started(&self, handle: &str) -> usize {
        self.triggers()
            .iter()
            .filter(|l| l.starts_with(&format!("trigger role={handle} ")))
            .count()
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

    fn lease_holder(&self) -> Option<String> {
        self.store()
            .working_tree_lease_row()
            .expect("the lease row reads")
            .map(|(holder, _)| holder)
    }

    fn lease_expires(&self) -> String {
        self.store()
            .working_tree_lease_row()
            .expect("the lease row reads")
            .expect("somebody holds the working copy")
            .1
    }

    fn owes(&self, thread: &str, now: &str) -> Vec<String> {
        self.store()
            .thread_quorum(thread, now)
            .expect("quorum reads")
            .expect("the thread exists")
            .outstanding
    }

    fn park_branches(&self) -> String {
        git(self.root(), &["branch", "--list", "nxs/park/*"])
    }

    /// The commit message of the park on `branch` — what a reader of `git log` believes.
    fn park_message(&self, branch: &str) -> String {
        git(self.root(), &["log", "-1", "--format=%B", branch])
    }

    /// Open a thread under `parent`, as `opener`, at `now`.
    fn open_under(&self, id: &str, parent: &str, opener: &str, now: &str) {
        self.store().open_thread(
            id,
            &ThreadRoot {
                origin: self.origin.clone(),
                channel_id: "decl:coding".into(),
                opener: opener.into(),
                created: now.into(),
                parent: Some(parent.into()),
            },
            opener,
        );
    }

    fn the_copy_stayed(&self, root: &str) {
        assert_eq!(self.park_branches(), "", "nothing was parked");
        assert_eq!(
            self.lease_holder().as_deref(),
            Some(scope_of(root).as_str())
        );
        assert_eq!(self.started("solo"), 0, "the rival waits");
    }
}

fn scope_of(thread: &str) -> String {
    WorkScope::Thread(thread.to_string()).key()
}

/// How the measured stall is staged.
#[derive(Clone, Copy)]
struct Stage {
    /// A rival commission waits behind the copy.
    rival: bool,
    /// The killed advance left its empty `verify` slot behind.
    empty_slot: bool,
    /// The coder's end was recorded — not, when the `session ended` that would record it is still
    /// on its way.
    end_recorded: bool,
}

const LOST_ADVANCE: Stage = Stage {
    rival: true,
    empty_slot: false,
    end_recorded: true,
};

/// The id the killed advance's empty slot is given.
const EMPTY_SLOT: &str = "m-01EMPTYVERIFYSLOT0000000000";

/// **The measured shape, built the way it came about.** Returns `(root, build slot, coder session,
/// rival)`.
///
/// The coder's reply goes through the engine while its process is alive, so the advance is refused
/// by the real brought-along hurdle. Its process then goes and its end is recorded the way the
/// killed `session ended` recorded it — the end landed, the advance behind it did not. With
/// `empty_slot`, the slot that advance opened is left exactly as it was found: under the root,
/// opened by the supervisor, with a deadline and nothing else.
fn a_stalled_chain(b: &Bench, stage: Stage) -> (String, String, String, Option<String>) {
    let root = b.commission(T0, "coding");
    let coder = b.session_of("coder");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));
    assert_eq!(
        b.lease_expires(),
        THE_BOUND,
        "the premise: six hours from T0"
    );
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();

    let build = b
        .store()
        .supervised_children(&root)
        .expect("slots read")
        .into_iter()
        .next()
        .expect("the build slot");
    b.reply(ANSWERED, &coder, "coder", &build, false);
    assert_eq!(
        b.started("verifier"),
        0,
        "the hurdle held the verifier back"
    );

    let rival = stage.rival.then(|| {
        let solo = b.commission(QUEUED, "solo");
        assert_eq!(b.started("solo"), 0, "the rival is queued, not started");
        solo
    });

    b.worker.running.lock().unwrap().remove(&coder);
    if stage.end_recorded {
        b.store()
            .mark_session_ended(&coder, ENDED)
            .expect("the end is recorded");
    }
    if stage.empty_slot {
        b.open_under(EMPTY_SLOT, &root, &b.supervisor(), ENDED);
        b.store()
            .set_deadline(EMPTY_SLOT, "2026-11-02T15:22:00Z", &b.supervisor());
        assert!(
            b.owes(EMPTY_SLOT, QUIET).is_empty(),
            "the slot owes nothing"
        );
    }

    assert_eq!(
        b.owes(&root, QUIET),
        vec![b.supervisor()],
        "the premise: the root waits on its supervisor and on nothing else"
    );
    (root, build, coder, rival)
}

/// The stalled chain was NOT taken: no hand-off, nothing parked, the rival still waiting.
fn kept(b: &Bench, root: &str, receipt: &TickReceipt) {
    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none(), "{:?}", receipt.working_tree);
    b.the_copy_stayed(root);
}

// ---- (a) the dead-holder sweep -------------------------------------------------------------------

/// **The heart of (a).** Only the engine's own `__channel__` wait is open, nothing has a process,
/// every session reported its end, nothing is on its way in the operation, and it has been quiet
/// for longer than the engine ever takes to finish what it started — so the sweep takes the copy
/// inside the bound, parks the work first under a reason that says what happened, and the rival
/// starts.
#[test]
fn a_chain_stalled_on_its_supervisor_is_taken_inside_its_bound() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let rival = rival.unwrap();

    let receipt = b.tick(QUIET, &rival);

    let handed = receipt
        .handed_on
        .clone()
        .unwrap_or_else(|| panic!("the stalled chain kept the copy: {receipt:?}"));
    assert_eq!(handed.scope, scope_of(&root));
    assert_eq!(handed.occasion, ParkOccasion::StalledOnItsSupervisor);
    let parked = handed.parked.clone().expect("its work was parked first");
    assert!(parked.committed, "there was work to save");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim()
    );
    let message = b.park_message(&parked.branch);
    assert!(message.contains("chain stalled"), "{message}");
    assert!(!message.contains("died"), "no process died: {message}");
    assert_eq!(git(b.root(), &["rev-parse", "--abbrev-ref", "HEAD"]), BASE);
    assert_eq!(b.started("solo"), 1, "the rival started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&rival).as_str()));
    let json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(json["handed_on"]["occasion"], "stalled_on_its_supervisor");
}

/// **The quiet is five minutes, to the second.** The last thing that happened is the recorded end
/// at 09:22:00; at 09:26:59 the advance that end owes may still be on its way, at 09:27:00 it is not.
#[test]
fn the_quiet_is_five_minutes_to_the_second() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let rival = rival.unwrap();

    let early = b.tick(ONE_SECOND_SHORT, &rival);
    kept(&b, &root, &early);

    let on_time = b.tick(FIVE_MINUTES, &rival);
    assert_eq!(
        on_time.handed_on.map(|h| h.occasion),
        Some(ParkOccasion::StalledOnItsSupervisor)
    );
}

/// **…and a chain that has only just moved keeps the copy** — one minute after the end.
#[test]
fn a_chain_that_has_only_just_moved_keeps_the_copy() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);

    let receipt = b.tick(JUST_AFTER, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **A worker that cannot answer the liveness question never takes it early** — "nothing is
/// running" from a worker that never looked is not evidence, for this shape as for every other.
#[test]
fn a_worker_that_cannot_look_never_takes_a_stalled_chain_early() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    b.worker.looks.store(false, Ordering::SeqCst);

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **A process gone whose `session ended` has not landed is not a stalled chain.** That call can
/// take thirty seconds and more on a busy database, and when it lands it advances the chain — so
/// until every session in the operation has a recorded end, the copy stays.
#[test]
fn a_session_gone_without_a_recorded_end_keeps_the_copy() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(
        &b,
        Stage {
            end_recorded: false,
            ..LOST_ADVANCE
        },
    );

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **A commission of the operation's own, queued for the copy, is on its way** — the chain is not
/// stalled, it is waiting for its own next step to start.
#[test]
fn a_commission_queued_inside_the_operation_keeps_the_copy() {
    let b = Bench::new();
    let (root, build, coder, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    b.store()
        .enqueue_working_tree(
            &QueuedTrigger {
                id: 0,
                scope_key: scope_of(&root),
                role: "verifier".into(),
                session: format!("{coder}-v"),
                thread: Some(build),
                message: "verify".into(),
                model: None,
                depth: 1,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            ENDED,
        )
        .expect("queued");

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **A hand-back waits for a PERSON** — an escalation or a question in the operation is the
/// escalation rule's, with its own thirty minutes, never this one's.
#[test]
fn an_unanswered_hand_back_in_the_operation_keeps_the_copy() {
    let b = Bench::new();
    let (root, build, coder, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    // Posted at ENDED, so the quiet clock alone would let the sweep act at QUIET: what keeps the copy
    // here has to be the hand-back.
    let mut store = b.store();
    store.set_wall_clock(ENDED);
    store.post_message(&MessageEnvelope {
        origin: b.origin.clone(),
        channel_id: "decl:coding".into(),
        sender: b.handle("coder"),
        kind: MessageKind::Escalation,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(build),
        refs: Refs {
            session_id: Some(coder),
            ..Refs::default()
        },
        body: "which of the two APIs did you mean?".into(),
    });
    drop(store);

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **Work open across the border is running somewhere else.**
#[test]
fn an_open_border_thread_in_the_operation_keeps_the_copy() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let border = "m-01BORDERTHREAD000000000000";
    b.open_under(border, &root, &b.handle("carsten"), ENDED);
    b.store()
        .connection()
        .execute(
            "INSERT INTO border_threads(thread_id, direction, peer, persona, state, created, updated)
             VALUES (?1, 'out', 'other', 'reviewer', 'working', ?2, ?2)",
            [border, ENDED],
        )
        .expect("the border row");

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **The supervisor must be the ONLY one owed.** A thread of the operation that waits on a person
/// — who has no session to die — makes it a wait for that person, whatever the root waits on.
#[test]
fn a_thread_owing_a_person_beside_the_supervisor_keeps_the_copy() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let question = "m-01QUESTIONFORAPERSON0000000";
    b.open_under(question, &root, &b.handle("coder"), ENDED);
    b.store().set_expects_reply_from(
        question,
        &serde_json::to_string(&[b.handle("carsten")]).unwrap(),
        &b.handle("coder"),
    );
    assert_eq!(b.owes(question, QUIET), vec![b.handle("carsten")]);

    let receipt = b.tick(QUIET, &rival.unwrap());

    kept(&b, &root, &receipt);
}

/// **A declared hurdle that refused the next step waits for the requester.** Nothing retries it —
/// asking again is the requester's move — and the person may be fixing the tree right now; parking
/// and checking out the base under them is the one thing that must not happen.
#[test]
fn a_declared_hurdle_that_stopped_the_chain_keeps_the_copy() {
    let b = Bench::with(CHANNELS_WITH_A_HURDLE);
    let root = b.commission(T0, "coding");
    let coder = b.session_of("coder");
    std::fs::write(b.root().join("src.txt"), HALF_FINISHED).unwrap();
    let build = b.store().supervised_children(&root).unwrap()[0].clone();
    b.reply(ANSWERED, &coder, "coder", &build, false);
    let solo = b.commission(QUEUED, "solo");

    // The coder's process goes and its end arrives — the advance runs, and the PROJECT's rule says no.
    b.worker.running.lock().unwrap().remove(&coder);
    b.worker.blocked.store(true, Ordering::SeqCst);
    b.engine()
        .session_ended(
            Caller {
                session: Some(&coder),
                actor: Some("coder"),
                now: Some(ENDED),
            },
            &coder,
        )
        .expect("the end is recorded");
    assert_eq!(
        b.started("verifier"),
        0,
        "the declared hurdle refused verify"
    );
    assert_eq!(b.owes(&root, QUIET), vec![b.supervisor()], "the premise");

    let receipt = b.tick(QUIET, &solo);

    kept(&b, &root, &receipt);
}

/// **A chain the tick can revive is not parked — it is revived.** With the killed advance's empty
/// slot standing, the sweep declines and arms a tick of the channel thread; that tick moves the
/// chain on to `verify`, which keeps the copy in its own operation.
#[test]
fn a_chain_with_a_slot_never_commissioned_is_revived_rather_than_parked() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(
        &b,
        Stage {
            empty_slot: true,
            ..LOST_ADVANCE
        },
    );

    let receipt = b.tick(REVIVING, &rival.unwrap());

    kept(&b, &root, &receipt);
    // Armed under the SLOT's own key, at the instant the slot counts out of the set…
    assert_eq!(
        *b.timer.looks.lock().unwrap(),
        vec![(
            format!("{EMPTY_SLOT}-revive"),
            root.clone(),
            SLOT_COUNTS_OUT.to_string()
        )],
    );
    // …and NOT as the channel thread's own window, which it would have replaced.
    assert!(
        !b.timer
            .armed
            .lock()
            .unwrap()
            .iter()
            .any(|(thread, _)| thread == &root),
        "the channel thread's window job is left alone: {:?}",
        b.timer.armed.lock().unwrap()
    );

    // The revival itself — the channel thread's own tick — arms nothing again: its channel half is
    // what moves the chain, a second later is not another chance.
    b.timer.looks.lock().unwrap().clear();
    b.tick(REVIVING, &root);
    assert!(
        !b.timer
            .looks
            .lock()
            .unwrap()
            .iter()
            .any(|(slot, _, _)| slot.ends_with("-revive")),
        "the revival re-armed itself: {:?}",
        b.timer.looks.lock().unwrap()
    );
    assert_eq!(b.started("verifier"), 1, "the chain went on to verify");
}

/// **A revival that cannot move the chain is parked, not revived for the rest of the bound**
/// (re-review of PR #111, R1). Here the never-commissioned slot is the ONLY slot of its channel
/// thread — a nested step whose first member's commission was killed — so the tick keeps it (a
/// board with no members at all is not consolidated) and the chain cannot move. Inside the
/// revival's window the sweep arms its tick; past it, the chain is a stall like any other.
#[test]
fn a_revival_that_cannot_move_the_chain_is_parked_once_its_window_is_over() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let rival = rival.unwrap();
    let nested = "m-01NESTEDREVIEWCHANNEL00000";
    let only = "m-01NESTEDSLOTNEVERCOMMISSIONED";
    b.open_under(nested, &root, &b.supervisor(), ENDED);
    b.store().set_expects_reply_from(
        nested,
        &serde_json::to_string(&[b.supervisor()]).unwrap(),
        &b.supervisor(),
    );
    b.open_under(only, nested, &b.supervisor(), ENDED);
    assert_eq!(b.owes(nested, QUIET), vec![b.supervisor()], "the premise");

    let inside = b.tick(REVIVING, &rival);
    kept(&b, &root, &inside);
    assert_eq!(
        b.timer
            .looks
            .lock()
            .unwrap()
            .iter()
            .map(|(slot, thread, _)| (slot.clone(), thread.clone()))
            .collect::<Vec<_>>(),
        vec![(format!("{only}-revive"), nested.to_string())],
        "inside its window the revival is armed once"
    );

    // The revival runs and cannot move anything.
    b.tick(REVIVING, nested);
    assert_eq!(b.started("verifier"), 0);

    b.timer.looks.lock().unwrap().clear();
    let past = b.tick(QUIET, &rival);
    assert_eq!(
        past.handed_on.map(|h| h.occasion),
        Some(ParkOccasion::StalledOnItsSupervisor),
        "past its window the stall is parked"
    );
    assert!(
        !b.timer
            .looks
            .lock()
            .unwrap()
            .iter()
            .any(|(slot, _, _)| slot.ends_with("-revive")),
        "and its revival is not armed again: {:?}",
        b.timer.looks.lock().unwrap()
    );
    assert_eq!(b.started("solo"), 1);
}

/// **A revived chain that stalls AGAIN is taken.** The empty slot the revival moved past stays under
/// the root for the operation's whole life; it is history, not the newest slot, and must not keep
/// the rule declining when the verifier's advance is lost the same way the coder's was.
#[test]
fn a_revived_chain_that_stalls_again_is_taken() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(
        &b,
        Stage {
            empty_slot: true,
            ..LOST_ADVANCE
        },
    );
    let rival = rival.unwrap();
    b.tick(REVIVING, &root);
    assert_eq!(b.started("verifier"), 1, "revived: the verifier runs");
    let verifier = b.session_of("verifier");
    let verify = b
        .store()
        .supervised_children(&root)
        .unwrap()
        .into_iter()
        .find(|t| b.owes(t, QUIET) == vec![b.handle("verifier")])
        .expect("the verify slot");

    // The verifier answers while it still runs, ends, and the move after it is lost.
    b.reply(
        "2026-11-02T09:41:00Z",
        &verifier,
        "verifier",
        &verify,
        false,
    );
    b.worker.running.lock().unwrap().remove(&verifier);
    b.store()
        .mark_session_ended(&verifier, "2026-11-02T09:42:00Z")
        .unwrap();
    assert_eq!(b.owes(&root, QUIET), vec![b.supervisor()], "the premise");

    let receipt = b.tick("2026-11-02T09:50:00Z", &rival);

    assert_eq!(
        receipt.handed_on.map(|h| h.occasion),
        Some(ParkOccasion::StalledOnItsSupervisor),
        "looks armed: {:?}",
        b.timer.looks.lock().unwrap()
    );
    assert_eq!(b.started("solo"), 1);
}

/// **An OLD unended session on a thread a newer one replaced is history.** A start that failed and
/// was redone, or a session a restart killed, leaves a row with no recorded end; only the NEWEST
/// session of a thread is asked whether its end has landed.
#[test]
fn an_old_unended_session_replaced_by_a_newer_one_does_not_block_the_rule() {
    let b = Bench::new();
    let (root, build, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let mut store = b.store();
    // Minted before the coder's session, so it sorts as the older row on the same thread.
    store
        .create_pending_session("m-00OLDSTARTTHATFAILED00000", "coder")
        .unwrap();
    store
        .record_session_thread("m-00OLDSTARTTHATFAILED00000", Some(&build))
        .unwrap();
    drop(store);

    let receipt = b.tick(QUIET, &rival.unwrap());

    let handed = receipt
        .handed_on
        .unwrap_or_else(|| panic!("the old row kept the copy: {:?}", receipt.warnings));
    assert_eq!(handed.scope, scope_of(&root));
    assert_eq!(handed.occasion, ParkOccasion::StalledOnItsSupervisor);
}

// ---- (b) the bound ------------------------------------------------------------------------------

/// **Past the bound the queue moves** — with the early rule out of the way (a worker that cannot
/// look), the backstop alone has to do it, and it does: the stalled chain's work is parked and the
/// rival starts.
#[test]
fn past_its_bound_a_stalled_chain_hands_the_copy_to_whoever_waits() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(&b, LOST_ADVANCE);
    let rival = rival.unwrap();
    b.worker.looks.store(false, Ordering::SeqCst);

    let receipt = b.tick(PAST_THE_BOUND, &rival);

    let swept = receipt
        .working_tree
        .clone()
        .unwrap_or_else(|| panic!("nothing was swept past the bound: {receipt:?}"));
    assert_eq!(swept.scope, scope_of(&root));
    assert!(swept.parked.is_some(), "{swept:?}");
    assert_eq!(b.started("solo"), 1);
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&rival).as_str()));
}

/// **…and a commission sent after the bound, with nobody waiting, takes the copy at once.**
#[test]
fn past_its_bound_a_fresh_commission_takes_the_copy_of_a_stalled_chain() {
    let b = Bench::new();
    let (root, _, _, _) = a_stalled_chain(
        &b,
        Stage {
            rival: false,
            ..LOST_ADVANCE
        },
    );
    b.worker.looks.store(false, Ordering::SeqCst);
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));

    let solo = b.commission(PAST_THE_BOUND, "solo");

    assert_eq!(b.started("solo"), 1, "the fresh commission started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&solo).as_str()));
    assert_ne!(
        b.park_branches(),
        "",
        "the stalled chain's work was parked first"
    );
}

// ---- (c) withdraw -------------------------------------------------------------------------------

/// **The commissioner can take it back.** `withdraw` used to answer "nothing is waiting or running"
/// here — true, and the reason nothing ever released the copy. An operation that HOLDS the working
/// copy with nothing running in it is something to take back: its open threads are discharged, so
/// no tick commissions anything from it again, and the background tick parks its work — under a
/// reason that does not claim a running round was stopped — and hands the copy on.
#[test]
fn withdraw_takes_back_a_chain_that_holds_the_copy_with_nothing_running() {
    let b = Bench::new();
    let (root, _, _, rival) = a_stalled_chain(
        &b,
        Stage {
            empty_slot: true,
            ..LOST_ADVANCE
        },
    );
    let rival = rival.unwrap();

    let receipt = b
        .engine()
        .withdraw(Bench::human(JUST_AFTER), &root)
        .expect("the stalled operation is withdrawn");

    assert!(receipt.held_idle, "{receipt:?}");
    assert!(receipt.will_park, "{receipt:?}");
    assert!(receipt.stopped.is_empty() && receipt.withdrawn.is_empty());
    assert!(
        b.owes(&root, JUST_AFTER).is_empty(),
        "the root owes nothing any more"
    );
    assert_eq!(
        b.store()
            .withdrawn_holder(&scope_of(&root))
            .expect("the marker reads")
            .map(|m| m.by),
        Some(b.handle("carsten"))
    );

    let ticked = b.tick(JUST_AFTER, &rival);

    let handed = ticked
        .handed_on
        .clone()
        .unwrap_or_else(|| panic!("the withdrawn chain kept the copy: {ticked:?}"));
    assert_eq!(handed.scope, scope_of(&root));
    assert_eq!(handed.occasion, ParkOccasion::WithdrawnIdle);
    let parked = handed.parked.expect("its work was parked first");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim()
    );
    let message = b.park_message(&parked.branch);
    assert!(message.contains("nothing running"), "{message}");
    assert!(
        !message.contains("running round"),
        "no round was running: {message}"
    );
    assert_eq!(b.started("solo"), 1, "the rival started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&rival).as_str()));

    // …and the root's own tick, once the slot is long past its grace, commissions nothing.
    b.tick(QUIET, &root);
    assert_eq!(
        b.started("verifier"),
        0,
        "a withdrawn chain commissions nothing further"
    );
}

/// **The one door of 6j6v.2pnn this covers: a copy held by an escalation with nothing running.**
/// A persona hands its task back and its session ends; the escalation holds the copy, and
/// `withdraw` used to answer "nothing is waiting or running". Now the commissioner takes it back
/// and the copy moves. (How a SUPERSEDED escalation came to hold a delivered round in 2pnn is not
/// addressed here.)
#[test]
fn withdraw_takes_back_a_copy_held_by_an_escalation_with_nothing_running() {
    let b = Bench::new();
    let thread = b.commission(T0, "solo");
    let session = b.session_of("solo");
    b.reply(ANSWERED, &session, "solo", &thread, true);
    b.worker.running.lock().unwrap().remove(&session);
    b.store().mark_session_ended(&session, ENDED).unwrap();
    assert_eq!(
        b.lease_holder().as_deref(),
        Some(scope_of(&thread).as_str())
    );

    let receipt = b
        .engine()
        .withdraw(Bench::human(JUST_AFTER), &thread)
        .expect("an escalation holding the copy with nothing running is withdrawn");
    assert!(receipt.held_idle, "{receipt:?}");

    let ticked = b.tick(JUST_AFTER, &thread);
    assert_eq!(
        ticked.handed_on.map(|h| h.occasion),
        Some(ParkOccasion::WithdrawnIdle)
    );
    assert_eq!(
        b.lease_holder(),
        None,
        "nobody waits, so the copy is simply free"
    );
}

fn nxc(b: &Bench) -> assert_cmd::Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(b.root())
        .env_remove("NXC_SESSION")
        .env_remove("NXC_ORIGIN")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", JUST_AFTER)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", b.root().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", b.root().join("timer.log"));
    c
}

/// **The same door from the command line** — what the commissioner actually types.
#[test]
fn nxc_withdraw_takes_back_a_chain_that_holds_the_copy_with_nothing_running() {
    let b = Bench::new();
    let (root, _, _, _) = a_stalled_chain(&b, LOST_ADVANCE);

    let out = nxc(&b)
        .args(["withdraw", "--thread", &root])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();

    assert!(
        stdout.contains("held this workspace's working copy with nothing running"),
        "{stdout}"
    );
    assert!(
        b.owes(&root, JUST_AFTER).is_empty(),
        "the root owes nothing any more"
    );
    assert!(b
        .store()
        .withdrawn_holder(&scope_of(&root))
        .expect("the marker reads")
        .is_some());
}

/// **…and its `--json` carries the fact as `held_idle`.**
#[test]
fn nxc_withdraw_json_says_held_idle() {
    let b = Bench::new();
    let (root, _, _, _) = a_stalled_chain(&b, LOST_ADVANCE);

    let out = nxc(&b)
        .args(["withdraw", "--json", "--thread", &root])
        .assert()
        .success();
    let json: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("a withdraw receipt");

    assert_eq!(json["held_idle"], true, "{json}");
    // The dry worker names no working copy, so this host can only hand the copy on unparked — and
    // the receipt says so rather than promising a branch.
    assert_eq!(json["will_park"], false, "{json}");
    assert!(json["cannot_park"].is_string(), "{json}");
}

/// **An operation that holds nothing and runs nothing is still refused** — the new door is the
/// held copy, not a licence to "withdraw" any finished conversation.
#[test]
fn withdraw_still_refuses_an_operation_that_holds_nothing_and_runs_nothing() {
    let b = Bench::new();
    let (root, _, _, _) = a_stalled_chain(
        &b,
        Stage {
            rival: false,
            ..LOST_ADVANCE
        },
    );
    // Somebody else takes the copy over first (past the bound, so it may).
    b.worker.looks.store(false, Ordering::SeqCst);
    b.commission(PAST_THE_BOUND, "solo");
    assert_ne!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));

    let err = b
        .engine()
        .withdraw(Bench::human(PAST_THE_BOUND), &root)
        .expect_err("nothing to withdraw");
    assert!(err.to_string().contains("is waiting or running"), "{err}");
}

// ---- (d) the empty slot -------------------------------------------------------------------------

/// **A slot the engine opened and never commissioned does not hold the flow.** The killed advance
/// left a `verify` slot with nobody expected in it; a tick on the root read it as a member still
/// working and answered `not_due` for ever. Once it is older than the engine ever takes to finish a
/// commission, it is not a member of anything, and the tick moves the chain on to `verify`.
#[test]
fn a_tick_moves_the_chain_on_past_a_slot_that_was_never_commissioned() {
    let b = Bench::new();
    let (root, _, _, _) = a_stalled_chain(
        &b,
        Stage {
            rival: false,
            empty_slot: true,
            end_recorded: true,
        },
    );

    let early = b.tick(JUST_AFTER, &root);
    assert_eq!(
        b.started("verifier"),
        0,
        "inside the grace the slot may still be being written: {early:?}"
    );

    b.tick(QUIET, &root);

    assert_eq!(b.started("verifier"), 1, "the chain went on to verify");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));
}
