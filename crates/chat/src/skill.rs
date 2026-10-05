//! A persona declared as a SKILL (nxf 6j6v.dw16, epic 6j6v.phcx): a folder
//! `.nxs-personas/<name>/SKILL.md` in the form of the Agent Skills specification
//! (<https://agentskills.io/specification>), read into the same [`RoleDecl`] the YAML form has
//! always produced.
//!
//! **The point is that a published skill needs no rewriting.** Copied unchanged into the
//! declaration folder it is a persona the next time a catalogue is resolved — so this parser takes
//! the specification's fields at the top level and nexus-flow's own under ONE key, `nxs:`, and
//! leaves everything else a skill may carry alone: Claude Code adds about fifteen fields of its own
//! at the top level (`when_to_use`, `model`, `hooks`, …) and other runtimes will add more.
//!
//! | Skill form | [`RoleDecl`] |
//! | --- | --- |
//! | `name` | `handle` — the source of truth; the folder name is convention |
//! | `description` | `job_description` |
//! | the body after the frontmatter | `system_prompt` |
//! | `allowed-tools` | `tools`, with its three states (see [`parse_allowed_tools`]) |
//! | `nxs.title` | `job_title` |
//! | `nxs.<field>` | the YAML field of the same name, through the same parser and default |
//! | `nxs.requires` | nothing — read and shown only ([`DeclarationFile::requires`]) |
//! | `license`, `compatibility`, `metadata` | nothing — passed through ([`DeclarationFile`]) |
//!
//! **`allowed-tools` pre-approves in the specification and in Claude Code; it does not restrict.**
//! Mapping it onto `tools:` is still faithful here, because `tools:` already does both halves: its
//! entries are handed to the SDK as the approval list verbatim and, reduced to their tool names, as
//! the base toolset (`agent-sidecar/src/spec-helpers.mjs`). A persona runs without anybody at the
//! keyboard to approve a prompt, so a tool it was not approved for is a tool it cannot use.
//!
//! **Relative references in the body mean the persona's OWN folder** — the specification's rule,
//! and deliberately not the rule a YAML declaration keeps (relative to the repository the session
//! runs in). The session is told which rule applies to it: the "Declared in" line of its brief
//! ([`crate::persona::PersonaBrief::declared_in`]).

use std::path::Path;

use serde_yaml::{Mapping, Value};

use crate::definitions::{DeclarationFile, DeclarationForm, DeclarationKind, Ignored};
use crate::error::{NxfError, Result};
use crate::role::RoleDecl;

/// The file a skill folder must carry.
pub const SKILL_FILE: &str = "SKILL.md";

/// The key every nexus-flow field of a skill lives under.
pub const NXS_KEY: &str = "nxs";

/// What a skill may declare under `nxs:` — the YAML fields of the same name (with `title` for
/// `job_title`, because beside the specification's `name` and `description` a `job_` prefix says
/// nothing), plus `requires`.
pub const NXS_FIELDS: &[&str] = &[
    "title",
    "expected_output",
    "stage",
    "model",
    "addressable",
    "prime",
    "claude_md",
    "base_prompt",
    "permissions",
    "working_tree",
    "machine",
    "requires",
];

/// Keys that are read and IGNORED, with a declaration warning, in either form: `address_book` is
/// retired (nxf 6j6v.xjh3 — the persona addressed says who may address it, and a caller's view is
/// derived from that), the other three have never had an effect.
pub const RETIRED_FIELD: &str = "address_book";
pub const INERT_FIELDS: &[&str] = &["session", "sub_agents", "reports_to"];

/// Keys that would say under `nxs:` what the specification's own top-level fields say — refused,
/// because a persona whose tools or prompt are written twice has no single answer, and picking one
/// silently is the guess this loader does not make.
const TOP_LEVEL_ONLY: &[(&str, &str)] = &[
    ("name", "name"),
    ("handle", "name"),
    ("description", "description"),
    ("job_description", "description"),
    ("allowed-tools", "allowed-tools"),
    ("tools", "allowed-tools"),
    ("system_prompt", "the body of SKILL.md"),
    ("job_title", "nxs.title"),
];

