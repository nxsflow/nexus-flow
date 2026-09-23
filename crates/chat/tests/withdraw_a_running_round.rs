//! **`withdraw` takes back a RUNNING round and parks its work** (nxf 6j6v.b9nf) — the half of the
//! verb `withdraw_a_parked_commission.rs` deliberately left open, driven through the LIBRARY HANDLE
//! (`Engine::withdraw`) with a worker that can answer the two questions the running case turns on:
//! *is this session running* and *stop it*.
//!
//! Until this item a round that had STARTED could not be taken back at all: `nxc withdraw` refused
//! it as `not_found`, and the only way to stop a session that was writing into the checkout was to
//! find its pid by hand — after which nothing parked what it had left in the tree, and the next
//! holder started inside it. The owner's decision of 2026-09-17 is that `withdraw` is the ONE verb
//! for taking a commission back, queued or running, and that the uncommitted work of a withdrawn
//! running round is **parked automatically, never rolled back**.
//!
//! Three properties are load-bearing and each has its own case below:
//!
//! * the withdrawal DISCHARGES the thread and STOPS the session, and says both on the receipt
//!   (`stopped`, `will_park`); a host that cannot stop a session is refused by name before anything
//!   changes;
//! * the park happens on the TICK, only once nothing in the claim area is running any more — a
//!   `git checkout` under a live process is the collision this whole epic exists to prevent — and
//!   while a process is still there the tick re-arms at the liveness cadence;
//! * the withdrawn holder does not let the working copy go by ANY other path — its own teardown
//!   (`session ended`, a late reply) must not hand the copy on with the work still in the tree.
//!
//! **Everything here is real** except the worker, exactly as in
//! `an_unanswered_escalation_parks_and_comes_back.rs`: a real git repository in a temporary
//! directory, the real engine verbs, the real lease and queue. [`StoppableWorker`] starts no
//! operating-system process; it records what it was handed (as `DryWorker` does), answers liveness
//! from a set this file toggles (as `LiveSessionWorker` does next door), names the repository as the
//! working copy, and stops a session by taking it out of that set — or, for the one case about the
//! wait, by NOT doing so until the test says the process is gone.

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::StatusScope;
use nexus_chat::orchestration::{
    self, Caller, ConsequenceClass, Ctx, ParkOccasion, TickReceipt, TickRequest, WithdrawReceipt,
};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerConfig, TimerHandle};
use nexus_chat::worker::{
    DryWorker, SessionStop, TriggerError, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here and starts writing.
const T0: &str = "2026-09-19T09:00:00Z";
/// A rival arrives and queues behind the copy.
const QUEUED: &str = "2026-09-19T09:05:00Z";
/// The human takes the running round back.
const WITHDRAWN: &str = "2026-09-19T09:10:00Z";
/// `WITHDRAWN` + `SESSION_LIVENESS_RECHECK_SECS` — the instant the withdrawal arms a look for.
const WITHDRAWN_PLUS_CADENCE: &str = "2026-09-19T09:11:00Z";
/// The background service's next tick.
const TICK: &str = "2026-09-19T09:11:30Z";
/// `TICK` + `SESSION_LIVENESS_RECHECK_SECS`.
const TICK_PLUS_CADENCE: &str = "2026-09-19T09:12:30Z";
/// A later tick, once the stopped process is really gone.
const LATER: &str = "2026-09-19T09:13:00Z";
/// The rival finishes and the human comes back for the parked work.
const COME_BACK: &str = "2026-09-19T09:30:00Z";

/// What a coding agent leaves behind in the tree it works in.
const HALF_FINISHED: &str = "as it was\nhalf finished\n";
const NOTES: &str = "my working notes\n";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records like [`DryWorker`], answers liveness from a set this file toggles, names
/// the repository as the working copy, and STOPS a session by taking it out of that set.
struct StoppableWorker {
    log: PathBuf,
    /// What the runtime says sessions run in: the repository, a directory that is not one, or
    /// nothing at all — the three the park's READ-ONLY preconditions branch on.
    working_copy: Option<PathBuf>,
    running: Mutex<HashSet<String>>,
    /// Every session this worker was asked to stop, in order.
    stopped: Mutex<Vec<String>>,
    /// Whether a stop takes the session out of `running` at once. `false` models what the real
    /// sidecar does: the request is DELIVERED and the process is still there until its teardown
    /// has run — the caller has to watch the liveness read turn false.
    stop_marks_gone: bool,
    /// **What every stop AFTER the first answers**, when set (nxf 6j6v.27b9; review of PR #483,
    /// Test Quality #2) — the tick's one further SIGTERM meeting a claim it cannot prove, a signal
    /// the system refused, or a process that is gone by then. `None` answers as the first stop did.
    later_stops: Mutex<Option<std::result::Result<SessionStop, String>>>,
    /// **Refuse a spawn for a session whose process is still running**, as the shipped
    /// `SidecarWorker` does through its session lock (`TriggerError::AlreadyRunning`) — the one
    /// shape in which a follow-up into a withdrawn thread starts NOTHING (review of PR #483,
    /// Integrity #1). `false` starts a second process beside it, as `DryWorker` does.
    refuse_a_live_spawn: Mutex<bool>,
    /// **A thread to read, from a second connection, at the moment of every stop** — and what its
    /// register said then (review of PR #483, Integrity #2): the one window into the ORDER of
    /// `withdraw`'s writes against its signals, `RecordingTimer::watch`'s technique on the stop.
    look_at_on_stop: Mutex<Option<String>>,
    seen_on_stop: Mutex<Vec<Vec<String>>>,
}

impl StoppableWorker {
    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
    fn stopped(&self) -> Vec<String> {
        self.stopped.lock().unwrap().clone()
    }
}

impl Worker for StoppableWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if *self.refuse_a_live_spawn.lock().unwrap()
            && self.session_is_running(&req.internal_session)
        {
            return Err(TriggerError::AlreadyRunning {
                session: req.internal_session,
                pid: 4123,
            });
        }
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        self.working_copy.clone()
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
        let earlier = {
            let mut stopped = self.stopped.lock().unwrap();
            stopped.push(internal_session.to_string());
            stopped.len() - 1
        };
        if let Some(thread) = self.look_at_on_stop.lock().unwrap().clone() {
            let root = self
                .log
                .parent()
                .expect("the log sits in the workspace root");
            let expects = Workspace::resolve(None, root)
                .expect("resolve workspace")
                .open_chat_store()
                .expect("open chat store")
                .thread_quorum(&thread, WITHDRAWN)
                .expect("quorum reads")
                .map(|q| q.expects)
                .unwrap_or_default();
            self.seen_on_stop.lock().unwrap().push(expects);
        }
        if earlier > 0 {
            if let Some(answer) = self.later_stops.lock().unwrap().clone() {
                return answer;
            }
        }
        if self.stop_marks_gone {
            self.mark_gone(internal_session);
        }
        Ok(SessionStop::Requested)
    }
}

/// The same worker with `stops_sessions` / `stop_session` left on the trait DEFAULT — the shape
/// every host that has not implemented the stop presents, and the one `withdraw` must refuse by
/// name rather than believe.
struct CannotStopWorker(Arc<StoppableWorker>);

impl Worker for CannotStopWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.0.trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.0.session_is_running(internal_session)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        self.0.working_copy()
    }
}

/// A timer that records every window it is asked to arm — `a_refused_park_follows_the_rule.rs`'s,
/// for its reason: the cadence a withdrawn holder is looked at again on is a fact about WHAT IS
/// ARMED, and nothing else can see it. Arming replaces by key, so [`Self::last_for`] is what a key
/// will actually fire at.
///
/// **And, when [`Self::watch`] has been set, it is also this file's one window into the ORDER of a
/// single call's writes.** `schedule` is a seam the verb calls out through while it is still
/// running, so a reader opened there sees exactly what a CONCURRENT process would see in that
/// instant — which is the whole subject of the marker's argument. See
/// `a_queued_commission_and_a_running_round_in_one_area_are_withdrawn_behind_one_marker`.
#[derive(Default)]
struct RecordingTimer {
    armed: Mutex<Vec<(String, String)>>,
    watch: Mutex<Option<Watch>>,
    watched: Mutex<Vec<Watched>>,
    /// **When set, this timer schedules perfectly well and reports that NOTHING will run what it
    /// schedules** — `service_not_running.rs`'s `HealthyButUnattended`, for that file's reason: the
    /// two facts are independent, and a workspace whose windows arm fine can still have no process
    /// behind them.
    unattended: Mutex<bool>,
}

/// Where to look, and for what, when a window is armed.
struct Watch {
    root: PathBuf,
    scope_key: String,
    threads: Vec<String>,
}

/// What that second reader saw.
#[derive(Debug, Clone)]
struct Watched {
    /// The key the window was armed under.
    key: String,
    /// Whether the withdrawn holder's marker already stood.
    marker: bool,
    /// Which of the watched threads had already been discharged onto the withdrawn identity.
    discharged: Vec<String>,
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

    fn watch(&self, root: &Path, scope_key: &str, threads: &[String]) {
        *self.watch.lock().unwrap() = Some(Watch {
            root: root.to_path_buf(),
            scope_key: scope_key.to_string(),
            threads: threads.to_vec(),
        });
    }

