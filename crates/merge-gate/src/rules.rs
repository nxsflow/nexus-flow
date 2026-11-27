//! Every rule the gate judges, read from the base commit (RFC 0010, "`nxs merge-gate rules`"): the
//! rules on what comes back to the owner from `factory/GENESIS.md`, in their order, then the rules
//! of `factory/merge-gate/gate.toml`, in theirs, with the gate's settings.
//!
//! Both files are read as git objects of the base commit, so an edit in the working tree or on the
//! pull request's head changes nothing. When anything cannot be read, the report holds no rule and
//! no setting, and names every problem: the gate then sends every pull request to the owner.

use std::path::Path;

use crate::gate_toml::{read_gate_toml, Effect, GateToml, GateTomlError, RuleRef};
use crate::genesis_rules::{read_genesis_rules, GenesisError, GenesisRule};
use crate::git::{read_file, resolve_commit};

pub use crate::git::GitError;

/// Where GENESIS.md lies, relative to the repository's root.
pub const GENESIS_PATH: &str = "factory/GENESIS.md";
/// Where gate.toml lies, relative to the repository's root.
pub const GATE_TOML_PATH: &str = "factory/merge-gate/gate.toml";

/// The gate's settings from `gate.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub default: Effect,
    pub owners: Vec<String>,
    pub verdict_authors: Vec<String>,
}

/// One rule the gate judges, with the file it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub effect: Effect,
    pub source: &'static str,
    pub text: String,
}

/// One reason the rules cannot be read: the file, the error's code, and the rule it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesError {
    pub source: &'static str,
    pub code: &'static str,
    pub rule: Option<RuleRef>,
}

/// The rules at a base commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesReport {
    /// The full object id of the base commit.
    pub base: String,
    /// The settings; `None` when anything cannot be read.
    pub settings: Option<Settings>,
    /// The rules, GENESIS's first; empty when anything cannot be read.
    pub rules: Vec<Rule>,
    /// Every problem, GENESIS's first; empty when the rules are readable.
    pub errors: Vec<RulesError>,
}

impl RulesReport {
    /// Whether every rule and setting could be read.
    pub fn readable(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The rules at the commit `base` names, for the repository whose root is `root`.
pub fn rules_at(root: &Path, base: &str) -> Result<RulesReport, GitError> {
    let base = resolve_commit(root, base)?;
    let genesis = read_file(root, &base, GENESIS_PATH)?;
    let gate = read_file(root, &base, GATE_TOML_PATH)?;
    // CommonMark reads invalid UTF-8 as U+FFFD; TOML must be UTF-8, so other bytes are invalid.
    let genesis = genesis.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let gate = match gate.map(String::from_utf8) {
        None => read_gate_toml(None),
        Some(Ok(text)) => read_gate_toml(Some(&text)),
        Some(Err(_)) => Err(vec![GateTomlError::TomlInvalid]),
    };
    Ok(report(base, read_genesis_rules(genesis.as_deref()), gate))
}

fn report(
    base: String,
    genesis: Result<Vec<GenesisRule>, GenesisError>,
    gate: Result<GateToml, Vec<GateTomlError>>,
) -> RulesReport {
    match (genesis, gate) {
        (Ok(genesis), Ok(gate)) => {
            let genesis = genesis.into_iter().map(|rule| Rule {
                id: rule.id,
                effect: Effect::Owner,
                source: GENESIS_PATH,
                text: rule.text,
            });
            let own = gate.rules.into_iter().map(|rule| Rule {
                id: rule.id,
                effect: rule.effect,
                source: GATE_TOML_PATH,
                text: rule.text,
            });
            RulesReport {
                base,
                settings: Some(Settings {
                    default: gate.default,
                    owners: gate.owners,
                    verdict_authors: gate.verdict_authors,
                }),
                rules: genesis.chain(own).collect(),
                errors: Vec::new(),
            }
        }
        (genesis, gate) => {
            let genesis = genesis.err().map(|e| RulesError {
                source: GENESIS_PATH,
                code: e.code(),
                rule: None,
            });
            let gate = gate.err().into_iter().flatten().map(|e| RulesError {
                source: GATE_TOML_PATH,
                code: e.code(),
                rule: e.rule().cloned(),
            });
            RulesReport {
                base,
                settings: None,
                rules: Vec::new(),
                errors: genesis.into_iter().chain(gate).collect(),
            }
        }
    }
}
