//! What the tree tests stand on: temporary repositories with a file-path origin, and a real `GitTree` bound to a
//! lease nobody moves. Nothing here ever reaches this checkout, its `.nxs`, or a real `<workspace>-v2`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use nexus_run_contracts::{Outcome, RunId, StoreError, Timestamp};
use nexus_run_host::git_tree::{GitBounds, GitTree};
use nexus_run_host::ports::{Clock, SystemClock};
use nexus_run_host::{LeaseView, TreeBinding, TreeLedger, TreeOp, TreeResult, WorkTree};
use nexus_run_store_sqlite::Lease;

const OWNER: &str = "the-owner";

/// Every git in this binary, the tree's children included, sees an empty global config and nothing of the
/// session it was started from: the developer's excludes, signing and identity would otherwise decide a test.
pub fn hermetic() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = std::env::temp_dir().join(format!("zjyv-git-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("a home");
        let config = home.join("gitconfig");
        std::fs::write(&config, b"").expect("an empty config");
        std::env::set_var("GIT_CONFIG_GLOBAL", &config);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        // The default `core.excludesFile` is under here, and a machine's own may ignore what a test creates.
        std::env::set_var("XDG_CONFIG_HOME", &home);
        let inherited: Vec<_> = std::env::vars_os()
            .map(|(name, _)| name)
            .filter(|name| {
                let name = name.to_string_lossy();
                name.starts_with("NXC_")
                    || matches!(
                        name.as_ref(),
                        "GIT_DIR" | "GIT_WORK_TREE" | "GIT_INDEX_FILE" | "GIT_COMMON_DIR"
                    )
            })
            .collect();
        for name in inherited {
            std::env::remove_var(name);
        }
    });
}