/// nexus-flow keys written at the TOP level of a skill instead of under `nxs:`. Ignored there — a
/// top-level key belongs to whatever runtime defines it — but said, because `addressable: none`
/// silently ignored opens a persona its author meant to close. `model` is not on the list: it is a
/// Claude Code field of its own at the top level, with its own meaning.
const NXS_ONLY: &[&str] = &[
    "title",
    "job_title",
    "expected_output",
    "stage",
    "addressable",
    "prime",
    "claude_md",
    "base_prompt",
    "permissions",
    "working_tree",
    "machine",
    "requires",
    "address_book",
    "handle",
    "system_prompt",
    "tools",
    "job_description",
];

/// One persona as loaded from a folder: the declaration every seam resolves through, and the
/// account of the file it came from.
#[derive(Debug, Clone)]
pub struct LoadedPersona {
    pub decl: RoleDecl,
    pub file: DeclarationFile,
}

/// Load `<folder>/SKILL.md` as a persona.
pub fn load_skill(path: &Path) -> Result<LoadedPersona> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| NxfError::io(format!("reading skill file {}: {e}", path.display())))?;
    parse_skill(path, &content)
}

/// [`load_skill`] over text already in hand, read as if it were the file at `path` — what the
/// migration uses to prove a rewrite before it writes it ([`crate::persona_migration`]).
pub fn parse_skill(path: &Path, content: &str) -> Result<LoadedPersona> {
    let invalid = |what: String| NxfError::validation(format!("{}: {what}", path.display()));
    let (frontmatter, body) = split_frontmatter(content).ok_or_else(|| {
        invalid(
            "a SKILL.md starts with a YAML frontmatter between two `---` lines, and this one \
             does not"
                .to_string(),
        )
    })?;
    let top: Mapping = match serde_yaml::from_str::<Value>(frontmatter)
        .map_err(|e| invalid(format!("parsing the frontmatter: {e}")))?
    {
        Value::Mapping(map) => map,
        Value::Null => Mapping::new(),
        _ => {
            return Err(invalid(
                "the frontmatter is not a map of fields".to_string(),
            ))
        }
    };

    let name = match top.get("name") {
        Some(Value::String(name)) => name.clone(),
        Some(_) => return Err(invalid("`name` is not text".to_string())),
        None => {
            return Err(invalid(
                "`name` is not declared — a skill's `name` is the persona's handle".to_string(),
            ))
        }
    };

    let nxs = match top.get(NXS_KEY) {
        None | Some(Value::Null) => Mapping::new(),
        Some(Value::Mapping(map)) => map.clone(),
        Some(_) => return Err(invalid("`nxs:` is not a map of fields".to_string())),
    };

    let mut ignored = Vec::new();
    let mut role = Mapping::new();
    role.insert("handle".into(), Value::String(name.clone()));
    role.insert("system_prompt".into(), Value::String(body.to_string()));
    match top.get("description") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => {
            role.insert("job_description".into(), Value::String(text.clone()));
        }
        Some(_) => return Err(invalid("`description` is not text".to_string())),
    }
    if let Some(value) = top.get("allowed-tools") {
        let tools =
            parse_allowed_tools(value).map_err(|e| invalid(format!("`allowed-tools` {e}")))?;
        role.insert(
            "tools".into(),
            Value::Sequence(tools.into_iter().map(Value::String).collect()),
        );
    }

    let mut requires = Vec::new();
    for (key, value) in &nxs {
        let Some(key) = key.as_str() else {
            return Err(invalid("a key under `nxs:` is not text".to_string()));
        };
        if let Some((_, instead)) = TOP_LEVEL_ONLY.iter().find(|(k, _)| *k == key) {
            return Err(invalid(format!(
                "`nxs.{key}` is not a field of the skill form — write it as {instead}"
            )));
        }
        match key {
            "title" => {
                role.insert("job_title".into(), value.clone());
            }
            "requires" => {
                requires =
                    parse_requires(value).map_err(|e| invalid(format!("`nxs.requires` {e}")))?;
            }
            RETIRED_FIELD => ignored.push(Ignored::AddressBook {
                entries: value.as_sequence().map(Vec::len).unwrap_or(0),
            }),
            k if INERT_FIELDS.contains(&k) => ignored.push(Ignored::Inert(k.to_string())),
            k if NXS_FIELDS.contains(&k) => {
                role.insert(k.into(), value.clone());
            }
            other => ignored.push(Ignored::Unknown(format!("nxs.{other}"))),
        }
    }
    for key in top.keys().filter_map(Value::as_str) {
        if NXS_ONLY.contains(&key) {
            ignored.push(Ignored::Misplaced(key.to_string()));
        }
    }

    let decl: RoleDecl = serde_yaml::from_value(Value::Mapping(role))
        .map_err(|e| invalid(format!("reading the persona's fields: {e}")))?;
    let folder = path.parent().unwrap_or(Path::new("")).to_path_buf();
    Ok(LoadedPersona {
        file: DeclarationFile {
            kind: DeclarationKind::Persona,
            name,
            file: path.to_path_buf(),
            form: DeclarationForm::Skill,
            folder,
            requires,
            license: passthrough_text(&top, "license"),
            compatibility: passthrough_text(&top, "compatibility"),
            metadata: passthrough(&top, "metadata"),
            ignored,
        },
        decl,
    })
}

