//! `Engine`, the seam's entry point (RFC 0001, "The library seam"): the device member's
//! database, open on a repository's spaces or on one space by id. Each module RFC adds its reads
//! and writes to it, with derived state computed in the store.
//!
//! `open` and `open_space` take the data directory from the environment (`NXS_DATA_DIR`, else
//! the platform's) and keep the device's keys in the platform's key store. A caller that keeps
//! them elsewhere, such as a machine with no key store, names both with `open_with` and
//! `open_space_with`.

use std::fmt;
use std::path::Path;

use nexus_flow_store::data_dir::{DataDir, DataDirError};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::{DeviceKeys, InitError};

/// Where the device member's database and keys live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// nexus-flow's data directory, which holds the database under `device/`.
    pub data: DataDir,
    /// Where the device's keys live.
    pub keys: DeviceKeys,
}

impl Options {
    /// The data directory from the environment, and the keys in the platform's key store.
    pub fn from_env() -> Result<Self, DataDirError> {
        Ok(Options {
            data: DataDir::from_env()?,
            keys: DeviceKeys::KeyStore,
        })
    }
}

/// Why an engine could not be opened.
#[derive(Debug)]
pub enum Error {
    /// The data directory could not be found from the environment.
    DataDir(DataDirError),
    /// The repository, its `nexus-flow.toml` or the database could not be opened.
    Open(InitError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataDir(e) => e.fmt(f),
            Error::Open(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<DataDirError> for Error {
    fn from(e: DataDirError) -> Self {
        Error::DataDir(e)
    }
}

impl From<InitError> for Error {
    fn from(e: InitError) -> Self {
        Error::Open(e)
    }
}

/// The device member's database, open on the spaces a caller reads.
#[derive(Debug)]
pub struct Engine {
    _opened: Opened,
}

impl Engine {
    /// Opens the device member's database and the spaces `nexus-flow.toml` names, for the
    /// repository at or above `repository`.
    pub async fn open(repository: &Path) -> Result<Self, Error> {
        Self::open_with(repository, Options::from_env()?).await
    }

    /// [`Engine::open`], with the data directory and keys `options` names.
    pub async fn open_with(repository: &Path, options: Options) -> Result<Self, Error> {
        let opened = opened::repository(repository, &options.data, options.keys).await?;
        Ok(Engine { _opened: opened })
    }

    /// Opens the device member's database on the space `space`, for a caller without a checkout;
    /// "on main" is then `null`. With `git`, "on main" is read from that repository's checkout.
    pub async fn open_space(space: SpaceId, git: Option<&Path>) -> Result<Self, Error> {
        Self::open_space_with(space, git, Options::from_env()?).await
    }

    /// [`Engine::open_space`], with the data directory and keys `options` names.
    pub async fn open_space_with(
        space: SpaceId,
        git: Option<&Path>,
        options: Options,
    ) -> Result<Self, Error> {
        let opened = opened::space(space, git, &options.data, options.keys).await?;
        Ok(Engine { _opened: opened })
    }
}
