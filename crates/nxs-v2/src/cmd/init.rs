//! `nxs init` (RFC 0001, "Creating and finding a space"): gives the repository its space through
//! the seam's `init`, and prints what it did. The factory and genesis steps join it later (NEX-139).

use std::path::Path;

use nexus_flow_seam::{init_with, DataDirError, InitReport, Options, SyncMode};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::CliError;
use crate::output::Render;

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
}

/// Runs `nxs init` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
) -> Result<Init, CliError> {
    let options = options()?;
    let report = block_on(init_with(dir, options))?;
    Ok(Init::from(report))
}

impl From<InitReport> for Init {
    fn from(report: InitReport) -> Self {
        Init {
            space: report.space,
            created: report.created,
            sync: match report.sync {
                SyncMode::Island => "island",
                SyncMode::Relay => "relay",
            },
            v1_workspace: report.v1_workspace,
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
