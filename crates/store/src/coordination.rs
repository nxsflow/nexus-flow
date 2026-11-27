//! Recording the repository's coordination space (RFC 0001, "Creating and finding a space";
//! "Spaces": a repository belongs to at most one coordination space): the store half of
//! `nxs space join-coordination <space id>`.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::ConfigError;
use crate::id::SpaceId;
use crate::repo_root::RootError;

/// What `join` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinReport {
    /// The coordination space `nexus-flow.toml` names now.
    pub coordination: SpaceId,
    /// Whether this run wrote the file; `false` when it already named this space.
    pub written: bool,
}

/// Why the coordination space was not recorded.
#[derive(Debug)]
pub enum JoinError {
    /// No `nexus-flow.toml` was found, or the data directory lies inside the root.
    Root(RootError),
    /// `nexus-flow.toml` could not be read, or is not one this nxs reads.
    Config(ConfigError),
    /// The id is the repository space's own.
    OwnSpace { file: PathBuf, space: SpaceId },
    /// The file already names another coordination space.
    OtherCoordination { file: PathBuf, recorded: String },
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for JoinError {}

/// Records `coordination` in the `nexus-flow.toml` of the repository at or above `dir`.
pub fn join(_dir: &Path, coordination: SpaceId) -> Result<JoinReport, JoinError> {
    Ok(JoinReport {
        coordination,
        written: false,
    })
}
