//! ACCEPTANCE of nxf 6j6v.3rrv and 6j6v.2pnn — **a chain that stalled on its own supervisor gives
//! the working copy back**.
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
//! This file pins the way out on each of those doors. Everything is real — a git repository, the
//! lease, the queue, the park; the engine handle carries every commission and the withdrawal, and
//! `orchestration::tick` carries the tick, which is what the background service runs. The worker is
//! the only stand-in, and only for what a worker does.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::ThreadRoot;
use nexus_chat::orchestration::{self, Caller, Ctx, ParkOccasion, TickReceipt, TickRequest};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::DisabledTimer;
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The run is commissioned and the coder takes the working copy.
const T0: &str = "2026-11-02T09:00:00Z";
/// The coder answers while its process still runs — the advance is refused, correctly.
const ANSWERED: &str = "2026-11-02T09:20:00Z";
/// A rival commission queues behind the copy.
const QUEUED: &str = "2026-11-02T09:21:00Z";
/// The coder's end is recorded and the killed `session ended` leaves its empty `verify` slot.
const ENDED: &str = "2026-11-02T09:22:00Z";
/// One minute after the last thing that happened in the operation: too early to call it stalled.
const JUST_AFTER: &str = "2026-11-02T09:23:00Z";
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

// ---- harness ------------------------------------------------------------------------------------

/// Records its triggers like [`DryWorker`], names where sessions run, and answers which of them
/// still have a process. `blind` makes it a worker that cannot answer the liveness question at all.
struct LiveWorker {
    log: PathBuf,
    working_copy: PathBuf,
    running: Mutex<HashSet<String>>,
    looks: AtomicBool,
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

fn workspace() -> TempDir {
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
    std::fs::write(roles.join("channels.yaml"), CHANNELS).unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
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
    /// The workspace's own replica prefix — the `<origin>` of every qualified handle minted here,
    /// the supervisor's included.
    origin: String,
}

impl Bench {
    fn new() -> Self {
        let tmp = workspace();
        let worker = Arc::new(LiveWorker {
            log: tmp.path().join("dry.log"),
            working_copy: tmp.path().to_path_buf(),
            running: Mutex::new(HashSet::new()),
            looks: AtomicBool::new(true),
        });
        let mut b = Bench {
            tmp,
            worker,
            origin: String::new(),
        };
        b.origin = b.engine().origin();
        b
    }

