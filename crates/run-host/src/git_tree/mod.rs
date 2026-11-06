//! The v2 working directory made real (§5, §6.6): `<workspace root>-v2`, this workspace's own clone beside the
//! registered root, so the two engines share no tree and no claim. `GitTree` is a [`TreeRunner`] over the tree worker:
//! every git call is a bounded, pipe-gated child in its own process group, recorded in the worker's ledger.
//!
//! Every step of an operation reads the tree before it acts, so a crash anywhere re-runs from the first: the tree is
//! the ledger for WHAT is done.

mod park;
mod prepare;
mod runner;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nexus_run_contracts::TreeRefusal;

use crate::tree_child::run_bounded;
use crate::tree_ledger::TreeLedger;
use crate::tree_worker::{
    InFlight, TreeBinding, TreeCx, TreeOp, TreeResult, TreeRunner, TreeState, TreeStateError,
    TreeWorker, WorkTree,
};
use nexus_run_session_claude::procs::{ProcessGuard, SystemProcesses};

pub use runner::{normalise_origin_url, scrub};

/// The status read's bound, on all of its calls together (§5).
const STATE_BOUND: Duration = Duration::from_secs(10);

/// How long one git call may take (§5). A test injects shorter ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitBounds {
    pub clone: Duration,
    pub fetch: Duration,
    pub other: Duration,
    /// A park's `add` and `commit`: a large untracked directory takes as long as it takes, and a refusal there would
    /// repeat on the next attempt and end no withdrawal. Inside the operation's own deadline.
    pub park: Duration,
}

impl Default for GitBounds {
    fn default() -> Self {
        Self {
            clone: Duration::from_secs(10 * 60),
            fetch: Duration::from_secs(2 * 60),
            other: Duration::from_secs(60),
            park: Duration::from_secs(10 * 60),
        }
    }
}

/// What ended an operation early: the worker, or a refusal with its text for the kv row.
pub(crate) enum Stopped {
    Worker(crate::tree_worker::TreeStop),
    Refused(TreeRefusal, String),
}

impl From<crate::tree_worker::TreeStop> for Stopped {
    fn from(stop: crate::tree_worker::TreeStop) -> Self {
        Self::Worker(stop)
    }
}

struct GitRunner {
    root: PathBuf,
    git: PathBuf,
    bounds: GitBounds,
}

impl TreeRunner for GitRunner {
    fn run(&mut self, op: &TreeOp, cx: &mut TreeCx<'_>) -> TreeResult {
        let mut git = runner::Git::new(&self.git, op.dir(), self.bounds, cx);
        let done = match op {
            TreeOp::Prepare { run, dir } => prepare::prepare(&mut git, &self.root, run, dir),
            TreeOp::Park { run, dir, occasion } => {
                park::park(&mut git, &self.root, run, dir, occasion.as_str())
                    .map(TreeResult::Parked)
            }
        };
        done.unwrap_or_else(|stopped| match stopped {
            Stopped::Worker(stop) => stop.into_result(),
            Stopped::Refused(reason, text) => TreeResult::Refused {
                reason,
                text: scrub(&text),
            },
        })
    }
}

/// The directory, as a port. One operation in flight; the tick only starts and polls.
pub struct GitTree {
    worker: TreeWorker<GitRunner>,
}

impl GitTree {
    pub fn new(root: &Path, ledger: TreeLedger, bounds: GitBounds) -> Self {
        Self::over(root, ledger, bounds, Path::new("git"))
    }

    /// With another program for `git`: how a test puts a hung, lock-holding or byte-spewing git under the real
    /// runner.
    #[cfg(feature = "test-support")]
    pub fn with_git(root: &Path, ledger: TreeLedger, bounds: GitBounds, git: &Path) -> Self {
        Self::over(root, ledger, bounds, git)
    }

    fn over(root: &Path, ledger: TreeLedger, bounds: GitBounds, git: &Path) -> Self {
        let runner = GitRunner {
            root: root.to_owned(),
            git: git.to_owned(),
            bounds,
        };
        Self {
            worker: TreeWorker::new(runner, ledger),
        }
    }

    /// Whether an operation is running or its result waits for `poll`.
    pub fn busy(&self) -> bool {
        self.worker.busy()
    }

    pub fn ledger(&self) -> &TreeLedger {
        self.worker.ledger()
    }
}

impl WorkTree for GitTree {
    fn begin(&self, op: TreeOp) -> Result<(), InFlight> {
        self.worker.begin(op)
    }

    fn poll(&self) -> Option<(TreeOp, TreeResult)> {
        self.worker.poll()
    }

    fn cancel(&self) {
        self.worker.cancel();
    }

    fn bind(&self, binding: TreeBinding) {
        self.worker.bind(binding);
    }
}

/// The one synchronous read of a directory, run by the bridge in the CLI process and never by the tick (§5): bounded
/// at 10 s, `GIT_OPTIONAL_LOCKS=0`. `tree.in_flight` is not here: the CLI process reads it from the worker's ledger
/// file, with the pid checked ([`TreeLedger::in_flight`]).
pub fn tree_state(root: &Path, dir: &Path) -> Result<TreeState, TreeStateError> {
    read_state(Path::new("git"), root, dir)
}

/// [`tree_state`] with another program for `git`.
#[cfg(feature = "test-support")]
pub fn tree_state_with_git(
    git: &Path,
    root: &Path,
    dir: &Path,
) -> Result<TreeState, TreeStateError> {
    read_state(git, root, dir)
}

fn read_state(git: &Path, root: &Path, dir: &Path) -> Result<TreeState, TreeStateError> {
    if !exists(dir) {
        // An unmounted volume is no missing directory: `missing` releases a claim and starts a clone.
        return if prepare::parent_and_root_exist(root, dir) {
            Ok(TreeState {
                missing: true,
                ..TreeState::default()
            })
        } else {
            Err(TreeStateError {
                what: "the directory's parent or the root is not there".to_owned(),
            })
        };
    }
    let until = Instant::now() + STATE_BOUND;
    let call = |args: &[&str]| {
        let left = until.saturating_duration_since(Instant::now());
        let guard: &dyn ProcessGuard = &SystemProcesses;
        run_bounded(runner::command(git, dir, args), left, guard)
            .map_err(|what| TreeStateError { what })
    };
    let head = call(&["symbolic-ref", "-q", "--short", "HEAD"])?;
    let commit = call(&["rev-parse", "-q", "--verify", "HEAD"])?;
    let status = call(&["status", "--porcelain"])?;
    if !status.success {
        return Err(TreeStateError {
            what: scrub(&String::from_utf8_lossy(&status.stderr)),
        });
    }
    Ok(TreeState {
        commit: commit
            .success
            .then(|| String::from_utf8_lossy(&commit.stdout).trim().to_owned()),
        branch: head
            .success
            .then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned()),
        // The bytes decide: anything at all is dirty, whatever it decodes to (6j6v.2af2).
        dirty: !status.stdout.is_empty(),
        mid_sequence: runner::sequence_in_progress(dir).is_some(),
        missing: false,
    })
}

/// There, even as a link that resolves nowhere.
pub(crate) fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[cfg(test)]
mod tests;
