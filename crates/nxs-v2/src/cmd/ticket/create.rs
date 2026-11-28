//! `nxs ticket create` (RFC 0002, "Commands, JSON and the seam"), through the seam's
//! `Engine::ticket_create`: it mints the ticket's key, writes the ticket in one change and prints
//! it as `nxs ticket show` prints it, or, with `-q`, its key alone on one line. The checks are the
//! store's, so a missing flag is refused with the same message as an empty one.
//!
//! The description comes from `--description`, from `--description-file`, or from stdin when
//! either is `-`. A lone `-` reads the whole ticket from stdin instead, as one JSON object whose
//! keys are the flags' names (`type`, `title`, `description`, `priority`, `design`, `dod`, `due`,
//! `defer`, `parent`, `depends_on`, `labels`, `assignee`), as v1's `create -` reads it; a key it
//! does not know, or a field flag beside it, is refused.

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::Args;
use nexus_flow_seam::{DataDirError, Engine, NewTicket, Options};
use serde::Deserialize;

use super::record::Record;
use crate::block_on::block_on;
use crate::error::{CliError, Kind};

/// The arguments of `nxs ticket create`. The required ones are checked by the store.
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// One of epic, bug, feature, chore, decision, question.
    #[arg(long = "type", value_name = "type")]
    kind: Option<String>,
    /// One line.
    #[arg(long, value_name = "text")]
    title: Option<String>,
    /// Why the ticket exists and its goal; `-` reads it from stdin.
    #[arg(long, value_name = "text | -", conflicts_with = "description_file")]
    description: Option<String>,
    /// A file that holds the description; `-` reads stdin.
    #[arg(long, value_name = "path")]
    description_file: Option<PathBuf>,
    /// P0 to P4, or the ordinal 0 to 4; not set when left out.
    #[arg(long, value_name = "P0..P4 | 0..4")]
    priority: Option<String>,
    /// The path to the goal.
    #[arg(long, value_name = "text")]
    design: Option<String>,
    /// The acceptance criteria.
    #[arg(long, value_name = "text")]
    dod: Option<String>,
    /// The due time, RFC 3339.
    #[arg(long, value_name = "time")]
    due: Option<String>,
    /// The time until which it is deferred, RFC 3339.
    #[arg(long, value_name = "time")]
    defer: Option<String>,
    /// The epic it belongs to, by key or object id.
    #[arg(long, value_name = "key")]
    parent: Option<String>,
    /// A ticket that blocks it, by key or object id; repeatable.
    #[arg(long = "depends-on", value_name = "key")]
    depends_on: Vec<String>,
    /// A label; repeatable.
    #[arg(long = "label", value_name = "label")]
    labels: Vec<String>,
    /// Who holds it: member:<member id> or account:<account fingerprint>.
    #[arg(long, value_name = "identity")]
    assignee: Option<String>,
    /// Print the new ticket's key alone.
    #[arg(short = 'q')]
    quiet: bool,
    /// `-` reads the whole ticket as one JSON object from stdin.
    #[arg(value_name = "-")]
    payload: Option<String>,
}

/// What `create` prints.
pub enum Created {
    /// With `-q`: the key as `nxs` shows it.
    Key(String),
    Ticket(Box<Record>),
}

/// The ticket as `nxs ticket create -` reads it from stdin.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    priority: Option<serde_json::Value>,
    #[serde(default)]
    design: Option<String>,
    #[serde(default)]
    dod: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    defer: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    assignee: Option<String>,
}

pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    args: CreateArgs,
    stdin: &mut dyn Read,
) -> Result<Created, CliError> {
    let quiet = args.quiet;
    let ticket = match args.payload.as_deref() {
        Some("-") => from_payload(&args, stdin)?,
        Some(other) => {
            return Err(validation(format!(
                "{other:?} is no argument of nxs ticket create; only - reads the ticket from stdin"
            )))
        }
        None => from_flags(dir, args, stdin)?,
    };
    let engine = block_on(Engine::open_with(dir, options()?))?;
    let created = block_on(engine.ticket_create(ticket))?;
    Ok(if quiet {
        Created::Key(created.key)
    } else {
        Created::Ticket(Box::new(created.into()))
    })
}

fn from_flags(dir: &Path, args: CreateArgs, stdin: &mut dyn Read) -> Result<NewTicket, CliError> {
    let description = match (args.description, args.description_file) {
        (Some(text), _) if text == "-" => read_stdin(stdin)?,
        (Some(text), _) => text,
        (None, Some(path)) if path.as_os_str() == "-" => read_stdin(stdin)?,
        (None, Some(path)) => read_file(&dir.join(path))?,
        (None, None) => String::new(),
    };
    Ok(NewTicket {
        kind: args.kind.unwrap_or_default(),
        title: args.title.unwrap_or_default(),
        description,
        priority: args.priority,
        design: args.design,
        completion_criterion: args.dod,
        due: args.due,
        defer_until: args.defer,
        parent: args.parent,
        depends_on: args.depends_on,
        labels: args.labels,
        assignee: args.assignee,
    })
}

fn from_payload(args: &CreateArgs, stdin: &mut dyn Read) -> Result<NewTicket, CliError> {
    let flags = args.kind.is_some()
        || args.title.is_some()
        || args.description.is_some()
        || args.description_file.is_some()
        || args.priority.is_some()
        || args.design.is_some()
        || args.dod.is_some()
        || args.due.is_some()
        || args.defer.is_some()
        || args.parent.is_some()
        || !args.depends_on.is_empty()
        || !args.labels.is_empty()
        || args.assignee.is_some();
    if flags {
        return Err(validation(
            "- reads the whole ticket from stdin, so it takes no field flag beside it".to_owned(),
        ));
    }
    let text = read_stdin(stdin)?;
    let payload: Payload = serde_json::from_str(&text)
        .map_err(|e| validation(format!("the ticket on stdin is no ticket object: {e}")))?;
    let priority = match payload.priority {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) => Some(text),
        Some(other) => Some(other.to_string()),
    };
    Ok(NewTicket {
        kind: payload.kind,
        title: payload.title,
        description: payload.description,
        priority,
        design: payload.design,
        completion_criterion: payload.dod,
        due: payload.due,
        defer_until: payload.defer,
        parent: payload.parent,
        depends_on: payload.depends_on,
        labels: payload.labels,
        assignee: payload.assignee,
    })
}

fn read_stdin(stdin: &mut dyn Read) -> Result<String, CliError> {
    let mut text = String::new();
    stdin.read_to_string(&mut text).map_err(|e| CliError {
        kind: Kind::Io,
        msg: format!("stdin cannot be read as UTF-8 text: {e}"),
    })?;
    Ok(text)
}

fn read_file(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|e| CliError {
        kind: Kind::Io,
        msg: format!(
            "the description file {} cannot be read: {e}",
            path.display()
        ),
    })
}

fn validation(msg: String) -> CliError {
    CliError {
        kind: Kind::Validation,
        msg,
    }
}
