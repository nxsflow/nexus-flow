//! `nxs genesis create [--update]` (RFC 0004, "Installing the template"): writes
//! `factory/GENESIS.md` from the template through the seam, only where the path is free, installs
//! the guide, and prints the report with the CODEOWNERS check. It opens no database. A GENESIS git still tracks is refused: the report goes
//! to stdout as always, the error naming the file to stderr, and the exit code is 1.

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::genesis::{FileReport, GenesisError, Report, PATH};
use nexus_flow_seam::install::codeowners;
use nexus_flow_seam::{Engine, Problem, RootError};
use serde::Serialize;

use crate::error::{CliError, Kind};
use crate::output::{Format, Render};
use crate::Outcome;

/// The subcommands of `nxs genesis`.
#[derive(Debug, Subcommand)]
pub enum GenesisCommand {
    /// Write factory/GENESIS.md from the template where none exists; never overwrite one.
    Create(CreateArgs),
}

/// The flags of `nxs genesis create`.
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Update the guide where it is an earlier shipped version; never touches GENESIS.md.
    #[arg(long)]
    update: bool,
}

/// What `nxs genesis create` prints (RFC 0004, "Output"), its keys in the RFC's order.
#[derive(Debug, Serialize)]
pub struct Create {
    files: Vec<File>,
    /// `null` when the repository has no CODEOWNERS file.
    codeowners: Option<Codeowners>,
}

/// The CODEOWNERS file found, and the paths no rule in it covers.
#[derive(Debug, Serialize)]
struct Codeowners {
    file: String,
    uncovered: Vec<String>,
}

/// One file of the report.
#[derive(Debug, Serialize)]
struct File {
    path: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    found: Option<String>,
}

/// Runs `nxs genesis <command>` in `dir`, printed in `format`.
pub fn run(dir: &Path, command: GenesisCommand, format: Format) -> Outcome {
    let GenesisCommand::Create(args) = command;
    let report = match Engine::genesis_create(dir, args.update) {
        Ok(report) => report,
        Err(e) => return CliError::from(e).outcome(format),
    };
    let stdout = format.render(&Create::from(&report)).into_bytes();
    if report.is_refused() {
        let msg = format!(
            "{PATH} is missing from the working tree but git still tracks it; nothing was written. Restore it from git, such as with `git checkout -- {PATH}` or `git restore {PATH}`"
        );
        return Outcome {
            stdout,
            stderr: format!("error: {msg}\n").into_bytes(),
            code: 1,
        };
    }
    Outcome {
        stdout,
        stderr: Vec::new(),
        code: 0,
    }
}

impl From<&Report> for Create {
    fn from(report: &Report) -> Self {
        Create {
            files: report.files.iter().map(File::from).collect(),
            codeowners: report.codeowners.as_ref().map(Codeowners::from),
        }
    }
}

impl From<&codeowners::Report> for Codeowners {
    fn from(report: &codeowners::Report) -> Self {
        Codeowners {
            file: report.file.clone(),
            uncovered: report.uncovered.clone(),
        }
    }
}

impl From<&FileReport> for File {
    fn from(file: &FileReport) -> Self {
        File {
            path: file.path.clone(),
            state: file.state.as_str(),
            found: file.found.clone(),
        }
    }
}

impl Render for Create {
    fn markdown(&self) -> String {
        let mut text: String = self.files.iter().map(File::markdown).collect();
        text.push_str(&match &self.codeowners {
            None => format!(
                "- CODEOWNERS: there is no CODEOWNERS file, so no rule sends a change of `{PATH}` to the owner\n"
            ),
            Some(c) if c.uncovered.is_empty() => {
                format!("- CODEOWNERS: `{}` covers `{PATH}`\n", c.file)
            }
            Some(c) => {
                let uncovered: Vec<_> = c.uncovered.iter().map(|p| format!("`{p}`")).collect();
                format!(
                    "- CODEOWNERS: no rule in `{}` covers {}; add one so that its changes come back to the owner\n",
                    c.file,
                    uncovered.join(", ")
                )
            }
        });
        text
    }
}

impl File {
    /// The file's line in the Markdown report.
    fn markdown(&self) -> String {
        let path = &self.path;
        if path != PATH {
            let what = match self.state {
                "written" => "written: the current guide; commit it",
                "current" => "current: the current guide, so nothing was done",
                "older_default" => {
                    "older_default: an earlier shipped guide, left as it is; `--update` replaces it"
                }
                "updated" => "updated: replaced by the current guide; commit it",
                "team" => "team: the team changed it, so it was left as it is",
                "retired" => "retired: nexus-flow no longer ships it, so it was left as it is",
                state => state,
            };
            return format!("- `{path}`: {what}\n");
        }
        let what = match (self.state, &self.found) {
            ("written", _) => "written from the template; fill it in and commit it".to_owned(),
            ("team", Some(found)) => format!("team: `{found}` is there, so nothing was written"),
            ("team", None) => "team: it is there, so nothing was written".to_owned(),
            ("tracked", _) => {
                "tracked: missing here but tracked in git, so nothing was written; restore it from git"
                    .to_owned()
            }
            (state, _) => state.to_owned(),
        };
        format!("- `{path}`: {what}\n")
    }
}

impl From<GenesisError> for CliError {
    fn from(e: GenesisError) -> Self {
        CliError {
            kind: genesis_kind(&e),
            msg: e.to_string(),
        }
    }
}

/// The stable code of an error of GENESIS's create, from `nxs genesis create` or `nxs init`.
pub fn genesis_kind(e: &GenesisError) -> Kind {
    match e {
        GenesisError::Root(RootError::NotFound { .. }) => Kind::NoWorkspace,
        GenesisError::Root(_) => Kind::Validation,
        GenesisError::Config(config) => match config.problem {
            Problem::Io { .. } => Kind::Io,
            _ => Kind::Validation,
        },
        // An unreadable shipped list is a bug in this release, read from nxs's own bytes.
        GenesisError::Io { .. } | GenesisError::Git(_) | GenesisError::Shipped(_) => Kind::Io,
    }
}
