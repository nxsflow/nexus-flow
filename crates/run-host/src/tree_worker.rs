//! How a tree operation runs: never on the tick (§6.6). `Host::tick` runs on the service's one thread and
//! a `git clone` over HTTPS has no bound of its own, so the tick only starts and polls, and one worker per
//! workspace runs the operation as children in their own process groups.
//!
//! The tree itself is the ledger for WHAT is done; the file `<agent-logs>/v2/tree/<workspace_id>.json` is
//! the ledger for WHO is doing it. It is written before each spawn, with the owner and epoch the operation
//! runs under, and read at every `begin`: a recorded child is killed only when its owner's lease is lost or
//! when it is this host's own, else the answer is [`InFlight`]. What a run still owes after a restart comes
//! from `pending_effects`, never from this file.
//!
//! `GitTree` (ticket j6ba) is a [`TreeRunner`] over this worker; `ScriptedTree` is the other.

use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nexus_run_contracts::status::InFlight as InFlightStatus;
use nexus_run_contracts::{
    Base, Epoch, GitReason, Outcome, ParkOutcome, RunId, RunStore, StoreError, Timestamp,
    TreeRefusal,
};
use nexus_run_store_sqlite::{Lease, SqliteRunStore};
use serde::{Deserialize, Serialize};

use crate::boundary::{crash_at, Boundary};
use crate::ports::Clock;
use crate::session_stub::{ProcessGuard, SystemProcesses};

/// The overall deadline of one operation, beside the bound on each of its calls. A clone's own bound is
/// 10 min, a fetch's 2 min, and every other call is 60 s.
pub const OPERATION_DEADLINE: Duration = Duration::from_secs(15 * 60);

const WAIT_STEP: Duration = Duration::from_millis(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeOp {
    Prepare {
        run: RunId,
        dir: PathBuf,
        origin_url: String,
    },
    Park {
        run: RunId,
        dir: PathBuf,
        occasion: Outcome,
    },
}

impl TreeOp {
    pub fn run(&self) -> &RunId {
        match self {
            Self::Prepare { run, .. } | Self::Park { run, .. } => run,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Prepare { .. } => "prepare",
            Self::Park { .. } => "park",
        }
    }
}

/// The closed reason goes to the log; `text`, the command and git's stderr, capped and scrubbed, goes to
/// `kv['tree_refusal/<run>']` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeResult {
    Prepared {
        base: Base,
        found: Option<ParkOutcome>,
    },
    Parked(ParkOutcome),
    Refused {
        reason: TreeRefusal,
        text: String,
    },
}

/// An operation is already running: the effect stays owed and nothing is started twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InFlight;

/// The v2 working directory as a port. Not synchronous: `begin` hands an operation to the worker and
/// returns at once, and a later `poll` hands the finished one back.
pub trait WorkTree: Send + Sync {
    fn begin(&self, op: TreeOp) -> Result<(), InFlight>;
    fn poll(&self) -> Option<(TreeOp, TreeResult)>;
    fn cancel(&self);
    /// Tells the tree whose lease it works under. `Host::new` calls it.
    fn bind(&self, _binding: TreeBinding) {}
}

/// Reads a run's lease: the owner and epoch a ledger entry is checked against.
pub trait LeaseView: Send + Sync {
    fn lease(&self, run: &RunId) -> Result<Option<Lease>, StoreError>;
}

impl LeaseView for SqliteRunStore {
    fn lease(&self, run: &RunId) -> Result<Option<Lease>, StoreError> {
        let key = format!("owner/{run}");
        let Some(bytes) = self.get(&key)? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| StoreError::DamagedRow { key, seq: None })
    }
}

#[derive(Clone)]
pub struct TreeBinding {
    pub owner_id: String,
    pub leases: Arc<dyn LeaseView>,
    pub clock: Arc<dyn Clock>,
}