    fn watched(&self) -> Vec<Watched> {
        self.watched.lock().unwrap().clone()
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
        if let Some(watch) = self.watch.lock().unwrap().as_ref() {
            // A SECOND connection to the same workspace, opened while the verb that armed this
            // window is still inside its own call. Every op this store writes commits in a
            // transaction of its own (`foundation::store::build_and_ingest`), and the database is
            // WAL with a busy timeout, so this reads precisely what another `nxc` process would
            // read in this instant — no more, and nothing that is still in flight.
            let store = Workspace::resolve(None, &watch.root)
                .expect("resolve workspace")
                .open_chat_store()
                .expect("open chat store");
            let marker = store
                .withdrawn_holder(&watch.scope_key)
                .expect("marker reads")
                .is_some();
            let discharged = watch
                .threads
                .iter()
                .filter(|t| {
                    store
                        .thread_quorum(t, deadline)
                        .expect("quorum reads")
                        .is_some_and(|q| q.expects.iter().any(|h| h.ends_with(WITHDRAWN_HANDLE)))
                })
                .cloned()
                .collect();
            self.watched.lock().unwrap().push(Watched {
                key: thread_id.to_string(),
                marker,
                discharged,
            });
        }
        Ok(TimerHandle(format!("recorded:{thread_id}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> nexus_chat::error::Result<()> {
        Ok(())
    }
    fn service_fault(&self) -> Option<nxs_service::ServiceFault> {
        self.unattended
            .lock()
            .unwrap()
            .then_some(nxs_service::ServiceFault::NotRunning)
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

fn head_branch(root: &Path) -> String {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

/// **The ordered, exclusive channel the channel cases run** (fix round 2 of this item's review,
/// Test Quality #5) — THREE steps, so the step that is taken back can be the MIDDLE one: the shape
/// the review asked about is a running member that is not the whole pass, with an answered step
/// behind it and an unrun step in front of it.
const ORDERED_CODING: &str = "- name: coding\n  members: [coder, reviewer, finisher]\n  \
                              flow: sequential\n  working_tree: exclusive\n\
                              - name: walked\n  members: [coder, reviewer, finisher]\n  \
                              working_tree: exclusive\n  steps:\n    \
                              - id: build\n      target: coder\n      next: check\n    \
                              - id: check\n      target: reviewer\n      next: finish\n    \
                              - id: finish\n      target: finisher\n\
                              - name: checked\n  members: [reviewer]\n  \
                              working_tree: exclusive\n";

/// A chat workspace that is ALSO a git repository on `release/1.2` — never `main`, so a park that
/// hardcoded the base would show — with two personas that each want the working copy to
/// themselves, one that never takes it, and the ordered channel of [`ORDERED_CODING`].
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
    // A persona that does NOT declare `working_tree: exclusive`: it runs beside whoever holds the
    // copy and never takes one, so withdrawing it has nothing to park.
    std::fs::write(
        roles.join("scribe.yaml"),
        "handle: scribe\njob_title: Writer\nsystem_prompt: You are scribe.\n",
    )
    .unwrap();
    // The other two steps of `coding`. The claim is the CHANNEL's (`working_tree: exclusive` on the
    // declaration), so these two take none of their own.
    for handle in ["reviewer", "finisher"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\njob_title: Reader\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    // An agent that commissions rounds of its own — the requester of a NESTED round, which is the
    // one a withdrawal of the operation's first thread must not wake (nxf 6j6v.s2cj).
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\njob_title: Planner\nsystem_prompt: You are pm.\n",
    )
    .unwrap();
    std::fs::write(roles.join("channels.yaml"), ORDERED_CODING).unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", "release/1.2"]);
    tmp
}

struct Bench {
    tmp: TempDir,
    worker: Arc<StoppableWorker>,
    /// `true` hands the engine a [`CannotStopWorker`] over the same inner worker.
    cannot_stop: bool,
    /// Keeps a working copy that is NOT the workspace root alive for the bench's lifetime.
    _elsewhere: Option<TempDir>,
    timer: RecordingTimer,
}

impl Bench {
    fn new() -> Self {
        Self::with(true)
    }

    /// A bench whose worker delivers the stop but leaves the process THERE until
    /// [`StoppableWorker::mark_gone`] says otherwise.
    fn whose_stop_is_only_delivered() -> Self {
        Self::with(false)
    }

    /// A bench whose worker cannot stop a session at all.
    fn that_cannot_stop() -> Self {
        let mut b = Self::with(true);
        b.cannot_stop = true;
        b
    }

    /// **The runtime names a directory, and that directory is not a git repository** — one of the
    /// three PERMANENT park refusals, and the shape every workspace in
    /// `working_tree_two_process_e2e.rs` has.
    fn with_a_working_copy_that_is_not_a_repository() -> Self {
        let elsewhere = TempDir::new().unwrap();
        let mut b = Self::with(true);
        Arc::get_mut(&mut b.worker)
            .expect("the worker is not shared yet")
            .working_copy = Some(elsewhere.path().to_path_buf());
        b._elsewhere = Some(elsewhere);
        b
    }

    /// **The runtime names no directory at all** — the DEFAULT of every worker but the sidecar,
    /// and the second permanent refusal.
    fn without_a_working_copy() -> Self {
        let mut b = Self::with(true);
        Arc::get_mut(&mut b.worker)
            .expect("the worker is not shared yet")
            .working_copy = None;
        b
    }

    fn with(stop_marks_gone: bool) -> Self {
        let tmp = workspace();
        let worker = Arc::new(StoppableWorker {
            log: tmp.path().join("dry.log"),
            working_copy: Some(tmp.path().to_path_buf()),
            running: Mutex::default(),
            stopped: Mutex::default(),
            stop_marks_gone,
            later_stops: Mutex::default(),
            refuse_a_live_spawn: Mutex::default(),
            look_at_on_stop: Mutex::default(),
            seen_on_stop: Mutex::default(),
        });
        Bench {
            tmp,
            worker,
            cannot_stop: false,
            _elsewhere: None,
            timer: RecordingTimer::default(),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn worker_config(&self) -> WorkerConfig {
        if self.cannot_stop {
            WorkerConfig::Custom(Arc::new(CannotStopWorker(self.worker.clone())))
        } else {
            WorkerConfig::Custom(self.worker.clone())
        }
    }

    /// A fresh handle per call, dropped after it — each verb opens the database cleanly, exactly as
    /// the CLI's processes do.
    fn engine(&self) -> Engine {
        Engine::open_with(
            None,
            self.root(),
            EngineConfig {
                worker: self.worker_config(),
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

    /// Run `f` with a fully built [`Ctx`] — [`RecordingTimer`] on it, so what a verb ARMS can be
    /// read back — and an open store, then drop both. The origin is the WORKSPACE's own, exactly
    /// as [`Engine`] resolves it, so a verb driven here reads the boards the handle wrote.
    fn with_ctx<T>(&self, now: &str, f: impl FnOnce(&Ctx, &mut ChatStore) -> T) -> T {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let origin = nexus_chat::workspace::origin_of(&ws).to_string();
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker: &dyn Worker = &*self.worker;
        let ctx = Ctx {
            now,
            origin: &origin,
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &defs,
            worker,
            timer: &self.timer,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        };
        f(&ctx, &mut store)
    }

    fn human<'a>(&self, now: &'a str) -> Caller<'a> {
        Caller {
            session: None,
            actor: Some("carsten"),
            now: Some(now),
        }
    }

    /// The human commissions a persona. Returns the thread it opened.
    fn commission(&self, now: &str, to: &str) -> String {
        self.engine()
            .send_to(
                self.human(now),
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

    /// **The seam under test**: `Engine::withdraw`.
    fn withdraw(&self, now: &str, thread: &str) -> nexus_chat::error::Result<WithdrawReceipt> {
        self.engine().withdraw(self.human(now), thread)
    }

    /// The same verb through the compute layer with the [`RecordingTimer`] — for the one question
    /// the handle cannot answer: what the withdrawal ARMED.
    fn withdraw_recording_the_timer(&self, now: &str, thread: &str) -> WithdrawReceipt {
        self.with_ctx(now, |ctx, store| {
            orchestration::withdraw(ctx, store, thread).expect("withdraw")
        })
    }

    /// The tick the background service runs — through the compute layer, because it is not a verb
    /// on the handle; with [`RecordingTimer`], so what it arms can be read back.
    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with_ctx(now, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread }).expect("tick")
        })
    }

    /// A persona answers into its thread — the CLI's own door, `Engine::reply_thread`.
    fn reply(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        thread: &str,
        body: &str,
    ) -> orchestration::ReplyReceipt {
        self.engine()
            .reply_thread(
                Caller {
                    session,
                    actor: Some(actor),
                    now: Some(now),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("the reply is posted")
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

    /// The lease ROW's holder, whether or not its bound has run out.
    fn holder(&self) -> Option<String> {
        self.store()
            .working_tree_lease_row()
            .expect("lease row")
            .map(|(k, _)| k)
    }

    fn expects_of(&self, now: &str, thread: &str) -> Vec<String> {
        self.store()
            .thread_quorum(thread, now)
            .expect("quorum reads")
            .expect("the thread has a board")
            .expects
    }

    fn owes_nothing(&self, now: &str, thread: &str) -> bool {
        let q = self
            .store()
            .thread_quorum(thread, now)
            .expect("quorum reads")
            .expect("the thread has a board");
        q.complete && q.outstanding.is_empty()
    }

    fn parked_rows(&self, scope: &str) -> Vec<nexus_chat::park::ParkedWork> {
        self.store().parked_work(scope).expect("parked work reads")
    }

    fn dirty_the_tree(&self) {
        std::fs::write(self.root().join("src.txt"), HALF_FINISHED).unwrap();
        std::fs::write(self.root().join("notes.md"), NOTES).unwrap();
    }

    /// The operation rooted at `root` on the DEFAULT `nxc status` listing — absent there is a
    /// failure of its own, which is why this panics rather than answering `None`.
    fn operation(&self, now: &str, root: &str) -> nexus_chat::facade::StatusOperation {
        self.engine()
            .status(now, StatusScope::Workspace)
            .expect("status reads")
            .operations
            .iter()
            .find(|op| op.root == root)
            .cloned()
            .unwrap_or_else(|| panic!("operation {root} is not on the default listing"))
    }

    /// Put the tree in the middle of a merge — `a_refused_park_follows_the_rule.rs`'s technique and
    /// its reason: `MERGE_HEAD` is the marker `WorkingCopy::mid_sequence` asks git for, and writing
    /// it rather than provoking a real conflict keeps the operation's uncommitted work exactly where
    /// it is.
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

/// The reserved identity a withdrawn thread is discharged onto, under whatever origin the
/// workspace mints.
const WITHDRAWN_HANDLE: &str = "/__withdrawn__";

/// The coder takes the copy and is WRITING (its session is running), leaves work in the tree, and
/// `solo` queues behind it. Returns `(coder's thread, coder's session, solo's thread)`.
fn a_running_holder_with_a_rival(b: &Bench) -> (String, String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    b.worker.mark_running(&session);
    b.dirty_the_tree();
    let solo = b.commission(QUEUED, "solo");
    assert!(!b.started("solo"), "the rival is queued, not started");
    assert_eq!(b.holder(), Some(scope_of(&thread)));
    (thread, session, solo)
}

/// **Hang `thread` under `parent`, by hand** — the one shape that puts a QUEUED commission inside
/// the claim area that HOLDS the working copy, and the reason it has to be built rather than
/// commissioned is written on the test that uses it.
///
/// It re-opens the thread with its own recorded root fields and one changed edge, which is exactly
/// what `ChatStore::open_thread`'s doc calls a hand-built tree: the `thread`/`open` op folds
/// `parent=excluded.parent`, so the later op is the edge that stands.
fn hang_under(b: &Bench, thread: &str, parent: &str, created: &str) {
    let ws = Workspace::resolve(None, b.root()).expect("resolve workspace");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let root = nexus_chat::model::ThreadRoot {
        origin,
        channel_id: store
            .thread_channel(thread)
            .expect("the thread was opened in a channel"),
        // Read back and written back unchanged: the discharge below re-declares what the thread
        // expects AS ITS OPENER, so an opener this rewrote would turn the withdrawal into a
        // `forbidden` rather than into the shape under test.
        opener: store
            .thread_opener(thread)
            .expect("opener reads")
            .expect("the thread has an opener"),
        created: created.to_string(),
        parent: Some(parent.to_string()),
    };
    store.open_thread(thread, &root, "carsten");
}

// ---- the run ------------------------------------------------------------------------------------

/// **The whole of the item, in one run**: the running round is stopped and discharged, the tick
/// parks its work and hands the copy on, and the work comes back through the next commission into
/// the same thread.
#[test]
fn withdrawing_a_running_round_stops_its_session_and_parks_its_work() {
    let b = Bench::new();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);
    let base_commit = git(b.root(), &["rev-parse", "HEAD"]);

    let receipt = b
        .withdraw(WITHDRAWN, &thread)
        .expect("a running round is withdrawn");

    // ---- the receipt ---------------------------------------------------------------------------
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].thread, thread);
    assert_eq!(receipt.stopped[0].role, "coder");
    assert_eq!(receipt.stopped[0].session, session);
    assert!(
        receipt.will_park,
        "the withdrawn round holds the working copy, so its work WILL be parked: {receipt:?}"
    );
    assert!(
        receipt.withdrawn.is_empty() && receipt.started_meanwhile.is_empty(),
        "nothing was queued under this thread — the running case reports on `stopped`: {receipt:?}"
    );
    assert!(receipt.warnings.is_empty(), "{:?}", receipt.warnings);
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone()],
        "the worker was asked to stop exactly that session"
    );

    // ---- the thread no longer owes an answer ---------------------------------------------------
    assert!(
        b.owes_nothing(WITHDRAWN, &thread),
        "the debt is discharged the way a withdrawn queued commission's is"
    );
    let expects = b.expects_of(WITHDRAWN, &thread);
    assert!(
        expects.len() == 1 && expects[0].ends_with(WITHDRAWN_HANDLE),
        "the register names the withdrawn identity: {expects:?}"
    );

    // ---- nothing has moved YET: the copy is still the coder's, and the tree is untouched --------
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the withdrawal itself hands nothing on: the park is the tick's, once nothing is running"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
    assert!(!b.started("solo"));

    // ---- the tick: parked, tree back on its base, rival started --------------------------------
    let ticked = b.tick(TICK, &solo);
    let handed = ticked
        .handed_on
        .expect("the tick handed the withdrawn holder's copy on");
    assert_eq!(handed.scope, scope);
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.clone().expect("and parked its work first");
    assert!(parked.committed && parked.created_branch, "{parked:?}");
    assert!(
        handed.unparked.is_none() && ticked.warnings.is_empty(),
        "{:?}",
        ticked.warnings
    );
    assert_eq!(
        handed
            .promotions
            .started
            .iter()
            .map(|p| p.role.as_str())
            .collect::<Vec<_>>(),
        vec!["solo"],
        "whoever was waiting starts: {:?}",
        handed.promotions
    );
    assert!(b.started("solo"));
    assert_eq!(b.holder().as_deref(), Some(scope_of(&solo).as_str()));

    let rows = b.parked_rows(&scope);
    assert_eq!(
        rows.len(),
        1,
        "the way back is recorded against the operation"
    );
    assert_eq!(rows[0].parked.branch, parked.branch);
    assert_eq!(rows[0].parked.base_branch, "release/1.2");
    assert_eq!(rows[0].parked.base_commit, base_commit);

    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\n",
        "the tree is back on its base for the rival"
    );
    assert!(!b.root().join("notes.md").exists());

    // `nxc status` names the parked work under the operation until somebody comes back for it.
    let status = b
        .engine()
        .status(TICK, StatusScope::Workspace)
        .expect("status reads");
    let op = status
        .operations
        .iter()
        .find(|op| op.root == thread)
        .expect(
            "the withdrawn operation is still listed — it has parked work nobody came back for",
        );
    assert_eq!(op.parked.len(), 1, "{op:?}");
    assert_eq!(op.parked[0].parked.branch, parked.branch);

    // ---- the way back: the rival finishes, and the next commission into the SAME thread --------
    let solo_session = b.session_of("solo");
    b.reply(COME_BACK, "solo", Some(&solo_session), &solo, "done");
    assert!(
        b.holder().is_none(),
        "a finished operation gives the copy back"
    );

    let before = b.triggers().len();
    let follow_up = b.reply(
        COME_BACK,
        "carsten",
        None,
        &thread,
        "carry on where you were",
    );
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=coder "))
        .cloned()
        .unwrap_or_else(|| {
            panic!("the follow-up into the withdrawn thread resumed the coder: {follow_up:?}")
        });
    assert_eq!(
        head_branch(b.root()),
        parked.branch,
        "the tree is back on the park branch"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and the work is in it"
    );
    assert!(resumed.contains("PARKED WORK"), "{resumed}");
    assert!(resumed.contains(&parked.branch), "{resumed}");
    assert!(
        b.parked_rows(&scope).is_empty(),
        "the row is stamped as come back for"
    );
}

// ---- WHO may take a round back (nxf 6j6v.ezbr) -------------------------------------------------
//
// The owner's decision of 2026-09-20 (6j6v.s8gf): `withdraw` is a human's verb. A caller that is a
// session this workspace started may not run it at all — not the running case, not the queued one
// — and whoever does run it must be the one who sent the operation's FIRST `send`, and must name
// that thread. Each rule has its own case below, and each asserts that NOTHING changed.

/// A call made from inside a session this workspace minted — the shape every spawned persona's
/// in-session `nxc` has, since the sidecar stamps its session into the environment.
fn from_session<'a>(session: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(now),
    }
}

/// Nothing a refused withdrawal could have touched has moved: the holder still runs and still owes,
/// nobody was signalled, no marker stands, and `queued` is still waiting behind the copy.
fn nothing_was_taken_back(b: &Bench, thread: &str, queued: &str) {
    assert!(b.worker.stopped().is_empty(), "nobody was signalled");
    assert!(!b.owes_nothing(WITHDRAWN, thread), "the round still owes");
    assert!(!b.owes_nothing(WITHDRAWN, queued), "so does the queued one");
    assert!(
        b.store()
            .withdrawn_holder(&scope_of(thread))
            .expect("marker reads")
            .is_none(),
        "no marker was written"
    );
    assert!(
        b.store()
            .list_working_tree_queue()
            .expect("queue reads")
            .iter()
            .any(|q| q.thread.as_deref() == Some(queued)),
        "the queued commission is still in the queue"
    );
    assert_eq!(b.holder(), Some(scope_of(thread)));
}

/// **A session this workspace started may not withdraw — a running round or a queued commission,
/// its own round or anybody else's — and the refusal says what to do instead** (nxf 6j6v.ezbr).
///
/// Two agents try it: the round's OWN coder, and a scribe running beside it. Both are refused by
/// name, before anything changes, and each is pointed at escalation on the thread IT stands in —
/// which is the way up to whoever may decide. The refusal names the session and the role it was
/// started for, which is exactly what the session map records and so exactly what this gate can
/// back; it claims nothing about callers it cannot see.
#[test]
fn a_session_this_workspace_started_may_not_withdraw_a_running_round_or_a_queued_one() {
    let b = Bench::new();
    let (thread, coder, solo) = a_running_holder_with_a_rival(&b);
    let scribe_thread = b.commission(QUEUED, "scribe");
    let scribe = b.session_of("scribe");
    b.worker.mark_running(&scribe);

    for (session, role, own_thread) in [
        (&coder, "coder", &thread),
        (&scribe, "scribe", &scribe_thread),
    ] {
        for target in [&thread, &solo] {
            let err = b
                .engine()
                .withdraw(from_session(session, WITHDRAWN), target)
                .expect_err("a session this workspace started is refused");
            assert_eq!(err.kind, ErrorKind::Forbidden, "{}", err.msg);
            assert!(
                err.msg.contains(session.as_str()) && err.msg.contains(role),
                "it names the session and the role it was started for: {}",
                err.msg
            );
            assert!(
                err.msg
                    .contains(&format!("nxc reply --thread {own_thread} --escalate")),
                "and names escalation, on the caller's OWN thread, as the way to ask: {}",
                err.msg
            );
            assert!(err.msg.contains("nothing was withdrawn"), "{}", err.msg);
        }
    }
    nothing_was_taken_back(&b, &thread, &solo);
}

/// **Only the caller who opened the operation may take it back, and the refusal names who did**
/// (nxf 6j6v.ezbr). The caller carries no session — so the engine cannot tell it from a person —
/// and CAN read the round: it is a declared member of the channel the round runs in. Reading was
/// the whole bar until this item; what it did not do was send the round.
#[test]
fn a_caller_who_did_not_open_the_operation_is_refused_and_told_who_did() {
    let b = Bench::new();
    let channel_thread = b.commission(T0, "coding");
    let coder = b.session_of("coder");
    b.worker.mark_running(&coder);
    let solo = b.commission(QUEUED, "solo");

    let err = b
        .engine()
        .withdraw(
            Caller {
                session: None,
                actor: Some("finisher"),
                now: Some(WITHDRAWN),
            },
            &channel_thread,
        )
        .expect_err("somebody who did not send the round may not take it back");
    assert_eq!(err.kind, ErrorKind::Forbidden, "{}", err.msg);
    assert!(
        err.msg.contains("/finisher did not open") && err.msg.contains("/carsten"),
        "it names the caller and the one who DID open the operation: {}",
        err.msg
    );
    assert!(err.msg.contains("nothing was withdrawn"), "{}", err.msg);

    // Naming a MEMBER thread changes nothing about who is asking: the opener rule answers before
    // the root rule, so this caller is told who opened the operation, not which thread to name.
    let member = b
        .store()
        .session_thread(&coder)
        .expect("session map reads")
        .expect("the step stands in a thread");
    let err = b
        .engine()
        .withdraw(
            Caller {
                session: None,
                actor: Some("finisher"),
                now: Some(WITHDRAWN),
            },
            &member,
        )
        .expect_err("a member thread does not make a non-opener the opener");
    assert_eq!(err.kind, ErrorKind::Forbidden, "{}", err.msg);
    assert!(err.msg.contains("/carsten"), "{}", err.msg);
    nothing_was_taken_back(&b, &channel_thread, &solo);
}

/// **A session id this workspace never minted is not what refuses a caller** (nxf 6j6v.ezbr;
/// review of PR #483, Test Quality #7). The gate refuses what the session map can prove; an id it
/// has no row for — one from another workspace, a stale one — names no session of ours, exactly as
/// `Caller::resolve_actor` reads it, and the caller is judged by the rules after it: here the
/// person who opened the operation, whose withdrawal goes through.
#[test]
fn a_session_id_this_workspace_never_minted_does_not_refuse_the_opener() {
    let b = Bench::new();
    let (thread, session, _solo) = a_running_holder_with_a_rival(&b);
    let receipt = b
        .engine()
        .withdraw(
            Caller {
                session: Some("m-never-minted-here"),
                actor: Some("carsten"),
                now: Some(WITHDRAWN),
            },
            &thread,
        )
        .expect("the opener withdraws, whatever foreign id it carries");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, session);
}

/// **A withdrawal names the operation's FIRST thread — a member thread is refused and the root is
/// named instead** (nxf 6j6v.ezbr). The opener is the caller here, so the only thing wrong is
/// WHICH thread it named: a step of the channel round rather than the channel thread its `send`
/// handed back.
#[test]
fn naming_a_member_thread_is_refused_and_the_operations_first_thread_is_named_instead() {
    let b = Bench::new();
    let channel_thread = b.commission(T0, "coding");
    let coder = b.session_of("coder");
    b.worker.mark_running(&coder);
    let member = b
        .store()
        .session_thread(&coder)
        .expect("session map reads")
        .expect("the step stands in a thread");
    assert_ne!(
        member, channel_thread,
        "the premise: a member thread below the root"
    );
    let solo = b.commission(QUEUED, "solo");

    let err = b
        .withdraw(WITHDRAWN, &member)
        .expect_err("a member thread is not what a withdrawal names");
    assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    assert!(
        err.msg
            .contains(&format!("nxc withdraw --thread {channel_thread}")),
        "it names the thread to use instead: {}",
        err.msg
    );
    assert!(err.msg.contains("nothing was withdrawn"), "{}", err.msg);
    nothing_was_taken_back(&b, &channel_thread, &solo);

    // …and the thread it names is the one that works.
    let receipt = b
        .withdraw(WITHDRAWN, &channel_thread)
        .expect("the operation's first thread is withdrawn");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].thread, member);
}

