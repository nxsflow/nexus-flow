//! nexus-flow's data directory (RFC 0001, "The local data on disk"): where the member databases
//! live, outside the repository, and how they are laid out in it.
//!
//! | Platform | Data directory |
//! | --- | --- |
//! | macOS | `~/Library/Application Support/nexus-flow/` |
//! | Linux and other Unix | `$XDG_DATA_HOME/nexus-flow/`, else `~/.local/share/nexus-flow/` |
//! | Windows | `%LOCALAPPDATA%\nexus-flow\` |
//!
//! `NXS_DATA_DIR` overrides the directory on every platform. Every path is absolute: a relative
//! one would resolve against the working directory, which is usually a repository's working tree.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The directory's own name under the platform's data directory.
const NAME: &str = "nexus-flow";

/// The platform whose rule places the data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    /// Linux and every other Unix: the XDG base directory rule.
    Linux,
    Windows,
}

impl Platform {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Why the data directory could not be resolved.
#[derive(Debug, PartialEq, Eq)]
pub enum DataDirError {
    /// The variable the platform's rule needs is unset or empty, and `NXS_DATA_DIR` is too.
    Unset { variable: &'static str },
    /// The variable holds a relative path.
    Relative {
        variable: &'static str,
        path: PathBuf,
    },
    /// A session id, member id or run is not one plain name, so its folder could lie outside
    /// the folder it belongs in.
    NotAName { kind: &'static str, id: String },
}

impl fmt::Display for DataDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataDirError::Unset { variable } => write!(
                f,
                "cannot find nexus-flow's data directory: {variable} is not set; set NXS_DATA_DIR to an absolute path"
            ),
            DataDirError::Relative { variable, path } => write!(
                f,
                "{variable} must be an absolute path, not {}",
                path.display()
            ),
            DataDirError::NotAName { kind, id } => {
                write!(f, "{kind} {id:?} is not a plain name for a folder")
            }
        }
    }
}

impl std::error::Error for DataDirError {}

/// nexus-flow's data directory, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// Resolves the data directory by `platform`'s rule, reading variables through `var`. An empty
    /// variable counts as unset. Touches no file.
    pub fn resolve(
        platform: Platform,
        var: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, DataDirError> {
        let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let absolute = |variable: &'static str| match set(variable) {
            None => Err(DataDirError::Unset { variable }),
            Some(path) if path.is_absolute() => Ok(path),
            Some(path) => Err(DataDirError::Relative { variable, path }),
        };

        if set("NXS_DATA_DIR").is_some() {
            return absolute("NXS_DATA_DIR").map(|root| DataDir { root });
        }
        let root = match platform {
            Platform::MacOs => absolute("HOME")?
                .join("Library")
                .join("Application Support")
                .join(NAME),
            Platform::Linux => match set("XDG_DATA_HOME").filter(|p| p.is_absolute()) {
                Some(data_home) => data_home.join(NAME),
                None => absolute("HOME")?.join(".local").join("share").join(NAME),
            },
            Platform::Windows => absolute("LOCALAPPDATA")?.join(NAME),
        };
        Ok(DataDir { root })
    }

    /// Resolves the data directory by the rule of the platform this binary was built for, from
    /// the process's environment. Touches no file.
    pub fn from_env() -> Result<Self, DataDirError> {
        Self::resolve(Platform::current(), |name| std::env::var_os(name))
    }

    /// Creates the directory, and its parents, if it does not exist yet.
    pub fn open(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)
    }

    /// The directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The device member's NoCentralDB database.
    pub fn device(&self) -> PathBuf {
        self.root.join("device")
    }

    /// One session member's database and keys, created by `nxs session start`.
    pub fn session(&self, session_id: &str) -> Result<PathBuf, DataDirError> {
        self.under("sessions", "session id", session_id)
    }

    /// nexus-flow's rebuildable cache for one member, deletable at any time.
    pub fn derived(&self, member_id: &str) -> Result<PathBuf, DataDirError> {
        self.under("derived", "member id", member_id)
    }

    /// The migration member's database and keys for one run of RFC 0008's move.
    pub fn migration(&self, run: &str) -> Result<PathBuf, DataDirError> {
        self.under("migration", "migration run", run)
    }

    fn under(&self, folder: &str, kind: &'static str, id: &str) -> Result<PathBuf, DataDirError> {
        if !is_plain_name(id) {
            return Err(DataDirError::NotAName {
                kind,
                id: id.to_owned(),
            });
        }
        Ok(self.root.join(folder).join(id))
    }
}

/// Whether `id` is one plain name for a folder, so that the folder it names lies inside the one
/// it belongs in.
pub(crate) fn is_plain_name(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains(['/', '\\'])
        && Path::new(id).components().count() == 1
}
