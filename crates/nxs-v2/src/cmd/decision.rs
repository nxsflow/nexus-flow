//! `nxs decision add` (RFC 0003, "Writing a decision", "Public surfaces this RFC brings"),
//! through the seam's `Engine::decision_add`: it writes a decision with its topics, subjects and
//! supersessions in one change, and prints it in full, as `nxs decision show` will. Each refusal
//! names its problem and writes nothing. `author` is not a flag: it is always the writer's member
//! id.
//!
//! The handle is not unique (RFC 0003, "Handles"): when the local copy holds another decision of
//! the same handle, the decision is written all the same, and a warning on stderr names the
//! decisions that share it, through `Engine::decisions_with_handle`.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use nexus_flow_seam::{DataDirError, Decision, Engine, Linked, NewDecision, Options};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::{CliError, Kind};
use crate::output::Render;

/// The verbs of `nxs decision`.
#[derive(Debug, Subcommand)]
pub enum DecisionCommand {
    /// Write a decision with its topics, tickets and the decisions it supersedes.
    Add(Box<AddArgs>),
}

/// The arguments of `nxs decision add`. The required ones are checked by the seam, so that a
/// missing one is refused with the same message as an empty one.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// The decision, stated in one line of at most 200 characters.
    #[arg(long, default_value = "")]
    title: String,
    /// Why it was decided.
    #[arg(long, default_value = "")]
    why: String,
    /// The options not taken.
    #[arg(long, default_value = "")]
    instead_of: String,
    /// A lowercase tag; at least one.
    #[arg(long = "topic", value_name = "tag")]
    topics: Vec<String>,
    /// A ticket id like NEX-22, or a range like "NEX-6 to NEX-13"; the first names the handle.
    #[arg(long = "ticket", value_name = "label")]
    tickets: Vec<String>,
    /// A decision this one supersedes, by handle or object id.
    #[arg(long = "supersedes", value_name = "handle or id")]
    supersedes: Vec<String>,
    /// The https URL of the pull request whose code the decision describes.
    #[arg(long, value_name = "url")]
    pr: Option<String>,
    /// Who decided; the writer's role from its role record when left out.
    #[arg(long, value_name = "text")]
    decided_by: Option<String>,
    /// The day it was decided, YYYY-MM-DD; today's in UTC when left out.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<String>,
    /// The handle's slug instead of the title's.
    #[arg(long)]
    slug: Option<String>,
    /// The text below the fields, as Markdown.
    #[arg(long, value_name = "text", conflicts_with = "body_file")]
    body: Option<String>,
    /// A file that holds the text below the fields.
    #[arg(long, value_name = "path")]
    body_file: Option<PathBuf>,
}

/// A decision in full, as RFC 0003, "Reading decisions", prints it.
#[derive(Debug, Serialize)]
pub struct Full {
    id: String,
    handle: String,
    date: String,
    tickets: String,
    subjects: Vec<SubjectRow>,
    title: String,
    decided_by: String,
    why: String,
    instead_of: String,
    topics: Vec<String>,
    body: String,
    supersedes: Vec<LinkedRow>,
    superseded_by: Vec<LinkedRow>,
    in_force: bool,
    pr: Option<String>,
    on_main: Option<bool>,
    author: String,
    edited: bool,
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct SubjectRow {
    label: String,
    kind: &'static str,
    space: Option<String>,
    object: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Serialize)]
struct LinkedRow {
    id: String,
    handle: String,
}

/// Runs `nxs decision <verb>` in `dir`, and returns what it prints and its warnings.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: DecisionCommand,
) -> Result<(Full, Vec<String>), CliError> {
    let DecisionCommand::Add(args) = command;
    let args = *args;
    let body = match (args.body, &args.body_file) {
        (Some(body), _) => Some(body),
        (None, Some(path)) => Some(read(&dir.join(path))?),
        (None, None) => None,
    };
    let decision = NewDecision {
        title: args.title,
        why: args.why,
        instead_of: args.instead_of,
        topics: args.topics,
        tickets: args.tickets,
        supersedes: args.supersedes,
        pr: args.pr,
        decided_by: args.decided_by,
        date: args.date,
        slug: args.slug,
        body,
    };
    let engine = block_on(Engine::open_with(dir, options()?))?;
    let added = block_on(engine.decision_add(decision))?;
    let others: Vec<String> = block_on(engine.decisions_with_handle(&added.handle))?
        .into_iter()
        .filter(|id| *id != added.id)
        .collect();
    let warnings = if others.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "the handle {} is taken already in the local copy by {}; the decision is written all \
             the same, and its object id {} names it",
            added.handle,
            others.join(", "),
            added.id
        )]
    };
    Ok((added.into(), warnings))
}

fn read(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|e| CliError {
        kind: Kind::Io,
        msg: format!("the body file {} cannot be read: {e}", path.display()),
    })
}

fn linked(list: Vec<Linked>) -> Vec<LinkedRow> {
    list.into_iter()
        .map(|l| LinkedRow {
            id: l.id,
            handle: l.handle,
        })
        .collect()
}

impl From<Decision> for Full {
    fn from(d: Decision) -> Self {
        Full {
            id: d.id,
            handle: d.handle,
            date: d.date,
            tickets: d.tickets,
            subjects: d
                .subjects
                .into_iter()
                .map(|s| SubjectRow {
                    label: s.label,
                    kind: s.kind.as_str(),
                    space: s.space,
                    object: s.object,
                    status: s.status,
                })
                .collect(),
            title: d.title,
            decided_by: d.decided_by,
            why: d.why,
            instead_of: d.instead_of,
            topics: d.topics,
            body: d.body,
            supersedes: linked(d.supersedes),
            superseded_by: linked(d.superseded_by),
            in_force: d.in_force,
            pr: d.pr,
            on_main: d.on_main,
            author: d.author,
            edited: d.edited,
            deleted: d.deleted,
        }
    }
}

impl Render for Full {
    /// The decision as a file in `factory/decisions/` reads: heading, fields and body, then
    /// `> Superseded by …` when it is superseded and `> Not yet on main (<pr>)` when it is
    /// known not to be on main.
    fn markdown(&self) -> String {
        let mut text = format!(
            "# {} · {} · {}\n\n- Decided by: {}\n- Why: {}\n- Instead of: {}\n- Topics: {}\n",
            self.date,
            self.tickets,
            self.title,
            self.decided_by,
            self.why,
            self.instead_of,
            self.topics.join(", ")
        );
        let handles = |list: &[LinkedRow]| {
            list.iter()
                .map(|l| l.handle.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !self.supersedes.is_empty() {
            text.push_str(&format!("- Supersedes: {}\n", handles(&self.supersedes)));
        }
        if !self.body.trim().is_empty() {
            text.push('\n');
            text.push_str(self.body.trim_end());
            text.push('\n');
        }
        if !self.superseded_by.is_empty() {
            text.push_str(&format!(
                "\n> Superseded by {}\n",
                handles(&self.superseded_by)
            ));
        }
        if let (Some(false), Some(pr)) = (self.on_main, &self.pr) {
            text.push_str(&format!("\n> Not yet on main ({pr})\n"));
        }
        text
    }
}
