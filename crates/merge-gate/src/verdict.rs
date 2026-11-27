//! The verdict in format 1, checked against the rules at the base commit (RFC 0010, "The verdict",
//! and step 2 of "`nxs merge-gate decide`").
//!
//! A verdict is valid when it is a JSON object with `format` 1; its `base` and `head` are the ones
//! given; it has exactly one entry for each rule and none for an id that is not a rule; each
//! outcome is one its rule's effect allows; each `reason` holds more than whitespace; and
//! `decision` is present exactly for `decided`, with a `path` that names a file at the base commit.
//! Unknown fields are ignored. Every problem is named, one line each, so that the owner and pm see
//! them all at once.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::gate_toml::Effect;
use crate::git::is_file;
use crate::rules::{GitError, Rule};

/// What the verdict says of one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An `owner` rule: the pull request does nothing the rule names.
    NotConcerned,
    /// An `owner` rule: all it does under the rule carries out a decision the owner made.
    Decided,
    /// An `owner` rule: it falls under the rule and needs the owner.
    Applies,
    /// Either effect: the sub-agent cannot tell.
    Unsure,
    /// A `pm` rule: the pull request meets it.
    Met,
    /// A `pm` rule: the pull request does not meet it.
    NotMet,
}

impl Outcome {
    /// The outcome written as `outcome`, if a rule of `effect` allows it.
    fn parse(outcome: &str, effect: Effect) -> Option<Outcome> {
        match (effect, outcome) {
            (Effect::Owner, "not_concerned") => Some(Outcome::NotConcerned),
            (Effect::Owner, "decided") => Some(Outcome::Decided),
            (Effect::Owner, "applies") => Some(Outcome::Applies),
            (Effect::Pm, "met") => Some(Outcome::Met),
            (Effect::Pm, "not_met") => Some(Outcome::NotMet),
            (_, "unsure") => Some(Outcome::Unsure),
            _ => None,
        }
    }
}

/// The decision a `decided` outcome cites: a file at the base commit, and optionally its part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub path: String,
    pub section: Option<String>,
}

/// The verdict on one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleVerdict {
    pub rule: String,
    pub outcome: Outcome,
    pub reason: String,
    pub decision: Option<Decision>,
}

/// A valid verdict: the commits it judged, and one verdict per rule, in the rules' order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub base: String,
    pub head: String,
    pub verdicts: Vec<RuleVerdict>,
}

/// Why there is no valid verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictError {
    /// The verdict is not valid: every problem, one line each.
    Invalid(Vec<String>),
    /// The local git could not say whether a cited path is a file at the base commit.
    Git(GitError),
}

impl From<GitError> for VerdictError {
    fn from(e: GitError) -> Self {
        VerdictError::Git(e)
    }
}

