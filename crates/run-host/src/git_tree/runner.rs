//! One bounded git child (§5): the environment and the `-c` list every call gets, output as bytes, and what a failed
//! call turns into. The gate, the group, the ledger and the kill are `TreeCx::spawn`'s.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nexus_run_contracts::{GitReason, SequenceKind, TreeRefusal};

use super::{GitBounds, Stopped};
use crate::tree_worker::{StepOutput, TreeCx};

/// Removed from every call: each one could point git somewhere else than the directory it was started in.
pub(crate) const REMOVED_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_CEILING_DIRECTORIES",
];

/// Set on every call. `GIT_ALLOW_PROTOCOL` keeps a URL from running a transport of its own (`ext::`). An empty
/// `core.sshCommand` alone stops ssh altogether ("cannot run", measured on j6ba's step 0), because git takes the
/// empty value as the command; `GIT_SSH_COMMAND` is read before it, so a repository's config still picks nothing and
/// an ssh origin works.
pub(crate) const SET_ENV: [(&str, &str); 7] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_ASKPASS", ""),
    ("SSH_ASKPASS", ""),
    ("GIT_PAGER", "cat"),
    ("GIT_ALLOW_PROTOCOL", "https:ssh:file"),
    ("GIT_SSH_COMMAND", "ssh"),
];

/// Before every subcommand.
pub(crate) const CONFIG: [&str; 4] = [
    "core.hooksPath=",
    "gc.auto=0",
    "core.fsmonitor=false",
    "core.sshCommand=",
];

/// A git call in `cwd`, which is where discovery starts and ends: a directory that is not a repository answers so,
/// even inside another repository, which is what `not_a_repository` rests on.
pub(crate) fn command(git: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(git);
    command.current_dir(cwd);
    for name in REMOVED_ENV {
        command.env_remove(name);
    }
    for (name, value) in SET_ENV {
        command.env(name, value);
    }
    if let Some(parent) = cwd.parent() {
        command.env("GIT_CEILING_DIRECTORIES", parent);
    }
    for config in CONFIG {
        command.args(["-c", config]);
    }
    command.args(args);
    command
}

/// git's calls, through the worker.
pub(crate) struct Git<'a, 'b> {
    program: PathBuf,
    /// The directory of the operation, whatever a call's `cwd`. A child answers for the locks of this repository and
    /// of no other: a read in the root must never clear a lock the old engine holds there.
    dir: PathBuf,
    pub(crate) bounds: GitBounds,
    pub(crate) cx: &'a mut TreeCx<'b>,
}

impl<'a, 'b> Git<'a, 'b> {
    pub(crate) fn new(
        program: &Path,
        dir: &Path,
        bounds: GitBounds,
        cx: &'a mut TreeCx<'b>,
    ) -> Self {
        Self {
            program: program.to_owned(),
            dir: dir.to_owned(),
            bounds,
            cx,
        }
    }

    /// One call, whatever it answers.
    pub(crate) fn call(
        &mut self,
        step: &str,
        cwd: &Path,
        args: &[&str],
        bound: Duration,
    ) -> Result<StepOutput, Stopped> {
        let command = command(&self.program, cwd, args);
        Ok(self.cx.spawn(step, command, bound, &self.dir)?)
    }

    /// One call under the common bound, whatever it answers: for the questions where non-zero is an answer.
    pub(crate) fn ask(
        &mut self,
        step: &str,
        cwd: &Path,
        args: &[&str],
    ) -> Result<StepOutput, Stopped> {
        self.call(step, cwd, args, self.bounds.other)
    }

    /// One call that has to succeed.
    pub(crate) fn run(
        &mut self,
        step: &str,
        cwd: &Path,
        args: &[&str],
    ) -> Result<StepOutput, Stopped> {
        let output = self.ask(step, cwd, args)?;
        if output.success {
            Ok(output)
        } else {
            Err(failed(step, &output))
        }
    }

    /// The one-line answer of a call that has to succeed.
    pub(crate) fn text(
        &mut self,
        step: &str,
        cwd: &Path,
        args: &[&str],
    ) -> Result<String, Stopped> {
        let output = self.run(step, cwd, args)?;
        line_of(step, &output)
    }
}

/// A name or a commit id, strictly: a lossy branch name would be passed back to git as another branch (6j6v.2af2).
pub(crate) fn line_of(step: &str, output: &StepOutput) -> Result<String, Stopped> {
    match std::str::from_utf8(&output.stdout) {
        Ok(text) => Ok(text.trim().to_owned()),
        Err(_) => Err(Stopped::Refused(
            exit_nonzero(),
            format!("git {step}: the answer is not UTF-8"),
        )),
    }
}

pub(crate) fn exit_nonzero() -> TreeRefusal {
    TreeRefusal::Git {
        reason: GitReason::ExitNonzero,
    }
}

/// A call that failed, with git's own stderr for the kv row. The closed reason is all the log gets.
pub(crate) fn failed(step: &str, output: &StepOutput) -> Stopped {
    Stopped::Refused(
        exit_nonzero(),
        format!("git {step}: {}", String::from_utf8_lossy(&output.stderr)),
    )
}

/// The sequence the tree is in the middle of, from the files git keeps for it (the list `crates/chat/src/park.rs` keeps in `mid_sequence`).
pub(crate) fn sequence_in_progress(dir: &Path) -> Option<SequenceKind> {
    let git = dir.join(".git");
    [
        ("MERGE_HEAD", SequenceKind::Merge),
        ("rebase-merge", SequenceKind::Rebase),
        ("rebase-apply", SequenceKind::Rebase),
        ("CHERRY_PICK_HEAD", SequenceKind::CherryPick),
        ("REVERT_HEAD", SequenceKind::Revert),
        ("BISECT_LOG", SequenceKind::Bisect),
    ]
    .into_iter()
    .find_map(|(file, kind)| git.join(file).exists().then_some(kind))
}

/// The same repository however it is written: scheme, user, `.git` suffix and trailing slash make no difference, and
/// neither do the scheme's and the host's case. The result names no credential.
pub fn normalise_origin_url(url: &str) -> String {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, url),
    };
    let normal = match (scheme.as_deref(), rest.split_once('/')) {
        (Some("file"), _) => rest.to_owned(),
        (Some(_), Some((authority, path))) => {
            format!("{}/{path}", host_of(authority))
        }
        (Some(_), None) => host_of(rest),
        // `user@host:path`, git's scp form, unless a slash comes first: then it is a path.
        (None, _) => match rest.split_once(':') {
            Some((authority, path)) if !authority.contains('/') => {
                format!("{}/{}", host_of(authority), path.trim_start_matches('/'))
            }
            _ => rest.to_owned(),
        },
    };
    let normal = normal.trim_end_matches('/');
    normal.strip_suffix(".git").unwrap_or(normal).to_owned()
}

fn host_of(authority: &str) -> String {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_ascii_lowercase()
}

const SCRUB_CAP: usize = 2_000;

/// What may reach a kv row of git's text: capped, and with anything between `://` and `@` removed, so a token in a URL
/// never gets there.
pub fn scrub(text: &str) -> String {
    let mut clean = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("://") {
        let (head, tail) = rest.split_at(start + 3);
        clean.push_str(head);
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        rest = match tail[..end].rfind('@') {
            Some(at) => &tail[at + 1..],
            None => tail,
        };
    }
    clean.push_str(rest);
    clean.chars().take(SCRUB_CAP).collect()
}
