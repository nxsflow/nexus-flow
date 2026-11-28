//! `nxs init` (RFC 0001, "Creating and finding a space"): gives the repository its space through
//! the seam's `init`, which ends with the factory's install and GENESIS's create (RFC 0006, RFC
//! 0004), and prints what it did to the space.

use std::path::Path;

use nexus_flow_seam::{init_with, DataDirError, Init as Report, Options, SyncMode};
use serde::Serialize;

use crate::block_on::block_on;
use crate::cmd::factory::Install;
use crate::cmd::genesis::{refused_unless_written, Create};
use crate::error::CliError;
use crate::output::{Format, Render};
use crate::Outcome;

/// What `nxs init` prints.
#[derive(Debug, Serialize)]
pub struct Init {
    /// The repository space's id, as `nexus-flow.toml` names it.
    space: String,
    /// Whether this run created the space; `false` when the file already named one.
    created: bool,
    /// How the space syncs: `island` while no relay is set, `relay` once one is.
    sync: &'static str,
    /// Whether the root holds a v1 workspace, `.nxs/`, which was left as it was.
    v1_workspace: bool,
    /// What the factory's install did, as `nxs factory install` prints it.
    factory: Install,
    /// What GENESIS's create did, as `nxs genesis create` prints it.
    genesis: Create,
}

/// Runs `nxs init` in `dir`, printed in `format`. A GENESIS git still tracks is not written: the
/// record goes to stdout as always, the error naming GENESIS to stderr, and the exit code is 1,
/// as with `nxs genesis create`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    format: Format,
) -> Outcome {
    let report = match options()
        .map_err(CliError::from)
        .and_then(|options| block_on(init_with(dir, options)).map_err(CliError::from))
    {
        Ok(report) => report,
        Err(e) => return e.outcome(format),
    };
    let stdout = format.render(&Init::from(&report)).into_bytes();
    refused_unless_written(&report.genesis, stdout)
}

impl From<&Report> for Init {
    fn from(report: &Report) -> Self {
        Init {
            space: report.space.clone(),
            created: report.created,
            sync: match report.sync {
                SyncMode::Island => "island",
                SyncMode::Relay => "relay",
            },
            v1_workspace: report.v1_workspace,
            factory: Install::from(&report.factory),
            genesis: Create::from(&report.genesis),
        }
    }
}

impl Render for Init {
    fn markdown(&self) -> String {
        let space = &self.space;
        let mut lines = Vec::new();
        if self.created {
            lines.push(format!(
                "created the repository space `{space}` and recorded it in `nexus-flow.toml`; commit the file"
            ));
        } else {
            lines.push(format!(
                "`nexus-flow.toml` already names the repository space `{space}`; nothing was created"
            ));
        }
        lines.push(match self.sync {
            "relay" => "the space syncs through the relay set on this device; `nxs sync` waits until nothing is behind or unconfirmed".to_owned(),
            _ => "no relay is set, so the space stays on this device in island mode; `nxs sync --relay <url>` connects it".to_owned(),
        });
        if self.v1_workspace {
            lines.push(
                "the v1 data in `.nxs/` was not moved into the space and is left as it was"
                    .to_owned(),
            );
        }
        if self.created {
            lines.push(
                "this device is the space's only admin: if it is lost, nobody can add or remove members again; add a second admin device"
                    .to_owned(),
            );
        }
        lines.iter().map(|line| format!("- {line}\n")).collect()
    }
}