/// The verdict in `verdict`, checked against `rules`, the rules at `base`, and the commits `base`
/// and `head`, both full object ids, in the repository whose root is `root`.
pub fn validate_verdict(
    root: &Path,
    base: &str,
    head: &str,
    rules: &[Rule],
    verdict: &[u8],
) -> Result<Verdict, VerdictError> {
    let invalid = |problem: &str| VerdictError::Invalid(vec![problem.to_string()]);
    let value: Value =
        serde_json::from_slice(verdict).map_err(|_| invalid("the verdict is not JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("the verdict is not a JSON object"))?;

    let mut problems = Vec::new();
    if object.get("format").and_then(Value::as_u64) != Some(1) {
        problems.push("format is not 1".to_string());
    }
    for (key, expected) in [("base", base), ("head", head)] {
        if object.get(key).and_then(Value::as_str) != Some(expected) {
            problems.push(format!("{key} is not {expected}"));
        }
    }
    let Some(entries) = object.get("verdicts").and_then(Value::as_array) else {
        problems.push("verdicts is missing or not a list".to_string());
        return Err(VerdictError::Invalid(problems));
    };

    let effects: HashMap<&str, Effect> = rules.iter().map(|r| (r.id.as_str(), r.effect)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut read: HashMap<String, RuleVerdict> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let position = i + 1;
        let Some(entry) = entry.as_object() else {
            problems.push(format!("entry {position} is not an object"));
            continue;
        };
        let Some(rule) = entry.get("rule").and_then(Value::as_str) else {
            problems.push(format!("entry {position} has no rule id"));
            continue;
        };
        let Some(&effect) = effects.get(rule) else {
            problems.push(format!("rule {rule} is not a rule at the base commit"));
            continue;
        };
        if !seen.insert(rule) {
            problems.push(format!("rule {rule} has more than one entry"));
            continue;
        }
        if let Some(verdict) = read_entry(root, base, rule, effect, entry, &mut problems)? {
            read.insert(rule.to_string(), verdict);
        }
    }
    for rule in rules {
        if !seen.contains(rule.id.as_str()) {
            problems.push(format!("rule {} has no entry", rule.id));
        }
    }

    if !problems.is_empty() {
        return Err(VerdictError::Invalid(problems));
    }
    Ok(Verdict {
        base: base.to_string(),
        head: head.to_string(),
        verdicts: rules
            .iter()
            .filter_map(|rule| read.remove(&rule.id))
            .collect(),
    })
}

/// One entry's verdict on `rule`, or `None` with its problems added to `problems`.
fn read_entry(
    root: &Path,
    base: &str,
    rule: &str,
    effect: Effect,
    entry: &Map<String, Value>,
    problems: &mut Vec<String>,
) -> Result<Option<RuleVerdict>, GitError> {
    let before = problems.len();
    let outcome = match entry.get("outcome") {
        None => {
            problems.push(format!("rule {rule} has no outcome"));
            None
        }
        Some(written) => {
            let outcome = written.as_str().and_then(|o| Outcome::parse(o, effect));
            if outcome.is_none() {
                let effect = match effect {
                    Effect::Owner => "an owner",
                    Effect::Pm => "a pm",
                };
                problems.push(format!(
                    "rule {rule}: outcome {written} is not one {effect} rule allows"
                ));
            }
            outcome
        }
    };
    let reason = entry.get("reason").and_then(Value::as_str);
    if reason.is_none_or(|r| r.trim().is_empty()) {
        problems.push(format!("rule {rule} has no reason"));
    }
    let decision = match (outcome, entry.get("decision")) {
        (Some(Outcome::Decided), None) => {
            problems.push(format!("rule {rule} is decided but cites no decision"));
            None
        }
        (Some(Outcome::Decided), Some(decision)) => {
            read_decision(root, base, rule, decision, problems)?
        }
        (Some(_), Some(_)) => {
            problems.push(format!(
                "rule {rule} cites a decision but its outcome is not decided"
            ));
            None
        }
        (_, _) => None,
    };
    if problems.len() > before {
        return Ok(None);
    }
    Ok(outcome.map(|outcome| RuleVerdict {
        rule: rule.to_string(),
        outcome,
        reason: reason.unwrap_or_default().to_string(),
        decision,
    }))
}

/// The decision `decision` cites, when its `path` is a file at the base commit.
fn read_decision(
    root: &Path,
    base: &str,
    rule: &str,
    decision: &Value,
    problems: &mut Vec<String>,
) -> Result<Option<Decision>, GitError> {
    let Some(path) = decision.get("path").and_then(Value::as_str) else {
        problems.push(format!("rule {rule}: the decision has no path"));
        return Ok(None);
    };
    let section = match decision.get("section") {
        None => None,
        Some(Value::String(section)) => Some(section.clone()),
        Some(_) => {
            problems.push(format!(
                "rule {rule}: the decision's section is not a string"
            ));
            return Ok(None);
        }
    };
    if !is_file(root, base, path)? {
        problems.push(format!(
            "rule {rule}: the decision's path {path} is not a file at the base commit"
        ));
        return Ok(None);
    }
    Ok(Some(Decision {
        path: path.to_string(),
        section,
    }))
}
