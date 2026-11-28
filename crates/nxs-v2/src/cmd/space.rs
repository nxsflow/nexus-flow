//! `nxs space create-coordination [--name]`, `join-coordination <space id> [--name]` and
//! `leave-coordination` (RFC 0001, "Creating and finding a space"; RFC 0009, "Creating it",
//! "Repositories in the space"), through the seam's `Engine::coordination_create`,
//! `coordination_join` and `coordination_leave`.
//!
//! Each prints `coordination`, the space created, joined or left; `written`, whether the run
//! wrote anything, the repository record or `nexus-flow.toml`; and `repository`, the checkout's
//! repository as `nxs repository list` prints it, or `null` after leaving a space that held no
//! record of it.

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::{Coordination, DataDirError, Engine, Options, SpaceId};
use serde::Serialize;

use crate::block_on::block_on;
use crate::cmd::repository::Row;
use crate::error::CliError;
use crate::output::Render;

/// The verbs of `nxs space`.
#[derive(Debug, Subcommand)]
pub enum SpaceCommand {
    /// Create a coordination space for a group of repositories, with this repository's record.
    #[command(name = "create-coordination")]
    Create(NameArgs),
    /// Join a coordination space this device holds: write this repository's record, and name the
    /// space in nexus-flow.toml.
    #[command(name = "join-coordination")]
    Join(JoinArgs),
    /// Leave the coordination space: mark this repository's record as left, and take the space
    /// out of nexus-flow.toml.
    #[command(name = "leave-coordination")]
    Leave,
}

/// The arguments of `nxs space create-coordination`.
#[derive(Debug, Args)]
pub struct NameArgs {
    /// The repository's short name in the group; by default the last path segment of the git
    /// remote origin, without .git.
    #[arg(long)]
    name: Option<String>,
}

/// The arguments of `nxs space join-coordination`.
#[derive(Debug, Args)]
pub struct JoinArgs {
    /// The coordination space's id, 32 lowercase hex digits.
    space: SpaceId,
    /// The repository's short name in the group; by default the name its record has, else the
    /// last path segment of the git remote origin, without .git.
    #[arg(long)]
    name: Option<String>,
}

/// What the `nxs space` verbs print.
#[derive(Debug, Serialize)]
pub struct Printed {
    coordination: String,
    written: bool,
    repository: Option<Row>,
    #[serde(skip)]
    verb: Verb,
}

#[derive(Debug, Clone, Copy)]
enum Verb {
    Create,
    Join,
    Leave,
}

/// Runs `nxs space <verb>` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: SpaceCommand,
) -> Result<Printed, CliError> {
    let mut engine = block_on(Engine::open_with(dir, options()?))?;
    let (verb, report) = match command {
        SpaceCommand::Create(args) => (
            Verb::Create,
            block_on(engine.coordination_create(args.name.as_deref()))?,
        ),
        SpaceCommand::Join(args) => (
            Verb::Join,
            block_on(engine.coordination_join(args.space, args.name.as_deref()))?,
        ),
        SpaceCommand::Leave => (Verb::Leave, block_on(engine.coordination_leave())?),
    };
    Ok(Printed::new(verb, report))
}

impl Printed {
    fn new(verb: Verb, report: Coordination) -> Self {
        Printed {
            coordination: report.coordination.to_string(),
            written: report.written,
            repository: report.repository.map(Row::from),
            verb,
        }
    }
}

impl Render for Printed {
    fn markdown(&self) -> String {
        let space = &self.coordination;
        let mut text = match (self.verb, self.written) {
            (Verb::Create, _) => format!(
                "- created the coordination space `{space}` and recorded it in `nexus-flow.toml`; commit the file\n"
            ),
            (Verb::Join, true) => format!(
                "- joined the coordination space `{space}` and recorded it in `nexus-flow.toml`; commit the file if it changed\n"
            ),
            (Verb::Join, false) => format!(
                "- `nexus-flow.toml` already names the coordination space `{space}`; nothing was written\n"
            ),
            (Verb::Leave, _) => format!(
                "- left the coordination space `{space}` and took it out of `nexus-flow.toml`; commit the file\n"
            ),
        };
        match (&self.repository, self.verb) {
            (Some(row), Verb::Leave) => text.push_str(&format!(
                "- the record of `{}`, its space `{}`, stays, marked as left\n",
                row.name, row.space
            )),
            (Some(row), _) => text.push_str(&format!(
                "- this repository is `{}` in it, its space `{}`\n",
                row.name, row.space
            )),
            (None, _) => text.push_str("- the space held no record of this repository\n"),
        }
        if let Verb::Create = self.verb {
            text.push_str(
                "- this device is the space's only admin: if it is lost, nobody can add or remove members again; add a second admin device\n",
            );
        }
        text
    }
}
