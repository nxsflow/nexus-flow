//! The genesis module's storage (RFC 0004): `factory/GENESIS.md`, written from the template only
//! where the path is free.

use std::fmt;
use std::io;
use std::path::Path;

use nexus_flow_store::config::ConfigError;
use nexus_flow_store::repo_root::RootError;

/// The template nexus-flow writes as a new GENESIS.
pub const TEMPLATE: &str = "";

/// The mark on the line before the list of rules on what comes back to the owner.
pub const OWNER_RULES_MARK: &str = "<!-- nexus-flow: owner-rules -->";

/// What `create` found at `factory/GENESIS.md`, and what it did there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisState {
    Written,
    Team,
    Tracked,
}

/// One file of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: String,
    pub state: GenesisState,
    pub found: Option<String>,
}

/// What `create` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub files: Vec<FileReport>,
}

impl Report {
    pub fn is_refused(&self) -> bool {
        false
    }
}

/// Why `create` could not run.
#[derive(Debug)]
pub enum GenesisError {
    Root(RootError),
    Config(ConfigError),
    Io(io::Error),
}

impl fmt::Display for GenesisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for GenesisError {}

pub fn create(_dir: &Path, _update: bool) -> Result<Report, GenesisError> {
    Ok(Report { files: Vec::new() })
}
