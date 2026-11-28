//! `nxs merge-gate` (RFC 0010): `rules [--base <ref>]` prints every rule the gate judges, read from
//! the base commit through the seam's `Engine::merge_gate_rules`; rules that cannot be read are
//! printed as the report, and the command exits non-zero. `decide` prints the gate's decision on a
//! pull request (`merge_gate_decide`). Neither opens a database.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use nexus_flow_seam::merge_gate::{RuleRef, RulesReport};
use nexus_flow_seam::Engine;
use serde::Serialize;

use crate::cmd::merge_gate_decide::{self, DecideFiles, Decision};
use crate::error::CliError;
use crate::output::{Format, Render};

/// The subcommands of `nxs merge-gate`.
#[derive(Debug, Args)]
pub struct MergeGateArgs {
    #[command(subcommand)]
    command: MergeGateCommand,
}

#[derive(Debug, Subcommand)]
enum MergeGateCommand {
    /// Print every rule the gate judges, read from the base commit.
    Rules {
        /// The commit to read the rules from; the branch `[git] base` names when not given.
        #[arg(long, value_name = "ref")]
        base: Option<String>,
    },
    /// Decide whether pm may merge the pull request or the owner reviews it first.
    Decide {
        /// The commit the pull request's base branch points at.
        #[arg(long, value_name = "sha")]
        base: String,
        /// The pull request's head commit.
        #[arg(long, value_name = "sha")]
        head: String,
        /// The pull request's comments, as the forge lists them, to find the verdict in.
        #[arg(long, value_name = "file", conflicts_with = "verdict")]
        comments: Option<PathBuf>,
        /// A file that holds the verdict alone.
        #[arg(long, value_name = "file")]
        verdict: Option<PathBuf>,
        /// The pull request's reviews, as the forge lists them.
        #[arg(long, value_name = "file")]
        reviews: Option<PathBuf>,
    },
}

/// What `nxs merge-gate rules` prints (RFC 0010, "Public surfaces this RFC brings").
#[derive(Debug, Serialize)]
pub struct Rules {
    base: String,
    readable: bool,
    default: Option<&'static str>,
    owners: Option<Vec<String>>,
    verdict_authors: Option<Vec<String>>,
    rules: Vec<Rule>,
    errors: Vec<RuleError>,
}

#[derive(Debug, Serialize)]
struct Rule {
    id: String,
    effect: &'static str,
    source: &'static str,
    text: String,
}

#[derive(Debug, Serialize)]
struct RuleError {
    source: &'static str,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<RuleAt>,
}

/// A rule's id, or its 1-based position when it has none.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RuleAt {
    Id(String),
    Position(usize),
}

impl Rules {
    /// The exit code: non-zero when the rules cannot be read.
    pub fn code(&self) -> i32 {
        if self.readable {
            0
        } else {
            1
        }
    }
}

/// Runs `nxs merge-gate` in `dir`, and returns what it prints in `format` and its exit code.
pub fn run(dir: &Path, args: MergeGateArgs, format: Format) -> Result<(String, i32), CliError> {
    match args.command {
        MergeGateCommand::Rules { base } => {
            let rules: Rules = Engine::merge_gate_rules(dir, base.as_deref())?.into();
            Ok((format.render(&rules), rules.code()))
        }
        MergeGateCommand::Decide {
            base,
            head,
            comments,
            verdict,
            reviews,
        } => {
            let files = DecideFiles {
                comments,
                verdict,
                reviews,
            };
            let decision: Decision = merge_gate_decide::run(dir, &base, &head, files)?;
            Ok((format.render(&decision), 0))
        }
    }
}

impl From<RulesReport> for Rules {
    fn from(report: RulesReport) -> Self {
        let readable = report.readable();
        let settings = report.settings;
        Rules {
            base: report.base,
            readable,
            default: settings.as_ref().map(|s| s.default.as_str()),
            owners: settings.as_ref().map(|s| s.owners.clone()),
            verdict_authors: settings.map(|s| s.verdict_authors),
            rules: report
                .rules
                .into_iter()
                .map(|rule| Rule {
                    id: rule.id,
                    effect: rule.effect.as_str(),
                    source: rule.source,
                    text: rule.text,
                })
                .collect(),
            errors: report
                .errors
                .into_iter()
                .map(|error| RuleError {
                    source: error.source,
                    code: error.code,
                    rule: error.rule.map(|rule| match rule {
                        RuleRef::Id(id) => RuleAt::Id(id),
                        RuleRef::Position(n) => RuleAt::Position(n),
                    }),
                })
                .collect(),
        }
    }
}

impl Render for Rules {
    fn markdown(&self) -> String {
        let base = &self.base;
        if !self.readable {
            let mut text = format!(
                "- the rules at `{base}` cannot be read, so the gate sends every pull request to the owner\n"
            );
            for error in &self.errors {
                let rule = match &error.rule {
                    Some(RuleAt::Id(id)) => format!(", rule `{id}`"),
                    Some(RuleAt::Position(n)) => format!(", rule {n}"),
                    None => String::new(),
                };
                text.push_str(&format!("- `{}`: `{}`{rule}\n", error.source, error.code));
            }
            return text;
        }
        let logins = |logins: &Option<Vec<String>>| {
            logins
                .iter()
                .flatten()
                .map(|login| format!("`{login}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut text = format!(
            "- the rules at `{base}`, which the gate judges every pull request by\n- default `{}`\n- owners {}\n- verdict authors {}\n",
            self.default.unwrap_or_default(),
            logins(&self.owners),
            logins(&self.verdict_authors),
        );
        for rule in &self.rules {
            text.push_str(&format!(
                "\n## `{}`: {}, from `{}`\n\n{}\n",
                rule.id,
                rule.effect,
                rule.source,
                rule.text.trim_end()
            ));
        }
        text
    }
}