/// One entry of the ledger file. `pid` and `start_time` are absent until the child exists: a crash in that
/// gap leaves a bounded child nobody can name, and the next prepare reads the tree before it acts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub run: RunId,
    pub op: String,
    pub step: String,
    pub pid: Option<u32>,
    pub start_time: Option<String>,
    pub began_at: Timestamp,
    pub owner_id: String,
    pub epoch: Epoch,
    /// Whether `index.lock` was there before the child: only a lock a call can prove it created is cleared.
    pub lock_existed_before: bool,
}

/// The file `<agent-logs>/v2/tree/<workspace_id>.json`.
#[derive(Clone, Debug)]
pub struct TreeLedger {
    path: PathBuf,
}

impl TreeLedger {
    pub fn new(agent_logs: &Path, workspace_id: &str) -> Self {
        Self {
            path: agent_logs
                .join("v2")
                .join("tree")
                .join(format!("{workspace_id}.json")),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `None` for a file that is gone, and for one that does not read: a torn entry names nobody to kill.
    pub fn read(&self) -> Option<LedgerEntry> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// By temp file and rename, so a reader never meets half an entry.
    pub fn write(&self, entry: &LedgerEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, serde_json::to_vec(entry)?)?;
        fs::rename(&temp, &self.path)
    }

    pub fn delete(&self) {
        // A file already gone is the wanted state.
        let _ = fs::remove_file(&self.path);
    }

    /// Deletes the entry only if `owner_id` wrote it: a worker that lost its run finishes after the new
    /// owner's worker has written, and that entry is not its to remove.
    fn delete_own(&self, owner_id: &str) {
        if self.read().is_some_and(|entry| entry.owner_id == owner_id) {
            self.delete();
        }
    }

    /// `tree.in_flight` of status: the entry with its pid checked, a dead child reading as none.
    pub fn in_flight(&self, guard: &dyn ProcessGuard) -> Option<InFlightStatus> {
        let entry = self.read()?;
        let (pid, start_time) = (entry.pid?, entry.start_time?);
        guard.is_alive(pid, &start_time).then_some(InFlightStatus {
            op: entry.op,
            step: entry.step,
            since: entry.began_at,
        })
    }
}

/// What the status read of a directory answers (§5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeState {
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
    pub mid_sequence: bool,
    pub missing: bool,
}

/// Why the status read of a directory has no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeStateError {
    pub what: String,
}

/// The one synchronous call, run by the bridge in the CLI process, never by the tick. The skeleton knows
/// whether the directory is there; the git half is ticket j6ba's.
pub fn tree_state(dir: &Path) -> Result<TreeState, TreeStateError> {
    if !dir.exists() {
        return Ok(TreeState {
            missing: true,
            ..TreeState::default()
        });
    }
    Err(TreeStateError {
        what: "reading a directory's git state arrives with GitTree".to_owned(),
    })
}

/// What ended an operation early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeStop {
    Cancelled,
    DeadlinePassed,
    LeaseLost,
    SpawnFailed,
}

impl TreeStop {
    pub fn into_result(self) -> TreeResult {
        let (reason, text) = match self {
            Self::Cancelled => (GitReason::BoundExceeded, "the operation was cancelled"),
            Self::DeadlinePassed => (GitReason::BoundExceeded, "the operation's deadline passed"),
            Self::LeaseLost => (GitReason::BoundExceeded, "the owner's lease moved on"),
            Self::SpawnFailed => (GitReason::ExitNonzero, "a child could not be started"),
        };
        TreeResult::Refused {
            reason: TreeRefusal::Git { reason },
            text: text.to_owned(),
        }
    }
}

/// What one child left behind, as bytes: output is never decoded lossily (6j6v.2af2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The sequence an operation runs, on the worker's thread.
pub trait TreeRunner: Send + 'static {
    fn run(&mut self, op: &TreeOp, cx: &mut TreeCx<'_>) -> TreeResult;
}

