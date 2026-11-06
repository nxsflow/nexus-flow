//! `PrepareTree` (§6.6, steps 0 to 5): the directory is this repository's clone, carrying the root's `.nxs` link,
//! with whatever a predecessor left saved, provably clean at the remote's default branch. Every step reads the tree
//! before it acts, so a crash or a refusal anywhere re-runs from the first.

use std::fs;
use std::path::{Path, PathBuf};

use nexus_run_contracts::{Base, GitReason, ParkOutcome, RunId, TreeRefusal};

use super::park::save_work;
use super::runner::{exit_nonzero, failed, line_of, normalise_origin_url, Git};
use super::{exists, Stopped};
use crate::boundary::{crash_at, Boundary};
use crate::tree_worker::TreeResult;

const EXCLUDED: [&str; 2] = [".nxs", ".nxf/"];

const SET_HEAD_REMEDY: &str = "remedy: `git remote set-head origin --auto` in the directory, once the remote has a default branch";

pub(crate) fn parent_and_root_exist(root: &Path, dir: &Path) -> bool {
    root.exists() && dir.parent().is_some_and(Path::exists)
}

fn refused<T>(reason: TreeRefusal, text: String) -> Result<T, Stopped> {
    Err(Stopped::Refused(reason, text))
}

/// A volume that is not mounted is no missing directory: it is neither cloned onto nor released.
pub(crate) fn refuse_without_parent_or_root<T>(root: &Path, dir: &Path) -> Result<T, Stopped> {
    refused(
        exit_nonzero(),
        format!(
            "{} or its parent is not there, or the root {} is not; remedy: mount the volume",
            dir.display(),
            root.display()
        ),
    )
}

/// What the registered root says this repository is: read from the root every time, as the worker's first bounded
/// step, and held by nothing longer than the operation, so the URL reaches neither the ledger nor status.
pub(crate) struct RootIdentity {
    url: String,
    link: PathBuf,
    root: PathBuf,
}

pub(crate) fn root_identity(git: &mut Git<'_, '_>, root: &Path) -> Result<RootIdentity, Stopped> {
    let origin = git.ask("origin", root, &["remote", "get-url", "origin"])?;
    if !origin.success {
        return refused(
            TreeRefusal::ForeignDirectory,
            format!(
                "the root {} has no `origin`; remedy: `git remote add origin <url>` in the root",
                root.display()
            ),
        );
    }
    let url = line_of("origin", &origin)?;
    let link = root.join(".nxs").canonicalize().or_else(|error| {
        refused(
            exit_nonzero(),
            format!("the root's .nxs does not resolve: {error}"),
        )
    })?;
    let root = root.canonicalize().or_else(|error| {
        refused(
            exit_nonzero(),
            format!("the root does not resolve: {error}"),
        )
    })?;
    Ok(RootIdentity { url, link, root })
}

/// Step (0): a directory present is this repository's clone, or it is left alone. A MISSING link is not a verdict, a
/// session's `git clean -fdx` is ordinary and prepare recreates it. A link PRESENT and resolving elsewhere is one:
/// silently repointing it is the one way the directory could change which board it writes to.
pub(crate) fn check_identity(
    git: &mut Git<'_, '_>,
    identity: &RootIdentity,
    dir: &Path,
) -> Result<(), Stopped> {
    let move_aside = format!(
        "remedy: move {} aside, and the next prepare clones",
        dir.display()
    );
    // The origin and the link of the registered root itself pass everything below, and this engine would then check
    // out and commit in the old engine's tree.
    if dir.canonicalize().ok().as_ref() == Some(&identity.root) {
        return refused(
            TreeRefusal::ForeignDirectory,
            format!(
                "{} resolves to the registered root; {move_aside}",
                dir.display()
            ),
        );
    }
    if !dir.is_dir()
        || !git
            .ask("identity", dir, &["rev-parse", "--git-dir"])?
            .success
    {
        return refused(
            TreeRefusal::NotARepository,
            format!("{} is not a repository; {move_aside}", dir.display()),
        );
    }
    let origin = git.ask("identity", dir, &["remote", "get-url", "origin"])?;
    if !origin.success {
        return refused(
            TreeRefusal::ForeignDirectory,
            format!("{} has no `origin`; {move_aside}", dir.display()),
        );
    }
    let same =
        normalise_origin_url(&line_of("identity", &origin)?) == normalise_origin_url(&identity.url);
    if !same {
        return refused(
            TreeRefusal::ForeignDirectory,
            format!(
                "{} is a clone of another repository than the root's; {move_aside}, or `git remote set-url origin` in it if it is the same one",
                dir.display()
            ),
        );
    }
    let link = dir.join(".nxs");
    if exists(&link) && link.canonicalize().ok().as_ref() != Some(&identity.link) {
        return refused(
            TreeRefusal::ForeignDirectory,
            format!(
                "{} is linked to another workspace than the root's; {move_aside}",
                link.display()
            ),
        );
    }
    Ok(())
}

