//! The team's merge rules and the gate's settings, read from `factory/merge-gate/gate.toml`
//! format 1 (RFC 0010, "The merge rules").
//!
//! The file names what the gate decides when no rule says otherwise, the logins whose review counts
//! as the owner's and whose comments may carry the verdict, and the `[[rule]]`s, in file order.
//! Unknown keys are ignored, so a team may keep notes beside the rules.

use toml::{Table, Value};

/// What a rule, or the gate's default, does: send the pull request to the owner, or let pm merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Owner,
    Pm,
}

impl Effect {
    /// The effect written as a string in `gate.toml`, or `None` for any other value.
    fn parse(value: Option<&Value>) -> Option<Effect> {
        match value?.as_str()? {
            "owner" => Some(Effect::Owner),
            "pm" => Some(Effect::Pm),
            _ => None,
        }
    }
}

/// One `[[rule]]` of `gate.toml`: its id, its effect, and its Markdown text as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRule {
    pub id: String,
    pub effect: Effect,
    pub text: String,
}

/// A readable `gate.toml`: the settings and the rules, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateToml {
    pub default: Effect,
    pub owners: Vec<String>,
    pub verdict_authors: Vec<String>,
    pub rules: Vec<GateRule>,
}

/// Which rule an error concerns: its id when it has one as a string, else its 1-based position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleRef {
    Id(String),
    Position(usize),
}

/// Why `gate.toml` cannot be read. Each one sends every pull request to the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateTomlError {
    /// The file does not exist at the base commit.
    FileMissing,
    /// The file is not valid TOML, or `rule` is not a list of tables.
    TomlInvalid,
    /// `format` is not the integer `1`.
    FormatUnknown,
    /// `default` is not `owner` or `pm`.
    DefaultInvalid,
    /// `owners` is missing, empty, or holds anything but non-empty strings.
    OwnersEmpty,
    /// `verdict_authors` is missing, empty, or holds anything but non-empty strings.
    VerdictAuthorsEmpty,
    /// A rule's `id` does not match `^[a-z][a-z0-9-]{0,62}$`.
    RuleIdInvalid(RuleRef),
    /// A rule's `id` is the id of an earlier rule.
    RuleIdDuplicate(RuleRef),
    /// A rule's `effect` is not `owner` or `pm`.
    EffectInvalid(RuleRef),
    /// A rule's `text` is missing or holds only whitespace.
    TextEmpty(RuleRef),
}

impl GateTomlError {
    /// The error's code in `nxs merge-gate rules` (RFC 0010).
    pub fn code(&self) -> &'static str {
        match self {
            GateTomlError::FileMissing => "file_missing",
            GateTomlError::TomlInvalid => "toml_invalid",
            GateTomlError::FormatUnknown => "format_unknown",
            GateTomlError::DefaultInvalid => "default_invalid",
            GateTomlError::OwnersEmpty => "owners_empty",
            GateTomlError::VerdictAuthorsEmpty => "verdict_authors_empty",
            GateTomlError::RuleIdInvalid(_) => "rule_id_invalid",
            GateTomlError::RuleIdDuplicate(_) => "rule_id_duplicate",
            GateTomlError::EffectInvalid(_) => "effect_invalid",
            GateTomlError::TextEmpty(_) => "text_empty",
        }
    }

    /// The rule the error concerns, for the errors that concern one.
    pub fn rule(&self) -> Option<&RuleRef> {
        match self {
            GateTomlError::RuleIdInvalid(rule)
            | GateTomlError::RuleIdDuplicate(rule)
            | GateTomlError::EffectInvalid(rule)
            | GateTomlError::TextEmpty(rule) => Some(rule),
            _ => None,
        }
    }
}

/// The settings and rules in `source`, the text of `gate.toml`, or `None` when the file is missing.
/// When it cannot be read, every problem is named, in the order of the file.
pub fn read_gate_toml(source: Option<&str>) -> Result<GateToml, Vec<GateTomlError>> {
    let source = source.ok_or_else(|| vec![GateTomlError::FileMissing])?;
    let table: Table = source
        .parse()
        .map_err(|_| vec![GateTomlError::TomlInvalid])?;
    let rule_tables = rule_tables(&table).ok_or_else(|| vec![GateTomlError::TomlInvalid])?;

    let mut errors = Vec::new();
    if table.get("format").and_then(Value::as_integer) != Some(1) {
        errors.push(GateTomlError::FormatUnknown);
    }
    let default = Effect::parse(table.get("default"));
    if default.is_none() {
        errors.push(GateTomlError::DefaultInvalid);
    }
    let owners = logins(table.get("owners"));
    if owners.is_none() {
        errors.push(GateTomlError::OwnersEmpty);
    }
    let verdict_authors = logins(table.get("verdict_authors"));
    if verdict_authors.is_none() {
        errors.push(GateTomlError::VerdictAuthorsEmpty);
    }
    let rules = read_rules(&rule_tables, &mut errors);

    match (default, owners, verdict_authors) {
        (Some(default), Some(owners), Some(verdict_authors)) if errors.is_empty() => Ok(GateToml {
            default,
            owners,
            verdict_authors,
            rules,
        }),
        _ => Err(errors),
    }
}

/// The tables of `[[rule]]`, none when the key is absent, or `None` when it is not a list of tables.
fn rule_tables(table: &Table) -> Option<Vec<&Table>> {
    match table.get("rule") {
        None => Some(Vec::new()),
        Some(value) => value.as_array()?.iter().map(Value::as_table).collect(),
    }
}

/// A non-empty list of non-empty login strings, or `None`.
fn logins(value: Option<&Value>) -> Option<Vec<String>> {
    let logins: Vec<String> = value?
        .as_array()?
        .iter()
        .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
        .collect::<Option<_>>()?;
    (!logins.is_empty()).then_some(logins)
}

/// The rules that read whole, in file order; each problem of the others goes to `errors`.
fn read_rules(tables: &[&Table], errors: &mut Vec<GateTomlError>) -> Vec<GateRule> {
    let mut rules = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (i, rule) in tables.iter().enumerate() {
        let id = rule.get("id").and_then(Value::as_str);
        let rule_ref = match id {
            Some(id) => RuleRef::Id(id.to_string()),
            None => RuleRef::Position(i + 1),
        };
        let mut whole = true;
        match id {
            Some(id) if id_is_valid(id) && seen.contains(&id) => {
                errors.push(GateTomlError::RuleIdDuplicate(rule_ref.clone()));
                whole = false;
            }
            Some(id) if id_is_valid(id) => seen.push(id),
            _ => {
                errors.push(GateTomlError::RuleIdInvalid(rule_ref.clone()));
                whole = false;
            }
        }
        let effect = Effect::parse(rule.get("effect"));
        if effect.is_none() {
            errors.push(GateTomlError::EffectInvalid(rule_ref.clone()));
        }
        let text = rule
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty());
        if text.is_none() {
            errors.push(GateTomlError::TextEmpty(rule_ref));
        }
        if let (true, Some(id), Some(effect), Some(text)) = (whole, id, effect, text) {
            rules.push(GateRule {
                id: id.to_string(),
                effect,
                text: text.to_string(),
            });
        }
    }
    rules
}

/// Whether `id` matches `^[a-z][a-z0-9-]{0,62}$`.
fn id_is_valid(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
