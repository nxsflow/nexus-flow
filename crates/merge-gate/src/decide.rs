//! The gate's decision in format 1 (RFC 0010, "`nxs merge-gate decide`"): from the rules at the
//! base commit, the verdict, the changed paths and the reviews, whether pm may merge or the owner
//! reviews first, and the status to post.
//!
//! The procedure is fixed for format 1, in this order:
//!
//! 1. Rules that cannot be read send the pull request to the owner, and nothing else is checked.
//! 2. A verdict that is not valid sends it to the owner, with what is wrong.
//! 3. Every reason it needs the owner is collected: CODEOWNERS on a changed path, an owner's latest
//!    request for changes, then each rule in order that is `unsure`, or an `owner` rule that
//!    `applies`. Any one sends it to the owner.
//! 4. Otherwise pm may merge when `default` is `pm` or a `pm` rule is `met`; else the owner
//!    decides, by `default`.
//!
//! The owner's approval of the head commit turns the status to `success` whatever the decision.

use std::path::Path;

use crate::gate_toml::Effect;
use crate::git::{changed_paths, resolve_commit, GitError};
use crate::reviews::Reviews;
use crate::rules::{rules_at, RulesError};
use crate::verdict::{validate_verdict, Outcome, VerdictError};
use crate::verdict_comments::verdict_from_comments;

/// The longest description a status may carry.
pub const DESCRIPTION_LIMIT: usize = 140;

/// The state to post as the commit status `merge-gate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Pending,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Pending => "pending",
        }
    }
}

/// One reason for the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The rules at the base cannot be read, with every problem.
    RulesUnreadable { errors: Vec<RulesError> },
    /// There is no valid verdict, with one line per problem.
    VerdictInvalid { detail: Vec<String> },
    /// The changed paths a CODEOWNERS rule at the base covers, sorted bytewise.
    Codeowners { paths: Vec<String> },
    /// An owner's latest deciding review requests changes.
    ChangesRequested { by: String },
    /// The verdict cannot tell for a rule.
    Unsure { rule: String, reason: String },
    /// The pull request falls under an `owner` rule and needs the owner.
    Applies { rule: String, reason: String },
    /// The `pm` rules the pull request meets.
    PmRule { rules: Vec<String> },
    /// What `default` decides.
    Default,
}

impl Reason {
    /// The reason's code in the decision's JSON.
    pub fn code(&self) -> &'static str {
        match self {
            Reason::RulesUnreadable { .. } => "rules_unreadable",
            Reason::VerdictInvalid { .. } => "verdict_invalid",
            Reason::Codeowners { .. } => "codeowners",
            Reason::ChangesRequested { .. } => "changes_requested",
            Reason::Unsure { .. } => "unsure",
            Reason::Applies { .. } => "applies",
            Reason::PmRule { .. } => "pm_rule",
            Reason::Default => "default",
        }
    }

    /// What the reason adds to the status's description.
    fn summary(&self) -> Vec<String> {
        match self {
            Reason::RulesUnreadable { .. } => vec!["rules unreadable".to_string()],
            Reason::VerdictInvalid { .. } => vec!["verdict invalid".to_string()],
            Reason::Codeowners { .. } => vec!["CODEOWNERS".to_string()],
            Reason::ChangesRequested { .. } => vec!["changes requested".to_string()],
            Reason::Unsure { rule, .. } => vec![format!("{rule} unsure")],
            Reason::Applies { rule, .. } => vec![rule.clone()],
            Reason::PmRule { rules } => rules.clone(),
            Reason::Default => vec!["default".to_string()],
        }
    }
}

/// The gate's decision on one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecideReport {
    /// The full object id of the base commit.
    pub base: String,
    /// The full object id of the head commit.
    pub head: String,
    /// Who decides whether it merges: the owner, or pm.
    pub decision: Effect,
    /// Why, in the order the procedure finds them.
    pub reasons: Vec<Reason>,
    /// Whether an owner's latest deciding review approves the head commit.
    pub owner_approved: bool,
    /// `success` when pm decides or the owner approved, else `pending`.
    pub status: Status,
    /// A line of at most 140 characters for the status.
    pub description: String,
}

/// What the gate decides from, besides the repository.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inputs<'a> {
    /// GitHub's list of the pull request's comments, which may carry the verdict.
    pub comments: Option<&'a [u8]>,
    /// The verdict alone.
    pub verdict: Option<&'a [u8]>,
    /// GitHub's list of the pull request's reviews.
    pub reviews: Option<&'a [u8]>,
}

/// Why the inputs cannot be decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    /// Both the comments and a verdict were given; the gate reads the verdict from one.
    CommentsAndVerdict,
    /// The reviews are not GitHub's list of reviews: every problem, one line each.
    ReviewsInvalid(Vec<String>),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputError::CommentsAndVerdict => {
                write!(f, "the comments and a verdict cannot be given together")
            }
            InputError::ReviewsInvalid(problems) => write!(f, "{}", problems.join("; ")),
        }
    }
}

impl std::error::Error for InputError {}

/// Why the gate cannot decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecideError {
    /// The local git could not give what the gate reads, or `base` or `head` is no commit.
    Git(GitError),
    /// The inputs cannot be decided on.
    Input(InputError),
}

impl From<GitError> for DecideError {
    fn from(e: GitError) -> Self {
        DecideError::Git(e)
    }
}

