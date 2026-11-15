//! The commit-role gate (NEX-37): every commit a pull request brings carries a `Role:` trailer
//! naming the session role that made it, such as `Role: coder on NEX-37` or `Role: pm`.
//! `factory/FACTORY.md` states the rule; `.github/workflows/commit-roles.yml` runs this check.
//!
//! Three kinds of commit are left out, because no session can give them the trailer:
//! - merge commits: bringing a branch up to date makes one that no session wrote, and the squash
//!   merge drops it anyway;
//! - commits reachable from an excluded ref, such as `origin/main`, which keeps shipping v1
//!   without the factory's rules and merges into v2 every week;
//! - commits authored before the rule landed on the base branch, the moment the commit that
//!   added [`RULE_FILE`] was committed there, so that a PR started earlier is not rewritten.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The file whose arrival on the base branch puts the rule in force.
pub const RULE_FILE: &str = ".github/workflows/commit-roles.yml";

/// Separates the fields of one commit in the `git log` output this module reads.
const FIELD: char = '\u{1f}';
/// Ends one commit's record in that output.
const RECORD: char = '\u{1e}';

/// The commits in `log` (the output of `git log` with [`LOG_FORMAT`]) that carry no `Role:`
/// trailer with a value, as `<short sha> <subject>`, in the order git listed them. A commit
/// authored before `landed` (Unix seconds), the moment the rule landed, is left out.
pub fn missing_role(log: &str, landed: Option<i64>) -> Vec<String> {
    log.split(RECORD)
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(4, FIELD);
            let sha = fields.next().filter(|sha| !sha.is_empty())?;
            let authored = fields.next().and_then(|time| time.parse::<i64>().ok());
            if let (Some(authored), Some(landed)) = (authored, landed) {
                if authored < landed {
                    return None;
                }
            }
            let subject = fields.next().unwrap_or_default();
            let roles = fields.next().unwrap_or_default();
            let named = roles.lines().any(|role| !role.trim().is_empty());
            (!named).then(|| format!("{sha} {subject}"))
        })
        .collect()
}

/// The `git log` format [`missing_role`] reads: sha, author time (Unix seconds), subject, and
/// the values of the `Role` trailers, one per line.
pub const LOG_FORMAT: &str = "%h%x1f%at%x1f%s%x1f%(trailers:key=Role,valueonly,separator=%x0a)%x1e";