/// The account of a YAML persona file — its passed-through fields and the keys it declares that are
/// ignored — read from the same text [`crate::role::load_role`] parsed.
pub(crate) fn yaml_file_record(path: &Path, decl: &RoleDecl) -> Result<DeclarationFile> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| NxfError::io(format!("reading role file {}: {e}", path.display())))?;
    yaml_record_from_str(path, &content, decl)
}

/// [`yaml_file_record`] over text already in hand.
pub(crate) fn yaml_record_from_str(
    path: &Path,
    content: &str,
    decl: &RoleDecl,
) -> Result<DeclarationFile> {
    let top = match serde_yaml::from_str::<Value>(content) {
        Ok(Value::Mapping(map)) => map,
        _ => Mapping::new(),
    };
    let mut ignored = Vec::new();
    if let Some(book) = top.get(RETIRED_FIELD) {
        ignored.push(Ignored::AddressBook {
            entries: book.as_sequence().map(Vec::len).unwrap_or(0),
        });
    }
    for key in INERT_FIELDS {
        if top.contains_key(*key) {
            ignored.push(Ignored::Inert(key.to_string()));
        }
    }
    let requires = match top.get("requires") {
        Some(value) => parse_requires(value)
            .map_err(|e| NxfError::validation(format!("{}: `requires` {e}", path.display())))?,
        None => Vec::new(),
    };
    Ok(DeclarationFile {
        kind: DeclarationKind::Persona,
        name: decl.handle.clone(),
        file: path.to_path_buf(),
        form: DeclarationForm::Yaml,
        folder: path.parent().unwrap_or(Path::new("")).to_path_buf(),
        requires,
        license: passthrough_text(&top, "license"),
        compatibility: passthrough_text(&top, "compatibility"),
        metadata: passthrough(&top, "metadata"),
        ignored,
    })
}

/// The frontmatter and the body of a `SKILL.md`: the text between a first line `---` and the next
/// line `---`, and everything after it. `None` when the file does not open with a frontmatter.
pub fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let first_end = content.find('\n')?;
    if content[..first_end].trim_end() != "---" {
        return None;
    }
    let rest = &content[first_end + 1..];
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let body = &rest[offset + line.len()..];
            return Some((&rest[..offset], body.strip_prefix('\n').unwrap_or(body)));
        }
        offset += line.len();
    }
    None
}

/// `allowed-tools` as the list `tools:` takes — keeping the three states that field has
/// (nxf 6j6v.zenf): the key ABSENT is the runtime's full default set (the caller does not get
/// here), an EMPTY value (`allowed-tools:` or `""` or `[]`) is no tools at all, anything else is
/// exactly those.
///
/// Three spellings are taken: the specification's space-separated text, Claude Code's
/// comma-separated text, and a YAML list. A rule may carry spaces itself — Claude Code's own
/// example is `Bash(git add *) Bash(git commit *)` — so the text is split only OUTSIDE parentheses.
pub fn parse_allowed_tools(value: &Value) -> std::result::Result<Vec<String>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(text) => Ok(split_tool_rules(text)),
        Value::Sequence(items) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => Ok(text.trim().to_string()),
                _ => Err("lists something that is not text".to_string()),
            })
            .filter(|item| item.as_ref().map_or(true, |t| !t.is_empty()))
            .collect(),
        _ => Err("is neither text nor a list".to_string()),
    }
}