/// **The whole call is refused, the QUEUED half included** (nxf 6j6v.b9nf).
///
/// The rival is hung UNDER the withdrawn thread (fix round 2 of this item's review, Test Quality
/// #6) and that is the premise the claim rests on. Commissioned as a separate ROOT, as it is
/// everywhere else in this file, a queued rival is not in the withdrawn thread's subtree at all —
/// so `withdraw` would never have looked at it, and "the queued rival was not withdrawn either"
/// held whether or not the refusal came before the queue loop. `hang_under`'s own doc says why this
/// shape has to be built rather than commissioned.
#[test]
fn a_host_that_cannot_stop_sessions_is_refused_and_nothing_changes() {
    let b = Bench::that_cannot_stop();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);
    hang_under(&b, &solo, &thread, QUEUED);
    assert_eq!(
        b.store().thread_subtree(&thread).expect("subtree reads"),
        vec![thread.clone(), solo.clone()],
        "the premise: the queued commission is INSIDE the area this call withdraws, so a refusal \
         that came after the queue loop would have taken it back"
    );
    let expects_before = b.expects_of(QUEUED, &thread);
    let queue_before = b.store().list_working_tree_queue().expect("queue").len();
    assert_eq!(queue_before, 1, "the premise: the rival is queued");

    let err = b
        .withdraw(WITHDRAWN, &thread)
        .expect_err("a host that cannot stop a session is refused");
    assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    assert!(
        err.msg.contains("cannot stop a running session")
            && err.msg.contains("nothing was withdrawn"),
        "the refusal names the capability and says nothing changed: {}",
        err.msg
    );

    // …and nothing changed: the expectation, the lease, the queue, the session, the tree.
    assert_eq!(b.expects_of(WITHDRAWN, &thread), expects_before);
    assert!(!b.owes_nothing(WITHDRAWN, &thread));
    assert_eq!(b.holder().as_deref(), Some(scope.as_str()));
    assert_eq!(
        b.store().list_working_tree_queue().expect("queue").len(),
        queue_before,
        "the WHOLE call is refused — the queued rival inside the area was not withdrawn either"
    );
    assert!(
        !b.owes_nothing(WITHDRAWN, &solo),
        "…and its thread still owes its answer"
    );
    assert!(b.worker.stopped().is_empty());
    assert!(b.worker.session_is_running(&session));
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "no marker: a later tick has nothing to park"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
}

/// **A host that cannot LOOK is refused by name, not told that nothing is running** (nxf
/// 6j6v.b9nf, fix round 3 of this item's review, Code Quality #7 (a)).
///
/// `withdraw`'s `not_found` says "no session there has a live process". That is a statement of fact,
/// and for a worker whose [`Worker::answers_liveness`] is `false` — a custom worker in front of a
/// runtime that cannot be asked, or any build on a platform with no liveness question — the code
/// never established it: the running list is empty because nobody looked. The by-name
/// `stops_sessions` refusal is then unreachable too, so a genuinely running round reads as "there is
/// nothing to withdraw", on the one surface a person uses to clear up a round they can see is going.
///
/// The round here IS running — the inner worker's set says so, and the tick would act on it — and
/// the worker the engine is handed simply cannot see that.
#[test]
fn a_host_that_cannot_answer_liveness_is_refused_by_name_rather_than_told_nothing_is_running() {
    /// A worker that starts sessions and cannot be asked about them: `session_is_running` and
    /// `answers_liveness` left on the trait DEFAULT, which is the shape every host that has not
    /// implemented the read presents.
    struct CannotLookWorker(Arc<StoppableWorker>);
    impl Worker for CannotLookWorker {
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.0.trigger(req)
        }
        fn working_copy(&self) -> Option<PathBuf> {
            self.0.working_copy()
        }
        fn stops_sessions(&self) -> bool {
            true
        }
    }
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    b.worker.mark_running(&session);
    let engine = Engine::open_with(
        None,
        b.root(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(CannotLookWorker(b.worker.clone()))),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let err = engine
        .withdraw(b.human(WITHDRAWN), &thread)
        .expect_err("a fact this host cannot establish is not asserted");
    assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    assert!(
        err.msg.contains(&session) && err.msg.contains("does not answer the liveness question"),
        "the refusal names the sessions it cannot answer for, and the capability that is missing: \
         {}",
        err.msg
    );
    assert!(
        !err.msg.contains("nothing to withdraw"),
        "and does NOT claim the round is over: {}",
        err.msg
    );
    assert!(
        !b.owes_nothing(WITHDRAWN, &thread),
        "nothing was withdrawn: the thread still owes its answer"
    );
}

/// The same worker with `answers_liveness` OVERRIDDEN to `true` — the shape a host that DOES
/// implement the liveness read presents, used to pin the OTHER half of the refusal the test above
/// covers: where [`CannotLookWorker`] cannot say and is refused BY NAME, this one LOOKS, finds
/// nothing, and is told the truth.
struct LooksAndFindsNothingWorker(Arc<StoppableWorker>);
impl Worker for LooksAndFindsNothingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.0.trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.0.session_is_running(internal_session)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        self.0.working_copy()
    }
    fn answers_liveness(&self) -> bool {
        true
    }
    fn stops_sessions(&self) -> bool {
        true
    }
}

/// **The `not_found` half [`CannotLookWorker`]'s sibling above does NOT get** (nxf 6j6v.b9nf): a
/// worker that DOES answer the liveness question and, having looked, finds no live process. That is
/// a FACT this host established, not a capability it lacks, so the refusal is allowed to say so
/// plainly — `not_found`, not `validation`.
///
/// This is the shape `embed_surface.rs`'s
/// `engine_withdraw_refuses_an_unknown_thread_and_refuses_by_name_a_host_that_cannot_answer_liveness`
/// and `withdraw_a_parked_commission.rs`'s
/// `a_thread_whose_only_session_the_dry_worker_cannot_answer_liveness_for_is_refused_by_name` both
/// point back to: with the dry worker (`answers_liveness` on the trait default, `false`) neither of
/// those files can reach `not_found` for a real thread any more, because both leave an unended
/// session in the area — so the "nothing is waiting or running" refusal needs a pin of its own,
/// here, where a worker that genuinely answers liveness is already on hand.
#[test]
fn a_host_that_answers_liveness_and_finds_nothing_running_is_a_not_found() {
    let b = Bench::new();
    let thread = b.commission(T0, "coder");
    // The session is never marked running: this worker LOOKS (`answers_liveness` is `true`) and its
    // look finds nothing, which is exactly the fact `not_found` gets to assert.
    let engine = Engine::open_with(
        None,
        b.root(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(LooksAndFindsNothingWorker(b.worker.clone()))),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let err = engine
        .withdraw(b.human(WITHDRAWN), &thread)
        .expect_err("the worker looked and found nothing: a fact, not a capability gap");
    assert_eq!(err.kind, ErrorKind::NotFound, "{}", err.msg);
    assert!(
        err.msg.contains("is waiting or running")
            && err.msg.contains("no session there has a live process"),
        "it states what was established, not a missing capability: {}",
        err.msg
    );
    assert!(
        !err.msg.contains("does not answer the liveness question"),
        "and does NOT claim this host cannot tell: {}",
        err.msg
    );
}

#[test]
fn withdrawing_a_running_round_that_holds_no_working_copy_parks_nothing() {
    let b = Bench::new();
    let thread = b.commission(T0, "scribe");
    let session = b.session_of("scribe");
    b.worker.mark_running(&session);
    assert!(
        b.holder().is_none(),
        "the premise: a shared persona takes no lease"
    );

    let receipt = b.withdraw(WITHDRAWN, &thread).expect("withdrawn");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, session);
    assert!(
        !receipt.will_park,
        "nothing of this round is in the checkout, so there is nothing to park: {receipt:?}"
    );
    assert!(b.owes_nothing(WITHDRAWN, &thread));
    assert!(
        b.store()
            .withdrawn_holder(&scope_of(&thread))
            .expect("marker reads")
            .is_none(),
        "no marker for a holder that holds nothing"
    );

    let ticked = b.tick(TICK, &thread);
    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none() && ticked.working_tree.is_none(),
        "the tick has nothing to do: {ticked:?}"
    );
    assert!(!b.store().any_parked_work().expect("parked work reads"));
}

#[test]
fn the_park_waits_until_the_stopped_process_is_gone() {
    let b = Bench::whose_stop_is_only_delivered();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);

    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert!(receipt.will_park, "{receipt:?}");
    assert_eq!(b.worker.stopped(), vec![session.clone()]);
    assert!(
        b.worker.session_is_running(&session),
        "the premise: the stop was DELIVERED and the process is still there"
    );
    assert_eq!(
        b.timer.last_for(&thread).as_deref(),
        Some(WITHDRAWN_PLUS_CADENCE),
        "the withdrawal arms a look at the liveness cadence: {:?}",
        b.timer.armed.lock().unwrap()
    );

    // ---- the first tick: the process is still there, so nothing is touched, and it re-arms ------
    let ticked = b.tick(TICK, &solo);
    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none() && ticked.working_tree.is_none(),
        "a `git checkout` under a live process is the collision this epic exists to prevent: \
         {ticked:?}"
    );
    assert_eq!(b.holder().as_deref(), Some(scope.as_str()));
    assert!(b.parked_rows(&scope).is_empty());
    assert!(!b.started("solo"));
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and the tree was not touched"
    );
    assert_eq!(
        b.timer.last_for(&thread).as_deref(),
        Some(TICK_PLUS_CADENCE),
        "re-armed at the liveness cadence: {:?}",
        b.timer.armed.lock().unwrap()
    );

    // ---- the process is gone: the next tick parks -----------------------------------------------
    b.worker.mark_gone(&session);
    let ticked = b.tick(LATER, &solo);
    let handed = ticked.handed_on.expect("now it parks and hands on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    assert!(handed.parked.is_some(), "{handed:?}");
    assert_eq!(b.parked_rows(&scope).len(), 1);
    assert!(b.started("solo"));
    assert_eq!(head_branch(b.root()), "release/1.2");
}