fn git_log(root: &Path, args: &[String]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .arg("log")
        .args(args)
        .output()
        .context("running git log")?;
    if !output.status.success() {
        bail!(
            "git log {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git log printed non-UTF-8")
}

/// When the rule landed on `base`: the commit time of the earliest commit there that added
/// [`RULE_FILE`], or `None` while it has not landed (then every commit is checked).
fn landed_on(root: &Path, base: &str) -> Result<Option<i64>> {
    let args = ["--diff-filter=A", "--format=%ct", base, "--", RULE_FILE].map(String::from);
    let times = git_log(root, &args)?;
    Ok(times.lines().filter_map(|time| time.parse().ok()).min())
}

/// Check every non-merge commit in `<base>..<head>` of the repository at `root` that no ref in
/// `exclude` reaches and that was authored after the rule landed on `base`.
pub fn run(root: &Path, base: &str, head: &str, exclude: &[String]) -> Result<()> {
    let mut args = vec![
        "--no-merges".to_string(),
        format!("--format={LOG_FORMAT}"),
        format!("{base}..{head}"),
    ];
    args.extend(exclude.iter().map(|excluded| format!("^{excluded}")));
    let log = git_log(root, &args)?;
    let missing = missing_role(&log, landed_on(root, base)?);
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
        record_at(sha, 1_000, subject, roles)
    }

    fn record_at(sha: &str, authored: i64, subject: &str, roles: &str) -> String {
        format!("{sha}{FIELD}{authored}{FIELD}{subject}{FIELD}{roles}{RECORD}")
    }

    #[test]
    fn a_commit_with_a_role_trailer_passes() {
        let log = record("abc1234", "NEX-37: check roles", "coder on NEX-37");
        assert!(missing_role(&log, None).is_empty());
    }

    #[test]
    fn a_commit_without_a_role_trailer_is_named() {
        let log = record("abc1234", "NEX-37: check roles", "coder on NEX-37")
            + "\n"
            + &record("def5678", "NEX-37: forgot the trailer", "");
        assert_eq!(
            missing_role(&log, None),
            vec!["def5678 NEX-37: forgot the trailer"]
        );
    }

    #[test]
    fn a_role_trailer_without_a_value_does_not_count() {
        let log = record("def5678", "NEX-37: empty role", "  \n");
        assert_eq!(missing_role(&log, None), vec!["def5678 NEX-37: empty role"]);
    }

    #[test]
    fn a_commit_authored_before_the_rule_landed_is_exempt() {
        let log = record_at("abc1234", 100, "NEX-9: older work", "")
            + "\n"
            + &record_at("def5678", 300, "NEX-9: after the rule", "");
        assert_eq!(
            missing_role(&log, Some(200)),
            vec!["def5678 NEX-9: after the rule"]
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        git_at(dir, "2026-11-15T12:00:00Z", args)
    }

    fn git_at(dir: &Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args([
                "-c",
                "user.name=coder on NEX-37",
                "-c",
                "user.email=a@example.com",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo_with(messages: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "base"]);
        git(
            dir.path(),
            &["commit", "-q", "--allow-empty", "-m", "before the rule"],
        );
        git(dir.path(), &["checkout", "-q", "-b", "work"]);
        for message in messages {
            git(
                dir.path(),
                &["commit", "-q", "--allow-empty", "-m", message],
            );
        }
        dir
    }

    #[test]
    fn run_fails_on_a_branch_commit_without_the_trailer() {
        let dir = repo_with(&[
            "NEX-37: with role\n\nBody.\n\nRole: coder on NEX-37\nClaude-Session: https://x",
            "NEX-37: without role\n\nClaude-Session: https://x",
        ]);
        let err = run(dir.path(), "base", "work", &[]).unwrap_err();
        assert!(err.to_string().contains("1 commit(s)"), "{err}");
    }

    #[test]
    fn run_passes_when_every_branch_commit_names_its_role() {
        let dir = repo_with(&["NEX-37: with role\n\nRole: coder on NEX-37"]);
        run(dir.path(), "base", "work", &[]).unwrap();
    }

    #[test]
    fn run_skips_commits_that_come_from_an_excluded_branch() {
        let dir = repo_with(&["NEX-37: with role\n\nRole: coder on NEX-37"]);
        git(dir.path(), &["checkout", "-q", "-b", "main", "base"]);
        git(
            dir.path(),
            &["commit", "-q", "--allow-empty", "-m", "v1 fix"],
        );
        git(dir.path(), &["checkout", "-q", "work"]);
        git(
            dir.path(),
            &["merge", "-q", "--no-ff", "-m", "Merge main", "main"],
        );
        run(dir.path(), "base", "work", &["main".to_string()]).unwrap();
    }

    #[test]
    fn run_exempts_commits_authored_before_the_rule_landed_on_the_base() {
        let dir = repo_with(&[]);
        git_at(
            dir.path(),
            "2026-11-15T13:00:00Z",
            &["commit", "-q", "--allow-empty", "-m", "old"],
        );
        git(dir.path(), &["checkout", "-q", "base"]);
        let workflows = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(workflows.join("commit-roles.yml"), "on: pull_request\n").unwrap();
        git(dir.path(), &["add", "."]);
        git_at(
            dir.path(),
            "2026-11-15T14:00:00Z",
            &["commit", "-q", "-m", "the rule lands"],
        );
        git(dir.path(), &["checkout", "-q", "work"]);
        git_at(
            dir.path(),
            "2026-11-15T15:00:00Z",
            &["commit", "-q", "--allow-empty", "-m", "new"],
        );
        let err = run(dir.path(), "base", "work", &[]).unwrap_err();
        assert!(err.to_string().starts_with("1 commit(s)"), "{err}");
    }

    /// The workflow checks only PRs against v2, and leaves out what comes from main.
    #[test]
    fn the_workflow_runs_on_v2_and_excludes_main() {
        let text = std::fs::read_to_string("../.github/workflows/commit-roles.yml").unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        let branches = &yaml["on"]["pull_request"]["branches"];
        assert_eq!(
            branches,
            &serde_yaml::from_str::<serde_yaml::Value>("[v2]").unwrap()
        );
        assert!(text.contains("--exclude origin/main"), "{text}");
        assert!(text.contains("git fetch --no-tags origin main"), "{text}");
    }
}
