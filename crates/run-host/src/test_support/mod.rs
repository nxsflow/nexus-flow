//! The crash harness's home (§11, §12.4), behind `test-support`: a rig of one host, file-backed ports and a
//! simulated clock over a temporary directory. Everything a test asserts on survives an abort of the process,
//! because it is in files.

pub(crate) mod crash;
mod file_ports;
pub mod panics;
pub mod scripted_backend;
pub mod scripted_tree;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use nexus_run_contracts::{Event, LivenessPolicy, RunId, RunStore, Secs, Timestamp};
use nexus_run_store_sqlite::SqliteRunStore;

use crate::bridge::{self, BridgeCtx, Caller, CommissionReceipt, ReplyFlags, HOST_BUSY_TIMEOUT};
use crate::git_tree::{GitBounds, GitTree};
use crate::host::{Host, Layout, Ports, TickReport};
use crate::ports::{Clock, Refs};
use crate::tree_ledger::TreeLedger;
use crate::tree_worker::WorkTree;
pub use file_ports::{coding_decl, read_port_lines, review_decl, FilePorts, PortLine};
use scripted_backend::{pass, Cue, Scenario, ScriptedBackend, SessionInfo};
use scripted_tree::ScriptedTree;

pub const WORKSPACE: &str = "ws-1";
pub const REQUESTER: &str = "person";
/// 2026-11-05T10:00:00Z, where the simulated clock starts.
pub const CLOCK_START: i64 = 1_793_872_800_000;

/// A clock a test moves.
#[derive(Debug)]
pub struct SimClock(AtomicI64);

impl SimClock {
    pub fn at(millis: i64) -> Self {
        Self(AtomicI64::new(millis))
    }

    pub fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn set(&self, millis: i64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}

impl Clock for SimClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_millis(self.0.load(Ordering::SeqCst))
    }
}

/// A policy with short human waits, so a harness run does not take thousands of ticks.
pub fn short_policy() -> LivenessPolicy {
    LivenessPolicy {
        start_deadline: Secs(60),
        stop_grace: Secs(30),
        ..LivenessPolicy::default()
    }
}

/// A git repository with one commit on `main`, the `origin` a prepare clones.
pub fn make_origin(path: &Path) {
    fs::create_dir_all(path).expect("the origin's directory");
    fs::write(path.join("README"), b"origin").expect("the origin's file");
    for args in [
        vec!["-c", "init.defaultBranch=main", "init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-verify",
            "-m",
            "first",
        ],
    ] {
        let status = Command::new("git")
            .current_dir(path)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git runs");
        assert!(status.success(), "git failed making the origin");
    }
}

/// What a rig needs of its tree beside the port: whether it is working, because that is real time.
pub trait RigTree: WorkTree + 'static {
    fn busy(&self) -> bool;
}

impl RigTree for ScriptedTree {
    fn busy(&self) -> bool {
        ScriptedTree::busy(self)
    }
}

impl RigTree for GitTree {
    fn busy(&self) -> bool {
        GitTree::busy(self)
    }
}

/// `git` for what a rig sets up. It has to work.
fn setup_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

/// A host and everything around it, on one directory, which may already hold a run: opening the rig again IS
/// the restart.
pub struct Rig<T: RigTree = ScriptedTree> {
    pub dir: PathBuf,
    pub store: Arc<SqliteRunStore>,
    pub clock: Arc<SimClock>,
    pub ports: Arc<FilePorts>,
    pub backend: Arc<ScriptedBackend>,
    pub tree: Arc<T>,
    pub host: Host,
    pub bridge: BridgeCtx,
    pub policy: LivenessPolicy,
    /// Simulated milliseconds per tick.
    pub step_ms: i64,
}

impl Rig<GitTree> {
    /// A rig on the real `GitTree`: an origin, a registered root with its `.nxs` link, and `root-v2` beside it.
    pub fn git_tree(dir: &Path, clock_ms: i64, scenario: Scenario) -> Self {
        Self::git_tree_bounded(dir, clock_ms, scenario, GitBounds::default())
    }

    pub fn git_tree_bounded(
        dir: &Path,
        clock_ms: i64,
        scenario: Scenario,
        bounds: GitBounds,
    ) -> Self {
        Self::git_tree_using(dir, clock_ms, scenario, bounds, Path::new("git"))
    }

    /// With another program for `git`.
    pub fn git_tree_using(
        dir: &Path,
        clock_ms: i64,
        scenario: Scenario,
        bounds: GitBounds,
        git: &Path,
    ) -> Self {
        fs::create_dir_all(dir).expect("the rig's directory");
        let origin = dir.join("origin");
        if !origin.exists() {
            make_origin(&origin);
        }
        let root = dir.join("root");
        if !root.exists() {
            setup_git(dir, &["clone", "-q", "origin", "root"]);
            let target = dir.join("nxs-target");
            fs::create_dir_all(&target).expect("the link's target");
            let target = target.canonicalize().expect("a real path");
            std::os::unix::fs::symlink(target, root.join(".nxs")).expect("the root's link");
        }
        let tree = GitTree::with_git(&root, Rig::<ScriptedTree>::ledger(dir), bounds, git);
        Self::open(dir, clock_ms, scenario, tree)
    }

