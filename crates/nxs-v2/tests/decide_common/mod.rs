//! What the tests of `Engine::merge_gate_decide` share: a scratch git repository whose `main`
//! holds readable rules, a CODEOWNERS file and a decided RFC, a pull request's head on the branch
//! `pr` that changes one uncovered file, and a valid verdict for it that lets pm merge.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use nexus_flow_seam::merge_gate::{DecideReport, Reason};
use nexus_flow_seam::{Engine, Error};
use serde_json::{json, Value};

pub const GENESIS_PATH: &str = "factory/GENESIS.md";
pub const GATE_PATH: &str = "factory/merge-gate/gate.toml";
pub const RFC_PATH: &str = "docs/rfc/0010-merge-gate.md";

/// A GENESIS.md with two owner rules.
pub const GENESIS: &str = "# GENESIS\n\n<!-- nexus-flow: owner-rules -->\n\n- Anything a user sees first.\n- A change to a stored format.\n";

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

/// CODEOWNERS covers `.github/` and the factory's rules, as this repository's does.
pub const CODEOWNERS: &str = "/.github/ @cabcookie\n/factory/ @cabcookie\n";

/// A scratch repository, removed when dropped, with the base and head of one pull request.
pub struct Pr {
    pub path: PathBuf,
    pub base: String,
    pub head: String,
}

impl Pr {
    /// `main` holds the rules, CODEOWNERS, the RFC and `src/lib.rs`; `pr` changes `src/lib.rs`.
    pub fn new(test: &str) -> Pr {
        Pr::with_base(test, &[(GENESIS_PATH, GENESIS), (GATE_PATH, GATE)])
    }

    /// As [`Pr::new`], with `files` at the base in place of the rules.
    pub fn with_base(test: &str, files: &[(&str, &str)]) -> Pr {
        let path =
            std::env::temp_dir().join(format!("nxs-v2-decide-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let mut pr = Pr {
            path: path.canonicalize().unwrap(),
            base: String::new(),
            head: String::new(),
        };
        pr.git(&["init", "-q", "-b", "main"]);
        pr.write(
            "nexus-flow.toml",
            "format = 1\n\n[space]\nrepository = \"0123456789abcdef0123456789abcdef\"\n",
        );
        pr.write(".github/CODEOWNERS", CODEOWNERS);
        pr.write(RFC_PATH, "# RFC 0010\n");
        pr.write("src/lib.rs", "// one\n");
        for (file, text) in files {
            pr.write(file, text);
        }
        pr.base = pr.commit("base");
        pr.git(&["checkout", "-q", "-b", "pr"]);
        pr.write("src/lib.rs", "// two\n");
        pr.head = pr.commit("head");
        pr
    }

    /// Writes `text` to `file`, relative to the root.
    pub fn write(&self, file: &str, text: &str) {
        let path = self.path.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Commits everything in the working tree, and returns the commit's full id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Commits a change on the head, and makes that commit the head.
    pub fn push(&mut self, file: &str, text: &str) {
        self.write(file, text);
        self.head = self.commit("push");
    }

    /// Runs git with fixed identities and dates, and returns its stdout.
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

    /// A valid verdict for this pull request's four rules that lets pm merge.
    pub fn verdict(&self) -> Value {
        json!({
            "format": 1,
            "base": self.base,
            "head": self.head,
            "verdicts": [
                { "rule": "genesis.1", "outcome": "not_concerned", "reason": "Nothing a user sees." },
                { "rule": "genesis.2", "outcome": "decided", "reason": "The format RFC 0010 names.",
                  "decision": { "path": RFC_PATH } },
                { "rule": "decided-design", "outcome": "met", "reason": "Only implements RFC 0010." },
                { "rule": "stored-format", "outcome": "not_concerned", "reason": "No stored format changes." }
            ]
        })
    }

    /// Decides with `verdict` given as a verdict file, and no reviews.
    pub fn decide(&self, verdict: &Value) -> DecideReport {
        self.decide_with(None, Some(verdict), None).unwrap()
    }

    /// Decides with the inputs given, each as JSON.
    pub fn decide_with(
        &self,
        comments: Option<&Value>,
        verdict: Option<&Value>,
        reviews: Option<&Value>,
    ) -> Result<DecideReport, Error> {
        let bytes = |v: Option<&Value>| v.map(|v| v.to_string().into_bytes());
        let (comments, verdict, reviews) = (bytes(comments), bytes(verdict), bytes(reviews));
        Engine::merge_gate_decide(
            &self.path,
            &self.base,
            &self.head,
            comments.as_deref(),
            verdict.as_deref(),
            reviews.as_deref(),
        )
    }
}

impl Drop for Pr {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The verdict on `rule` in `verdict`, to change it.
pub fn entry<'a>(verdict: &'a mut Value, rule: &str) -> &'a mut Value {
    verdict["verdicts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["rule"] == rule)
        .unwrap()
}

/// The codes of a report's reasons, in order.
pub fn codes(report: &DecideReport) -> Vec<&'static str> {
    report.reasons.iter().map(Reason::code).collect()
}

/// Asserts that the pull request waits for the owner: decision `owner`, status `pending`.
pub fn assert_waits_for_owner(report: &DecideReport) {
    assert_eq!(report.decision.as_str(), "owner", "{report:?}");
    assert_eq!(report.status.as_str(), "pending", "{report:?}");
    assert!(!report.owner_approved, "{report:?}");
}

/// Asserts that pm may merge: decision `pm`, status `success`.
pub fn assert_pm_merges(report: &DecideReport) {
    assert_eq!(report.decision.as_str(), "pm", "{report:?}");
    assert_eq!(report.status.as_str(), "success", "{report:?}");
}
