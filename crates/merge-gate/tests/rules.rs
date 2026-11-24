//! NEX-171: every rule the gate judges, read from the base commit as git objects (RFC 0010, "Which
//! copy of the rules judges a pull request", "`nxs merge-gate rules`"): the GENESIS rules first,
//! then those of `gate.toml`; when anything cannot be read, no rule, no setting, and every error.

mod common;

use common::{Repo, GATE, GATE_PATH, GENESIS, GENESIS_PATH};
use nexus_flow_merge_gate::gate_toml::{Effect, RuleRef};
use nexus_flow_merge_gate::rules::{rules_at, GitError, Rule, RulesError, Settings};

fn rule(id: &str, effect: Effect, source: &'static str, text: &str) -> Rule {
    Rule {
        id: id.to_string(),
        effect,
        source,
        text: text.to_string(),
    }
}

#[test]
fn reads_every_rule_from_the_base_commit_genesis_first() {
    let repo = Repo::new("reads");
    repo.write(GENESIS_PATH, GENESIS);
    repo.write(GATE_PATH, GATE);
    let base = repo.commit("rules");

    let report = rules_at(&repo.path, "main").unwrap();

    assert_eq!(report.base, base);
    assert!(report.readable());
    assert_eq!(
        report.settings,
        Some(Settings {
            default: Effect::Owner,
            owners: vec!["cabcookie".into()],
            verdict_authors: vec!["nexusflow-ug".into()],
        })
    );
    assert_eq!(
        report.rules,
        vec![
            rule("genesis.1", Effect::Owner, GENESIS_PATH, "Anything a user sees first."),
            rule(
                "genesis.2",
                Effect::Owner,
                GENESIS_PATH,
                "A change to a stored format,\n  - such as a new field."
            ),
            rule(
                "decided-design",
                Effect::Pm,
                GATE_PATH,
                "The pull request only carries out decisions the owner has already made."
            ),
            rule(
                "stored-format",
                Effect::Owner,
                GATE_PATH,
                "The pull request changes a stored format."
            ),
        ]
    );
    assert!(report.errors.is_empty());
}

#[test]
fn a_working_tree_edit_to_genesis_does_not_change_the_rules() {
    let repo = Repo::new("worktree");
    repo.write(GENESIS_PATH, GENESIS);
    repo.write(GATE_PATH, GATE);
    repo.commit("rules");
    let before = rules_at(&repo.path, "main").unwrap();

    repo.write(GENESIS_PATH, "# GENESIS\n\nNo rules any more.\n");
    repo.remove(GATE_PATH);

    assert_eq!(rules_at(&repo.path, "main").unwrap(), before);
}

#[test]
fn reads_the_commit_base_names_not_the_head() {
    let repo = Repo::new("older");
    repo.write(GENESIS_PATH, GENESIS);
    repo.write(GATE_PATH, GATE);
    let base = repo.commit("rules");
    repo.write(GENESIS_PATH, "# GENESIS\n\nNo rules any more.\n");
    repo.commit("drop the rules");

    let report = rules_at(&repo.path, &base[..12]).unwrap();

    assert_eq!(report.base, base);
    assert!(report.readable());
    assert_eq!(report.rules.len(), 4);
}

#[test]
fn unreadable_rules_give_no_rule_no_setting_and_every_error() {
    let repo = Repo::new("unreadable");
    repo.write(GENESIS_PATH, "# GENESIS\n\nNo mark here.\n");
    repo.write(
        GATE_PATH,
        "format = 1\ndefault = \"owner\"\nowners = []\nverdict_authors = [\"x\"]\n\n[[rule]]\nid = \"a\"\neffect = \"maybe\"\ntext = \"t\"\n",
    );
    repo.commit("broken rules");

    let report = rules_at(&repo.path, "main").unwrap();

    assert!(!report.readable());
    assert!(report.rules.is_empty());
    assert_eq!(report.settings, None);
    assert_eq!(
        report.errors,
        vec![
            RulesError {
                source: GENESIS_PATH,
                code: "mark_missing",
                rule: None,
            },
            RulesError {
                source: GATE_PATH,
                code: "owners_empty",
                rule: None,
            },
            RulesError {
                source: GATE_PATH,
                code: "effect_invalid",
                rule: Some(RuleRef::Id("a".into())),
            },
        ]
    );
}

#[test]
fn a_readable_genesis_with_an_unreadable_gate_toml_gives_no_rule() {
    let repo = Repo::new("half");
    repo.write(GENESIS_PATH, GENESIS);
    repo.commit("genesis only");

    let report = rules_at(&repo.path, "main").unwrap();

    assert!(!report.readable());
    assert!(report.rules.is_empty());
    assert_eq!(report.settings, None);
    assert_eq!(
        report.errors,
        vec![RulesError {
            source: GATE_PATH,
            code: "file_missing",
            rule: None,
        }]
    );
}

#[test]
fn a_gate_toml_that_is_not_utf8_is_not_valid_toml() {
    let repo = Repo::new("bytes");
    repo.write(GENESIS_PATH, GENESIS);
    std::fs::create_dir_all(repo.path.join("factory/merge-gate")).unwrap();
    std::fs::write(repo.path.join(GATE_PATH), b"format = 1\n# \xff\n").unwrap();
    repo.commit("bytes");

    let report = rules_at(&repo.path, "main").unwrap();

    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "toml_invalid");
}

#[test]
fn a_base_that_is_no_commit_is_an_error_not_a_report() {
    let repo = Repo::new("no-base");
    repo.write(GENESIS_PATH, GENESIS);
    repo.commit("rules");

    let error = rules_at(&repo.path, "no-such-branch").unwrap_err();

    assert_eq!(
        error,
        GitError::NotACommit {
            reference: "no-such-branch".into()
        }
    );
}

#[test]
fn a_base_that_looks_like_an_option_is_no_commit() {
    let repo = Repo::new("option");
    repo.commit("empty");

    let error = rules_at(&repo.path, "--all").unwrap_err();

    assert!(matches!(error, GitError::NotACommit { .. }), "{error:?}");
}

#[test]
fn a_repository_root_below_the_git_root_reads_its_own_factory() {
    let repo = Repo::new("nested");
    repo.write(&format!("app/{GENESIS_PATH}"), GENESIS);
    repo.write(&format!("app/{GATE_PATH}"), GATE);
    repo.commit("nested");

    let report = rules_at(&repo.path.join("app"), "main").unwrap();

    assert!(report.readable(), "{:?}", report.errors);
    assert_eq!(report.rules.len(), 4);
}
