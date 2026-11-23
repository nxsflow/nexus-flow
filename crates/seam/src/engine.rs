//! `Engine`, the seam's entry point (RFC 0001, "The library seam"): the device member's
//! database, open on a repository's spaces or on one space by id. Each module RFC adds its reads
//! and writes to it, with derived state computed in the store.
//!
//! `open` and `open_space` take the data directory from the environment (`NXS_DATA_DIR`, else
//! the platform's) and keep the device's keys in the platform's key store. A caller that keeps
//! them elsewhere, such as a machine with no key store, names both with `open_with` and
//! `open_space_with`.
//!
//! An open engine syncs through the relay the device's settings name (RFC 0001, "Sync"):
//! [`Engine::sync`] waits until nothing is behind or unconfirmed, and [`Engine::sync_status`]
//! reads the sync state from the local database without the network. [`set_relay`] sets the
//! relay, in the data directory.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use nexus_flow_store::data_dir::{DataDir, DataDirError};
use nexus_flow_store::device::{RelayUrl, SettingsError};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::{DeviceKeys, InitError};
use nexus_flow_store::sync::{self, SyncError, SyncStatus};

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
    /// Sync did not finish: no relay is set, or the timeout passed.
    Sync(SyncError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataDir(e) => e.fmt(f),
            Error::Open(e) => e.fmt(f),
            Error::Sync(e) => e.fmt(f),
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

impl From<SyncError> for Error {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Open(e) => Error::Open(e),
            e => Error::Sync(e),
        }
    }
}

impl From<SettingsError> for Error {
    fn from(e: SettingsError) -> Self {
        Error::Open(InitError::Settings(e))
    }
}

/// Sets the relay the device's database syncs through, in the data directory `options` names;
/// it is never written into `nexus-flow.toml`. An engine opened afterwards syncs through it.
pub fn set_relay(options: &Options, relay: &RelayUrl) -> Result<(), Error> {
    Ok(sync::set_relay(&options.data, relay)?)
}

/// The device member's database, open on the spaces a caller reads.
#[derive(Debug)]
pub struct Engine {
    opened: Opened,
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
        Ok(Engine { opened })
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
        Ok(Engine { opened })
    }

    /// Waits until nothing in the open spaces is behind or unconfirmed, and reports the sync
    /// state. With `timeout`, a wait that outlasts it fails with what is still behind. Without a
    /// relay it fails at once: there is nothing to sync with.
    pub async fn sync(&mut self, timeout: Option<Duration>) -> Result<SyncStatus, Error> {
        Ok(sync::wait(&mut self.opened, timeout).await?)
    }

    /// The sync state of the device's database for the repository at or above `repository`,
    /// read from the local database: it opens the database off the network.
    pub async fn sync_status(repository: &Path) -> Result<SyncStatus, Error> {
        Self::sync_status_with(repository, Options::from_env()?).await
    }

    /// [`Engine::sync_status`], with the data directory and keys `options` names.
    pub async fn sync_status_with(
        repository: &Path,
        options: Options,
    ) -> Result<SyncStatus, Error> {
        Ok(sync::status(repository, &options.data, options.keys).await?)
    }
}
