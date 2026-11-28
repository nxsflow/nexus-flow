//! `nxs merge-gate decide` (RFC 0010, "`nxs merge-gate decide`"): the gate's decision on a pull
//! request, through the seam's `Engine::merge_gate_decide`. It opens no database. It exits 0
//! whenever it decided, `owner` included; it fails only when it cannot decide.

use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_seam::merge_gate::{DecideReport, Reason, RuleRef};
use nexus_flow_seam::Engine;
use serde::Serialize;

use crate::error::{CliError, Kind};
use crate::output::Render;

/// The files `nxs merge-gate decide` reads its inputs from.
pub struct DecideFiles {
    pub comments: Option<PathBuf>,
    pub verdict: Option<PathBuf>,
    pub reviews: Option<PathBuf>,
}

/// What `nxs merge-gate decide` prints (RFC 0010, "Public surfaces this RFC brings").
#[derive(Debug, Serialize)]
pub struct Decision {
    base: String,
    head: String,
    decision: &'static str,
    reasons: Vec<ReasonOut>,
    owner_approved: bool,
    status: &'static str,
    description: String,
}

/// One reason, with the fields its code carries.
#[derive(Debug, Serialize)]
struct ReasonOut {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<ErrorOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rules: Option<Vec<String>>,
}

/// A rules error, as `nxs merge-gate rules` prints it.
#[derive(Debug, Serialize)]
struct ErrorOut {
    source: &'static str,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<RuleAt>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RuleAt {
    Id(String),
    Position(usize),
}

/// Decides on the pull request from `base` to `head` in the repository at or above `dir`. A
/// comments or verdict file that cannot be read gives no verdict, which the gate decides on; a
/// reviews file that cannot be read is an error.
pub fn run(dir: &Path, base: &str, head: &str, files: DecideFiles) -> Result<Decision, CliError> {
    let comments = files
        .comments
        .as_deref()
        .and_then(|path| fs::read(path).ok());
    let verdict = files
        .verdict
        .as_deref()
        .and_then(|path| fs::read(path).ok());
    let reviews = match &files.reviews {
        None => None,
        Some(path) => Some(fs::read(path).map_err(|e| CliError {
            kind: Kind::Validation,
            msg: format!("the reviews file {} cannot be read: {e}", path.display()),
        })?),
    };
    let report = Engine::merge_gate_decide(
        dir,
        base,
        head,
        comments.as_deref(),
        verdict.as_deref(),
        reviews.as_deref(),
    )?;
    Ok(report.into())
}

impl From<DecideReport> for Decision {
    fn from(report: DecideReport) -> Self {
        Decision {
            base: report.base,
            head: report.head,
            decision: report.decision.as_str(),
            reasons: report.reasons.into_iter().map(reason).collect(),
            owner_approved: report.owner_approved,
            status: report.status.as_str(),
            description: report.description,
        }
    }
}

fn reason(reason: Reason) -> ReasonOut {
    let mut out = ReasonOut {
        code: reason.code(),
        errors: None,
        detail: None,
        paths: None,
        by: None,
        rule: None,
        reason: None,
        rules: None,
    };
    match reason {
        Reason::RulesUnreadable { errors } => {
            out.errors = Some(
                errors
                    .into_iter()
                    .map(|e| ErrorOut {
                        source: e.source,
                        code: e.code,
                        rule: e.rule.map(|rule| match rule {
                            RuleRef::Id(id) => RuleAt::Id(id),
                            RuleRef::Position(n) => RuleAt::Position(n),
                        }),
                    })
                    .collect(),
            )
        }
        Reason::VerdictInvalid { detail } => out.detail = Some(detail),
        Reason::Codeowners { paths } => out.paths = Some(paths),
        Reason::ChangesRequested { by } => out.by = Some(by),
        Reason::Unsure { rule, reason } | Reason::Applies { rule, reason } => {
            out.rule = Some(rule);
            out.reason = Some(reason);
        }
        Reason::PmRule { rules } => out.rules = Some(rules),
        Reason::Default => {}
    }
    out
}

impl Render for Decision {
    fn markdown(&self) -> String {
        let code = |s: &str| format!("`{s}`");
        let list = |items: &[String]| items.iter().map(|s| code(s)).collect::<Vec<_>>().join(", ");
        let mut text = format!(
            "- decision `{}`, status `{}`: {}\n- base `{}`, head `{}`\n- owner approved the head: {}\n\n## Reasons\n\n",
            self.decision,
            self.status,
            self.description,
            self.base,
            self.head,
            if self.owner_approved { "yes" } else { "no" },
        );
        for r in &self.reasons {
            let line = match r.code {
                "rules_unreadable" => {
                    let errors = r.errors.iter().flatten().map(|e| {
                        let rule = match &e.rule {
                            Some(RuleAt::Id(id)) => format!(", rule `{id}`"),
                            Some(RuleAt::Position(n)) => format!(", rule {n}"),
                            None => String::new(),
                        };
                        format!("`{}`: `{}`{rule}", e.source, e.code)
                    });
                    format!(
                        "the rules cannot be read: {}",
                        errors.collect::<Vec<_>>().join("; ")
                    )
                }
                "verdict_invalid" => format!(
                    "no valid verdict: {}",
                    r.detail
                        .iter()
                        .flatten()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
                "codeowners" => format!(
                    "CODEOWNERS covers {}",
                    list(r.paths.as_deref().unwrap_or_default())
                ),
                "changes_requested" => format!(
                    "`{}` requested changes",
                    r.by.as_deref().unwrap_or_default()
                ),
                "unsure" | "applies" => format!(
                    "rule `{}`: {}",
                    r.rule.as_deref().unwrap_or_default(),
                    r.reason.as_deref().unwrap_or_default()
                ),
                "pm_rule" => format!(
                    "pm rules met: {}",
                    list(r.rules.as_deref().unwrap_or_default())
                ),
                _ => "the default decides".to_string(),
            };
            text.push_str(&format!("- `{}`: {line}\n", r.code));
        }
        text
    }
}