/// **`nxc status` names a pinned withdrawn holder and the sessions still pinning it** (Integrity
/// #2 of this item's review, fix round 3). Before this fix nothing on any surface said WHY a
/// discharged operation still held the working copy while its stopped session refused to leave:
/// `nxc status` carried no withdrawn field at all — the operation showed only as `holds working
/// tree` with its threads already answered — and "stop it by hand" was printed only when the
/// FIRST `stop_session` call had itself errored, never when the request was delivered and simply
/// ignored, which is the ordinary shape of a wedged sidecar or a custom worker whose stop does not
/// land.
///
/// This is the `--json` fact's test at the Engine seam
/// ([`nexus_chat::facade::StatusOperation::withdrawn`]); the human line has its own sibling below.
#[test]
fn nxc_status_names_a_pinned_withdrawn_holder_and_the_sessions_still_pinning_it() {
    let b = Bench::whose_stop_is_only_delivered();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);

    b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert!(
        b.worker.session_is_running(&session),
        "the premise: the stop was delivered and the process is still there"
    );

    // While the session pins the claim, the tick cannot park it — and `nxc status` says why.
    let ticked = b.tick(TICK, &solo);
    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none(),
        "the premise: nothing parked yet: {ticked:?}"
    );
    let marker = b
        .store()
        .withdrawn_holder(&scope)
        .expect("marker reads")
        .expect("the marker stands while the session pins the claim");
    let op = b.operation(TICK, &thread);
    let withdrawn = op
        .withdrawn
        .clone()
        .expect("the holding operation is marked withdrawn and waiting to park");
    assert_eq!(withdrawn.withdrawn_at, marker.withdrawn_at);
    assert_eq!(withdrawn.by, marker.by);
    assert_eq!(
        withdrawn.pinned_by,
        vec![session.clone()],
        "the session still pinning the claim is named, and only that one — `solo` never took \
         this lease: {withdrawn:?}"
    );

    // Once the process is really gone and the park has gone through, the mark goes with it.
    b.worker.mark_gone(&session);
    b.tick(LATER, &solo);
    assert!(
        b.operation(LATER, &thread).withdrawn.is_none(),
        "a parked, discharged holder is not withdrawn any more"
    );
}

/// **After five liveness cadences with a session the withdrawal stopped still pinning the claim, the
/// tick sends it ONE further SIGTERM, and names it** (nxf 6j6v.27b9 — the owner's decision of
/// 2026-09-20; nxf 6j6v.b9nf's Integrity #2 fix only named it). See
/// `WITHDRAWN_SESSION_WEDGED_AFTER_CADENCES` in `orchestration.rs` for why five and why once.
///
/// Below the threshold nothing but the silent re-arm fires. At it, the worker is asked to stop the
/// session a second time — through `Worker::stop_session`, the same door and the same identity
/// guard the first stop went through — and the tick's warning carries the session, its pid, its pid
/// file, and what was sent. After it, the session is named again and NEVER signalled a third time:
/// the further request is stamped, so a tick that comes again finds it already sent.
#[test]
fn a_session_still_pinning_after_five_cadences_gets_one_further_sigterm_and_is_named() {
    let b = Bench::whose_stop_is_only_delivered();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);

    b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert!(
        b.worker.session_is_running(&session),
        "the premise: the stop was delivered and the process is still there"
    );
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone()],
        "withdraw's one stop"
    );

    // A real pid file — this test double never writes one on its own — so the diagnostic has
    // something to read, exactly the shape `SessionLock` writes (nxf 6j6v.b9nf, fix round 3).
    let logs = b.root().join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join(format!("{session}.pid")),
        "4123\n2026-09-19T09:00:00Z\n",
    )
    .unwrap();

    // Below the threshold (four cadences): nothing names the session yet, and nothing is re-sent.
    let ticked = b.tick("2026-09-19T09:14:00Z", &solo);
    assert!(
        !ticked
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WithdrawnHolderWedged),
        "four cadences is not yet the threshold: {:?}",
        ticked.warnings
    );
    assert_eq!(b.worker.stopped(), vec![session.clone()]);

    // At the threshold (five cadences): asked to stop ONE more time, and named.
    let ticked = b.tick("2026-09-19T09:15:00Z", &solo);
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone(), session.clone()],
        "the tick asked the worker to stop that session once more"
    );
    let finding = ticked
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WithdrawnHolderWedged)
        .unwrap_or_else(|| panic!("the wedged session is named: {:?}", ticked.warnings));
    assert_eq!(finding.session.as_deref(), Some(session.as_str()));
    assert_eq!(finding.thread.as_deref(), Some(thread.as_str()));
    assert!(finding.detail.contains(&session), "{}", finding.detail);
    assert!(
        finding.detail.contains("4123"),
        "the pid: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains(".pid"),
        "the pid file's path: {}",
        finding.detail
    );
    assert!(
        finding
            .detail
            .contains("one further SIGTERM was sent just now"),
        "says what this tick sent: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("never SIGKILL"),
        "and what it will never send: {}",
        finding.detail
    );

    // A later tick: named again, and never signalled a third time.
    let ticked = b.tick("2026-09-19T09:20:00Z", &solo);
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone(), session.clone()],
        "at most ONE further SIGTERM per withdrawal"
    );
    let finding = ticked
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WithdrawnHolderWedged)
        .unwrap_or_else(|| panic!("still named: {:?}", ticked.warnings));
    assert!(
        finding.detail.contains("at 2026-09-19T09:15:00Z"),
        "names when the further SIGTERM went: {}",
        finding.detail
    );
}

/// A withdrawn holder whose stop was only DELIVERED, with the pid file `SessionLock` would have
/// written, taken back at [`WITHDRAWN`] — the premise every case about the further SIGTERM shares.
/// Returns `(thread, session, solo)`.
fn a_withdrawn_holder_still_pinning(b: &Bench) -> (String, String, String) {
    let (thread, session, solo) = a_running_holder_with_a_rival(b);
    b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert!(
        b.worker.session_is_running(&session),
        "the premise: still there"
    );
    let logs = b.root().join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join(format!("{session}.pid")),
        "4123\n2026-09-19T09:00:00Z\n",
    )
    .unwrap();
    (thread, session, solo)
}

fn wedged_detail(ticked: &TickReceipt) -> String {
    ticked
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WithdrawnHolderWedged)
        .unwrap_or_else(|| panic!("the wedged session is named: {:?}", ticked.warnings))
        .detail
        .clone()
}

/// **A further SIGTERM that does not land is never called "sent", and never sent again** (nxf
/// 6j6v.27b9; review of PR #483, Integrity #3 and Test Quality #2). The worker refuses the second
/// signal — a claim it cannot prove, a signal the system refused. The tick that tried says so in
/// the worker's words; every later tick says it was ATTEMPTED at that instant and did not land;
/// and no tick tries again.
#[test]
fn a_further_sigterm_that_does_not_land_is_never_called_sent_nor_repeated() {
    let b = Bench::whose_stop_is_only_delivered();
    *b.worker.later_stops.lock().unwrap() =
        Some(Err("the operating system refused the signal".to_string()));
    let (_thread, session, solo) = a_withdrawn_holder_still_pinning(&b);

    let detail = wedged_detail(&b.tick("2026-09-19T09:15:00Z", &solo));
    assert!(
        detail.contains(
            "one further SIGTERM was attempted just now and did not land (the operating system \
             refused the signal)"
        ),
        "{detail}"
    );
    for later in ["2026-09-19T09:20:00Z", "2026-09-19T09:25:00Z"] {
        let detail = wedged_detail(&b.tick(later, &solo));
        assert!(
            detail.contains(
                "one further SIGTERM was attempted at 2026-09-19T09:15:00Z and did not land"
            ),
            "{detail}"
        );
        assert!(!detail.contains("was sent"), "never called sent: {detail}");
    }
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone(), session],
        "withdraw's stop and ONE further attempt — a refused one is not retried"
    );
}

/// **A further SIGTERM that finds nothing to stop names nothing** (nxf 6j6v.27b9; review of PR
/// #483, Test Quality #2). The process went between the liveness read and the signal — the state
/// that was wanted — so the tick names no wedged session, and none of the ticks after it signals
/// again. (This double keeps answering "running" afterwards, which a real process that is gone
/// would not; it is what lets the no-repeat half be seen at all.)
#[test]
fn a_further_sigterm_that_finds_nothing_to_stop_names_no_wedged_session() {
    let b = Bench::whose_stop_is_only_delivered();
    *b.worker.later_stops.lock().unwrap() = Some(Ok(SessionStop::NothingToStop(
        "its pid is gone".to_string(),
    )));
    let (_thread, session, solo) = a_withdrawn_holder_still_pinning(&b);

    let ticked = b.tick("2026-09-19T09:15:00Z", &solo);
    assert!(
        !ticked
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WithdrawnHolderWedged),
        "nothing to stop is not a wedged process: {:?}",
        ticked.warnings
    );
    let later = b.tick("2026-09-19T09:20:00Z", &solo);
    assert_eq!(b.worker.stopped(), vec![session.clone(), session]);
    assert!(
        !later.warnings.iter().any(|w| w.detail.contains("was sent")),
        "nothing was signalled, so nothing is ever called sent: {:?}",
        later.warnings
    );
}

/// **A follow-up the worker REFUSES leaves the stopped process answerable** (nxf 6j6v.27b9; review
/// of PR #483, Integrity #1). The human nudges the withdrawn thread while the stopped process is
/// still there; the shipped worker refuses a second process for a session that has one
/// (`AlreadyRunning`), so nothing starts — and the process the withdrawal stopped is still the one
/// pinning the claim. Forgotten at the funnel before the spawn, as it first was, the refused
/// follow-up disarmed the further SIGTERM and the warning for exactly the process that was ignoring
/// the first signal.
#[test]
fn a_follow_up_refused_while_the_stopped_process_still_runs_keeps_it_answerable() {
    let b = Bench::whose_stop_is_only_delivered();
    *b.worker.refuse_a_live_spawn.lock().unwrap() = true;
    let (thread, session, solo) = a_withdrawn_holder_still_pinning(&b);

    let before = b.triggers().len();
    b.reply(TICK, "carsten", None, &thread, "actually, carry on");
    assert_eq!(
        b.triggers().len(),
        before,
        "the premise: the worker refused the spawn, nothing started"
    );

    let detail = wedged_detail(&b.tick("2026-09-19T09:15:00Z", &solo));
    assert!(
        detail.contains("one further SIGTERM was sent just now"),
        "{detail}"
    );
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone(), session],
        "the process the withdrawal stopped is still the one asked again"
    );
}

/// **A session put back in motion is never signalled again, nor named** (nxf 6j6v.27b9; moved here
/// from the follow-up case below, whose name did not say it pinned this — review of PR #483, Test
/// Quality #8). The stopped process is gone, the human's follow-up resumes the SAME session in a
/// new process, and that process runs past five cadences: it is the work somebody just asked for.
#[test]
fn a_session_put_back_in_motion_is_never_signalled_again_nor_named() {
    let b = Bench::new();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    b.withdraw(WITHDRAWN, &thread).expect("withdrawn");
    assert!(
        !b.worker.session_is_running(&session),
        "the premise: the stop landed"
    );
    b.reply(TICK, "carsten", None, &thread, "actually, carry on");
    assert!(
        b.triggers()
            .iter()
            .filter(|l| l.contains(&format!("session={session}")))
            .count()
            == 2,
        "the premise: the follow-up resumed the same session: {:?}",
        b.triggers()
    );
    b.worker.mark_running(&session);

    let ticked = b.tick("2026-09-19T09:16:00Z", &solo);
    assert_eq!(
        b.worker.stopped(),
        vec![session.clone()],
        "only withdraw's own stop, never a further one at a resumed session"
    );
    assert!(
        !ticked
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WithdrawnHolderWedged),
        "a resumed session is not a wedged one: {:?}",
        ticked.warnings
    );
}

#[test]
fn nothing_on_the_withdraw_path_rolls_work_back() {
    let b = Bench::new();
    let (thread, _session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);

    b.withdraw(WITHDRAWN, &thread).expect("withdrawn");
    let ticked = b.tick(TICK, &solo);
    let parked = ticked.handed_on.and_then(|h| h.parked).expect("parked");

    // The uncommitted work — the tracked change AND the untracked file — is on the park branch,
    // byte for byte. Nothing was reset, checked out over, or cleaned away.
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim_end()
    );
    assert_eq!(
        git(b.root(), &["show", &format!("{}:notes.md", parked.branch)]),
        NOTES.trim_end()
    );
    let rows = b.parked_rows(&scope);
    assert_eq!(
        rows[0].parked.commit,
        git(b.root(), &["rev-parse", &parked.branch])
    );
}

/// **The two teardown doors, and what each half of this case actually covers** (nxf 6j6v.b9nf; the
/// second paragraph is fix round 2 of this item's review, Test Quality #7).
///
/// * The `session ended` half pins a SHAPE, not a guard, and says so rather than implying one: on a
///   DIRECT persona thread that verb returns `not_a_channel_step` before it reaches any release at
///   all, so the holder assertion under it would hold with the marker deleted. What it is worth is
///   that it would stop holding the day this path grew a release — which is exactly what the
///   receipt's own reason is asserted for here. The half of that door that CAN fail is a supervised
///   step, and it has its own cases:
///   `the_end_of_a_withdrawn_step_of_a_declared_state_machine_does_not_route_the_next_one_either`.
/// * The late-REPLY half is the one that carries the guard here. It ends in the normal
///   end-of-operation release, and the discharged area owes nothing — so without the marker this is
///   precisely the path that hands the copy on with the work still in the tree.
#[test]
fn a_withdrawn_holder_is_not_released_unparked_by_its_own_teardown() {
    let b = Bench::new();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);

    b.withdraw(WITHDRAWN, &thread).expect("withdrawn");

    // ---- the sidecar's teardown announces the session's end -----------------------------------
    // The stopped sidecar still runs `nxc session ended` on its way out (nxf 6j6v.b9nf).
    let ended = b
        .engine()
        .session_ended(b.human(WITHDRAWN), &session)
        .expect("the announcement is taken");
    assert_eq!(
        ended.reason, "not_a_channel_step",
        "what this half covers, stated: a direct persona thread's end announcement returns here \
         before any release, so what is pinned is that shape — the supervised half is pinned next \
         door: {ended:?}"
    );
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the withdrawn holder keeps the copy until the tick parks it"
    );
    assert!(b.parked_rows(&scope).is_empty());
    assert!(!b.started("solo"));

    // ---- a late reply on the discharged thread ------------------------------------------------
    // The reply path ends in the normal end-of-operation release, and the discharged area owes
    // nothing — so without the marker this is exactly the path that would hand the copy on with
    // the work still in the tree.
    b.reply(
        WITHDRAWN,
        "coder",
        Some(&session),
        &thread,
        "one last word before I go",
    );
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the marker wins over the reply path's release"
    );
    assert!(b.parked_rows(&scope).is_empty());
    assert!(
        !b.started("solo"),
        "the rival did not start into the coder's uncommitted work"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // ---- the tick is the one way such a holder lets go ----------------------------------------
    let ticked = b.tick(TICK, &solo);
    let handed = ticked.handed_on.expect("the tick parks and hands on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    assert!(handed.parked.is_some());
    assert!(b.started("solo"));
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "the marker goes with the copy"
    );
}

