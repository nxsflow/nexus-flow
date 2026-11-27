//! What the tests that read rules from git share: a scratch git repository whose commits have
//! fixed authors and dates, so the same files give the same commit ids on every machine.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// A scratch git repository, removed when dropped.
pub struct Repo {
    pub path: PathBuf,
}

impl Repo {
    /// A fresh repository for one test, on the branch `main`.
    pub fn new(test: &str) -> Repo {
        let path = std::env::temp_dir().join(format!("merge-gate-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let repo = Repo {
            path: path.canonicalize().unwrap(),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    /// Writes `text` to `file`, a path relative to the repository's root.
    pub fn write(&self, file: &str, text: &str) {
        let path = self.path.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Removes `file` from the working tree.
    pub fn remove(&self, file: &str) {
        fs::remove_file(self.path.join(file)).unwrap();
    }

    /// Commits everything in the working tree, and returns the commit's full id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Runs git in the repository with fixed identities and dates, and returns its stdout.
    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(&self.path)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A GENESIS.md with two owner rules, the second with a nested item.
pub const GENESIS: &str = "# GENESIS\n\n<!-- nexus-flow: owner-rules -->\n\n- Anything a user sees first.\n- A change to a stored format,\n  - such as a new field.\n";

/// A readable gate.toml with one rule of each effect.
pub const GATE: &str = r#"format = 1
default = "owner"
owners = ["cabcookie"]
verdict_authors = ["nexusflow-ug"]

[[rule]]
id = "decided-design"
effect = "pm"
text = "The pull request only carries out decisions the owner has already made."

[[rule]]
id = "stored-format"
effect = "owner"
text = "The pull request changes a stored format."
"#;

pub const GENESIS_PATH: &str = "factory/GENESIS.md";
pub const GATE_PATH: &str = "factory/merge-gate/gate.toml";
