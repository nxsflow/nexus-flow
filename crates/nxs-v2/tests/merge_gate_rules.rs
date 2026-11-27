//! NEX-171: `nxs merge-gate rules [--base <ref>]` (RFC 0010, "`nxs merge-gate rules`"): every rule
//! the gate judges, read from the base commit as git objects, as JSON by default and Markdown with
//! `--md`. Unreadable rules print the report and exit non-zero. It opens no database, and its
//! result is `Engine::merge_gate_rules`'s.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nexus_flow_seam::merge_gate::RuleRef;
use nexus_flow_seam::Engine;
use nxs_v2::Outcome;
use serde_json::{json, Value};

const SPACE: &str = "0123456789abcdef0123456789abcdef";
const GENESIS: &str = "# GENESIS\n\n<!-- nexus-flow: owner-rules -->\n\n- Anything a user sees first.\n- A change to a stored format,\n  - such as a new field.\n";
const GATE: &str = r#"format = 1
default = "owner"
owners = ["cabcookie"]
verdict_authors = ["nexusflow-ug"]

[[rule]]
id = "decided-design"
effect = "pm"
text = """
The pull request only carries out decisions the owner has already made.
"""
"#;
const BROKEN_GATE: &str = r#"format = 1
default = "owner"
owners = ["cabcookie"]
verdict_authors = []

[[rule]]
id = "a"
effect = "maybe"
text = "t"

[[rule]]
effect = "pm"
text = "no id"
"#;

/// A git repository for one test, on `main`, with `nexus-flow.toml` in its working tree. Its
/// commits have fixed authors and dates, so the same files give the same commit ids.
struct Repo {
    path: PathBuf,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let path = std::env::temp_dir().join(format!("nxs-v2-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let repo = Repo {
            path: path.canonicalize().unwrap(),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        fs::write(
            repo.path.join("nexus-flow.toml"),
            format!("format = 1\n\n[space]\nrepository = \"{SPACE}\"\n"),
        )
        .unwrap();
        repo
    }

    /// A repository whose `main` holds `genesis` and `gate`, committed.
    fn with_rules(test: &str, genesis: &str, gate: &str) -> Repo {
        let repo = Repo::new(test);
        repo.write("factory/GENESIS.md", genesis);
        repo.write("factory/merge-gate/gate.toml", gate);
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "rules"]);
        repo
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.path.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
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
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Runs `nxs <args>` in `dir`; a command that asks for the database's options fails the test.
    fn nxs_in(&self, dir: &Path, args: &[&str]) -> Outcome {
        let argv = std::iter::once("nxs").chain(args.iter().copied());
        nxs_v2::run(argv, dir, || {
            panic!("nxs merge-gate rules opens no database")
        })
    }