/// **One call takes back a QUEUED commission and the RUNNING round that holds the copy — and the
/// marker stands before either of them is discharged** (fix round 2 of this item's review,
/// Important #1).
///
/// The invariant the marker exists for is written on [`orchestration::withdraw`]: there must be no
/// instant at which the claim area owes NOTHING and the guard is not there. The stopped session is
/// a separate PROCESS, and its in-flight `nxc reply` can land in any window this call leaves open —
/// that reply ends in `release_working_tree_if_scope_is_done`, which hands the copy on for exactly
/// that state. Until this round the marker was written BETWEEN the queued loop and the running one,
/// so the queued discharge — the write that stops the area owing anything once the running thread's
/// own reply has landed — happened with nothing guarding the release.
///
/// **What this test pins, and what it cannot.** The race needs two processes interleaved inside one
/// call, and nothing between the queued discharge and the marker write calls out to anything a test
/// can hook, so it cannot be made to happen here. What is pinned instead is the ORDER, which is the
/// property the comment claims: at the instant the withdrawal arms its look — the first thing that
/// happens after the marker is written — a second connection to the same database sees the marker
/// standing and NOTHING discharged yet. Move the marker write back below the queued loop and the
/// queued thread is already discharged at that instant, and this fails.
///
/// **The tree is hand-built, and that is a finding of its own** (reported with this round): a queued
/// commission inside the claim area that holds the copy cannot be COMMISSIONED today. The claim key
/// is the thread's root (`lease_claim_root`), so work commissioned inside a held area asks for the
/// very same key, which `ChatStore::acquire_working_tree` grants as an inherit — it never queues.
/// `inherits_the_held_claim`'s own doc names the two shapes that produce this one anyway, "a
/// re-parenting or a hand-built tree", and [`hang_under`] builds the second. The ordering rule is
/// written for the shape rather than for its reachability: a guard deleted because the state it
/// catches is unreachable is how the state comes back.
#[test]
fn a_queued_commission_and_a_running_round_in_one_area_are_withdrawn_behind_one_marker() {
    let b = Bench::new();
    let (thread, session, rival) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);
    hang_under(&b, &rival, &thread, QUEUED);
    assert_eq!(
        b.store().thread_subtree(&thread).expect("subtree reads"),
        vec![thread.clone(), rival.clone()],
        "the premise: ONE claim area holding the running round AND the queued commission"
    );
    b.timer
        .watch(b.root(), &scope, &[thread.clone(), rival.clone()]);

    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert_eq!(
        receipt.withdrawn.len(),
        1,
        "the queued commission was taken back too: {receipt:?}"
    );
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert!(receipt.will_park, "{receipt:?}");

    // ---- the ORDER: the guard was there before the first discharge ------------------------------
    let seen = b.timer.watched();
    let look = seen
        .iter()
        .find(|w| w.key == thread)
        .unwrap_or_else(|| panic!("the withdrawal arms a look at the holder: {seen:?}"));
    assert!(
        look.marker,
        "the marker stands by the time anything else in this call has run: {look:?}"
    );
    assert!(
        look.discharged.is_empty(),
        "and it stands BEFORE the first discharge, the QUEUED one included — these were already \
         discharged with no guard standing: {:?}",
        look.discharged
    );

    // ---- both halves are discharged, and the area now owes nothing -------------------------------
    for taken in [&thread, &rival] {
        assert!(
            b.owes_nothing(WITHDRAWN, taken),
            "thread {taken} was taken back, so it owes nothing"
        );
        assert!(
            b.expects_of(WITHDRAWN, taken)
                .iter()
                .any(|h| h.ends_with(WITHDRAWN_HANDLE)),
            "…and says so in the register: {:?}",
            b.expects_of(WITHDRAWN, taken)
        );
    }

    // ---- a release attempted in that window does NOT hand the copy on ---------------------------
    // The stopped session's own last word, through the reply path, which ends in the normal
    // end-of-operation release — the door the marker is written for.
    b.reply(
        WITHDRAWN,
        "coder",
        Some(&session),
        &thread,
        "one last word before I go",
    );
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the marker wins over the reply path's release"
    );
    assert!(b.parked_rows(&scope).is_empty());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and the half-finished work is still in the tree, unparked and untouched"
    );

    // ---- the tick is the one way such a holder lets go ------------------------------------------
    let ticked = b.tick(TICK, &rival);
    let handed = ticked.handed_on.expect("the tick parks and hands on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.expect("parked before the copy went on");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim_end(),
        "the work is on the branch, nothing rolled back"
    );
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "the marker goes with the copy"
    );
}

/// **A follow-up that arrives BEFORE the tick parked still ends with a park, never with an
/// unparked release.** The way back is the next commission into the same thread, and it can arrive
/// while the withdrawn marker still stands: the stopped session is then resumed straight into the
/// tree its work is still in, which is right. What the marker guarantees from there is the literal
/// rule (fix round 1 of this item's review): the withdrawal ends only when the copy goes on. The
/// resumed session's NORMAL end does not release — the guard holds — the tick parks once nothing
/// runs and hands the copy on, and the next commission restores the branch.
///
/// The rule it replaced dropped the marker for any session starting in the withdrawn claim, keyed
/// on the claim root — so a nested shared session resumed on its own, with no lease and nothing of
/// its own in the tree, ended the withdrawal of the exclusive holder above it, whose normal end then
/// handed the copy on with the half-finished work in it and nothing said. A park the round may not
/// have needed costs a branch; a release that was not guarded costs the work.
#[test]
fn a_follow_up_that_arrives_before_the_park_is_resumed_in_the_tree_and_still_parks_at_its_end() {
    let b = Bench::new();
    let (thread, session, solo) = a_running_holder_with_a_rival(&b);
    let scope = scope_of(&thread);

    b.withdraw(WITHDRAWN, &thread).expect("withdrawn");
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_some(),
        "the premise: the marker stands, nothing has been parked yet"
    );

    // The human changes their mind before the tick comes: the follow-up resumes the stopped
    // session, which INHERITS the copy it still holds — and the work is right there in the tree.
    let before = b.triggers().len();
    b.reply(TICK, "carsten", None, &thread, "actually, carry on");
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=coder "))
        .cloned()
        .expect("the follow-up resumed the coder");
    assert!(resumed.contains(&format!("session={session}")), "{resumed}");
    assert!(
        !resumed.contains("PARKED WORK"),
        "nothing was parked, so there is nothing to tell it about: {resumed}"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and it continues in the tree exactly as it left it"
    );
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_some(),
        "the marker STAYS: a session starting in the withdrawn claim does not end the withdrawal"
    );
    b.worker.mark_running(&session);

    // While it runs, the tick has nothing to do but look again.
    let ticked = b.tick(LATER, &solo);
    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none() && ticked.working_tree.is_none(),
        "{ticked:?}"
    );
    assert_eq!(b.holder().as_deref(), Some(scope.as_str()));
    assert!(!b.started("solo"));

    // …and the resumed round's NORMAL end does not release: the guard holds the copy, so the
    // rival does not start into the tree, and nothing has moved.
    b.reply(COME_BACK, "coder", Some(&session), &thread, "done");
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the normal end of a withdrawn holder's round does not hand the copy on"
    );
    assert!(!b.started("solo"));
    assert!(b.parked_rows(&scope).is_empty());

    // Once nothing runs there, the tick parks — the work goes on a branch, the tree back on its
    // base, the rival starts — and the marker goes with the copy.
    b.worker.mark_gone(&session);
    let ticked = b.tick(COME_BACK, &solo);
    let handed = ticked.handed_on.expect("the tick parks and hands on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.expect("parked first");
    assert!(b.started("solo"));
    assert_eq!(b.parked_rows(&scope).len(), 1);
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim_end(),
        "the work is on the branch, nothing rolled back"
    );
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "the one thing that ends a withdrawal: the copy went on"
    );
}

#[test]
fn a_stop_that_does_not_land_is_a_warning_on_the_receipt_and_the_debt_is_still_discharged() {
    /// A worker that says it stops sessions and then cannot — a pid file that went missing, a
    /// signal the operating system refused.
    struct RefusingStop(Arc<StoppableWorker>);
    impl Worker for RefusingStop {
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.0.trigger(req)
        }
        fn session_is_running(&self, internal_session: &str) -> bool {
            self.0.session_is_running(internal_session)
        }
        fn working_copy(&self) -> Option<PathBuf> {
            self.0.working_copy()
        }
        fn stops_sessions(&self) -> bool {
            true
        }
        fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
            Err(format!("stub: SIGTERM to {internal_session} was refused"))
        }
    }
    let b = Bench::new();
    let (thread, session, _solo) = a_running_holder_with_a_rival(&b);
    let engine = Engine::open_with(
        None,
        b.root(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(RefusingStop(b.worker.clone()))),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let receipt = engine
        .withdraw(b.human(WITHDRAWN), &thread)
        .expect("the call itself is not a failure: the debt is discharged");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.warnings.len(), 1, "{:?}", receipt.warnings);
    let w = &receipt.warnings[0];
    assert_eq!(w.class, ConsequenceClass::SessionNotStopped);
    assert_eq!(w.session.as_deref(), Some(session.as_str()));
    assert!(w.detail.contains("was refused"), "{}", w.detail);
    assert!(b.owes_nothing(WITHDRAWN, &thread));
}

/// **A session that was already over is a NOTE on the receipt, not a warning** (nxf 6j6v.b9nf, fix
/// round 3 of this item's review, Code Quality #6).
///
/// The withdrawal reads liveness, decides, and signals — three steps, and a round that was about to
/// finish can end in between. That is the ordinary race on this path and it lands on exactly the
/// state the caller asked for: nothing of the round is running any more. It used to be an `Err` from
/// the worker, which became a `session_not_stopped` warning saying "the process is still there;
/// stop it by hand" about a process that had gone — and `nxc withdraw` exited 1, because a warning
/// of that class changes the exit code.
///
/// So the receipt carries the worker's own sentence and `warnings` stays empty, which is what keeps
/// the exit code at 0 for the goal state: nothing was added to the class that turns it red.
#[test]
fn a_session_that_had_already_ended_is_a_note_on_the_receipt_rather_than_a_failure() {
    /// A worker whose stop finds nothing to do — the shape `SidecarWorker` reports for a pid that
    /// is gone, and for one the identity check proves belongs to somebody else now.
    struct AlreadyOver(Arc<StoppableWorker>);
    impl Worker for AlreadyOver {
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.0.trigger(req)
        }
        fn session_is_running(&self, internal_session: &str) -> bool {
            self.0.session_is_running(internal_session)
        }
        fn working_copy(&self) -> Option<PathBuf> {
            self.0.working_copy()
        }
        fn stops_sessions(&self) -> bool {
            true
        }
        fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
            Ok(SessionStop::NothingToStop(format!(
                "session {internal_session} has no live process — pid 4242 is gone, so there was \
                 nothing to stop"
            )))
        }
    }
    let b = Bench::new();
    let (thread, session, _solo) = a_running_holder_with_a_rival(&b);
    let engine = Engine::open_with(
        None,
        b.root(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(AlreadyOver(b.worker.clone()))),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let receipt = engine
        .withdraw(b.human(WITHDRAWN), &thread)
        .expect("the call is not a failure");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, session);
    assert!(
        receipt.stopped[0]
            .already_gone
            .as_deref()
            .is_some_and(|note| note.contains("nothing to stop")),
        "the receipt carries what the worker found: {receipt:?}"
    );
    assert!(
        receipt.warnings.is_empty(),
        "the state the call was asked for is not a failure, and a warning here is what made the \
         command exit 1 on it: {:?}",
        receipt.warnings
    );
    assert!(
        b.owes_nothing(WITHDRAWN, &thread),
        "and the withdrawal itself stands: the thread owes nothing"
    );
}

/// **A withdrawal that promises a park says when there is nothing to run the tick that does it**
/// (nxf 6j6v.0j12's class at a fourth verb; fix round 2 of this item's review, Integrity #4).
///
/// `send`, `reply` and `tick` all ask this and `withdraw` did not — and it is the verb with the
/// most riding on the answer. While the marker stands EVERY other release path declines by design,
/// so on a machine whose background service is down the copy is held by a round that is over, the
/// work sits unparked in the tree, and the receipt says "its work will be parked" to nobody.
///
/// The class is exempt from the exit code (`changes_the_exit_code`), so this is a sentence, not a
/// failure: the withdrawal itself stands.
#[test]
fn a_withdrawal_that_leaves_work_for_the_tick_says_when_nothing_will_run_one() {
    let b = Bench::new();
    *b.timer.unattended.lock().unwrap() = true;
    let (thread, _session, _rival) = a_running_holder_with_a_rival(&b);

    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);

    assert!(
        receipt.will_park,
        "the premise: a park is promised: {receipt:?}"
    );
    let finding = receipt
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::ServiceNotRunning)
        .unwrap_or_else(|| {
            panic!(
                "a promise about a tick, with nothing to run one: {:?}",
                receipt.warnings
            )
        });
    assert_eq!(finding.thread.as_deref(), Some(thread.as_str()));
}

/// …and a withdrawal that leaves NOTHING for the tick says nothing about the service. The class is
/// for a caller who was just promised something that needs one; a shared persona's round needs no
/// tick, and a line on every withdrawal is a line readers learn to skip.
#[test]
fn a_withdrawal_with_nothing_in_the_checkout_is_silent_about_the_service() {
    let b = Bench::new();
    *b.timer.unattended.lock().unwrap() = true;
    let thread = b.commission(T0, "scribe");
    let session = b.session_of("scribe");
    b.worker.mark_running(&session);
    assert!(b.holder().is_none(), "the premise: it takes no lease");

    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);

    assert!(
        !receipt.will_park && receipt.cannot_park.is_none(),
        "{receipt:?}"
    );
    assert!(
        !receipt
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::ServiceNotRunning),
        "nothing here waits on a tick: {:?}",
        receipt.warnings
    );
}

// ---- the withdrawn occasion's own REFUSAL paths (fix round 2 of this review, Test Quality #1) ----
//
// Owner decision 5 is that the withdrawn park follows the SAME refusal rule as every other occasion
// ([`park_and_hand_on`]): a transient refusal keeps the claim and the marker and is retried, a
// permanent one hands the copy on UNPARKED and loudly, and the marker goes with the copy. None of
// that had a test — `ParkOccasion::Withdrawn` appeared only on the five success-path assertions
// above, and deleting the one piece of retry logic this occasion owns left every test green.

/// The coder takes the copy and is WRITING, leaves work in the tree, and is withdrawn — with
/// **nobody queued behind it**, which is the whole point of the case that uses this. Returns
/// `(the thread, its session)`.
fn a_withdrawn_holder_nobody_is_waiting_for(b: &Bench) -> (String, String) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    b.worker.mark_running(&session);
    b.dirty_the_tree();
    assert_eq!(b.holder(), Some(scope_of(&thread)));
    assert!(
        b.store()
            .list_working_tree_queue()
            .expect("queue reads")
            .is_empty(),
        "the premise: NOBODY is waiting for this copy"
    );
    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert!(receipt.will_park, "{receipt:?}");
    assert_eq!(
        b.timer.last_for(&thread).as_deref(),
        Some(WITHDRAWN_PLUS_CADENCE),
        "the withdrawal armed its own look: {:?}",
        b.timer.armed.lock().unwrap()
    );
    (thread, session)
}

