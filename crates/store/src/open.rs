//! Opening the device member's database (RFC 0001, "The local data on disk", "Reading and
//! writing"): once the repository's root is found, the data directory is checked to lie outside
//! it, and only then is anything created. The database lives under `device/` in the data
//! directory and opens from the committed model lock.

use std::fmt;
use std::io;
use std::path::PathBuf;

use nocentraldb::{Db, OpenOptions};

use crate::config::ConfigError;
use crate::data_dir::DataDir;
use crate::model;
use crate::repo_root::{RepoRoot, RootError};

/// Where the device member's keys live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceKeys {
    /// The platform's key store: the Keychain, the Credential Manager or the Secret Service
    /// (RFC 0001, "Members").
    KeyStore,
    /// A file, for a machine with no key store. Only the file's permissions protect it.
    File(PathBuf),
}

/// Why the device's database could not be opened, or the space not created.
#[derive(Debug)]
pub enum InitError {
    /// The root could not be found, or the data directory lies inside it.
    Root(RootError),
    /// `nexus-flow.toml` could not be read or written.
    Config(ConfigError),
    /// The data directory could not be created.
    DataDir { path: PathBuf, error: io::Error },
    /// NoCentralDB refused to open the database or to create the space.
    Database { message: String },
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::Root(e) => e.fmt(f),
            InitError::Config(e) => e.fmt(f),
            InitError::DataDir { path, error } => write!(
                f,
                "cannot create the data directory {}: {error}",
                path.display()
            ),
            InitError::Database { message } => {
                write!(f, "the device's database: {message}")
            }
        }
    }
}

impl std::error::Error for InitError {}

impl From<RootError> for InitError {
    fn from(e: RootError) -> Self {
        InitError::Root(e)
    }
}

impl From<ConfigError> for InitError {
    fn from(e: ConfigError) -> Self {
        InitError::Config(e)
    }
}

impl From<nocentraldb::Error> for InitError {
    fn from(e: nocentraldb::Error) -> Self {
        InitError::Database {
            message: e.to_string(),
        }
    }
}

/// Opens the device member's database for the repository at `root`, creating it on first open.
/// A data directory inside the root is refused before anything is created. The database opens
/// without a relay: no relay is configured yet, so it works in island mode.
pub(crate) async fn open_device(
    root: &RepoRoot,
    data: &DataDir,
    keys: DeviceKeys,
) -> Result<Db, InitError> {
    root.check_data_dir(data)?;
    data.open().map_err(|error| InitError::DataDir {
        path: data.root().to_path_buf(),
        error,
    })?;
    let options = OpenOptions::new(model::LOCK).path(data.device());
    let options = match keys {
        DeviceKeys::KeyStore => options,
        DeviceKeys::File(file) => options.key_file(file),
    };
    Ok(Db::open(options).await?)
}
