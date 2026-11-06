//! `ParkTree` (§6.6, "The park, v2's own"), and the saving it shares with prepare's rescue. The return to the base is
//! not part of a park: the next prepare checks the base out afresh, so a crash after the commit leaves HEAD off the
//! base and the retry finds its branch again.

use std::path::Path;

use nexus_run_contracts::{GitReason, ParkOutcome, RunId, SequenceKind, Timestamp, TreeRefusal};

use super::prepare::{
    check_identity, known_default, parent_and_root_exist, refuse_without_parent_or_root,
    root_identity,
};
use super::runner::{failed, line_of, sequence_in_progress, Git};
use super::{exists, Stopped};
use crate::boundary::{crash_at, Boundary};
use crate::tree_child::git_locks;

const PARK_PREFIX: &str = "nxs/park/";

pub(crate) fn park(
    git: &mut Git<'_, '_>,
    root: &Path,
    run: &RunId,
    dir: &Path,
    occasion: &str,
) -> Result<ParkOutcome, Stopped> {
    if !exists(dir) {
        return if parent_and_root_exist(root, dir) {
            Ok(ParkOutcome::Missing)
        } else {
            refuse_without_parent_or_root(root, dir)
        };
    }
    // First, before anything can write: a directory that is not this repository's clone is not this run's to touch.
    // The run was refused on it, and a withdrawal has to end. The contract's name for "no git touched a tree that is
    // not ours" is `not_held`, and it releases.
    let identity = match root_identity(git, root) {
        Ok(identity) => identity,
        Err(stopped) if is_a_mismatch(&stopped) => return Ok(ParkOutcome::NotHeld),
        Err(stopped) => return Err(stopped),
    };
    match check_identity(git, &identity, dir) {
        Err(stopped) if is_a_mismatch(&stopped) => return Ok(ParkOutcome::NotHeld),
        checked => checked?,
    }
    // Refused at prepare, where a run would start on no base. A withdrawal has to end, and saving needs no base.
    let default = known_default(git, dir)?;
    save_work(git, run, dir, default.as_deref(), occasion)
}

fn is_a_mismatch(stopped: &Stopped) -> bool {
    matches!(
        stopped,
        Stopped::Refused(
            TreeRefusal::NotARepository | TreeRefusal::ForeignDirectory,
            _
        )
    )
}