    /// The directory as a predecessor left it: a clone of the origin, linked, with an uncommitted file.
    pub fn leave_a_dirty_directory(dir: &Path) {
        let v2 = dir.join("root-v2");
        if v2.exists() {
            return;
        }
        setup_git(dir, &["clone", "-q", "origin", "root-v2"]);
        let target = dir
            .join("root")
            .join(".nxs")
            .canonicalize()
            .expect("the root's link");
        std::os::unix::fs::symlink(target, v2.join(".nxs")).expect("the link");
        fs::write(v2.join(".git/info/exclude"), ".nxs\n").expect("the exclude");
        fs::write(v2.join("stray.txt"), b"a predecessor's work").expect("the file");
    }
}

impl Rig {
    pub fn layout(dir: &Path) -> Layout {
        Layout {
            root: dir.join("root"),
            agent_logs: dir.join("agent-logs"),
            session_env: Vec::new(),
        }
    }

    fn ledger(dir: &Path) -> TreeLedger {
        TreeLedger::new(&dir.join("agent-logs"), WORKSPACE)
    }

    /// A rig whose tree answers from a script.
    pub fn scripted(dir: &Path, clock_ms: i64, scenario: Scenario) -> Self {
        let tree = ScriptedTree::new(Self::ledger(dir));
        Self::open(dir, clock_ms, scenario, tree)
    }

    /// A rig whose tree works in a real repository.
    pub fn in_repo(dir: &Path, clock_ms: i64, scenario: Scenario) -> Self {
        let origin = dir.join("origin");
        if !origin.exists() {
            make_origin(&origin);
        }
        let tree = ScriptedTree::in_repo(Self::ledger(dir), &origin);
        Self::open(dir, clock_ms, scenario, tree)
    }

    /// A second host on the same store and the same world, as another service would be.
    pub fn second_host(&self) -> (Host, Arc<ScriptedTree>) {
        let store = Arc::new(
            SqliteRunStore::open(&self.dir.join("run.sqlite"), HOST_BUSY_TIMEOUT)
                .expect("a second connection"),
        );
        let tree = Arc::new(ScriptedTree::new(Self::ledger(&self.dir)));
        let ports = Ports {
            messages: self.ports.clone(),
            declarations: self.ports.clone(),
            sidecar: self.ports.clone(),
            layout: Self::layout(&self.dir),
        };
        let host = Host::new(
            store,
            ports,
            self.backend.clone(),
            tree.clone(),
            self.clock.clone(),
        );
        (host, tree)
    }

    pub fn restart(&mut self) {
        self.restart_with(ScriptedTree::new(Self::ledger(&self.dir)));
    }

    pub fn requester() -> Caller {
        Caller {
            actor: REQUESTER.to_owned(),
            session: None,
        }
    }
}

impl<T: RigTree> Rig<T> {
    pub fn open(dir: &Path, clock_ms: i64, scenario: Scenario, tree: T) -> Self {
        fs::create_dir_all(dir).expect("the rig's directory");
        let store = Arc::new(
            SqliteRunStore::open(&dir.join("run.sqlite"), HOST_BUSY_TIMEOUT).expect("the store"),
        );
        let bridge_store = Arc::new(
            SqliteRunStore::open(&dir.join("run.sqlite"), bridge::BRIDGE_BUSY_TIMEOUT)
                .expect("the bridge's store"),
        );
        let clock = Arc::new(SimClock::at(clock_ms));
        let ports = Arc::new(FilePorts::new(&dir.join("messages.jsonl")));
        let layout = Rig::<ScriptedTree>::layout(dir);
        let backend = Arc::new(ScriptedBackend::new(
            &layout.agent_logs,
            &dir.join("timeline.jsonl"),
            &layout.v2_dir(),
            scenario,
        ));
        let tree = Arc::new(tree);
        let host_ports = Ports {
            messages: ports.clone(),
            declarations: ports.clone(),
            sidecar: ports.clone(),
            layout: layout.clone(),
        };
        let host = Host::new(
            store.clone(),
            host_ports,
            backend.clone(),
            tree.clone(),
            clock.clone(),
        );
        let bridge = BridgeCtx {
            store: bridge_store,
            messages: ports.clone(),
            declarations: ports.clone(),
            layout,
            workspace_id: WORKSPACE.to_owned(),
        };
        Self {
            dir: dir.to_owned(),
            store,
            clock,
            ports,
            backend,
            tree,
            host,
            bridge,
            policy: short_policy(),
            step_ms: 2_000,
        }
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// The service restarted: a new host on the same store and clock, with a new owner id and nothing in
    /// memory. It has to wait out the old lease, as a real restart does. The tree is the caller's: a real
    /// worker dies with its process.
    pub fn restart_with(&mut self, tree: T) {
        let store = Arc::new(
            SqliteRunStore::open(&self.dir.join("run.sqlite"), HOST_BUSY_TIMEOUT)
                .expect("the store reopens"),
        );
        let tree = Arc::new(tree);
        let ports = Ports {
            messages: self.ports.clone(),
            declarations: self.ports.clone(),
            sidecar: self.ports.clone(),
            layout: Rig::<ScriptedTree>::layout(&self.dir),
        };
        self.host = Host::new(
            store.clone(),
            ports,
            self.backend.clone(),
            tree.clone(),
            self.clock.clone(),
        );
        self.store = store;
        self.tree = tree;
    }

    pub fn commission(&self, body: &str) -> CommissionReceipt {
        self.commission_refs(body, &Refs::default())
    }

    pub fn commission_refs(&self, body: &str, refs: &Refs) -> CommissionReceipt {
        bridge::commission(
            &self.bridge,
            "coding",
            body,
            refs,
            REQUESTER,
            self.policy.clone(),
            self.now(),
        )
        .expect("the commission is accepted")
    }

    /// One tick, then the sessions, then the clock: unless the tree is working, because that is real time and
    /// a simulated window must not close around it.
    pub fn tick(&mut self) -> TickReport {
        let report = self.host.tick(self.now());
        self.backend.advance(&self.bridge, self.now());
        if self.tree.busy() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            self.clock.advance(self.step_ms);
        }
        report
    }

