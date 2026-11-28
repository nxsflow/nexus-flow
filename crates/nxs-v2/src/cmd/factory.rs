//! `nxs factory install [--update]` (RFC 0006, "Installing the shipped defaults"): installs the
//! factory's shipped defaults through the seam's `Engine::factory_install`, and prints each
//! shipped file's state and the factory's paths no CODEOWNERS rule covers.

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::install::factory::Report;
use nexus_flow_seam::{DataDirError, Engine, Options};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::CliError;
use crate::output::Render;

/// The subcommands of `nxs factory`.
#[derive(Debug, Subcommand)]
pub enum FactoryCommand {
    /// Write the factory's shipped defaults that are missing under factory/, and report CODEOWNERS gaps.
    Install(InstallArgs),
}

/// The flags of `nxs factory install`.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Also replace a file that is still an earlier shipped default with the current one.
    #[arg(long)]
    update: bool,
}

/// What `nxs factory install` prints (RFC 0006, "Public surfaces this RFC brings").
#[derive(Debug, Serialize)]
pub struct Install {
    /// Each shipped file, ordered by path bytewise.
    files: Vec<File>,
    /// The CODEOWNERS file and the paths it leaves uncovered; `null` without one.
    codeowners: Option<Codeowners>,
}

#[derive(Debug, Serialize)]
struct File {
    path: String,
    state: &'static str,
}

#[derive(Debug, Serialize)]
struct Codeowners {
    file: String,
    uncovered: Vec<String>,
}

/// Runs `nxs factory <command>` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: FactoryCommand,
) -> Result<Install, CliError> {
    let FactoryCommand::Install(args) = command;
    let engine = block_on(Engine::open_with(dir, options()?))?;
    Ok(engine.factory_install(args.update)?.into())
}

impl From<Report> for Install {
    fn from(report: Report) -> Self {
        Install::from(&report)
    }
}

impl From<&Report> for Install {
    fn from(report: &Report) -> Self {
        Install {
            files: report
                .files
                .iter()
                .map(|f| File {
                    path: f.path.clone(),
                    state: f.state.as_str(),
                })
                .collect(),
            codeowners: report.codeowners.as_ref().map(|c| Codeowners {
                file: c.file.clone(),
                uncovered: c.uncovered.clone(),
            }),
        }
    }
}

impl Render for Install {
    fn markdown(&self) -> String {
        let mut out: String = self
            .files
            .iter()
            .map(|f| format!("- `{}`: {}\n", f.path, f.state))
            .collect();
        match &self.codeowners {
            None => out.push_str(
                "\nNo CODEOWNERS file: no code owner reviews a change to the factory's files.\n",
            ),
            Some(c) if c.uncovered.is_empty() => out.push_str(&format!(
                "\n`{}` gives every file above a code owner.\n",
                c.file
            )),
            Some(c) => {
                out.push_str(&format!("\n`{}` gives no code owner to:\n\n", c.file));
                for path in &c.uncovered {
                    out.push_str(&format!("- `{path}`\n"));
                }
            }
        }
        out
    }
}
