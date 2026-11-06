//! A tree scripted like the backend (§5, §12.4). It runs ON the real worker, so the ledger, the deadline and
//! `cancel` are the code under test. It can answer from a script, or work in a real temporary git repository,
//! which is what the crash harness asserts on: the park below has the shape of §6.6, and `GitTree` with its
//! runner requirements is ticket j6ba's.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nexus_run_contracts::{Base, GitReason, ParkOutcome, RunId, TreeRefusal};

use crate::boundary::{crash_at, Boundary};
use crate::session_stub::{ProcessGuard, SystemProcesses};
use crate::tree_ledger::TreeLedger;
use crate::tree_worker::{
    InFlight, StepOutput, TreeBinding, TreeCx, TreeOp, TreeResult, TreeRunner, TreeStop,
    TreeWorker, WorkTree, OPERATION_DEADLINE,
};

const BOUND: Duration = Duration::from_secs(120);

/// One thing a scripted operation does.
#[derive(Clone, Debug)]
pub enum TreeStep {
    /// Blocks for this long on the injected clock.
    Delay(Duration),
    /// Runs a real child in its own process group, under the worker's ledger.
    Child {
        program: String,
        args: Vec<String>,
    },
    Panic,
    Answer(TreeResult),
}

enum Mode {
    /// Answers from a script per operation, then by default.
    Scripted(Arc<Mutex<VecDeque<Vec<TreeStep>>>>),
    /// Works in a real git repository cloned from this origin.
    Repo { origin: PathBuf },
}

struct ScriptedRunner {
    mode: Mode,
}

fn default_result(op: &TreeOp) -> TreeResult {
    match op {
        TreeOp::Prepare { .. } => TreeResult::Prepared {
            base: Base {
                branch: "main".to_owned(),
                commit: "c0".to_owned(),
            },
            found: None,
        },
        TreeOp::Park { .. } => TreeResult::Parked(ParkOutcome::Clean { branch: None }),
    }
}

impl TreeRunner for ScriptedRunner {
    fn run(&mut self, op: &TreeOp, cx: &mut TreeCx<'_>) -> TreeResult {
        let done = match &self.mode {
            Mode::Scripted(script) => {
                let steps = script
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front()
                    .unwrap_or_default();
                run_script(op, steps, cx)
            }
            Mode::Repo { origin } => {
                let origin = origin.clone();
                run_repo(op, &origin, cx)
            }
        };
        done.unwrap_or_else(TreeStop::into_result)
    }
}

fn run_script(
    op: &TreeOp,
    steps: Vec<TreeStep>,
    cx: &mut TreeCx<'_>,
) -> Result<TreeResult, TreeStop> {
    let dir = PathBuf::from(".");
    for step in steps {
        match step {
            TreeStep::Delay(span) => cx.wait(span)?,
            TreeStep::Child { program, args } => {
                let mut command = Command::new(program);
                command.args(args);
                cx.spawn("child", command, BOUND, &dir)?;
            }
            TreeStep::Panic => panic!("a scripted tree panic"),
            TreeStep::Answer(result) => return Ok(result),
        }
    }
    Ok(default_result(op))
}

/// Why a repository operation stopped: the worker stopped it, or git refused, which is a transient refusal the
/// host retries. A failed call is never passed over, or a park would report a commit it did not make.
enum Stopped {
    Worker(TreeStop),
    Refused(TreeResult),
}

impl From<TreeStop> for Stopped {
    fn from(stop: TreeStop) -> Self {
        Self::Worker(stop)
    }
}

fn git_call(
    cx: &mut TreeCx<'_>,
    dir: &Path,
    step: &str,
    args: &[&str],
) -> Result<StepOutput, TreeStop> {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "-c",
            "user.name=nxs",
            "-c",
            "user.email=nxs@localhost",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
        ]);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(var);
    }
    command.args(args);
    cx.spawn(step, command, BOUND, dir)
}

