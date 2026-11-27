//! `nxs space join-coordination <space id>` (RFC 0001, "Creating and finding a space").

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::SpaceId;
use serde::Serialize;

use crate::error::CliError;
use crate::output::Render;

/// The verbs of `nxs space`.
#[derive(Debug, Subcommand)]
pub enum SpaceCommand {
    /// Record the coordination space in nexus-flow.toml.
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
    coordination: String,
    written: bool,
}

/// Runs `nxs space <verb>` in `dir`.
pub fn run(_dir: &Path, command: SpaceCommand) -> Result<Joined, CliError> {
    let SpaceCommand::JoinCoordination(args) = command;
    Ok(Joined {
        coordination: args.space.to_string(),
        written: false,
    })
}

impl Render for Joined {
    fn markdown(&self) -> String {
        String::new()
    }
}