/// Split `allowed-tools` text into rules, on whitespace and commas outside parentheses.
pub fn split_tool_rules(text: &str) -> Vec<String> {
    let mut rules = Vec::new();
    let (mut current, mut depth) = (String::new(), 0usize);
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c if depth == 0 && (c.is_whitespace() || c == ',') => {
                if !current.is_empty() {
                    rules.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        rules.push(current);
    }
    rules
}

/// `requires` — a list of free names. Read and shown, never checked against a vocabulary and never
/// bound: binding a capability per environment is a later slice (epic 6j6v.phcx, slice 6).
fn parse_requires(value: &Value) -> std::result::Result<Vec<String>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Sequence(items) => items
            .iter()
            .map(|item| match item {
                Value::String(name) => Ok(name.clone()),
                _ => Err("lists something that is not a name".to_string()),
            })
            .collect(),
        _ => Err("is not a list of names".to_string()),
    }
}

fn passthrough_text(top: &Mapping, key: &str) -> Option<String> {
    match top.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => serde_yaml::to_string(other)
            .ok()
            .map(|s| s.trim().to_string()),
    }
}

fn passthrough(top: &Mapping, key: &str) -> Option<serde_json::Value> {
    top.get(key)
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::to_value(v).ok())
}

/// What the Agent Skills name rule says about `name`, or `None` when it holds: 1–64 characters,
/// lowercase letters, digits and hyphens, no hyphen at either edge, no two in a row — and, for the
/// skill form, equal to the name of its folder. A WARNING, never a refusal (nxf 6j6v.dw16): every
/// handle in use complies, and refusing a team over a naming convention would be the wrong trade.
pub fn name_rule_violation(name: &str, folder: Option<&str>) -> Option<String> {
    let mut problems = Vec::new();
    let len = name.chars().count();
    if len == 0 || len > 64 {
        problems.push(format!("is {len} characters long, not 1 to 64"));
    }
    if name
        .chars()
        .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    {
        problems.push("may only carry lowercase letters, digits and hyphens".to_string());
    }
    if name.starts_with('-') || name.ends_with('-') {
        problems.push("may not start or end with a hyphen".to_string());
    }
    if name.contains("--") {
        problems.push("may not carry two hyphens in a row".to_string());
    }
    if let Some(folder) = folder {
        if folder != name {
            problems.push(format!("differs from its folder name `{folder}`"));
        }
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_tools_splits_outside_parentheses_on_spaces_and_commas() {
        assert_eq!(
            split_tool_rules("Bash(git add *) Bash(git commit *) Read"),
            vec!["Bash(git add *)", "Bash(git commit *)", "Read"]
        );
        assert_eq!(split_tool_rules("Read, Grep"), vec!["Read", "Grep"]);
        assert_eq!(split_tool_rules("  "), Vec::<String>::new());
    }

    #[test]
    fn allowed_tools_keeps_empty_apart_from_a_list() {
        assert_eq!(parse_allowed_tools(&Value::Null), Ok(vec![]));
        assert_eq!(
            parse_allowed_tools(&Value::String(String::new())),
            Ok(vec![])
        );
        let list: Value = serde_yaml::from_str("[Read, 'Bash(gh *)']").unwrap();
        assert_eq!(
            parse_allowed_tools(&list),
            Ok(vec!["Read".to_string(), "Bash(gh *)".to_string()])
        );
        assert!(parse_allowed_tools(&Value::Bool(true)).is_err());
    }

    #[test]
    fn the_frontmatter_is_split_from_the_body() {
        let (front, body) = split_frontmatter("---\nname: a\n---\n\nYou are a.\n").unwrap();
        assert_eq!(front, "name: a\n");
        assert_eq!(body, "You are a.\n");
        assert!(split_frontmatter("name: a\n").is_none());
        assert!(split_frontmatter("---\nname: a\n").is_none());
        let (_, body) = split_frontmatter("\u{feff}---\r\nname: a\r\n---\r\nbody").unwrap();
        assert_eq!(body, "body");
    }

    #[test]
    fn the_name_rule_of_the_specification() {
        assert_eq!(
            name_rule_violation("pdf-processing", Some("pdf-processing")),
            None
        );
        assert!(name_rule_violation("PDF", None).is_some());
        assert!(name_rule_violation("-pdf", None).is_some());
        assert!(name_rule_violation("pdf--x", None).is_some());
        assert!(name_rule_violation(&"a".repeat(65), None).is_some());
        assert!(name_rule_violation("pm", Some("product-manager"))
            .unwrap()
            .contains("folder name `product-manager`"));
    }
}