/// What a runner reaches the world through, so every child is recorded, bounded and killable.
pub struct TreeCx<'a> {
    op: &'a TreeOp,
    shared: &'a Shared,
    binding: &'a TreeBinding,
    epoch: Epoch,
    began_at: Timestamp,
    deadline: Timestamp,
    /// The running child, killed if the runner panics.
    child: Option<(u32, String)>,
}

impl TreeCx<'_> {
    /// Whether the operation should go on. A runner asks between its steps.
    pub fn checkpoint(&self) -> Result<(), TreeStop> {
        if self.shared.cancel.load(Ordering::SeqCst) {
            return Err(TreeStop::Cancelled);
        }
        if self.binding.clock.now() >= self.deadline {
            return Err(TreeStop::DeadlinePassed);
        }
        Ok(())
    }

    /// Blocks for `span` of the injected clock, or until the operation is stopped.
    pub fn wait(&self, span: Duration) -> Result<(), TreeStop> {
        let until = self.instant_after(span);
        while self.binding.clock.now() < until {
            self.checkpoint()?;
            std::thread::sleep(WAIT_STEP);
        }
        Ok(())
    }

    fn instant_after(&self, span: Duration) -> Timestamp {
        let millis = i64::try_from(span.as_millis()).unwrap_or(i64::MAX);
        Timestamp::from_unix_millis(
            self.binding
                .clock
                .now()
                .unix_millis()
                .saturating_add(millis),
        )
    }

    /// The lease still names this owner at this epoch. A worker whose host lost the run starts no child.
    fn lease_is_ours(&self) -> bool {
        match self.binding.leases.lease(self.op.run()) {
            Ok(Some(lease)) => lease.owner_id == self.binding.owner_id && lease.epoch == self.epoch,
            // A store it cannot read right now is not a lost lease; the next call asks again.
            Err(_) => true,
            Ok(None) => false,
        }
    }

    fn entry(&self, step: &str, child: Option<(u32, String)>, lock_existed: bool) -> LedgerEntry {
        let (pid, start_time) = child.unzip();
        LedgerEntry {
            run: self.op.run().clone(),
            op: self.op.name().to_owned(),
            step: step.to_owned(),
            pid,
            start_time,
            began_at: self.began_at,
            owner_id: self.binding.owner_id.clone(),
            epoch: self.epoch,
            lock_existed_before: lock_existed,
        }
    }

    /// Runs one child in its own process group, recorded in the ledger before it is spawned, killed by
    /// group when `bound`, the deadline or a cancel passes.
    pub fn spawn(
        &mut self,
        step: &str,
        mut command: Command,
        bound: Duration,
        dir: &Path,
    ) -> Result<StepOutput, TreeStop> {
        self.checkpoint()?;
        if !self.lease_is_ours() {
            return Err(TreeStop::LeaseLost);
        }
        let lock_existed = dir.join(".git").join("index.lock").exists();
        crash_at(Boundary::BeforeTreeLedger);
        let ledger = &self.shared.ledger;
        ledger
            .write(&self.entry(step, None, lock_existed))
            .map_err(|_| TreeStop::SpawnFailed)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        own_process_group(&mut command);
        let mut child = command.spawn().map_err(|_| TreeStop::SpawnFailed)?;
        crash_at(Boundary::AfterGitSpawnBeforeRecord);
        let pid = child.id();
        let start_time = self.shared.guard.start_time(pid).unwrap_or_default();
        self.child = Some((pid, start_time.clone()));
        ledger
            .write(&self.entry(step, Some((pid, start_time)), lock_existed))
            .map_err(|_| TreeStop::SpawnFailed)?;
        let stopped = self.wait_for_child(&mut child, bound);
        self.child = None;
        stopped
    }

    fn wait_for_child(
        &mut self,
        child: &mut Child,
        bound: Duration,
    ) -> Result<StepOutput, TreeStop> {
        let stdout = read_all(child.stdout.take());
        let stderr = read_all(child.stderr.take());
        let until = self.instant_after(bound);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(StepOutput {
                        success: status.success(),
                        stdout: stdout.join().unwrap_or_default(),
                        stderr: stderr.join().unwrap_or_default(),
                    });
                }
                Ok(None) => {}
                Err(_) => return Err(self.end_child(child, TreeStop::SpawnFailed)),
            }
            if let Err(stop) = self.checkpoint() {
                return Err(self.end_child(child, stop));
            }
            if self.binding.clock.now() >= until {
                return Err(self.end_child(child, TreeStop::DeadlinePassed));
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    fn end_child(&mut self, child: &mut Child, stop: TreeStop) -> TreeStop {
        if let Some((pid, start_time)) = &self.child {
            self.shared.guard.kill_group(*pid, start_time);
        }
        // Our own child: reaped, or it answers alive as a zombie (§8.3).
        let _ = child.kill();
        let _ = child.wait();
        stop
    }
}