/// **(i) A TRANSIENT refusal keeps the holder, the marker and a clock of its own** (nxf 6j6v.b9nf,
/// fix round 2 of this item's review, Test Quality #1).
///
/// The tree is mid-merge when the tick comes, which is `ParkRefusal::MidSequence` — a person
/// finishes or aborts it and the next tick parks. The claim therefore stays, the marker stays
/// (nothing has gone on the copy, so the withdrawal has not ended), `nxc status` carries the refusal
/// stamped with THIS occasion, and a clock is armed to come back.
///
/// **That clock is the load-bearing part, and nobody is queued on purpose.** `park_and_hand_on`
/// arms the retry through `arm_the_park_retry`, which is keyed on the queue HEAD and answers `None`
/// when there is none — right for every other occasion, because a copy nobody waits for is never
/// handed on from the tick. A withdrawn round has no contention requirement: it is parked whether or
/// not anybody is waiting. So the occasion arms the holder's own look beside it, and with an empty
/// queue that look is the only thing that will ever come back for this park. Delete that block and
/// the last armed instant for this key stays the withdrawal's own.
#[test]
fn a_withdrawn_park_refused_for_now_keeps_the_holder_the_marker_and_arms_its_own_look() {
    let b = Bench::new();
    let (thread, _session) = a_withdrawn_holder_nobody_is_waiting_for(&b);
    let scope = scope_of(&thread);
    b.start_a_merge();

    let ticked = b.tick(TICK, &thread);

    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none(),
        "nothing was parked and nothing was handed on: {ticked:?}"
    );
    let finding = ticked
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WorkNotParked)
        .unwrap_or_else(|| panic!("the refusal is reported: {:?}", ticked.warnings));
    assert_eq!(finding.thread.as_deref(), Some(thread.as_str()));
    assert!(
        finding.detail.contains("a merge is in progress"),
        "it names the state: {}",
        finding.detail
    );
    assert!(
        !ticked
            .warnings
            .iter()
            .any(|w| w.class == ConsequenceClass::WorkHandedOnUnparked),
        "a refusal is held OR handed on, never both: {:?}",
        ticked.warnings
    );

    // The claim stays, the marker stays, the tree is untouched.
    assert_eq!(b.holder().as_deref(), Some(scope.as_str()));
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_some(),
        "a held park has not ended the withdrawal: the copy has not gone on"
    );
    assert!(b.parked_rows(&scope).is_empty());
    assert!(b.merge_head().exists(), "the merge state is left alone");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // `nxc status` says so, stamped with THIS occasion.
    let op = b.operation(TICK, &thread);
    let note = op
        .park_refused
        .clone()
        .expect("the refusal is on the holding operation");
    assert_eq!(note.refusal, "mid_sequence");
    assert_eq!(
        note.occasion, "withdrawn",
        "the mark names the trouble that is trying to park: {note:?}"
    );
    assert_eq!(note.since, TICK);

    // …and the look is armed on the HOLDER, which with an empty queue is the only clock there is.
    assert_eq!(
        b.timer.last_for(&thread).as_deref(),
        Some(TICK_PLUS_CADENCE),
        "the holder's own look was re-armed by the tick that held: {:?}",
        b.timer.armed.lock().unwrap()
    );

    // The merge is resolved, and the next tick parks.
    b.finish_the_merge();
    let ticked = b.tick(LATER, &thread);
    let handed = ticked.handed_on.expect("the retry parked and handed on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.expect("with the work on a branch");
    assert_eq!(
        git(b.root(), &["show", &format!("{}:src.txt", parked.branch)]),
        HALF_FINISHED.trim_end()
    );
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(b.parked_rows(&scope).len(), 1);
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "the marker goes with the copy"
    );
    assert!(
        b.operation(LATER, &thread).park_refused.is_none(),
        "and the refusal goes with the claim it was about"
    );
}

/// **(ii) A workspace that is not a git repository hands the copy on UNPARKED** — the permanent
/// half of the same rule, and the answer `withdraw` has to stop promising a branch for (nxf
/// 6j6v.b9nf, fix round 2 of this item's review, Test Quality #1, Code Quality #1).
#[test]
fn a_withdrawn_holder_in_a_workspace_that_is_no_repository_is_handed_on_unparked() {
    let b = Bench::with_a_working_copy_that_is_not_a_repository();
    a_permanent_refusal_hands_the_copy_on_unparked(&b, "not a git repository");
}

/// **(iii) …and so does a host whose worker names NO working copy** — `stops_sessions()` is true,
/// so the running round is taken back and its session stopped, and `working_copy()` is `None`, so
/// there is nowhere to commit what it left. This is the DEFAULT of every worker but the sidecar,
/// which is why the receipt's promise mattered enough to fix: it is the commonest host of all.
#[test]
fn a_withdrawn_holder_on_a_host_that_names_no_working_copy_is_handed_on_unparked() {
    let b = Bench::without_a_working_copy();
    a_permanent_refusal_hands_the_copy_on_unparked(&b, "does not run sessions in a directory");
}

/// The body both permanent cases run: the withdrawal says plainly that there will be NO branch, the
/// tick hands the copy on all the same, and the marker goes with it.
fn a_permanent_refusal_hands_the_copy_on_unparked(b: &Bench, says: &str) {
    let thread = b.commission(T0, "coder");
    let session = b.session_of("coder");
    b.worker.mark_running(&session);
    b.dirty_the_tree();
    let scope = scope_of(&thread);
    assert_eq!(
        b.holder(),
        Some(scope.clone()),
        "the premise: it holds the claim"
    );

    let receipt = b.withdraw_recording_the_timer(WITHDRAWN, &thread);
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert!(
        !receipt.will_park,
        "a park here meets a PERMANENT refusal, so no branch is coming and the receipt may not \
         promise one: {receipt:?}"
    );
    let why = receipt
        .cannot_park
        .clone()
        .expect("…and it says why, because the round DOES hold the copy");
    assert!(why.contains(says), "the reason is the park's own: {why}");
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_some(),
        "the guard still stands: what ends this claim is the tick handing the copy on, parked or \
         not, and until then no other path may release it"
    );

    // The tick: handed on, loudly, with nothing parked.
    let ticked = b.tick(TICK, &thread);
    let handed = ticked
        .handed_on
        .expect("the copy went on — a permanent refusal is not a reason to hold it");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    assert!(
        handed.parked.is_none() && handed.unparked.is_some(),
        "unparked, and said so: {handed:?}"
    );
    let finding = ticked
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WorkHandedOnUnparked)
        .unwrap_or_else(|| panic!("and loudly: {:?}", ticked.warnings));
    assert!(finding.detail.contains(says), "{}", finding.detail);
    assert!(b.holder().is_none(), "the claim is gone with the copy");
    assert!(b.parked_rows(&scope).is_empty(), "nothing was parked");
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "and the marker went with the copy — the occasion ends one way"
    );
}

/// **The blast radius of the stop is the withdrawn thread's own subtree, and nothing else** (nxf
/// 6j6v.b9nf, fix round 2 of this item's review, Test Quality #2).
///
/// Until this case every test that marked a session running marked exactly ONE, so
/// `stopped() == vec![session]` held whether the code signalled that session or every live session
/// in the workspace. Here the `coder` holds the copy and is writing, a `scribe` is writing beside it
/// in a thread of its own, and the withdrawal names the SCRIBE's thread: what may be touched is that
/// thread and nothing above or beside it.
#[test]
fn withdrawing_one_thread_stops_only_that_thread_and_leaves_the_holder_alone() {
    let b = Bench::new();
    let coder_thread = b.commission(T0, "coder");
    let coder_session = b.session_of("coder");
    b.worker.mark_running(&coder_session);
    b.dirty_the_tree();
    let coder_scope = scope_of(&coder_thread);
    assert_eq!(b.holder(), Some(coder_scope.clone()));

    let scribe_thread = b.commission(QUEUED, "scribe");
    let scribe_session = b.session_of("scribe");
    b.worker.mark_running(&scribe_session);
    assert_eq!(
        b.holder(),
        Some(coder_scope.clone()),
        "the premise: the shared persona takes no lease, so the coder still holds the copy"
    );

    let receipt = b
        .withdraw(WITHDRAWN, &scribe_thread)
        .expect("the scribe's round is withdrawn");

    assert_eq!(
        b.worker.stopped(),
        vec![scribe_session.clone()],
        "ONLY the withdrawn thread's session was signalled"
    );
    assert!(
        b.worker.session_is_running(&coder_session),
        "the holder beside it is still writing"
    );
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, scribe_session);
    assert!(
        !receipt.will_park && receipt.cannot_park.is_none(),
        "the withdrawn thread is not in the claim that holds the copy, so there is nothing of ITS \
         to park — and nothing to explain away either: {receipt:?}"
    );
    assert!(
        b.store()
            .withdrawn_holder(&coder_scope)
            .expect("marker reads")
            .is_none(),
        "and no marker was written on the holder's scope: a later tick must not park a round \
         nobody took back"
    );
    assert!(
        b.store()
            .withdrawn_holder(&scope_of(&scribe_thread))
            .expect("marker reads")
            .is_none(),
        "nor on the withdrawn thread's own, which holds nothing"
    );

    // The coder's round is untouched: it still owes its answer and still holds the copy.
    assert!(!b.owes_nothing(WITHDRAWN, &coder_thread));
    assert!(b.owes_nothing(WITHDRAWN, &scribe_thread));
    assert_eq!(b.holder().as_deref(), Some(coder_scope.as_str()));
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // …and the tick has nothing to do: no marker means no withdrawn occasion.
    let ticked = b.tick(TICK, &coder_thread);
    assert!(
        ticked.parked.is_none() && ticked.handed_on.is_none(),
        "{ticked:?}"
    );
    assert_eq!(b.holder().as_deref(), Some(coder_scope.as_str()));
}

// ---- a CHANNEL round (fix round 2 of this item's review, Test Quality #5) ------------------------

/// Open the ordered `coding` channel and run its FIRST step to the end, so the pass that is
/// withdrawn below has an ANSWERED step behind it and an unrun step in front of it.
///
/// Returns `(the channel thread, the reviewer's session, the reviewer's member thread)`.
fn an_ordered_round_stopped_at_its_middle_step(
    b: &Bench,
    channel: &str,
) -> (String, String, String) {
    let channel_thread = b.commission(T0, channel);
    assert_eq!(
        b.holder().as_deref(),
        Some(scope_of(&channel_thread).as_str()),
        "the premise: a `working_tree: exclusive` channel claims from its first step, and the claim \
         is the OPERATION's — every step of it inherits this one key"
    );
    let coder_session = b.session_of("coder");
    let coder_thread = b
        .store()
        .session_thread(&coder_session)
        .expect("session map reads")
        .expect("the commissioned step stands in a thread");
    b.reply(T0, "coder", Some(&coder_session), &coder_thread, "built it");
    b.engine()
        .session_ended(b.human(T0), &coder_session)
        .expect("the first step's session announces its end");
    assert!(
        b.started("reviewer"),
        "the premise: the ordered flow advanced to its second step"
    );
    assert!(!b.started("finisher"), "…and not past it");

    let reviewer_session = b.session_of("reviewer");
    let reviewer_thread = b
        .store()
        .session_thread(&reviewer_session)
        .expect("session map reads")
        .expect("the commissioned step stands in a thread");
    b.worker.mark_running(&reviewer_session);
    b.dirty_the_tree();
    (channel_thread, reviewer_session, reviewer_thread)
}

/// **The stopped step's own end announcement does NOT commission the next step** (nxf 6j6v.b9nf,
/// fix round 2 of this item's review, Test Quality #5) — the body both channel shapes run.
///
/// The shape is the one the review named and could not settle by reading: a `working_tree:
/// exclusive` channel whose FIRST step is answered and over, whose SECOND step is RUNNING, and
/// which has a THIRD step waiting. The human takes the round back. The stopped sidecar then does
/// what a stopped sidecar does — it runs `nxc session ended` on its way out — and that
/// announcement reaches `Engine::session_ended`, which for a supervised step reconsiders the set.
///
/// When this was written every gate on that path was open, which is why it needed executing: the
/// channel thread still expected the supervisor (the round was only PARTLY withdrawn, and nxf
/// 6j6v.7me0 discharged a round only when every commission of its pass was taken back), the set
/// read settled (the withdrawn member's discharge completes its board), and nothing was writing
/// any more (the stop landed). Commissioning the successor would have put a fresh session into a
/// checkout still holding the withdrawn round's half-finished work, with the park still owed.
///
/// **Since nxf 6j6v.s2cj the first of those gates is CLOSED, and this asserts that it is**: a
/// withdrawal interrupts the chain below the thread it names, so the channel thread is discharged
/// onto the withdrawn identity with its member, and the supervisor — which moves a round on only
/// while the channel thread expects it — has nothing left to do. The marker gate this case was
/// written for is now the second line behind that; `a_claim_marked_withdrawn_opens_no_step_until_
/// its_park` reaches it on its own, with a marker and no discharge. What happens AFTER the park is
/// [`after_the_park_the_withdrawn_round_commissions_nothing`]'s.
fn the_next_step_is_not_commissioned_while_the_park_is_owed(b: &Bench, channel: &str) -> String {
    let (channel_thread, reviewer_session, reviewer_thread) =
        an_ordered_round_stopped_at_its_middle_step(b, channel);
    let scope = scope_of(&channel_thread);

    let receipt = b
        .withdraw(WITHDRAWN, &channel_thread)
        .expect("a running channel round is withdrawn");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, reviewer_session);
    assert_eq!(receipt.stopped[0].thread, reviewer_thread);
    assert!(
        receipt.will_park,
        "the running step's thread is in the claim that holds the copy: {receipt:?}"
    );
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_some(),
        "the premise: the claim area is marked as not parked yet"
    );
    let expects = b.expects_of(WITHDRAWN, &channel_thread);
    assert!(
        expects.len() == 1 && expects[0].ends_with(WITHDRAWN_HANDLE),
        "the withdrawal interrupts the round: the channel thread no longer expects its supervisor, \
         although its first step answered for real — {expects:?}"
    );

    // The stopped sidecar's last act, and the one the review asked about.
    b.worker.mark_gone(&reviewer_session);
    b.engine()
        .session_ended(b.human(WITHDRAWN), &reviewer_session)
        .expect("the announcement is taken");

    assert!(
        !b.started("finisher"),
        "the stopped session's own end must not commission the next step into a checkout that \
         still holds the withdrawn round's uncommitted work: {:?}",
        b.triggers()
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and nothing touched the tree"
    );
    assert_eq!(
        b.holder().as_deref(),
        Some(scope.as_str()),
        "the withdrawn holder keeps the copy until the tick parks it"
    );
    channel_thread
}