/// Decides on the pull request from `base` to `head` in the repository whose root is `root`.
/// `covered` says whether a CODEOWNERS rule at the base gives a path, relative to the git
/// repository's top, a code owner.
pub fn decide(
    root: &Path,
    base: &str,
    head: &str,
    inputs: Inputs,
    covered: impl Fn(&str) -> bool,
) -> Result<DecideReport, DecideError> {
    if inputs.comments.is_some() && inputs.verdict.is_some() {
        return Err(DecideError::Input(InputError::CommentsAndVerdict));
    }
    let reviews = match inputs.reviews {
        None => Reviews::default(),
        Some(bytes) => {
            Reviews::read(bytes).map_err(|p| DecideError::Input(InputError::ReviewsInvalid(p)))?
        }
    };
    let base = resolve_commit(root, base)?;
    let head = resolve_commit(root, head)?;
    let rules = rules_at(root, &base)?;

    let done = |reasons: Vec<Reason>, decision: Effect, owner_approved: bool| {
        Ok(report(
            base.clone(),
            head.clone(),
            decision,
            reasons,
            owner_approved,
        ))
    };
    let Some(settings) = rules.settings else {
        let errors = rules.errors;
        return done(
            vec![Reason::RulesUnreadable { errors }],
            Effect::Owner,
            false,
        );
    };
    let owner_approved = reviews.owner_approved(&settings.owners, &head);

    let verdict = match (inputs.comments, inputs.verdict) {
        (Some(comments), _) => {
            verdict_from_comments(comments, &settings.verdict_authors).map(String::into_bytes)
        }
        (None, Some(verdict)) => Ok(verdict.to_vec()),
        (None, None) => Err(vec![
            "no verdict was given, or its file could not be read".to_string()
        ]),
    };
    let verdict = verdict
        .map_err(VerdictError::Invalid)
        .and_then(|bytes| validate_verdict(root, &base, &head, &rules.rules, &bytes));
    let verdict = match verdict {
        Ok(verdict) => verdict,
        Err(VerdictError::Git(e)) => return Err(e.into()),
        Err(VerdictError::Invalid(detail)) => {
            return done(
                vec![Reason::VerdictInvalid { detail }],
                Effect::Owner,
                owner_approved,
            )
        }
    };

    let mut reasons = Vec::new();
    let mut paths: Vec<String> = changed_paths(root, &base, &head)?
        .into_iter()
        .filter(|path| covered(path))
        .collect();
    paths.sort();
    paths.dedup();
    if !paths.is_empty() {
        reasons.push(Reason::Codeowners { paths });
    }
    for by in reviews.changes_requested_by(&settings.owners) {
        reasons.push(Reason::ChangesRequested { by });
    }
    for v in &verdict.verdicts {
        let (rule, reason) = (v.rule.clone(), v.reason.clone());
        match v.outcome {
            Outcome::Unsure => reasons.push(Reason::Unsure { rule, reason }),
            Outcome::Applies => reasons.push(Reason::Applies { rule, reason }),
            _ => {}
        }
    }
    if !reasons.is_empty() {
        return done(reasons, Effect::Owner, owner_approved);
    }

    let met: Vec<String> = verdict
        .verdicts
        .iter()
        .filter(|v| v.outcome == Outcome::Met)
        .map(|v| v.rule.clone())
        .collect();
    let mut decision = Effect::Owner;
    if settings.default == Effect::Pm {
        decision = Effect::Pm;
        reasons.push(Reason::Default);
    }
    if !met.is_empty() {
        decision = Effect::Pm;
        reasons.push(Reason::PmRule { rules: met });
    }
    if reasons.is_empty() {
        reasons.push(Reason::Default);
    }
    done(reasons, decision, owner_approved)
}

fn report(
    base: String,
    head: String,
    decision: Effect,
    reasons: Vec<Reason>,
    owner_approved: bool,
) -> DecideReport {
    let status = if decision == Effect::Pm || owner_approved {
        Status::Success
    } else {
        Status::Pending
    };
    let description = description(decision, owner_approved, &reasons);
    DecideReport {
        base,
        head,
        decision,
        reasons,
        owner_approved,
        status,
        description,
    }
}

/// The status's description: who decides, then each reason's summary once, in order, cut at a
/// whole summary with `…` so that it stays within [`DESCRIPTION_LIMIT`] characters.
fn description(decision: Effect, owner_approved: bool, reasons: &[Reason]) -> String {
    let lead = match (decision, owner_approved) {
        (Effect::Pm, _) => "pm may merge",
        (Effect::Owner, true) => "Approved by the owner",
        (Effect::Owner, false) => "Waits for the owner",
    };
    let mut items: Vec<String> = Vec::new();
    for item in reasons.iter().flat_map(Reason::summary) {
        if !items.contains(&item) {
            items.push(item);
        }
    }
    let mut text = format!("{lead}: ");
    for (i, item) in items.iter().enumerate() {
        let separator = if i == 0 { "" } else { ", " };
        let rest = if i + 1 == items.len() { "" } else { ", …" };
        let len = |s: &str| s.chars().count();
        if len(&text) + len(separator) + len(item) + len(rest) > DESCRIPTION_LIMIT {
            text.push_str(if i == 0 { "…" } else { ", …" });
            return text;
        }
        text.push_str(separator);
        text.push_str(item);
    }
    text
}