fn link_to_root(identity: &RootIdentity, dir: &Path) -> Result<(), Stopped> {
    let link = dir.join(".nxs");
    if exists(&link) {
        return Ok(());
    }
    symlink(&identity.link, &link).or_else(|error| {
        refused(
            exit_nonzero(),
            format!("{} could not be linked: {error}", link.display()),
        )
    })
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// On every prepare, for any workspace and any branch: without these a park's `git add -A` would commit the link, and
/// the finisher's clean-tree check would see the gate log.
fn exclude_local_files(dir: &Path) -> Result<(), Stopped> {
    let write = || -> std::io::Result<()> {
        let info = dir.join(".git").join("info");
        fs::create_dir_all(&info)?;
        let exclude = info.join("exclude");
        let mut text = match fs::read(&exclude) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let held: Vec<&[u8]> = text.split(|byte| *byte == b'\n').collect();
        let wanting: Vec<&str> = EXCLUDED
            .into_iter()
            .filter(|line| !held.contains(&line.as_bytes()))
            .collect();
        if wanting.is_empty() {
            return Ok(());
        }
        if !text.is_empty() && !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        for line in wanting {
            text.extend_from_slice(line.as_bytes());
            text.push(b'\n');
        }
        let temp = info.join("exclude.nxs-tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, &exclude)
    };
    write().or_else(|error| {
        refused(
            exit_nonzero(),
            format!("the exclude file could not be written: {error}"),
        )
    })
}

fn sibling_prefix(dir: &Path) -> String {
    format!(
        "{}.cloning-",
        dir.file_name().unwrap_or_default().to_string_lossy()
    )
}

/// What a crash during a clone left, and only that: the suffix this engine mints is a ULID, so a person's own
/// `<dir>.cloning-backup` is theirs. Nothing writes there by now: the worker killed any child the ledger named.
fn remove_clone_siblings(dir: &Path) -> Result<(), Stopped> {
    let Some(parent) = dir.parent() else {
        return Ok(());
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Ok(());
    };
    let prefix = sibling_prefix(dir);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name
            .to_string_lossy()
            .strip_prefix(&prefix)
            .is_some_and(|suffix| ulid::Ulid::from_string(suffix).is_ok());
        if ours {
            fs::remove_dir_all(entry.path()).or_else(|error| {
                refused(
                    exit_nonzero(),
                    format!("{} could not be removed: {error}", entry.path().display()),
                )
            })?;
        }
    }
    Ok(())
}

/// Step (1): through a sibling and one rename, so a crash never leaves half a directory at `dir`. The local default
/// branch goes, so a bare `git diff main` fails loudly instead of comparing with a stale branch for ever.
fn clone(git: &mut Git<'_, '_>, identity: &RootIdentity, dir: &Path) -> Result<(), Stopped> {
    let parent = dir.parent().unwrap_or(Path::new("."));
    let sibling = parent.join(format!("{}{}", sibling_prefix(dir), ulid::Ulid::new()));
    let target = sibling.to_string_lossy().into_owned();
    let bound = git.bounds.clone;
    let cloned = git.call(
        "clone",
        parent,
        &["clone", "-q", "--", &identity.url, &target],
        bound,
    )?;
    if !cloned.success {
        return Err(failed("clone", &cloned));
    }
    crash_at(Boundary::DuringClone);
    let finished = finish_clone(git, identity, &sibling);
    if finished.is_err() {
        // Nothing to keep; the next prepare would remove it anyway.
        let _ = fs::remove_dir_all(&sibling);
        return finished;
    }
    fs::rename(&sibling, dir).or_else(|error| {
        refused(
            exit_nonzero(),
            format!("{} could not be renamed: {error}", sibling.display()),
        )
    })
}

fn finish_clone(
    git: &mut Git<'_, '_>,
    identity: &RootIdentity,
    sibling: &Path,
) -> Result<(), Stopped> {
    link_to_root(identity, sibling)?;
    exclude_local_files(sibling)?;
    let default = default_branch(git, sibling)?;
    let base = format!("refs/remotes/origin/{default}");
    git.run("checkout", sibling, &["checkout", "-q", "--detach", &base])?;
    git.run("branch", sibling, &["branch", "-q", "-D", &default])?;
    Ok(())
}

/// `<default>`, as the clone recorded it in `refs/remotes/origin/HEAD`, readable without network. An empty remote,
/// a `remote set-head -d` and a default branch `fetch --prune` took away all leave no base to prepare on.
pub(crate) fn default_branch(git: &mut Git<'_, '_>, dir: &Path) -> Result<String, Stopped> {
    match read_default(git, dir)? {
        DefaultBranch::Known(default) => Ok(default),
        DefaultBranch::Unset => refused(
            exit_nonzero(),
            format!("the clone records no default branch: origin/HEAD is not set, the remote may be empty; {SET_HEAD_REMEDY}"),
        ),
        DefaultBranch::Dangling(default) => refused(
            exit_nonzero(),
            format!("origin/HEAD is dangling: it names {default}, which the remote no longer has; {SET_HEAD_REMEDY}"),
        ),
    }
}

/// The default branch where the clone still knows one. A park takes the answer as it comes: it has to end, and work
/// can be saved without a base.
pub(crate) fn known_default(git: &mut Git<'_, '_>, dir: &Path) -> Result<Option<String>, Stopped> {
    Ok(match read_default(git, dir)? {
        DefaultBranch::Known(default) => Some(default),
        DefaultBranch::Unset | DefaultBranch::Dangling(_) => None,
    })
}

enum DefaultBranch {
    Known(String),
    Unset,
    Dangling(String),
}

fn read_default(git: &mut Git<'_, '_>, dir: &Path) -> Result<DefaultBranch, Stopped> {
    let head = git.ask(
        "default",
        dir,
        &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
    )?;
    if !head.success {
        return Ok(DefaultBranch::Unset);
    }
    let full = line_of("default", &head)?;
    let default = full
        .strip_prefix("refs/remotes/origin/")
        .unwrap_or(&full)
        .to_owned();
    let commit = format!("refs/remotes/origin/{default}^{{commit}}");
    Ok(
        if git
            .ask("default", dir, &["rev-parse", "-q", "--verify", &commit])?
            .success
        {
            DefaultBranch::Known(default)
        } else {
            DefaultBranch::Dangling(default)
        },
    )
}

/// Step (2). `found` has to survive a failed fetch and a crash, so it is derived from the tree: `save_work` answers
/// for a HEAD still on this run's park branch or on a local branch ahead of its upstream, and a park branch of this run
/// that HEAD has already left is one this run's earlier attempt made.
fn rescue(
    git: &mut Git<'_, '_>,
    run: &RunId,
    dir: &Path,
    default: &str,
) -> Result<Option<ParkOutcome>, Stopped> {
    match save_work(git, run, dir, Some(default), "found_dirty_at_prepare")? {
        ParkOutcome::Clean { .. } => earlier_rescue(git, run, dir),
        outcome => Ok(Some(outcome)),
    }
}

fn earlier_rescue(
    git: &mut Git<'_, '_>,
    run: &RunId,
    dir: &Path,
) -> Result<Option<ParkOutcome>, Stopped> {
    let pattern = format!("refs/heads/nxs/park/run-{run}/");
    let listed = git.run(
        "rescue",
        dir,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            &pattern,
        ],
    )?;
    let listed = line_of("rescue", &listed)?;
    Ok(listed.lines().last().and_then(|line| {
        let (branch, commit) = line.split_once(' ')?;
        Some(ParkOutcome::Committed {
            branch: branch.to_owned(),
            commit: commit.to_owned(),
        })
    }))
}

