//! Finding the repository's root (RFC 0001, "Reading and writing").

use std::fmt;
use std::path::{Path, PathBuf};

/// The file whose folder is the repository's root.
pub const FILE_NAME: &str = "nexus-flow.toml";

/// Why the repository's root could not be found.
#[derive(Debug, PartialEq, Eq)]
pub enum RootError {
    /// No folder from `start` up to the filesystem's root holds `nexus-flow.toml`.
    NotFound { start: PathBuf },
}

impl fmt::Display for RootError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl std::error::Error for RootError {}

/// The repository's root: the folder that holds `nexus-flow.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot {
    path: PathBuf,
}

impl RepoRoot {
    pub fn find(start: &Path) -> Result<Self, RootError> {
        Err(RootError::NotFound {
            start: start.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config_file(&self) -> PathBuf {
        self.path.clone()
    }
}
