//! `nxs space join-coordination <space id>` (RFC 0001, "Creating and finding a space"): records
//! the repository's coordination space in `nexus-flow.toml` through the seam's
//! `Engine::space_join_coordination`, and prints the space and whether this run wrote the file.
//! It opens no database. `--name` and the repository record come with NEX-158.

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::{Engine, JoinReport, SpaceId};
use serde::Serialize;

use crate::error::CliError;
use crate::output::Render;

/// The verbs of `nxs space`.
#[derive(Debug, Subcommand)]
pub enum SpaceCommand {
    /// Record the coordination space an admin added this repository's pm to, in nexus-flow.toml.
    JoinCoordination(JoinArgs),
}

/// The arguments of `nxs space join-coordination`.
#[derive(Debug, Args)]
pub struct JoinArgs {
    /// The coordination space's id, 32 lowercase hex digits.
    space: SpaceId,
}

/// What `nxs space join-coordination` prints.
#[derive(Debug, Serialize)]
pub struct Joined {
    /// The coordination space `nexus-flow.toml` names now.
    coordination: String,
    /// Whether this run wrote the file; `false` when it already named this space.
    written: bool,
}

/// Runs `nxs space <verb>` in `dir`.
pub fn run(dir: &Path, command: SpaceCommand) -> Result<Joined, CliError> {
    let SpaceCommand::JoinCoordination(args) = command;
    Ok(Engine::space_join_coordination(dir, args.space)?.into())
}

impl From<JoinReport> for Joined {
    fn from(report: JoinReport) -> Self {
        Joined {
            coordination: report.coordination.to_string(),
            written: report.written,
        }
    }
}

impl Render for Joined {
    fn markdown(&self) -> String {
        let space = &self.coordination;
        if self.written {
            format!("- recorded the coordination space `{space}` in `nexus-flow.toml`; commit the file\n")
        } else {
            format!("- `nexus-flow.toml` already names the coordination space `{space}`; nothing was written\n")
        }
    }
}