fn git(cx: &mut TreeCx<'_>, dir: &Path, step: &str, args: &[&str]) -> Result<String, Stopped> {
    let output = git_call(cx, dir, step, args)?;
    if !output.success {
        return Err(Stopped::Refused(TreeResult::Refused {
            reason: TreeRefusal::Git {
                reason: GitReason::ExitNonzero,
            },
            text: String::from_utf8_lossy(&output.stderr).into_owned(),
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Every lock under `.git` but the index's, which the worker answers for: this harness kills git at every step, and
/// a kill can leave `HEAD.lock` or a ref's. Nothing else writes here, because the claim is held.
fn clear_stale_ref_locks(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            clear_stale_ref_locks(&path);
        } else if path.extension().is_some_and(|ext| ext == "lock")
            && path.file_name().is_some_and(|name| name != "index.lock")
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn run_repo(op: &TreeOp, origin: &Path, cx: &mut TreeCx<'_>) -> Result<TreeResult, TreeStop> {
    clear_stale_ref_locks(&op.dir().join(".git"));
    let done = match op {
        TreeOp::Prepare { run, dir, .. } => prepare(run, dir, origin, cx),
        TreeOp::Park { run, dir, .. } => {
            if !dir.exists() {
                return Ok(TreeResult::Parked(ParkOutcome::Missing));
            }
            base_of(dir, cx)
                .and_then(|base| park(run, dir, &base, cx))
                .map(TreeResult::Parked)
        }
    };
    match done {
        Ok(result) | Err(Stopped::Refused(result)) => Ok(result),
        Err(Stopped::Worker(stop)) => Err(stop),
    }
}

fn base_of(dir: &Path, cx: &mut TreeCx<'_>) -> Result<Base, Stopped> {
    let commit = git(cx, dir, "base", &["rev-parse", "origin/main"])?;
    Ok(Base {
        branch: "main".to_owned(),
        commit,
    })
}

/// Every step reads the tree before it acts, so a crash anywhere re-runs from the first.
fn prepare(
    run: &RunId,
    dir: &Path,
    origin: &Path,
    cx: &mut TreeCx<'_>,
) -> Result<TreeResult, Stopped> {
    let parent = dir.parent().unwrap_or(Path::new("."));
    let stem = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{stem}.cloning-"))
            {
                fs::remove_dir_all(entry.path()).map_err(|_| TreeStop::SpawnFailed)?;
            }
        }
    }
    if !dir.exists() {
        let sibling = parent.join(format!("{stem}.cloning-{}", ulid::Ulid::new()));
        let origin = origin.to_string_lossy().into_owned();
        let sibling_text = sibling.to_string_lossy().into_owned();
        git(
            cx,
            parent,
            "clone",
            &["clone", "-q", &origin, &sibling_text],
        )?;
        crash_at(Boundary::DuringClone);
        fs::rename(&sibling, dir).map_err(|_| TreeStop::SpawnFailed)?;
    }
    git(cx, dir, "fetch", &["fetch", "-q", "origin"])?;
    let base = base_of(dir, cx)?;
    let status = git(cx, dir, "status", &["status", "--porcelain"])?;
    let found = if status.is_empty() {
        None
    } else {
        Some(park(run, dir, &base, cx)?)
    };
    git(
        cx,
        dir,
        "checkout",
        &["checkout", "-q", "--detach", &base.commit],
    )?;
    Ok(TreeResult::Prepared { base, found })
}

/// A HEAD on the run's own branch is used as it is; a HEAD on the default branch or detached gets a park
/// branch, unless it is clean at the base. A retry after a crash finds HEAD on its park branch and reuses it.
fn park(run: &RunId, dir: &Path, base: &Base, cx: &mut TreeCx<'_>) -> Result<ParkOutcome, Stopped> {
    // A detached HEAD answers non-zero here, which is an answer and no refusal.
    let head = git_call(cx, dir, "head", &["symbolic-ref", "-q", "--short", "HEAD"])?;
    let on_branch = head.success;
    let current = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    let status = git(cx, dir, "status", &["status", "--porcelain"])?;
    let dirty = !status.is_empty();
    let commit_now = git(cx, dir, "rev-parse", &["rev-parse", "HEAD"])?;
    if !on_branch && !dirty && commit_now == base.commit {
        return Ok(ParkOutcome::Clean { branch: None });
    }
    let on_park = on_branch && current.starts_with("nxs/park/");
    let branch = if on_branch && current != base.branch {
        current
    } else {
        let branch = format!("nxs/park/run-{run}/{}", ulid::Ulid::new());
        git(cx, dir, "branch", &["checkout", "-q", "-b", &branch])?;
        branch
    };
    let fresh = branch.starts_with("nxs/park/");
    if dirty {
        git(cx, dir, "add", &["add", "-A"])?;
        let message = format!("park of run {run}");
        git(
            cx,
            dir,
            "commit",
            &["commit", "-q", "--no-verify", "-m", &message],
        )?;
    }
    let commit = git(cx, dir, "rev-parse", &["rev-parse", "HEAD"])?;
    crash_at(Boundary::AfterParkCommitBeforeTreeParked);
    Ok(if fresh || on_park {
        ParkOutcome::Committed { branch, commit }
    } else if dirty {
        ParkOutcome::OnOwnBranch { branch, commit }
    } else {
        ParkOutcome::Clean {
            branch: Some(branch),
        }
    })
}

/// What the tests and the crash box hold.
pub struct ScriptedTree {
    worker: TreeWorker<ScriptedRunner>,
    script: Arc<Mutex<VecDeque<Vec<TreeStep>>>>,
    delay: Mutex<Option<Duration>>,
    begins: AtomicUsize,
    cancels: AtomicUsize,
}

impl ScriptedTree {
    pub fn new(ledger: TreeLedger) -> Self {
        Self::with_deadline(ledger, OPERATION_DEADLINE)
    }

    pub fn with_deadline(ledger: TreeLedger, deadline: Duration) -> Self {
        let script = Arc::new(Mutex::new(VecDeque::new()));
        let runner = ScriptedRunner {
            mode: Mode::Scripted(Arc::clone(&script)),
        };
        Self::over(runner, ledger, deadline, script)
    }

    /// Works in a real repository cloned from `origin`.
    pub fn in_repo(ledger: TreeLedger, origin: &Path) -> Self {
        let runner = ScriptedRunner {
            mode: Mode::Repo {
                origin: origin.to_owned(),
            },
        };
        Self::over(runner, ledger, OPERATION_DEADLINE, Arc::default())
    }

    fn over(
        runner: ScriptedRunner,
        ledger: TreeLedger,
        deadline: Duration,
        script: Arc<Mutex<VecDeque<Vec<TreeStep>>>>,
    ) -> Self {
        let guard: Arc<dyn ProcessGuard> = Arc::new(SystemProcesses);
        Self {
            worker: TreeWorker::with_guard(runner, ledger, guard, deadline),
            script,
            delay: Mutex::new(None),
            begins: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
        }
    }

    /// Every operation blocks this long on the injected clock before it answers.
    pub fn with_delay(self, delay: Duration) -> Self {
        *self.delay.lock().unwrap_or_else(PoisonError::into_inner) = Some(delay);
        self
    }

    /// What the next operation does.
    pub fn script(&self, steps: Vec<TreeStep>) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(steps);
    }

    pub fn begins(&self) -> usize {
        self.begins.load(Ordering::SeqCst)
    }

    pub fn cancels(&self) -> usize {
        self.cancels.load(Ordering::SeqCst)
    }

    pub fn workers_started(&self) -> usize {
        self.worker.workers_started()
    }

    pub fn busy(&self) -> bool {
        self.worker.busy()
    }

    pub fn ledger(&self) -> &TreeLedger {
        self.worker.ledger()
    }
}

impl WorkTree for ScriptedTree {
    fn begin(&self, op: TreeOp) -> Result<(), InFlight> {
        let delay = *self.delay.lock().unwrap_or_else(PoisonError::into_inner);
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(delay) = delay {
            if script.is_empty() {
                script.push_back(vec![TreeStep::Delay(delay)]);
            }
        }
        drop(script);
        self.worker.begin(op)?;
        self.begins.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn poll(&self) -> Option<(TreeOp, TreeResult)> {
        self.worker.poll()
    }

    fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        self.worker.cancel();
    }

    fn bind(&self, binding: TreeBinding) {
        self.worker.bind(binding);
    }
}
