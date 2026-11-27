//! Reading the rules' files from a commit of the local git, as git objects, never from the working
//! tree (RFC 0010, "Which copy of the rules judges a pull request"). It runs the `git` the machine
//! has and never fetches: the gate reads only the local git (GENESIS.md, principle 3).

use std::fmt;
use std::path::Path;
use std::process::{Command, Output};

/// Why the local git could not give what the gate reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The reference names no commit in the local git.
    NotACommit { reference: String },
    /// git could not be run, or failed for another reason.
    Failed { message: String },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::NotACommit { reference } => {
                write!(f, "{reference:?} is not a commit in the local git")
            }
            GitError::Failed { message } => write!(f, "git failed: {message}"),
        }
    }
}

impl std::error::Error for GitError {}

/// The full object id of the commit `reference` names, read in the repository at `dir`.
pub fn resolve_commit(dir: &Path, reference: &str) -> Result<String, GitError> {
    let spec = format!("{reference}^{{commit}}");
    let out = git(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &spec,
        ],
    )?;
    if !out.status.success() {
        return Err(GitError::NotACommit {
            reference: reference.to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The bytes of the file at `path`, relative to `dir`, in `commit`; `None` when the commit holds
/// no file there.
pub fn read_file(dir: &Path, commit: &str, path: &str) -> Result<Option<Vec<u8>>, GitError> {
    // `./` makes the path relative to `dir`, which may lie below the git repository's root.
    let object = format!("{commit}:./{path}");
    let kind = git(dir, &["cat-file", "-t", &object])?;
    if !kind.status.success() || kind.stdout.trim_ascii() != b"blob" {
        return Ok(None);
    }
    let blob = git(dir, &["cat-file", "blob", &object])?;
    if !blob.status.success() {
        return Err(failed(&blob));
    }
    Ok(Some(blob.stdout))
}

fn git(dir: &Path, args: &[&str]) -> Result<Output, GitError> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| GitError::Failed {
            message: e.to_string(),
        })
}

fn failed(out: &Output) -> GitError {
    GitError::Failed {
        message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    }
}