/// **`flow: sequential`**, on the `session ended` path. When nxf 6j6v.b9nf's review wrote this case
/// it was safe by an accident worth writing down: the withdrawn discharge retargets the member
/// slot's `expects` onto the withdrawn identity, so `slot_target` no longer maps that slot to its
/// declared step, `current_pass` stops at the step nothing serves any more, and the settled slot is
/// then not in the derived pass at all. It was safe on THIS path only — the tick the withdrawal arms
/// re-commissioned the taken-back step (nxf 6j6v.s2cj; `after_the_park_…` below). Since s2cj what
/// holds here is the round's own discharge, which the shared body asserts; the marker gate is
/// pinned on its own by `a_claim_marked_withdrawn_…`.
#[test]
fn the_end_of_a_withdrawn_step_of_an_ordered_channel_does_not_start_the_next_one() {
    let b = Bench::new();
    the_next_step_is_not_commissioned_while_the_park_is_owed(&b, "coding");
}

/// **A channel that declares `steps:`**, where nxf 6j6v.b9nf's review first measured the defect on
/// this path. Its pass is derived from the recorded run mark (`run_slots`), not from what each slot
/// expects, so a discharged slot stays in the set and `supervisor_route_step` routed straight on:
/// the finisher's session started with `src.txt` still half-finished in the checkout and the park
/// still owed. The marker gate stopped that; since nxf 6j6v.s2cj the round's own discharge does,
/// before the gate is asked, and the gate is pinned on its own by `a_claim_marked_withdrawn_…`.
#[test]
fn the_end_of_a_withdrawn_step_of_a_declared_state_machine_does_not_route_the_next_one_either() {
    let b = Bench::new();
    the_next_step_is_not_commissioned_while_the_park_is_owed(&b, "walked");
}

/// **The chain is interrupted BEFORE the first session is signalled** (nxf 6j6v.s2cj; review of PR
/// #483, Integrity #2). The interrupt is the last write that decides whether the chain goes on; run
/// after the stops, a failure there left a stopped session under a round that carried on at the
/// next tick. Read from a second connection at the moment of the stop, the channel thread already
/// expects the withdrawn identity.
#[test]
fn the_chain_is_interrupted_before_the_first_session_is_signalled() {
    let b = Bench::new();
    let (channel_thread, reviewer_session, _reviewer_thread) =
        an_ordered_round_stopped_at_its_middle_step(&b, "coding");
    *b.worker.look_at_on_stop.lock().unwrap() = Some(channel_thread.clone());

    b.withdraw(WITHDRAWN, &channel_thread)
        .expect("a running channel round is withdrawn");
    assert_eq!(b.worker.stopped(), vec![reviewer_session]);
    let seen = b.worker.seen_on_stop.lock().unwrap().clone();
    assert!(
        seen.len() == 1 && seen[0].len() == 1 && seen[0][0].ends_with(WITHDRAWN_HANDLE),
        "at the stop, the round above was already interrupted: {seen:?}"
    );
}

/// **After the park, the withdrawn round commissions NOTHING — not its successor, and not the step
/// that was taken back** (nxf 6j6v.s2cj, the build half of the owner's decision 6j6v.ymvg of
/// 2026-09-20: a taken-back step does not commission its successor). The body both channel shapes
/// run, against the same scenario.
///
/// It picks up where [`the_next_step_is_not_commissioned_while_the_park_is_owed`] stops — the
/// stopped session has announced its end, and nothing has moved yet — and runs the tick the
/// withdrawal ARMED: keyed on the claim root, which for a channel round is the channel thread
/// itself. That tick parks FIRST and reconsiders the board in the same call
/// ([`orchestration::tick`]), so it is the path the background service actually takes, and the one
/// on which both shapes carried on before this item: `steps:` routed straight to its next step, and
/// `flow: sequential` — "safe by accident" on the `session ended` path — re-derived its pass without
/// the withdrawn slot and commissioned the taken-back step AGAIN. A second tick, with the copy free
/// and nothing running, must find nothing to do either.
fn after_the_park_the_withdrawn_round_commissions_nothing(b: &Bench, channel: &str) {
    let channel_thread = the_next_step_is_not_commissioned_while_the_park_is_owed(b, channel);
    let scope = scope_of(&channel_thread);
    let before = b.triggers().len();

    let ticked = b.tick(TICK, &channel_thread);
    let handed = ticked
        .handed_on
        .expect("the tick the withdrawal armed parks the work and hands the copy on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.clone().expect("and parked its work first");
    assert!(parked.committed && parked.created_branch, "{parked:?}");
    b.tick(LATER, &channel_thread);

    assert_eq!(
        &b.triggers()[before..],
        &[] as &[String],
        "a withdrawn round starts nothing once its work is parked — neither the successor of the \
         step that was taken back nor that step again"
    );
    assert!(!b.started("finisher"));

    // Nothing else about the park changed: the work is on its branch, recorded against the
    // operation, and the tree is back on its base.
    assert_eq!(b.parked_rows(&scope).len(), 1);
    assert_eq!(b.parked_rows(&scope)[0].parked.branch, parked.branch);
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\n"
    );
}

#[test]
fn after_the_park_a_withdrawn_ordered_round_does_not_commission_the_next_step() {
    let b = Bench::new();
    after_the_park_the_withdrawn_round_commissions_nothing(&b, "coding");
}

#[test]
fn after_the_park_a_withdrawn_declared_state_machine_does_not_route_the_next_step() {
    let b = Bench::new();
    after_the_park_the_withdrawn_round_commissions_nothing(&b, "walked");
}

/// **A human commissions a `pm`, and the `pm`, from inside its session, commissions `channel`**,
/// whose first step (`member`) starts; the `pm` ends its turn waiting for the round, as a caller of
/// a sub-round does. Returns `(pm thread, pm session, channel thread, member session)`.
fn a_pm_round(b: &Bench, channel: &str, member: &str) -> (String, String, String, String) {
    let pm_thread = b.commission(T0, "pm");
    let pm = b.session_of("pm");
    let channel_thread = b
        .engine()
        .send_to(
            from_session(&pm, T0),
            SendToRequest {
                machine: None,
                to: channel,
                body: "build it",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the pm commissions the channel")
        .thread_id;
    assert_eq!(
        b.store().thread_root(&channel_thread).expect("root reads"),
        pm_thread,
        "the premise: the round hangs under the pm's thread, in one operation"
    );
    let member_session = b.session_of(member);
    b.worker.mark_running(&member_session);
    (pm_thread, pm, channel_thread, member_session)
}

/// **A withdrawn round is not consolidated afterwards, and the agent that commissioned it is not
/// woken** (nxf 6j6v.s2cj) — the nested shape, and the path the flat cases above cannot see.
///
/// The human withdraws the operation's first thread. Everything between the member and that thread
/// is interrupted — the channel thread AND the `pm`'s own — and then the channel thread's own clock
/// ticks. On a channel thread a tick CONSOLIDATES a settled set and wakes whoever asked for it; for
/// a human requester there is nobody to wake, but here there is — the `pm`'s session is the round's
/// return address — and waking it would carry the chain on above the step that was taken back. The
/// tick answers `already_handled`, the marker a consolidation leaves, which is what the fragment
/// tells an embedder (review of PR #483, Test Quality #1); the case beside it runs the same tick on
/// a round nobody withdrew and shows the `pm` IS woken there, so this answer is the marker's doing.
fn a_tick_on_a_withdrawn_round_wakes_nobody(b: &Bench, channel: &str, member: &str) {
    let (pm_thread, _pm, channel_thread, member_session) = a_pm_round(b, channel, member);
    b.dirty_the_tree();

    let receipt = b
        .withdraw(WITHDRAWN, &pm_thread)
        .expect("the human withdraws the operation's first thread");
    assert_eq!(receipt.stopped.len(), 1, "{receipt:?}");
    assert_eq!(receipt.stopped[0].session, member_session);
    for waiting in [&channel_thread, &pm_thread] {
        let expects = b.expects_of(WITHDRAWN, waiting);
        assert!(
            expects.len() == 1 && expects[0].ends_with(WITHDRAWN_HANDLE),
            "every thread between the member and the named one is interrupted: {waiting} expects \
             {expects:?}"
        );
    }

    b.worker.mark_gone(&member_session);
    b.engine()
        .session_ended(b.human(WITHDRAWN), &member_session)
        .expect("the announcement is taken");
    let before = b.triggers().len();
    let ticked = b.tick(TICK, &channel_thread);
    assert!(
        !ticked.acted && ticked.reason == "already_handled",
        "the channel's own tick finds the round handled — by the withdrawal: {ticked:?}"
    );
    assert!(
        ticked.delivered.is_none() && ticked.woke.is_none(),
        "{ticked:?}"
    );
    b.tick(LATER, &pm_thread);
    assert_eq!(
        &b.triggers()[before..],
        &[] as &[String],
        "nobody was woken and nothing was commissioned"
    );
}

#[test]
fn a_tick_on_a_withdrawn_round_does_not_wake_the_agent_that_commissioned_it() {
    let b = Bench::new();
    a_tick_on_a_withdrawn_round_wakes_nobody(&b, "checked", "reviewer");
}

#[test]
fn a_tick_on_a_withdrawn_ordered_round_does_not_wake_the_agent_that_commissioned_it_either() {
    let b = Bench::new();
    a_tick_on_a_withdrawn_round_wakes_nobody(&b, "coding", "coder");
}

/// **The control: the same tick on a round nobody withdrew DOES wake the `pm`** (review of PR #483,
/// Test Quality #1). The member answers while its process is still running, so the round's
/// working-tree gate holds the consolidation; the process then goes without announcing it, and the
/// channel's own tick is what finds the set settled — the call the withdrawn case above makes, on
/// the same channel. Here it consolidates and resumes the `pm`, which is how the `pm`'s return
/// address resolves on this path, and what the withdrawn identity is the only thing standing in
/// front of there.
#[test]
fn the_same_tick_on_a_round_nobody_withdrew_wakes_the_agent_that_commissioned_it() {
    let b = Bench::new();
    let (_pm_thread, pm, channel_thread, reviewer) = a_pm_round(&b, "checked", "reviewer");
    let member_thread = b
        .store()
        .session_thread(&reviewer)
        .expect("session map reads")
        .expect("the step stands in a thread");
    b.reply(
        T0,
        "reviewer",
        Some(&reviewer),
        &member_thread,
        "checked it",
    );
    let before = b.triggers().len();
    assert!(
        !b.triggers()
            .iter()
            .any(|l| l.starts_with("trigger role=pm ") && l.contains("checked it")),
        "the premise: the gate held the consolidation while the reviewer's process ran"
    );

    b.worker.mark_gone(&reviewer);
    let ticked = b.tick(TICK, &channel_thread);
    assert!(ticked.acted && ticked.woke.is_some(), "{ticked:?}");
    assert!(
        b.triggers()[before..]
            .iter()
            .any(|l| l.starts_with("trigger role=pm ") && l.contains(&format!("session={pm}"))),
        "the pm was woken with the round: {:?}",
        &b.triggers()[before..]
    );
}

/// **…and the way back works in the nested shape too** (review of PR #483, Code Quality #8). The
/// guides promise that the work comes back when the person who withdrew it commissions the same
/// thread again; the flat case is `withdrawing_a_running_round_stops_its_session_and_parks_its_work`.
/// Here the thread is the `pm`'s, the work was the member's two levels down, and the `pm` stopped
/// nothing of its own — so the follow-up has to reach the `pm`, put the tree back on the park
/// branch, and tell it where the work is.
#[test]
fn a_withdrawn_nested_rounds_work_comes_back_on_the_next_commission_into_the_first_thread() {
    let b = Bench::new();
    let (pm_thread, pm, _channel_thread, reviewer) = a_pm_round(&b, "checked", "reviewer");
    let scope = scope_of(&pm_thread);
    b.dirty_the_tree();
    b.withdraw(WITHDRAWN, &pm_thread).expect("withdrawn");
    b.worker.mark_gone(&reviewer);
    let handed = b
        .tick(TICK, &pm_thread)
        .handed_on
        .expect("the tick parks the withdrawn work and hands the copy on");
    let parked = handed.parked.expect("parked first");
    assert_eq!(head_branch(b.root()), "release/1.2");

    let before = b.triggers().len();
    b.reply(COME_BACK, "carsten", None, &pm_thread, "try that again");
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=pm "))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "the follow-up into the first thread reaches the pm: {:?}",
                &b.triggers()[before..]
            )
        });
    assert!(
        resumed.contains(&format!("session={pm}")),
        "the pm's own session: {resumed}"
    );
    assert_eq!(
        head_branch(b.root()),
        parked.branch,
        "the tree is back on the park branch"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );
    assert!(resumed.contains("PARKED WORK"), "{resumed}");
    assert!(
        b.parked_rows(&scope).is_empty(),
        "the row is stamped as come back for"
    );
}

/// **A claim that is MARKED withdrawn opens no step until its park — and then the step it would
/// have opened is opened, onto the park branch** (nxf 6j6v.b9nf's marker gate, pinned on its own
/// since nxf 6j6v.s2cj made it the second line).
///
/// A withdrawal now discharges the round it cuts into, so the ordinary way to this gate goes
/// through the discharge first and never asks it. What still reaches it is a round a withdrawal did
/// NOT discharge: a queued commission that started while the withdrawal ran leaves a live session
/// in the claim and the marker standing (`started_meanwhile`) — a race no single-threaded test can
/// provoke. So the state is BUILT: the marker is written straight into the store, exactly as
/// `withdraw` writes it, and nothing is discharged. The running step then ANSWERS, for real, and
/// ends — every other gate of the supervisor is open, and a successor would start into a tree that
/// still holds the round's uncommitted work. The gate holds it; the tick parks, and the successor
/// it then opens finds the work where the claim's own way back put it.
fn a_claim_marked_withdrawn_opens_no_step_until_its_park(b: &Bench, channel: &str) {
    let (channel_thread, reviewer_session, reviewer_thread) =
        an_ordered_round_stopped_at_its_middle_step(b, channel);
    let scope = scope_of(&channel_thread);
    b.store()
        .note_withdrawn_holder(&scope, "carsten", WITHDRAWN)
        .expect("the marker, as `withdraw` writes it");

    b.reply(
        WITHDRAWN,
        "reviewer",
        Some(&reviewer_session),
        &reviewer_thread,
        "checked it",
    );
    b.worker.mark_gone(&reviewer_session);
    b.engine()
        .session_ended(b.human(WITHDRAWN), &reviewer_session)
        .expect("the announcement is taken");
    assert!(
        !b.started("finisher"),
        "a settled step with nothing writing must still not open the next one while the claim's \
         work is unparked: {:?}",
        b.triggers()
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED
    );

    // The gate delays; it does not cancel. The tick parks, and the round — which nobody discharged —
    // goes on to its next step, on the park branch.
    let ticked = b.tick(TICK, &channel_thread);
    let parked = ticked
        .handed_on
        .and_then(|h| h.parked)
        .expect("the tick parked the marked claim's work");
    assert!(b.started("finisher"), "{:?}", b.triggers());
    assert_eq!(head_branch(b.root()), parked.branch);
}