pub(crate) fn prepare(
    git: &mut Git<'_, '_>,
    root: &Path,
    run: &RunId,
    dir: &Path,
) -> Result<TreeResult, Stopped> {
    if !parent_and_root_exist(root, dir) {
        return refuse_without_parent_or_root(root, dir);
    }
    let identity = root_identity(git, root)?;
    remove_clone_siblings(dir)?;
    if !exists(dir) {
        clone(git, &identity, dir)?;
    }
    check_identity(git, &identity, dir)?;
    link_to_root(&identity, dir)?;
    exclude_local_files(dir)?;
    let default = default_branch(git, dir)?;
    let found = rescue(git, run, dir, &default)?;

    // Step (3): a run does not start on a base the remote has not confirmed.
    let bound = git.bounds.fetch;
    let fetched = git.call("fetch", dir, &["fetch", "-q", "--prune", "origin"], bound)?;
    if !fetched.success {
        return refused(
            TreeRefusal::Git {
                reason: GitReason::FetchFailed,
            },
            format!("git fetch: {}", String::from_utf8_lossy(&fetched.stderr)),
        );
    }

    // Step (4): `--prune` may have taken the default branch away.
    let default = default_branch(git, dir)?;
    let base = format!("refs/remotes/origin/{default}");
    git.run("checkout", dir, &["checkout", "-q", "--detach", &base])?;
    let commit = git.text("base", dir, &["rev-parse", "HEAD"])?;
    Ok(TreeResult::Prepared {
        base: Base {
            branch: default,
            commit,
        },
        found,
    })
}
