//! Finding the repository's root (RFC 0001, "Reading and writing"): `nxs` looks for
//! `nexus-flow.toml` in the working directory and then in each of its ancestors, and the first
//! folder that holds it is the root. A nested repository with a file of its own therefore wins
//! over one further up.

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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::NotFound { start } => write!(
                f,
                "no {FILE_NAME} in {} or any folder above it; run nxs init at the repository's root",
                start.display()
            ),
        }
    }
}

impl std::error::Error for RootError {}

/// The repository's root: the folder that holds `nexus-flow.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot {
    path: PathBuf,
}

impl RepoRoot {
    /// Finds the nearest folder, `start` or one of its ancestors, that holds `nexus-flow.toml` as
    /// a file. Reads nothing but the folders' entries.
    pub fn find(start: &Path) -> Result<Self, RootError> {
        start
            .ancestors()
            .find(|dir| dir.join(FILE_NAME).is_file())
            .map(|dir| RepoRoot {
                path: dir.to_path_buf(),
            })
            .ok_or_else(|| RootError::NotFound {
                start: start.to_path_buf(),
            })
    }

    /// The root folder itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The root's `nexus-flow.toml`.
    pub fn config_file(&self) -> PathBuf {
        self.path.join(FILE_NAME)
    }
}