#[test]
fn a_claim_marked_withdrawn_opens_no_step_of_an_ordered_channel_until_its_park() {
    let b = Bench::new();
    a_claim_marked_withdrawn_opens_no_step_until_its_park(&b, "coding");
}

#[test]
fn a_claim_marked_withdrawn_opens_no_step_of_a_declared_state_machine_until_its_park() {
    let b = Bench::new();
    a_claim_marked_withdrawn_opens_no_step_until_its_park(&b, "walked");
}

/// **…and the way back for a channel round is what the guides promise** (nxf 6j6v.b9nf, fix round
/// 2 of this item's review, Test Quality #5): the tick parks the withdrawn work and hands the copy
/// on, and the next commission into the CHANNEL thread puts the tree back on the park branch and
/// tells the session where its work is.
///
/// It is the channel twin of the direct-persona way back at the top of this file, and it is driven
/// the same way, with a RIVAL taking the copy in between. The tick that frees the copy is keyed on
/// the rival for a reason that changed with nxf 6j6v.s2cj: it used to be that a tick keyed on the
/// channel thread would commission the flow's own next step there and take the branch back through
/// the same seam, which is exactly what s2cj removed (`after_the_park_…` above) — now keying it on
/// the rival keeps it the flat twin of the persona case, a copy that another operation used in
/// between, and leaves the human's follow-up the only thing that brings the work back.
#[test]
fn a_withdrawn_channel_rounds_work_comes_back_on_the_next_commission_into_the_channel_thread() {
    let b = Bench::new();
    let channel_thread = the_next_step_is_not_commissioned_while_the_park_is_owed(&b, "coding");
    let scope = scope_of(&channel_thread);

    // A rival wants the copy, and the tick on ITS thread parks the withdrawn work and hands it over.
    let solo = b.commission(WITHDRAWN, "solo");
    assert!(
        !b.started("solo"),
        "the rival is queued behind the withdrawn claim"
    );
    let ticked = b.tick(TICK, &solo);
    let handed = ticked
        .handed_on
        .expect("the tick handed the withdrawn holder's copy on");
    assert_eq!(handed.occasion, ParkOccasion::Withdrawn);
    let parked = handed.parked.clone().expect("and parked its work first");
    assert!(b.started("solo"), "the rival started: {:?}", b.triggers());
    assert_eq!(
        head_branch(b.root()),
        "release/1.2",
        "the tree is back on its base for the rival"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\n"
    );
    assert!(
        b.store()
            .withdrawn_holder(&scope)
            .expect("marker reads")
            .is_none(),
        "the marker goes with the copy"
    );
    assert_eq!(b.parked_rows(&scope).len(), 1);

    // The rival finishes, and the human comes back into the CHANNEL thread.
    let solo_session = b.session_of("solo");
    b.reply(LATER, "solo", Some(&solo_session), &solo, "done");
    assert!(
        b.holder().is_none(),
        "a finished operation gives the copy back"
    );

    let before = b.triggers().len();
    b.reply(
        COME_BACK,
        "carsten",
        None,
        &channel_thread,
        "try that again",
    );
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role="))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "the follow-up into the channel thread commissions the round again: {:?}",
                &b.triggers()[before..]
            )
        });
    assert_eq!(
        head_branch(b.root()),
        parked.branch,
        "the tree is back on the park branch"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        HALF_FINISHED,
        "and the withdrawn round's work is in it"
    );
    assert!(resumed.contains("PARKED WORK"), "{resumed}");
    assert!(resumed.contains(&parked.branch), "{resumed}");
    assert!(
        b.parked_rows(&scope).is_empty(),
        "the row is stamped as come back for"
    );
}

// ---- the CLI: a real `SidecarWorker`, a real process, a real pid file ------------------------------

/// A child that is killed and reaped when the test ends, whichever way it ends — a `sleep 30` left
/// behind by a failed assertion would outlive the run. `a_worker_can_stop_a_session.rs`'s own
/// `Reaped`, for its reason (Test Quality #9 of this item's review: the two `nxc_withdraw_*` tests
/// below spawned a real `sleep 30` with no drop guard at all).
#[cfg(unix)]
struct Reaped(std::process::Child);

#[cfg(unix)]
impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
fn a_live_process() -> Reaped {
    use std::process::Stdio;
    Reaped(
        std::process::Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a real process to be alive"),
    )
}

/// **`nxc withdraw --thread <id> --json` on a running round carries `stopped` and `will_park`** —
/// through the shipped binary and the shipped worker, which is the proof Task 6's review recorded as
/// owed here: `LazyWorker` forwards `stops_sessions` / `stop_session` to the `SidecarWorker` it
/// selects, and the engine seam alone cannot show that (every seam test hands the engine its own
/// worker).
///
/// The technique is `working_tree_two_process_e2e.rs`'s: the dry worker starts nothing and leaves
/// no pid file, so a LIVE session is modelled by writing the pid of a real `sleep` into the claim
/// file `SidecarWorker` reads — and the stop is real too: the `sleep` dies of the SIGTERM the
/// worker sends it.
#[cfg(unix)]
#[test]
fn nxc_withdraw_on_a_running_round_stops_the_session_and_says_the_work_will_be_parked() {
    use assert_cmd::Command;
    use serde_json::Value;

    fn nxc(root: &Path) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(root)
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", T0)
            // **The SHIPPED worker for BOTH calls** (fix round 2 of this item's review, Code
            // Quality #1). The send used to run under `dry`, whose `working_copy()` is `None`, so
            // the acquire recorded no operation base — and since `will_park` now asks the park's own
            // read-only preconditions, that artefact of switching workers mid-scenario reads as
            // "this work can never be parked". No real host changes worker between commissioning a
            // round and taking it back. The stub sidecar is empty: node runs it and exits at once,
            // which is all this needs — the LIVE process is the `sleep` whose pid is written into
            // the claim file below.
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", root.join("stub-sidecar.mjs"))
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", root.join("dry.log"));
        c
    }

    let tmp = workspace();
    let root = tmp.path();
    std::fs::write(root.join("stub-sidecar.mjs"), "").unwrap();
    let out = nxc(root)
        .args(["--json", "send", "--no-ref", "--to", "coder", "build it"])
        .assert()
        .success();
    let holder: Value = serde_json::from_slice(&out.get_output().stdout).expect("a send receipt");
    let thread = holder["thread_id"].as_str().expect("thread").to_string();
    let session = holder["session"].as_str().expect("session").to_string();

    let logs = root.join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    let mut alive = a_live_process();
    // The claim exactly as `SessionLock` writes it: the pid AND the instant that process started,
    // which is what earns the signal — a claim that cannot prove which process it names is refused
    // rather than signalled (nxf 6j6v.b9nf, fix round 3).
    std::fs::write(
        logs.join(format!("{session}.pid")),
        nexus_chat::worker::session_claim_for(alive.0.id()),
    )
    .unwrap();

    let out = nxc(root)
        .args(["--json", "withdraw", "--thread", &thread])
        .assert()
        .success();
    let receipt: Value =
        serde_json::from_slice(&out.get_output().stdout).expect("a withdraw receipt");
    assert_eq!(
        receipt["stopped"][0]["session"],
        session.as_str(),
        "{receipt}"
    );
    assert_eq!(receipt["stopped"][0]["role"], "coder", "{receipt}");
    assert_eq!(receipt["will_park"], true, "{receipt}");
    assert!(
        receipt.get("warnings").is_none(),
        "no warning: the stop landed — {receipt}"
    );

    // The stop was REAL: the process died of the signal the worker sent it.
    let status = alive.0.wait().expect("the sleep is reaped");
    assert!(
        !status.success(),
        "the sleep did not run its thirty seconds out — it was stopped: {status:?}"
    );
}

/// **On the shipped binary, from inside the running session itself: refused, and nothing is
/// signalled** (nxf 6j6v.ezbr). The coder's own `nxc`, with the session the sidecar stamped into its
/// environment, tries to take its round back while presenting the OPENER's name — so the one thing
/// that can refuse it is the session map. The live process must come through untouched.
#[cfg(unix)]
#[test]
fn nxc_withdraw_from_a_session_this_workspace_started_is_refused_and_signals_nothing() {
    use assert_cmd::Command;
    use serde_json::Value;

    let tmp = workspace();
    let root = tmp.path();
    std::fs::write(root.join("stub-sidecar.mjs"), "").unwrap();
    let nxc = || -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(root)
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", T0)
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", root.join("stub-sidecar.mjs"))
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", root.join("dry.log"));
        c
    };
    let out = nxc()
        .args(["--json", "send", "--no-ref", "--to", "coder", "build it"])
        .assert()
        .success();
    let holder: Value = serde_json::from_slice(&out.get_output().stdout).expect("a send receipt");
    let thread = holder["thread_id"].as_str().expect("thread").to_string();
    let session = holder["session"].as_str().expect("session").to_string();
    let logs = root.join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    let mut alive = a_live_process();
    std::fs::write(
        logs.join(format!("{session}.pid")),
        nexus_chat::worker::session_claim_for(alive.0.id()),
    )
    .unwrap();

    let out = nxc()
        .env("NXC_SESSION", &session)
        .args(["withdraw", "--thread", &thread])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(&session) && stderr.contains("--escalate"),
        "refused by name, pointing at escalation: {stderr}"
    );
    // Polled for a moment rather than asked once: a signal sent just before the refusal returned
    // could take a few milliseconds to end a `sleep` (review of PR #483, Test Quality #6).
    for _ in 0..20 {
        assert!(
            alive
                .0
                .try_wait()
                .expect("the process can be asked")
                .is_none(),
            "and the running session was NOT signalled"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// The human rendering, on the shipped binary: one line per stopped session, and one saying the
/// work will be parked and how it comes back.
#[cfg(unix)]
#[test]
fn nxc_withdraw_says_in_words_what_it_stopped_and_what_becomes_of_the_work() {
    use assert_cmd::Command;
    use serde_json::Value;

    let tmp = workspace();
    let root = tmp.path();
    std::fs::write(root.join("stub-sidecar.mjs"), "").unwrap();
    // The shipped worker for both calls, for the reason written on the sibling above.
    let nxc = || -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(root)
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", T0)
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", root.join("stub-sidecar.mjs"))
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", root.join("dry.log"));
        c
    };
    let out = nxc()
        .args(["--json", "send", "--no-ref", "--to", "coder", "build it"])
        .assert()
        .success();
    let holder: Value = serde_json::from_slice(&out.get_output().stdout).expect("a send receipt");
    let thread = holder["thread_id"].as_str().expect("thread").to_string();
    let session = holder["session"].as_str().expect("session").to_string();
    let logs = root.join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    let mut alive = a_live_process();
    // The claim exactly as `SessionLock` writes it: the pid AND the instant that process started,
    // which is what earns the signal — a claim that cannot prove which process it names is refused
    // rather than signalled (nxf 6j6v.b9nf, fix round 3).
    std::fs::write(
        logs.join(format!("{session}.pid")),
        nexus_chat::worker::session_claim_for(alive.0.id()),
    )
    .unwrap();

    let out = nxc()
        .args(["withdraw", "--thread", &thread])
        .assert()
        .success();
    let _ = alive.0.wait();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains(&format!("stopped coder on thread {thread}")) && stdout.contains(&session),
        "one line per stopped session, naming it: {stdout}"
    );
    assert!(
        stdout.contains("parked") && stdout.contains(&format!("nxc reply --thread {thread}")),
        "and one saying the work will be parked and how it comes back: {stdout}"
    );
    // **And who runs the park** (fix round 2 of this item's review, Integrity #4): the line
    // promises a tick, so it names the command that IS one — on a workspace with no background
    // service that is the only thing that will ever do it.
    assert!(
        stdout.contains(&format!("nxc tick --thread {thread}")),
        "…and names the tick that does it by hand: {stdout}"
    );
    // …and closes with what the withdrawal did to the chain (nxf 6j6v.s2cj): nothing below the
    // named thread is commissioned any more, and nothing started meanwhile to except.
    assert!(
        stdout.contains(&format!(
            "operation {thread} is interrupted: nothing below it is commissioned or consolidated"
        )),
        "{stdout}"
    );
}

/// **The human line names a withdrawn holder, and `--json` carries the same fact** (nxf 6j6v.b9nf,
/// Integrity #2 of this item's review) — the CLI-level proof that
/// [`nexus_chat::facade::StatusOperation::withdrawn`] reaches `nxc status`'s rendering, in
/// `a_refused_park_follows_the_rule.rs`'s technique: HOW a marker comes to exist through `withdraw`
/// itself is the rest of THIS file's business; this writes the row straight into the store and
/// reads it back through the shipped binary. The dry worker a CLI test runs under never answers
/// `true` for `session_is_running`, so this also pins the OTHER half of the render — a marker with
/// nothing left pinning it — which `cli::tests::the_withdrawn_line_names_who_withdrew_it_and_who_
/// still_pins_it` pins the pure-function half of.
#[test]
fn nxc_status_names_a_withdrawn_holder_in_words() {
    use assert_cmd::Command;
    use serde_json::Value;

    fn nxc(root: &Path) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(root)
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", T0)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", root.join("dry.log"));
        c
    }

    let tmp = workspace();
    let root = tmp.path();
    let out = nxc(root)
        .args(["--json", "send", "--no-ref", "--to", "coder", "build it"])
        .assert()
        .success();
    let holder: Value = serde_json::from_slice(&out.get_output().stdout).expect("a send receipt");
    let thread = holder["thread_id"].as_str().expect("thread").to_string();

    Workspace::resolve(None, root)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
        .note_withdrawn_holder(
            &WorkScope::Thread(thread.clone()).key(),
            "local/carsten",
            T0,
        )
        .expect("note the marker directly — this is a rendering test, not a `withdraw` one");

    let out = nxc(root).args(["status"]).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let header = stdout
        .lines()
        .find(|l| l.starts_with(&format!("operation {thread}")))
        .unwrap_or_else(|| panic!("no operation line: {stdout}"));
    assert!(header.contains("WITHDRAWN"), "{header}");
    assert!(
        stdout.contains(&format!(
            "  withdrawn by local/carsten at {T0} — waiting to park; nothing is pinning it any \
             more, the next tick parks it"
        )),
        "{stdout}"
    );

    let out = nxc(root).args(["--json", "status"]).assert().success();
    let report: Value = serde_json::from_slice(&out.get_output().stdout).expect("valid json");
    let op = &report["operations"][0];
    assert_eq!(op["withdrawn"]["by"], "local/carsten", "{op}");
    assert_eq!(op["withdrawn"]["withdrawn_at"], T0, "{op}");
    assert_eq!(op["withdrawn"]["pinned_by"], serde_json::json!([]), "{op}");
}
