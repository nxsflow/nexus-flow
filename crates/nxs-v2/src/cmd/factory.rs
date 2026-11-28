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

impl Install {
    /// The report in one Markdown line, as `nxs init --md` prints it: the files written, and
    /// the factory's paths no CODEOWNERS rule covers.
    pub fn summary(&self) -> String {
        let written = list(
            self.files
                .iter()
                .filter(|f| f.state == "written")
                .map(|f| &f.path),
        );
        let mut line = match written {
            Some(written) => format!("- `nxs factory install` wrote {written}; commit them"),
            None => "- `nxs factory install` wrote nothing".to_owned(),
        };
        match &self.codeowners {
            None => line.push_str(
                "; there is no CODEOWNERS file, so no code owner reviews a change to the factory's files",
            ),
            Some(c) => match list(c.uncovered.iter()) {
                None => line.push_str(&format!(
                    "; `{}` gives each of the factory's files a code owner",
                    c.file
                )),
                Some(uncovered) => line.push_str(&format!(
                    "; `{}` gives no code owner to {uncovered}",
                    c.file
                )),
            },
        }
        line.push('\n');
        line
    }
}

/// `paths` in backticks, joined by commas; `None` when there are none.
pub fn list<'a>(paths: impl Iterator<Item = &'a String>) -> Option<String> {
    let paths: Vec<_> = paths.map(|p| format!("`{p}`")).collect();
    (!paths.is_empty()).then(|| paths.join(", "))
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
