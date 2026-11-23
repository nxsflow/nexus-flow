//! Finding the repository's root (RFC 0001, "Reading and writing"): `nxs` looks for
//! `nexus-flow.toml` in the working directory and then in each of its ancestors, and the first
//! folder that holds it is the root. A nested repository with a file of its own therefore wins
//! over one further up.
//!
//! Once the root is found, a data directory inside it is refused: the member databases live
//! outside the repository (RFC 0001, "The local data on disk"), so that a branch switch, a second
//! worktree or a fresh clone loses nothing and nothing is committed by mistake.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::data_dir::DataDir;

/// The file whose folder is the repository's root.
pub const FILE_NAME: &str = "nexus-flow.toml";

/// Why the repository's root could not be found.
#[derive(Debug, PartialEq, Eq)]
pub enum RootError {
    /// No folder from `start` up to the filesystem's root holds `nexus-flow.toml`.
    NotFound { start: PathBuf },
    /// The data directory lies inside the repository's root.
    DataDirInside { data_dir: PathBuf, root: PathBuf },
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::NotFound { start } => write!(
                f,
                "no {FILE_NAME} in {} or any folder above it; run nxs init at the repository's root",
                start.display()
            ),
            RootError::DataDirInside { data_dir, root } => write!(
                f,
                "the data directory {} lies inside the repository at {}; set NXS_DATA_DIR to a folder outside it",
                data_dir.display(),
                root.display()
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

    /// Refuses a data directory that is the root or lies inside it. Both paths are compared as
    /// the filesystem resolves them, so a symlink or a `..` that leads back into the root is
    /// refused too; the data directory need not exist yet.
    pub fn check_data_dir(&self, data: &DataDir) -> Result<(), RootError> {
        if resolved(data.root()).starts_with(resolved(&self.path)) {
            return Err(RootError::DataDirInside {
                data_dir: data.root().to_path_buf(),
                root: self.path.clone(),
            });
        }
        Ok(())
    }

    /// The root's `nexus-flow.toml`.
    pub fn config_file(&self) -> PathBuf {
        self.path.join(FILE_NAME)
    }
}

/// `path` as the filesystem resolves it: its longest existing ancestor with every symlink
/// followed, then the rest of its components, with `.` dropped and `..` taking one folder off.
fn resolved(path: &Path) -> PathBuf {
    let (mut out, rest) = path
        .ancestors()
        .find_map(|dir| {
            let real = dir.canonicalize().ok()?;
            Some((real, path.strip_prefix(dir).unwrap_or(Path::new(""))))
        })
        .unwrap_or_else(|| (PathBuf::new(), path));
    for component in rest.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