fn read_all<R: Read + Send + 'static>(source: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut source) = source {
            // What arrived before a read error is still what the child said.
            let _ = source.read_to_end(&mut bytes);
        }
        bytes
    })
}

#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn own_process_group(_command: &mut Command) {}

/// What the tick and the worker's thread share.
pub struct Shared {
    ledger: TreeLedger,
    guard: Arc<dyn ProcessGuard>,
    deadline: Duration,
    cancel: AtomicBool,
    in_flight: Mutex<Option<TreeOp>>,
    results: Mutex<VecDeque<(TreeOp, TreeResult)>>,
    binding: Mutex<Option<TreeBinding>>,
    started: AtomicUsize,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One operation in flight per workspace, on a thread started by the first `begin`, not per registered
/// workspace, so an idle workspace costs no thread.
pub struct TreeWorker<R: TreeRunner> {
    shared: Arc<Shared>,
    runner: Mutex<Option<R>>,
    jobs: Mutex<Option<Sender<TreeOp>>>,
}

impl<R: TreeRunner> TreeWorker<R> {
    pub fn new(runner: R, ledger: TreeLedger) -> Self {
        Self::with_guard(
            runner,
            ledger,
            Arc::new(SystemProcesses),
            OPERATION_DEADLINE,
        )
    }

    pub fn with_guard(
        runner: R,
        ledger: TreeLedger,
        guard: Arc<dyn ProcessGuard>,
        deadline: Duration,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                ledger,
                guard,
                deadline,
                cancel: AtomicBool::new(false),
                in_flight: Mutex::new(None),
                results: Mutex::new(VecDeque::new()),
                binding: Mutex::new(None),
                started: AtomicUsize::new(0),
            }),
            runner: Mutex::new(Some(runner)),
            jobs: Mutex::new(None),
        }
    }

    /// How many worker threads this tree ever started: zero until the first `begin`, and one after it.
    pub fn workers_started(&self) -> usize {
        self.shared.started.load(Ordering::SeqCst)
    }

    /// Whether an operation is running or its result waits for `poll`.
    pub fn busy(&self) -> bool {
        locked(&self.shared.in_flight).is_some() || !locked(&self.shared.results).is_empty()
    }

    pub fn ledger(&self) -> &TreeLedger {
        &self.shared.ledger
    }

    /// The ledger read of `begin`: what a recorded entry means for this host.
    fn settle_ledger(&self, binding: &TreeBinding) -> Result<(), InFlight> {
        let Some(entry) = self.shared.ledger.read() else {
            return Ok(());
        };
        if entry.owner_id != binding.owner_id && owner_is_live(&entry, binding) {
            return Err(InFlight);
        }
        if let (Some(pid), Some(start_time)) = (entry.pid, entry.start_time.as_deref()) {
            self.shared.guard.kill_group(pid, start_time);
        }
        self.shared.ledger.delete();
        Ok(())
    }

    fn sender(&self) -> Option<Sender<TreeOp>> {
        let mut jobs = locked(&self.jobs);
        if jobs.is_none() {
            let mut runner = locked(&self.runner).take()?;
            let (send, receive) = channel::<TreeOp>();
            let shared = Arc::clone(&self.shared);
            shared.started.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                while let Ok(op) = receive.recv() {
                    run_one(&shared, &mut runner, op);
                }
            });
            *jobs = Some(send);
        }
        jobs.clone()
    }
}