/// A tree in the middle of something is a person's: a commit now would seal half a merge. Transient, because a person
/// or the window ends it.
fn refuse_unfinished(dir: &Path) -> Result<(), Stopped> {
    if let Some(kind) = sequence_in_progress(dir) {
        let remedy = match kind {
            SequenceKind::Merge => "`git merge --abort`",
            SequenceKind::Rebase => "`git rebase --abort`",
            SequenceKind::CherryPick => "`git cherry-pick --abort`",
            SequenceKind::Revert => "`git revert --abort`",
            SequenceKind::Bisect => "`git bisect reset`",
            SequenceKind::Unknown(_) => "finishing or aborting the sequence",
        };
        return Err(Stopped::Refused(
            TreeRefusal::MidSequence { what: kind },
            format!(
                "{} is in the middle of a sequence; remedy: {remedy} there",
                dir.display()
            ),
        ));
    }
    if let Some(lock) = git_locks(dir).first() {
        return Err(Stopped::Refused(
            TreeRefusal::Git {
                reason: GitReason::LockHeld,
            },
            format!(
                "{} is there and no child of this engine can answer for it; remedy: remove it once no git runs in {}",
                dir.join(".git").join(lock).display(),
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// `20261105T100000Z`, the old engine's form, so recovering parked work is what it is today.
fn compact(instant: Timestamp) -> String {
    let text = instant.to_string();
    let seconds = text.split('.').next().unwrap_or(&text);
    let mut compact: String = seconds
        .chars()
        .filter(|c| !matches!(c, '-' | ':'))
        .collect();
    compact.push('Z');
    compact
}

/// What a commit needs when nothing is configured: `nxs <nxs@localhost>`, never a refusal. A park that cannot commit
/// loses work, so an unconfigured machine must not stop it.
fn identity_fallback(git: &mut Git<'_, '_>, dir: &Path) -> Result<Vec<String>, Stopped> {
    let mut args = Vec::new();
    for (key, fallback) in [("user.name", "nxs"), ("user.email", "nxs@localhost")] {
        let set = git.ask("identity", dir, &["config", "--get", key])?;
        if !set.success || set.stdout.iter().all(u8::is_ascii_whitespace) {
            args.extend(["-c".to_owned(), format!("{key}={fallback}")]);
        }
    }
    Ok(args)
}

fn commit(git: &mut Git<'_, '_>, run: &RunId, dir: &Path, occasion: &str) -> Result<(), Stopped> {
    let bound = git.bounds.park;
    let added = git.call("add", dir, &["add", "-A"], bound)?;
    if !added.success {
        return Err(failed("add", &added));
    }
    let message = format!("park of run {run}: {occasion} at {}", git.cx.now());
    let fallback = identity_fallback(git, dir)?;
    let mut args: Vec<&str> = fallback.iter().map(String::as_str).collect();
    args.extend([
        "commit",
        "-q",
        "--no-verify",
        "--no-gpg-sign",
        "-m",
        &message,
    ]);
    let committed = git.call("commit", dir, &args, bound)?;
    if !committed.success {
        return Err(failed("commit", &committed));
    }
    Ok(())
}

/// Whether HEAD holds commits the remote has not: past `origin/<branch>`, or past the base for a branch the remote
/// has never had. Those are what a notice is owed for.
fn ahead_of_the_remote(
    git: &mut Git<'_, '_>,
    dir: &Path,
    branch: &str,
    base_ref: Option<&str>,
) -> Result<bool, Stopped> {
    let upstream = format!("refs/remotes/origin/{branch}");
    let known = git
        .ask(
            "ahead",
            dir,
            &[
                "rev-parse",
                "-q",
                "--verify",
                &format!("{upstream}^{{commit}}"),
            ],
        )?
        .success;
    let from = if known {
        upstream.as_str()
    } else {
        match base_ref {
            Some(base_ref) => base_ref,
            // Nothing to measure against: a person hears of a branch once too often rather than never.
            None => return Ok(true),
        }
    };
    let count = git.text(
        "ahead",
        dir,
        &["rev-list", "--count", &format!("{from}..HEAD")],
    )?;
    Ok(count != "0")
}

/// Saves what the tree holds, or says there is nothing to save. A HEAD on the run's own branch is used as it is; a HEAD
/// on the local default branch or detached gets a fresh park branch, unless it is clean and holds nothing the base
/// lacks: ancestry, not inequality, so a clean HEAD behind the base gets no empty branch. With no base known (`default`
/// is `None`, a park only) the branch HEAD stands on is used as it is, and a detached HEAD always gets a name, since
/// nothing can say it holds nothing of its own.
pub(crate) fn save_work(
    git: &mut Git<'_, '_>,
    run: &RunId,
    dir: &Path,
    default: Option<&str>,
    occasion: &str,
) -> Result<ParkOutcome, Stopped> {
    refuse_unfinished(dir)?;
    let head = git.ask("head", dir, &["symbolic-ref", "-q", "--short", "HEAD"])?;
    let current = if head.success {
        Some(line_of("head", &head)?)
    } else {
        None
    };
    // The bytes decide: anything at all is dirty, whatever it decodes to (6j6v.2af2).
    let dirty = !git
        .run("status", dir, &["status", "--porcelain"])?
        .stdout
        .is_empty();
    let base_ref = default.map(|default| format!("refs/remotes/origin/{default}"));
    let (branch, fresh) = match current {
        Some(branch) if Some(branch.as_str()) != default => (branch, false),
        current => {
            let behind_or_at = match &base_ref {
                Some(base_ref) => {
                    git.ask(
                        "ancestry",
                        dir,
                        &["merge-base", "--is-ancestor", "HEAD", base_ref],
                    )?
                    .success
                }
                None => false,
            };
            if !dirty && behind_or_at {
                return Ok(ParkOutcome::Clean { branch: current });
            }
            let branch = format!("nxs/park/run-{run}/{}", compact(git.cx.now()));
            git.run("branch", dir, &["checkout", "-q", "-b", &branch])?;
            (branch, true)
        }
    };
    if dirty {
        commit(git, run, dir, occasion)?;
    }
    let commit = git.text("commit", dir, &["rev-parse", "HEAD"])?;
    crash_at(Boundary::AfterParkCommitBeforeTreeParked);
    if fresh || branch.starts_with(&format!("nxs/park/run-{run}/")) {
        return Ok(ParkOutcome::Committed { branch, commit });
    }
    // Another run's park branch was noticed when that run parked.
    let owes_a_notice = dirty
        || (!branch.starts_with(PARK_PREFIX)
            && ahead_of_the_remote(git, dir, &branch, base_ref.as_deref())?);
    Ok(if owes_a_notice {
        ParkOutcome::OnOwnBranch { branch, commit }
    } else {
        ParkOutcome::Clean {
            branch: Some(branch),
        }
    })
}