    pub fn is_finished(&self, run: &RunId) -> bool {
        !self
            .store
            .open_runs()
            .expect("the store reads")
            .iter()
            .any(|row| row.run == *run)
    }

    /// Ticks until `done`, or fails naming where the run stands.
    pub fn run_until(&mut self, max_ticks: u32, mut done: impl FnMut(&Self) -> bool) -> u32 {
        for tick in 0..max_ticks {
            if done(self) {
                return tick;
            }
            self.tick();
        }
        assert!(
            done(self),
            "not reached in {max_ticks} ticks: {:?}",
            self.kinds()
        );
        max_ticks
    }

    pub fn run_to_finished(&mut self, run: &RunId) -> u32 {
        let run = run.clone();
        self.run_until(20_000, move |rig| rig.is_finished(&run))
    }

    pub fn events(&self, run: &RunId) -> Vec<Event> {
        self.store.load(run, 0).expect("the log reads")
    }

    pub fn has(&self, run: &RunId, kind: &str) -> bool {
        self.events(run)
            .iter()
            .any(|event| event.body.kind() == kind)
    }

    /// The kinds of the newest open run's log, for a failure message.
    pub fn kinds(&self) -> Vec<String> {
        let rows = self.store.open_runs().unwrap_or_default();
        rows.last().map_or_else(Vec::new, |row| {
            self.events(&row.run)
                .iter()
                .map(|event| event.body.kind())
                .collect()
        })
    }

    pub fn state(&self, run: &RunId) -> nexus_run_engine::RunState {
        bridge::view(&self.bridge.store, run)
            .expect("the run reads")
            .expect("the run is known")
            .state
    }

    /// The session keys of a run the backend spawned, in order.
    pub fn spawned(&self) -> Vec<String> {
        self.backend
            .timeline()
            .into_iter()
            .filter(|entry| entry.what == "spawn")
            .map(|entry| entry.session_key)
            .collect()
    }
}

/// The scenario where every session replies and passes.
pub fn replies_pass(_: &SessionInfo) -> Cue {
    pass()
}

/// The crash box's scripts (§12.4), by name: what each session does.
pub fn scenario_named(name: &str) -> Scenario {
    match name {
        // The verifier sends the work back once.
        "rework" => Box::new(|info| {
            if info.step == "verify" && info.visit == 1 {
                Cue::Reply {
                    text: "not yet".to_owned(),
                    flags: ReplyFlags {
                        needs_rework: true,
                        escalate: false,
                    },
                }
            } else {
                pass()
            }
        }),
        // The coder leaves an uncommitted file and replies: the run delivers, and a park notice is owed.
        "leaves_work" => Box::new(|info| {
            if info.step == "build" {
                Cue::WriteFileThenPass {
                    name: "stray.txt".to_owned(),
                }
            } else {
                pass()
            }
        }),
        // The coder writes a file and works on, until it is withdrawn.
        "withdraw" => Box::new(|info| {
            if info.step == "build" {
                Cue::WriteFile {
                    name: "stray.txt".to_owned(),
                }
            } else {
                pass()
            }
        }),
        _ => Box::new(replies_pass),
    }
}

impl<T: RigTree> Rig<T> {
    pub fn clock_millis(&self) -> i64 {
        self.now().unix_millis()
    }
}

/// The file a test opens a second connection on, to do what no host would.
pub fn store_path(dir: &Path) -> PathBuf {
    dir.join("run.sqlite")
}

/// What a test asks of the store beside the contract.
pub trait StoreProbe {
    /// The run holding this workspace's claim.
    fn get_claim_holder(&self) -> Option<String>;
}

impl StoreProbe for SqliteRunStore {
    fn get_claim_holder(&self) -> Option<String> {
        self.get(&format!("claim/{WORKSPACE}"))
            .ok()
            .flatten()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }
}