/// The recorded owner still holds the run: its lease names it, at that epoch, unexpired. A lease that
/// cannot be read right now counts as held, because killing on a doubt is the wrong way round.
fn owner_is_live(entry: &LedgerEntry, binding: &TreeBinding) -> bool {
    match binding.leases.lease(&entry.run) {
        Ok(Some(lease)) => {
            lease.owner_id == entry.owner_id
                && lease.epoch == entry.epoch
                && lease.expires_at > binding.clock.now()
        }
        Ok(None) => false,
        Err(_) => true,
    }
}

fn run_one<R: TreeRunner>(shared: &Arc<Shared>, runner: &mut R, op: TreeOp) {
    let binding = locked(&shared.binding).clone();
    let result = match &binding {
        Some(binding) => {
            let result = run_bound(shared, runner, &op, binding);
            shared.ledger.delete_own(&binding.owner_id);
            result
        }
        None => TreeStop::LeaseLost.into_result(),
    };
    shared.cancel.store(false, Ordering::SeqCst);
    // In this order: whoever polls this result may begin the next operation at once.
    *locked(&shared.in_flight) = None;
    locked(&shared.results).push_back((op, result));
}

fn run_bound<R: TreeRunner>(
    shared: &Arc<Shared>,
    runner: &mut R,
    op: &TreeOp,
    binding: &TreeBinding,
) -> TreeResult {
    let began_at = binding.clock.now();
    let millis = i64::try_from(shared.deadline.as_millis()).unwrap_or(i64::MAX);
    let epoch = match binding.leases.lease(op.run()) {
        Ok(Some(lease)) if lease.owner_id == binding.owner_id => lease.epoch,
        _ => return TreeStop::LeaseLost.into_result(),
    };
    let mut cx = TreeCx {
        op,
        shared: shared.as_ref(),
        binding,
        epoch,
        began_at,
        deadline: Timestamp::from_unix_millis(began_at.unix_millis().saturating_add(millis)),
        child: None,
    };
    crash_at(Boundary::BeforeTreeLedger);
    if shared
        .ledger
        .write(&cx.entry("begin", None, false))
        .is_err()
    {
        return TreeStop::SpawnFailed.into_result();
    }
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner.run(op, &mut cx)));
    outcome.unwrap_or_else(|_| {
        // A panicked worker's child never outlives it; the refusal is what gets counted.
        if let Some((pid, start_time)) = cx.child.take() {
            shared.guard.kill_group(pid, &start_time);
        }
        TreeResult::Refused {
            reason: TreeRefusal::Git {
                reason: GitReason::ExitNonzero,
            },
            text: "the tree worker panicked".to_owned(),
        }
    })
}

impl<R: TreeRunner> WorkTree for TreeWorker<R> {
    fn begin(&self, op: TreeOp) -> Result<(), InFlight> {
        let binding = locked(&self.shared.binding).clone().ok_or(InFlight)?;
        let mut in_flight = locked(&self.shared.in_flight);
        if in_flight.is_some() {
            return Err(InFlight);
        }
        self.settle_ledger(&binding)?;
        let jobs = self.sender().ok_or(InFlight)?;
        *in_flight = Some(op.clone());
        self.shared.cancel.store(false, Ordering::SeqCst);
        jobs.send(op).map_err(|_| InFlight)
    }

    fn poll(&self) -> Option<(TreeOp, TreeResult)> {
        locked(&self.shared.results).pop_front()
    }

    fn cancel(&self) {
        if locked(&self.shared.in_flight).is_some() {
            self.shared.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn bind(&self, binding: TreeBinding) {
        *locked(&self.shared.binding) = Some(binding);
    }
}
