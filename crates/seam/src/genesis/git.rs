//! Whether git still knows a GENESIS that is missing from the working tree (RFC 0004, "Installing
//! the template", the `tracked` state): the path is tracked in the index, in `HEAD`, or on the
//! base branch, local or as any remote's tracking branch. It runs the local `git` and never
//! touches the network or writes anything.

use std::io;
use std::path::Path;
use std::process::{Command, Output};

/// Why git could not be asked.
#[derive(Debug)]
pub enum GitError {
    /// `git` could not be started.
    Spawn(io::Error),
    /// `git` ran and failed for a reason other than "not a git repository".
    Failed { args: String, stderr: String },
}

/// Whether `path`, relative to `root`, is tracked in the index, in `HEAD`, or on the base branch
/// `base` (`refs/heads/<base>`, or `refs/remotes/<remote>/<base>`). Outside a git repository
/// nothing is tracked.
pub fn tracked(root: &Path, path: &str, base: &str) -> Result<bool, GitError> {
    let probe = git(root, &["rev-parse", "--git-dir"])?;
    if !probe.status.success() {
        let stderr = String::from_utf8_lossy(&probe.stderr);
        if stderr.contains("not a git repository") {
            return Ok(false);
        }
        return Err(failed(&["rev-parse", "--git-dir"], &probe));
    }
    if !checked(root, &["ls-files", "-z", "--cached", "--", path])?.is_empty() {
        return Ok(true);
    }
    let mut revs = vec!["HEAD".to_owned(), format!("refs/heads/{base}")];
    let remotes = checked(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)",
            &format!("refs/remotes/*/{base}"),
        ],
    )?;
    revs.extend(String::from_utf8_lossy(&remotes).lines().map(str::to_owned));
    for rev in revs {
        if in_commit(root, &rev, path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the commit `rev` names holds `path`; `false` when `rev` names no commit, such as an
/// unborn `HEAD` or a branch that does not exist.
fn in_commit(root: &Path, rev: &str, path: &str) -> Result<bool, GitError> {
    let spec = format!("{rev}^{{commit}}");
    let resolved = git(root, &["rev-parse", "--verify", "--quiet", &spec])?;
    if !resolved.status.success() {
        return Ok(false);
    }
    let commit = String::from_utf8_lossy(&resolved.stdout).trim().to_owned();
    let listed = checked(root, &["ls-tree", "-z", "--name-only", &commit, "--", path])?;
    Ok(!listed.is_empty())
}

/// Runs git in `root`, with pathspecs taken literally and messages in English.
fn git(root: &Path, args: &[&str]) -> Result<Output, GitError> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("--literal-pathspecs")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .output()
        .map_err(GitError::Spawn)
}

/// Runs git and returns its stdout, failing when git fails.
fn checked(root: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let out = git(root, args)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(failed(args, &out))
    }
}

fn failed(args: &[&str], out: &Output) -> GitError {
    GitError::Failed {
        args: args.join(" "),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    }
}