/// `git` in a directory, which has to succeed. For what a test sets up.
pub fn run_git(dir: &Path, args: &[&str]) -> String {
    let output = raw_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub fn raw_git(dir: &Path, args: &[&str]) -> Output {
    hermetic();
    Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git runs")
}

/// A repository with one commit on `main` and the repository's own `.gitignore`, the origin everything clones.
pub fn origin_with_main(path: &Path) {
    std::fs::create_dir_all(path).expect("the origin's directory");
    std::fs::write(path.join("README"), b"origin\n").expect("a file");
    std::fs::write(path.join(".gitignore"), TRACKED_GITIGNORE).expect("the tracked ignore file");
    run_git(path, &["init", "-q"]);
    run_git(path, &["add", "-A"]);
    run_git(path, &["commit", "-q", "-m", "first"]);
}

/// The repository's tracked file, as it is in this commit.
pub const TRACKED_GITIGNORE: &str = include_str!("../../../../.gitignore");

/// A commit on the origin's `main`, which a fetch then finds.
pub fn advance_origin(origin: &Path) -> String {
    std::fs::write(origin.join("more"), b"more\n").expect("a file");
    run_git(origin, &["add", "-A"]);
    run_git(origin, &["commit", "-q", "-m", "more"]);
    run_git(origin, &["rev-parse", "HEAD"])
}

struct FixedLease;

impl LeaseView for FixedLease {
    fn lease(&self, _run: &RunId) -> Result<Option<Lease>, StoreError> {
        Ok(Some(Lease {
            owner_id: OWNER.to_owned(),
            epoch: 1,
            expires_at: Timestamp::parse("9000-01-01T00:00:00.000Z").expect("an instant"),
        }))
    }
}

/// An origin, a registered root with its `.nxs` link, and the place the directory goes.
pub struct Bench {
    pub temp: tempfile::TempDir,
    pub origin: PathBuf,
    pub root: PathBuf,
    pub dir: PathBuf,
    pub nxs_target: PathBuf,
    pub ledger: TreeLedger,
}

impl Bench {
    pub fn new() -> Self {
        hermetic();
        let temp = tempfile::tempdir().expect("a temp directory");
        // The canonical path: macOS hands out a /var that is a link, and a link target is compared canonically.
        let base = temp.path().canonicalize().expect("a real path");
        let origin = base.join("origin");
        origin_with_main(&origin);
        let root = base.join("root");
        run_git(&base, &["clone", "-q", "origin", "root"]);
        let nxs_target = base.join("nxs-target");
        std::fs::create_dir_all(&nxs_target).expect("the link's target");
        std::os::unix::fs::symlink(&nxs_target, root.join(".nxs")).expect("the root's link");
        let dir = base.join("root-v2");
        let ledger = TreeLedger::new(&base.join("agent-logs"), "ws-1");
        Self {
            temp,
            origin,
            root,
            dir,
            nxs_target,
            ledger,
        }
    }

    pub fn tree(&self) -> GitTree {
        self.tree_bounded(GitBounds::default())
    }

    pub fn tree_bounded(&self, bounds: GitBounds) -> GitTree {
        bound(GitTree::new(&self.root, self.ledger.clone(), bounds))
    }

    /// A tree whose `git` is the program at this path.
    pub fn tree_with_git(&self, git: &Path, bounds: GitBounds) -> GitTree {
        bound(GitTree::with_git(
            &self.root,
            self.ledger.clone(),
            bounds,
            git,
        ))
    }

    pub fn prepare_op(&self, run: &RunId) -> TreeOp {
        TreeOp::Prepare {
            run: run.clone(),
            dir: self.dir.clone(),
        }
    }

    pub fn park_op(&self, run: &RunId) -> TreeOp {
        TreeOp::Park {
            run: run.clone(),
            dir: self.dir.clone(),
            occasion: Outcome::Withdrawn,
        }
    }

    /// Prepare, to its result.
    pub fn prepare(&self, tree: &GitTree, run: &RunId) -> TreeResult {
        tree.begin(self.prepare_op(run)).expect("nothing in flight");
        finished(tree)
    }

    pub fn park(&self, tree: &GitTree, run: &RunId) -> TreeResult {
        tree.begin(self.park_op(run)).expect("nothing in flight");
        finished(tree)
    }

    /// A directory prepared once, under its own tree, so a test starts from a real clone.
    pub fn prepared(&self) -> GitTree {
        let tree = self.tree();
        let result = self.prepare(&tree, &RunId::new(ulid::Ulid::new()));
        assert!(matches!(result, TreeResult::Prepared { .. }), "{result:?}");
        tree
    }

    pub fn head(&self) -> String {
        run_git(&self.dir, &["rev-parse", "HEAD"])
    }

    pub fn origin_main(&self) -> String {
        run_git(&self.dir, &["rev-parse", "origin/main"])
    }

    pub fn park_branches(&self) -> Vec<String> {
        run_git(
            &self.dir,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/nxs/park/",
            ],
        )
        .lines()
        .map(str::to_owned)
        .collect()
    }

    /// A program for `git` that does `body`, in `sh`, before it becomes the real git. `$1` is the first argument
    /// after the runner's `-c` list only if the body looks for it, so bodies match on `"$*"`.
    pub fn fake_git(&self, body: &str) -> PathBuf {
        let real = which_git();
        let path = self.temp.path().join("fake-git");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\nexec {real} \"$@\"\n"))
            .expect("the program");
        make_executable(&path);
        path
    }
}

pub fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("executable");
}

pub fn which_git() -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("sh runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub fn bound(tree: GitTree) -> GitTree {
    tree.bind(TreeBinding {
        owner_id: OWNER.to_owned(),
        leases: Arc::new(FixedLease),
        clock: Arc::new(SystemClock) as Arc<dyn Clock>,
    });
    tree
}

/// The result of what is in flight. The ceiling only ends a hung test; no assertion rests on it.
pub fn finished(tree: &GitTree) -> TreeResult {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some((_, result)) = tree.poll() {
            return result;
        }
        assert!(Instant::now() < until, "no result");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Waits for what a process elsewhere does. The ceiling only ends a hung test.
pub fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < limit, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The live members of a process group. A zombie holds nothing.
pub fn group_members(pgid: u32) -> Vec<String> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pgid=", "-o", "stat=", "-o", "command="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(pgid.to_string().as_str())
                && !fields.next().unwrap_or("Z").starts_with('Z')
        })
        .map(str::to_owned)
        .collect()
}

pub fn pid_alive(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let stat = String::from_utf8_lossy(&output.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}
