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
//! `GitTree` is a [`TreeRunner`] over this worker; `ScriptedTree` is the other.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nexus_run_contracts::{
    Base, Epoch, GitReason, Outcome, ParkOutcome, RunId, RunStore, StoreError, Timestamp,
    TreeRefusal,
};
use nexus_run_store_sqlite::{Lease, SqliteRunStore};

use crate::boundary::{crash_at, Boundary};
use crate::ports::Clock;
use crate::tree_child::{
    clear_locks_of_a_gone_group, gated, git_locks, group_gone_soon, own_process_group, Output,
    DRAIN_GRACE,
};
use crate::tree_ledger::{LedgerEntry, TreeLedger};
use nexus_run_session_claude::procs::{ProcessGuard, SystemProcesses};

/// The overall deadline of one operation, beside the bound on each of its calls. A clone's own bound is
/// 10 min, a fetch's 2 min, and every other call is 60 s.
pub const OPERATION_DEADLINE: Duration = Duration::from_secs(15 * 60);

const WAIT_STEP: Duration = Duration::from_millis(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeOp {
    Prepare {
        run: RunId,
        dir: PathBuf,
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

    pub fn dir(&self) -> &Path {
        match self {
            Self::Prepare { dir, .. } | Self::Park { dir, .. } => dir,
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

/// What ended an operation early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeStop {
    Cancelled,
    DeadlinePassed,
    LeaseLost,
    SpawnFailed,
    /// The child ended and a helper of its still holds a pipe: what it said is not known.
    OutputUncollected,
}

impl TreeStop {
    pub fn into_result(self) -> TreeResult {
        let (reason, text) = match self {
            Self::Cancelled => (GitReason::BoundExceeded, "the operation was cancelled"),
            Self::DeadlinePassed => (GitReason::BoundExceeded, "the operation's deadline passed"),
            Self::LeaseLost => (GitReason::BoundExceeded, "the owner's lease moved on"),
            Self::SpawnFailed => (GitReason::ExitNonzero, "a child could not be started"),
            Self::OutputUncollected => (
                GitReason::ExitNonzero,
                "a child's output was not collected: a helper of it still holds a pipe",
            ),
        };
        TreeResult::Refused {
            reason: TreeRefusal::Git { reason },
            text: text.to_owned(),
        }
    }
}

const BEGIN_STEP: &str = "begin";

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

/// What the ledger says of `.git` before one child.
struct LocksBefore {
    index: bool,
    all: Vec<String>,
}

impl LocksBefore {
    fn of(dir: &Path) -> Self {
        Self {
            index: dir.join(".git").join("index.lock").exists(),
            all: git_locks(dir),
        }
    }
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

    /// Now, by the clock the operation runs on.
    pub fn now(&self) -> Timestamp {
        self.binding.clock.now()
    }

    fn entry(&self, step: &str, child: Option<(u32, String)>, locks: &LocksBefore) -> LedgerEntry {
        let (pid, start_time) = child.unzip();
        LedgerEntry {
            spawning: step != BEGIN_STEP,
            run: self.op.run().clone(),
            op: self.op.name().to_owned(),
            step: step.to_owned(),
            pid,
            start_time,
            began_at: self.began_at,
            owner_id: self.binding.owner_id.clone(),
            epoch: self.epoch,
            lock_existed_before: locks.index,
            locks_before: locks.all.clone(),
        }
    }

    /// Runs one child in its own process group, recorded in the ledger before it is spawned and again before it
    /// may start (the gate), killed by group when `bound`, the deadline or a cancel passes. `dir` is the repository
    /// whose locks this child answers for.
    pub fn spawn(
        &mut self,
        step: &str,
        command: Command,
        bound: Duration,
        dir: &Path,
    ) -> Result<StepOutput, TreeStop> {
        self.checkpoint()?;
        if !self.lease_is_ours() {
            return Err(TreeStop::LeaseLost);
        }
        let locks = LocksBefore::of(dir);
        crash_at(Boundary::BeforeTreeLedger);
        let ledger = &self.shared.ledger;
        // A record of the last child must not name this one.
        let _ = std::fs::remove_file(ledger.child_record());
        ledger
            .write(&self.entry(step, None, &locks))
            .map_err(|_| TreeStop::SpawnFailed)?;
        let mut command = gated(command, &ledger.child_record());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        own_process_group(&mut command);
        let mut child = command.spawn().map_err(|_| TreeStop::SpawnFailed)?;
        crash_at(Boundary::AfterGitSpawnBeforeRecord);
        // Dropped without a line, on any way out of here, this ends the child before it runs the program.
        let gate = child.stdin.take();
        let pid = child.id();
        let Some(start_time) = self.shared.guard.start_time(pid) else {
            // Nobody could kill this group later. Its program never started, so the gate closing ends it.
            drop(gate);
            let _ = child.wait();
            return Err(TreeStop::SpawnFailed);
        };
        self.child = Some((pid, start_time.clone()));
        let output = Output::collect(&mut child);
        let recorded = ledger.write(&self.entry(step, Some((pid, start_time)), &locks));
        if recorded.is_err() || !open_gate(gate) {
            return Err(self.end_child(&mut child, TreeStop::SpawnFailed, dir, &locks));
        }
        crash_at(Boundary::AfterGitReleasedBeforeResult);
        let stopped = self.wait_for_child(&mut child, output, bound, dir, &locks);
        self.child = None;
        stopped
    }

    fn wait_for_child(
        &mut self,
        child: &mut Child,
        output: Output,
        bound: Duration,
        dir: &Path,
        locks: &LocksBefore,
    ) -> Result<StepOutput, TreeStop> {
        let until = self.instant_after(bound);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return if output.wait_for_end(DRAIN_GRACE) {
                        Ok(output.into_step(status.success()))
                    } else {
                        Err(self.end_child(child, TreeStop::OutputUncollected, dir, locks))
                    };
                }
                Ok(None) => {}
                Err(_) => return Err(self.end_child(child, TreeStop::SpawnFailed, dir, locks)),
            }
            if let Err(stop) = self.checkpoint() {
                return Err(self.end_child(child, stop, dir, locks));
            }
            if self.binding.clock.now() >= until {
                return Err(self.end_child(child, TreeStop::DeadlinePassed, dir, locks));
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// Ends the group, and answers for its locks there and then: the ledger entry that could prove them goes with
    /// the operation. Whatever the group said by then does not count, because the stop is the answer, and a reader
    /// still blocked on a pipe an escaped helper holds is left to that helper.
    fn end_child(
        &mut self,
        child: &mut Child,
        stop: TreeStop,
        dir: &Path,
        locks: &LocksBefore,
    ) -> TreeStop {
        let guard = self.shared.guard.as_ref();
        if let Some((pid, start_time)) = self.child.take() {
            // A leader already ended and reaped cannot vouch for its group, so what is left goes member by member.
            if !guard.kill_group(pid, &start_time) {
                guard.kill_members(pid, &start_time);
            }
        }
        // Our own child: reaped, or it answers alive as a zombie (§8.3).
        let _ = child.kill();
        let _ = child.wait();
        if group_gone_soon(guard, child.id()) {
            clear_locks_of_a_gone_group(dir, &locks.all, locks.index);
        }
        stop
    }
}

/// Lets the program start. `false` when the child is already gone.
fn open_gate(gate: Option<ChildStdin>) -> bool {
    gate.is_some_and(|mut gate| gate.write_all(b"go\n").is_ok())
}

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
    fn settle_ledger(&self, binding: &TreeBinding, op: &TreeOp) -> Result<(), InFlight> {
        let Some(entry) = self.shared.ledger.read() else {
            return Ok(());
        };
        if entry.owner_id != binding.owner_id && owner_is_live(&entry, binding) {
            return Err(InFlight);
        }
        let mut child_of_the_entry = None;
        match (entry.pid, entry.start_time.as_deref()) {
            (Some(pid), Some(start_time)) => {
                self.shared.guard.kill_group(pid, start_time);
                child_of_the_entry = Some(pid);
            }
            _ => {
                // The parent died before it recorded the child: the child's own record names it. Until the
                // grace is over the child may still be on its way to writing that.
                let own = self.shared.ledger.self_recorded_pid();
                if own.is_none() && entry.spawning && self.shared.ledger.within_spawn_grace() {
                    return Err(InFlight);
                }
                if let Some(pid) = own {
                    if let Some(start_time) = self.shared.guard.start_time(pid) {
                        self.shared.guard.kill_group(pid, &start_time);
                    }
                    child_of_the_entry = Some(pid);
                }
            }
        }
        // A lock this call can prove the child made (§5): not there before it, and its group gone. A group that is
        // not gone keeps its locks, and whoever comes next reads `lock_held` naming them.
        if let Some(pid) = child_of_the_entry {
            if group_gone_soon(self.shared.guard.as_ref(), pid) {
                clear_locks_of_a_gone_group(
                    op.dir(),
                    &entry.locks_before,
                    entry.lock_existed_before,
                );
            }
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
    let no_locks = LocksBefore {
        index: false,
        all: Vec::new(),
    };
    if shared
        .ledger
        .write(&cx.entry(BEGIN_STEP, None, &no_locks))
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
        self.settle_ledger(&binding, &op)?;
        let jobs = self.sender().ok_or(InFlight)?;
        self.shared.cancel.store(false, Ordering::SeqCst);
        // Busy only once it is really handed over: the guard held here keeps the worker from finishing first.
        jobs.send(op.clone()).map_err(|_| InFlight)?;
        *in_flight = Some(op);
        Ok(())
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

#[cfg(test)]
mod tests;
