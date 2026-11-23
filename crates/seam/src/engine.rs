//! `Engine`, the seam's entry point (RFC 0001, "The library seam"): a member's database, open on
//! a repository's spaces or on one space by id. Each module RFC adds its reads and writes to it,
//! with derived state computed in the store.
//!
//! `open` and `open_space` take the data directory from the environment (`NXS_DATA_DIR`, else
//! the platform's) and run as the member a command runs as ("Reading and writing"): the session's
//! member when `NXS_SESSION` names a running session, otherwise the device's, whose keys stay in
//! the platform's key store. A caller that keeps them elsewhere, such as a machine with no key
//! store, names both with `open_with` and `open_space_with`, which open the device's member;
//! `as_session` reopens any engine on a running session's member.

use std::fmt;
use std::path::Path;

use nexus_flow_store::data_dir::{DataDir, DataDirError};
use nexus_flow_store::id::{SessionId, SpaceId};
use nexus_flow_store::member_select::{RunAs, SessionVarError};
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
    /// `NXS_SESSION` names no session.
    Session(SessionVarError),
    /// The repository, its `nexus-flow.toml` or the database could not be opened, or the session
    /// is not running.
    Open(InitError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataDir(e) => e.fmt(f),
            Error::Session(e) => e.fmt(f),
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

impl From<SessionVarError> for Error {
    fn from(e: SessionVarError) -> Self {
        Error::Session(e)
    }
}

impl From<InitError> for Error {
    fn from(e: InitError) -> Self {
        Error::Open(e)
    }
}

/// A member's database, open on the spaces a caller reads.
#[derive(Debug)]
pub struct Engine {
    opened: Opened,
    data: DataDir,
}

impl Engine {
    /// Opens the database of the member a command runs as and the spaces `nexus-flow.toml`
    /// names, for the repository at or above `repository`.
    pub async fn open(repository: &Path) -> Result<Self, Error> {
        let (options, run_as) = from_env()?;
        let opened = opened::repository_as(repository, &options.data, run_as).await?;
        Ok(Engine::new(opened, options))
    }

    /// [`Engine::open`] as the device's member, with the data directory and keys `options`
    /// names.
    pub async fn open_with(repository: &Path, options: Options) -> Result<Self, Error> {
        let opened = opened::repository(repository, &options.data, options.keys.clone()).await?;
        Ok(Engine::new(opened, options))
    }

    /// Opens the database of the member a command runs as on the space `space`, for a caller
    /// without a checkout; "on main" is then `null`. With `git`, "on main" is read from that
    /// repository's checkout.
    pub async fn open_space(space: SpaceId, git: Option<&Path>) -> Result<Self, Error> {
        let (options, run_as) = from_env()?;
        let opened = opened::space_as(space, git, &options.data, run_as).await?;
        Ok(Engine::new(opened, options))
    }

    /// [`Engine::open_space`] as the device's member, with the data directory and keys `options`
    /// names.
    pub async fn open_space_with(
        space: SpaceId,
        git: Option<&Path>,
        options: Options,
    ) -> Result<Self, Error> {
        let opened = opened::space(space, git, &options.data, options.keys.clone()).await?;
        Ok(Engine::new(opened, options))
    }

    /// The same engine, opened on the running session `session`'s member database: the same
    /// spaces, and the same checkout. A session that is not running fails, and nothing is
    /// created.
    pub async fn as_session(self, session: SessionId) -> Result<Self, Error> {
        let opened = self.opened.as_session(&self.data, session).await?;
        Ok(Engine {
            opened,
            data: self.data,
        })
    }

    fn new(opened: Opened, options: Options) -> Self {
        Engine {
            opened,
            data: options.data,
        }
    }
}

/// The options from the environment, and the member `NXS_SESSION` selects. `NXS_SESSION` is read
/// first, so that a bad one fails before anything else.
fn from_env() -> Result<(Options, RunAs), Error> {
    let session = RunAs::session_from_env()?;
    let options = Options::from_env()?;
    let run_as = match session {
        Some(session) => RunAs::Session(session),
        None => RunAs::Device(options.keys.clone()),
    };
    Ok((options, run_as))
}
