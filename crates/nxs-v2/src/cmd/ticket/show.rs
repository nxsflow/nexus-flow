//! `nxs ticket show <key>` (RFC 0002, "Commands, JSON and the seam"): one ticket, by key or
//! object id, as `Engine::ticket` returns it.

use std::path::Path;

use clap::Args;
use nexus_flow_seam::{DataDirError, Engine, Options};

use super::record::Record;
use crate::block_on::block_on;
use crate::error::CliError;

/// The arguments of `nxs ticket show`.
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The ticket's key, such as ka.42, or its object id.
    key: String,
}

pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    args: ShowArgs,
) -> Result<Record, CliError> {
    let engine = block_on(Engine::open_with(dir, options()?))?;
    Ok(block_on(engine.ticket(&args.key))?.into())
}
