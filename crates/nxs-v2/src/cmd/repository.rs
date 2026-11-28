//! `nxs repository list` (RFC 0009, "Public surfaces this RFC brings"): the repository records of
//! the coordination space, through the seam's `Engine::repositories`, in name order, each as an
//! object with `name`, `space` and `left`, as every command that names a repository prints it.

use std::path::Path;

use clap::Subcommand;
use nexus_flow_seam::{DataDirError, Engine, Options, Repository};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::CliError;
use crate::output::Render;

/// The verbs of `nxs repository`.
#[derive(Debug, Subcommand)]
pub enum RepositoryCommand {
    /// List the repositories of the coordination space, with their records.
    List,
}

/// A repository as `nxs` prints it.
#[derive(Debug, Serialize)]
pub struct Row {
    pub name: String,
    pub space: String,
    pub left: bool,
}

impl From<Repository> for Row {
    fn from(repository: Repository) -> Self {
        Row {
            name: repository.name,
            space: repository.space,
            left: repository.left,
        }
    }
}

/// What `nxs repository list` prints.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Listed(Vec<Row>);

/// Runs `nxs repository <verb>` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: RepositoryCommand,
) -> Result<Listed, CliError> {
    let RepositoryCommand::List = command;
    let engine = block_on(Engine::open_with(dir, options()?))?;
    let repositories = block_on(engine.repositories())?;
    Ok(Listed(repositories.into_iter().map(Row::from).collect()))
}

impl Render for Listed {
    fn markdown(&self) -> String {
        let mut text = String::from("| name | space | left |\n| --- | --- | --- |\n");
        for row in &self.0 {
            text.push_str(&format!(
                "| {} | `{}` | {} |\n",
                row.name, row.space, row.left
            ));
        }
        text
    }
}