    fn nxs(&self, args: &[&str]) -> Outcome {
        self.nxs_in(&self.path, args)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn stdout(outcome: &Outcome) -> String {
    String::from_utf8(outcome.stdout.clone()).unwrap()
}

fn json_of(outcome: &Outcome) -> Value {
    serde_json::from_slice(&outcome.stdout)
        .unwrap_or_else(|e| panic!("stdout is JSON ({e}): {}", stdout(outcome)))
}

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn json_by_default_lists_every_rule_genesis_first() {
    let repo = Repo::with_rules("gate-rules-json", GENESIS, GATE);
    let base = repo.git(&["rev-parse", "main"]);

    let out = repo.nxs(&["merge-gate", "rules"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    assert_eq!(
        json_of(&out),
        json!({
            "base": base,
            "readable": true,
            "default": "owner",
            "owners": ["cabcookie"],
            "verdict_authors": ["nexusflow-ug"],
            "rules": [
                { "id": "genesis.1", "effect": "owner", "source": "factory/GENESIS.md",
                  "text": "Anything a user sees first." },
                { "id": "genesis.2", "effect": "owner", "source": "factory/GENESIS.md",
                  "text": "A change to a stored format,\n  - such as a new field." },
                { "id": "decided-design", "effect": "pm", "source": "factory/merge-gate/gate.toml",
                  "text": "The pull request only carries out decisions the owner has already made.\n" },
            ],
            "errors": [],
        })
    );
}

#[test]
fn the_same_local_copy_gives_the_golden_bytes_every_time() {
    let repo = Repo::with_rules("gate-rules-golden", GENESIS, GATE);

    let first = repo.nxs(&["merge-gate", "rules"]);
    let second = repo.nxs(&["merge-gate", "rules"]);
    let md = repo.nxs(&["merge-gate", "rules", "--md"]);
    let md_again = repo.nxs(&["--md", "merge-gate", "rules"]);

    assert_eq!(stdout(&first), golden("merge-gate-rules.json"));
    assert_eq!(second.stdout, first.stdout);
    assert_eq!(md.code, 0);
    assert_eq!(stdout(&md), golden("merge-gate-rules.md"));
    assert_eq!(md_again.stdout, md.stdout);
}

#[test]
fn a_working_tree_edit_to_genesis_does_not_change_the_output() {
    let repo = Repo::with_rules("gate-rules-worktree", GENESIS, GATE);
    let before = repo.nxs(&["merge-gate", "rules"]);

    repo.write("factory/GENESIS.md", "# GENESIS\n\nThe rules are gone.\n");
    repo.write("factory/merge-gate/gate.toml", "format = 2\n");
    let after = repo.nxs(&["merge-gate", "rules"]);

    assert_eq!(after.code, 0, "{}", stdout(&after));
    assert_eq!(stdout(&after), stdout(&before));
}

#[test]
fn unreadable_rules_print_the_report_and_exit_non_zero() {
    let repo = Repo::with_rules("gate-rules-unreadable", "# GENESIS\n", BROKEN_GATE);
    let base = repo.git(&["rev-parse", "main"]);

    let out = repo.nxs(&["merge-gate", "rules"]);

    assert_ne!(out.code, 0);
    assert!(out.stderr.is_empty());
    assert_eq!(
        json_of(&out),
        json!({
            "base": base,
            "readable": false,
            "default": null,
            "owners": null,
            "verdict_authors": null,
            "rules": [],
            "errors": [
                { "source": "factory/GENESIS.md", "code": "mark_missing" },
                { "source": "factory/merge-gate/gate.toml", "code": "verdict_authors_empty" },
                { "source": "factory/merge-gate/gate.toml", "code": "effect_invalid", "rule": "a" },
                { "source": "factory/merge-gate/gate.toml", "code": "rule_id_invalid", "rule": 2 },
            ],
        })
    );
}

#[test]
fn unreadable_rules_give_the_golden_bytes_in_both_formats() {
    let repo = Repo::with_rules("gate-rules-unreadable-golden", "# GENESIS\n", BROKEN_GATE);

    let json = repo.nxs(&["merge-gate", "rules"]);
    let md = repo.nxs(&["merge-gate", "rules", "--md"]);

    assert_eq!(stdout(&json), golden("merge-gate-rules-unreadable.json"));
    assert_ne!(md.code, 0);
    assert!(md.stderr.is_empty());
    assert_eq!(stdout(&md), golden("merge-gate-rules-unreadable.md"));
}

#[test]
fn base_names_the_commit_to_read() {
    let repo = Repo::with_rules("gate-rules-base", GENESIS, GATE);
    let old = repo.git(&["rev-parse", "main"]);
    repo.write("factory/GENESIS.md", "# GENESIS\n");
    repo.git(&["commit", "-q", "-am", "drop the rules"]);

    let head = repo.nxs(&["merge-gate", "rules"]);
    let at_old = repo.nxs(&["merge-gate", "rules", "--base", &old[..10]]);

    assert_ne!(head.code, 0);
    assert_eq!(at_old.code, 0, "{}", stdout(&at_old));
    assert_eq!(json_of(&at_old)["base"], json!(old));
}

#[test]
fn a_base_that_is_no_commit_is_a_validation_error() {
    let repo = Repo::with_rules("gate-rules-no-base", GENESIS, GATE);

    let out = repo.nxs(&["merge-gate", "rules", "--base", "nowhere"]);

    assert_eq!(out.code, 1);
    assert_eq!(json_of(&out)["error"]["kind"], "validation");
}

#[test]
fn outside_a_repository_it_names_no_workspace() {
    let repo = Repo::new("gate-rules-no-workspace");
    fs::remove_file(repo.path.join("nexus-flow.toml")).unwrap();

    let out = repo.nxs(&["merge-gate", "rules", "--base", "main"]);

    assert_eq!(out.code, 1);
    assert_eq!(json_of(&out)["error"]["kind"], "no_workspace");
}

#[test]
fn the_command_prints_what_the_engine_returns() {
    let repo = Repo::with_rules("gate-rules-parity", GENESIS, BROKEN_GATE);
    for (genesis, gate) in [(GENESIS, GATE), ("# GENESIS\n", BROKEN_GATE)] {
        repo.write("factory/GENESIS.md", genesis);
        repo.write("factory/merge-gate/gate.toml", gate);
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "--allow-empty", "-m", "rules"]);
        let nested = repo.path.join("src");
        fs::create_dir_all(&nested).unwrap();

        let printed = json_of(&repo.nxs_in(&nested, &["merge-gate", "rules"]));
        let report = Engine::merge_gate_rules(&nested, None).unwrap();

        assert_eq!(printed["base"], json!(report.base));
        assert_eq!(printed["readable"], json!(report.readable()));
        let settings = report.settings.as_ref();
        assert_eq!(
            printed["default"],
            json!(settings.map(|s| effect(s.default)))
        );
        assert_eq!(printed["owners"], json!(settings.map(|s| s.owners.clone())));
        assert_eq!(
            printed["verdict_authors"],
            json!(settings.map(|s| s.verdict_authors.clone()))
        );
        let rules: Vec<Value> = report
            .rules
            .iter()
            .map(|r| json!([r.id, effect(r.effect), r.source, r.text]))
            .collect();
        let printed_rules: Vec<Value> = printed["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| json!([r["id"], r["effect"], r["source"], r["text"]]))
            .collect();
        assert_eq!(printed_rules, rules);
        let errors: Vec<Value> = report
            .errors
            .iter()
            .map(|e| {
                let rule = match &e.rule {
                    Some(RuleRef::Id(id)) => json!(id),
                    Some(RuleRef::Position(n)) => json!(n),
                    None => Value::Null,
                };
                json!([e.source, e.code, rule])
            })
            .collect();
        let printed_errors: Vec<Value> = printed["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| json!([e["source"], e["code"], e["rule"]]))
            .collect();
        assert_eq!(printed_errors, errors);
    }
}

fn effect(effect: nexus_flow_seam::merge_gate::Effect) -> &'static str {
    match effect {
        nexus_flow_seam::merge_gate::Effect::Owner => "owner",
        nexus_flow_seam::merge_gate::Effect::Pm => "pm",
    }
}
