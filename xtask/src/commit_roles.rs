//! The commit-role gate (NEX-37): every commit a pull request brings carries a `Role:` trailer
//! naming the session role that made it, such as `Role: coder on NEX-37` or `Role: pm`.
//! `factory/FACTORY.md` states the rule; `.github/workflows/commit-roles.yml` runs this check.
//!
//! Merge commits are left out: a branch brought up to date with its base gets a merge commit
//! that no session wrote, and the squash merge drops it anyway.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Separates the fields of one commit in the `git log` output this module reads.
const FIELD: char = '\u{1f}';
/// Ends one commit's record in that output.
const RECORD: char = '\u{1e}';

/// The commits in `log` (the output of `git log` with [`LOG_FORMAT`]) that carry no `Role:`
/// trailer with a value, as `<short sha> <subject>`, in the order git listed them.
pub fn missing_role(log: &str) -> Vec<String> {
    log.split(RECORD)
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(3, FIELD);
            let sha = fields.next().filter(|sha| !sha.is_empty())?;
            let subject = fields.next().unwrap_or_default();
            let roles = fields.next().unwrap_or_default();
            let named = roles.lines().any(|role| !role.trim().is_empty());
            (!named).then(|| format!("{sha} {subject}"))
        })
        .collect()
}

/// The `git log` format [`missing_role`] reads: sha, subject, and the values of the `Role`
/// trailers, one per line.
pub const LOG_FORMAT: &str = "%h%x1f%s%x1f%(trailers:key=Role,valueonly,separator=%x0a)%x1e";

/// Check every non-merge commit in `<base>..<head>` of the repository at `root`.
pub fn run(root: &Path, base: &str, head: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["log", "--no-merges", &format!("--format={LOG_FORMAT}")])
        .arg(format!("{base}..{head}"))
        .output()
        .context("running git log")?;
    if !output.status.success() {
        bail!(
            "git log {base}..{head} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let log = String::from_utf8(output.stdout).context("git log printed non-UTF-8")?;
    let missing = missing_role(&log);
    if missing.is_empty() {
        println!("commit roles: every commit in {base}..{head} names its role");
        return Ok(());
    }
    for commit in &missing {
        eprintln!("commit roles: no `Role:` trailer: {commit}");
    }
    bail!(
        "{} commit(s) lack a `Role: <role>` trailer, such as `Role: coder on NEX-37` \
         (factory/FACTORY.md, \"Rules for every coding agent\")",
        missing.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sha: &str, subject: &str, roles: &str) -> String {
        format!("{sha}{FIELD}{subject}{FIELD}{roles}{RECORD}")
    }

    #[test]
    fn a_commit_with_a_role_trailer_passes() {
        let log = record("abc1234", "NEX-37: check roles", "coder on NEX-37");
        assert!(missing_role(&log).is_empty());
    }

    #[test]
    fn a_commit_without_a_role_trailer_is_named() {
        let log = record("abc1234", "NEX-37: check roles", "coder on NEX-37")
            + "\n"
            + &record("def5678", "NEX-37: forgot the trailer", "");
        assert_eq!(missing_role(&log), vec!["def5678 NEX-37: forgot the trailer"]);
    }

    #[test]
    fn a_role_trailer_without_a_value_does_not_count() {
        let log = record("def5678", "NEX-37: empty role", "  \n");
        assert_eq!(missing_role(&log), vec!["def5678 NEX-37: empty role"]);
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=coder on NEX-37", "-c", "user.email=a@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo_with(messages: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "base"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "before the rule"]);
        git(dir.path(), &["checkout", "-q", "-b", "work"]);
        for message in messages {
            git(dir.path(), &["commit", "-q", "--allow-empty", "-m", message]);
        }
        dir
    }

    #[test]
    fn run_fails_on_a_branch_commit_without_the_trailer() {
        let dir = repo_with(&[
            "NEX-37: with role\n\nBody.\n\nRole: coder on NEX-37\nClaude-Session: https://x",
            "NEX-37: without role\n\nClaude-Session: https://x",
        ]);
        let err = run(dir.path(), "base", "work").unwrap_err();
        assert!(err.to_string().contains("1 commit(s)"), "{err}");
    }

    #[test]
    fn run_passes_when_every_branch_commit_names_its_role() {
        let dir = repo_with(&["NEX-37: with role\n\nRole: coder on NEX-37"]);
        run(dir.path(), "base", "work").unwrap();
    }
}