    fn supervisor(&self) -> String {
        format!("{}/__channel__", self.origin)
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
            timer: &DisabledTimer,
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
}

fn scope_of(thread: &str) -> String {
    WorkScope::Thread(thread.to_string()).key()
}

/// **The measured shape, built the way it came about.** Returns `(root, rival)` — `rival` is
/// `None` when `with_rival` is false.
///
/// The coder's reply goes through the engine while its process is alive, so the advance is refused
/// by the real hurdle. The end is then recorded the way the killed `session ended` recorded it —
/// the end landed, the advance behind it did not — and the slot that advance opened is left exactly
/// as it was found: a thread under the root, opened by the supervisor, with a deadline and nothing
/// else.
fn a_stalled_chain(b: &Bench, with_rival: bool) -> (String, Option<String>) {
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
    b.engine()
        .reply_thread(
            Caller {
                session: Some(&coder),
                actor: Some("coder"),
                now: Some(ANSWERED),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &build,
                body: "built, tests green",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the coder's answer is posted");
    assert_eq!(
        b.started("verifier"),
        0,
        "the hurdle held the verifier back"
    );

    let rival = with_rival.then(|| {
        let solo = b.commission(QUEUED, "solo");
        assert_eq!(b.started("solo"), 0, "the rival is queued, not started");
        solo
    });

    // The killed `session ended`: its end landed, and so did the first two writes of the advance.
    b.worker.running.lock().unwrap().remove(&coder);
    let mut store = b.store();
    store
        .mark_session_ended(&coder, ENDED)
        .expect("the end is recorded");
    let empty = "m-01EMPTYVERIFYSLOT0000000000";
    store.open_thread(
        empty,
        &ThreadRoot {
            origin: b.origin.clone(),
            channel_id: "decl:coding".into(),
            opener: b.supervisor(),
            created: ENDED.into(),
            parent: Some(root.clone()),
        },
        &b.supervisor(),
    );
    store.set_deadline(empty, "2026-11-02T15:22:00Z", &b.supervisor());
    drop(store);

    assert_eq!(
        b.owes(&root, QUIET),
        vec![b.supervisor()],
        "the premise: the root waits on its supervisor and on nothing else"
    );
    assert!(b.owes(empty, QUIET).is_empty(), "the slot owes nothing");
    (root, rival)
}

// ---- (a) the dead-holder sweep -------------------------------------------------------------------

/// **The heart of (a).** Only the engine's own `__channel__` wait is open, nothing has a process,
/// nothing is queued in the operation, and it has been quiet for longer than the engine ever takes
/// to finish what it started — so the sweep takes the copy inside the bound, parks the work first,
/// and the rival starts.
#[test]
fn a_chain_stalled_on_its_supervisor_is_taken_inside_its_bound() {
    let b = Bench::new();
    let (root, rival) = a_stalled_chain(&b, true);
    let rival = rival.unwrap();

    let receipt = b.tick(QUIET, &rival);

    let handed = receipt
        .handed_on
        .clone()
        .unwrap_or_else(|| panic!("the stalled chain kept the copy: {receipt:?}"));
    assert_eq!(handed.scope, scope_of(&root));
    assert_eq!(handed.occasion, ParkOccasion::DiedInsideItsBound);
    let parked = handed.parked.clone().expect("its work was parked first");
    assert!(parked.committed, "there was work to save");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim()
    );
    assert_eq!(git(b.root(), &["rev-parse", "--abbrev-ref", "HEAD"]), BASE);
    assert_eq!(b.started("solo"), 1, "the rival started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&rival).as_str()));
}

/// **…but not while the operation has only just moved.** One minute after the end was recorded the
/// advance that end owes may still be on its way — a `session ended` under a busy database can take
/// that long — and taking the copy then would park work out from under a step that is about to
/// start.
#[test]
fn a_chain_that_has_only_just_moved_keeps_the_copy() {
    let b = Bench::new();
    let (root, rival) = a_stalled_chain(&b, true);

    let receipt = b.tick(JUST_AFTER, &rival.unwrap());

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert!(receipt.working_tree.is_none());
    assert_eq!(b.park_branches(), "");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));
    assert_eq!(b.started("solo"), 0);
}

/// **A worker that cannot answer the liveness question never takes it early** — "nothing is
/// running" from a worker that never looked is not evidence, for this shape as for every other.
#[test]
fn a_worker_that_cannot_look_never_takes_a_stalled_chain_early() {
    let b = Bench::new();
    let (root, rival) = a_stalled_chain(&b, true);
    b.worker.looks.store(false, Ordering::SeqCst);

    let receipt = b.tick(QUIET, &rival.unwrap());

    assert!(receipt.handed_on.is_none(), "{:?}", receipt.handed_on);
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&root).as_str()));
    assert_eq!(b.started("solo"), 0);
}

// ---- (b) the bound ------------------------------------------------------------------------------

/// **Past the bound the queue moves** — with the early rule out of the way (a worker that cannot
/// look), the backstop alone has to do it, and it does: the stalled chain's work is parked and the
/// rival starts.
#[test]
fn past_its_bound_a_stalled_chain_hands_the_copy_to_whoever_waits() {
    let b = Bench::new();
    let (root, rival) = a_stalled_chain(&b, true);
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
    let (root, _) = a_stalled_chain(&b, false);
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
/// no tick commissions anything from it again, and the background tick parks its work and hands the
/// copy on.
#[test]
fn withdraw_takes_back_a_chain_that_holds_the_copy_with_nothing_running() {
    let b = Bench::new();
    let (root, rival) = a_stalled_chain(&b, true);
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
        Some(format!("{}/carsten", b.origin))
    );

    let ticked = b.tick(JUST_AFTER, &rival);

    let handed = ticked
        .handed_on
        .clone()
        .unwrap_or_else(|| panic!("the withdrawn chain kept the copy: {ticked:?}"));
    assert_eq!(handed.scope, scope_of(&root));
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.expect("its work was parked first");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim()
    );
    assert_eq!(b.started("solo"), 1, "the rival started");
    assert_eq!(b.lease_holder().as_deref(), Some(scope_of(&rival).as_str()));
    assert_eq!(
        b.started("verifier"),
        0,
        "a withdrawn chain commissions nothing further"
    );
}

/// **The same door from the command line** — what the commissioner actually types.
#[test]
fn nxc_withdraw_takes_back_a_chain_that_holds_the_copy_with_nothing_running() {
    let b = Bench::new();
    let (root, _) = a_stalled_chain(&b, true);

    let out = nxs_test_support::cargo_bin("nxc")
        .current_dir(b.root())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", JUST_AFTER)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", b.root().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", b.root().join("timer.log"))
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

/// **An operation that holds nothing and runs nothing is still refused** — the new door is the
/// held copy, not a licence to "withdraw" any finished conversation.
#[test]
fn withdraw_still_refuses_an_operation_that_holds_nothing_and_runs_nothing() {
    let b = Bench::new();
    let (root, _) = a_stalled_chain(&b, false);
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
    let (root, _) = a_stalled_chain(&b, false);

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
