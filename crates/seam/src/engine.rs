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
//!
//! An open engine adds, removes and lists the members of its spaces, each with its role record
//! (RFC 0001, "Members"): [`Engine::member_add`], [`Engine::member_remove`] and
//! [`Engine::members`], the counterparts of `nxs member add`, `remove` and `list`.
//!
//! An open engine syncs through the relay the device's settings name (RFC 0001, "Sync"):
//! [`Engine::sync`] waits until nothing is behind or unconfirmed, and [`Engine::sync_status`]
//! reads the device's sync state from the local database without the network. [`set_relay`] sets
//! the device's relay, in the data directory. A session's member has no relay setting yet, so an
//! engine on it syncs with none.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use nexus_flow_store::data_dir::{DataDir, DataDirError};
use nexus_flow_store::device::{RelayUrl, SettingsError};
use nexus_flow_store::id::{SessionId, SpaceId};
use nexus_flow_store::member_select::{RunAs, SessionVarError};
use nexus_flow_store::members::{self, Member, MemberError};
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
    /// `NXS_SESSION` names no session.
    Session(SessionVarError),
    /// The repository, its `nexus-flow.toml` or the database could not be opened, or the session
    /// is not running.
    Open(InitError),
    /// Sync did not finish: no relay is set, or the timeout passed.
    Sync(SyncError),
    /// A member could not be added, removed or listed.
    Member(MemberError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataDir(e) => e.fmt(f),
            Error::Session(e) => e.fmt(f),
            Error::Open(e) => e.fmt(f),
            Error::Sync(e) => e.fmt(f),
            Error::Member(e) => e.fmt(f),
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

impl From<SyncError> for Error {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Open(e) => Error::Open(e),
            e => Error::Sync(e),
        }
    }
}

impl From<MemberError> for Error {
    fn from(e: MemberError) -> Self {
        Error::Member(e)
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

    /// Adds the member of `invitation` with the role `role`, and the ticket `ticket` it was
    /// started on, written `<space id>/<object id>`: to the repository space, and to the
    /// coordination space unless it is a role session or the migration member, each with its
    /// `spaceMember` record. Returns it as [`Engine::members`] lists it. Only an admin of those
    /// spaces adds, and a refusal changes nothing.
    pub async fn member_add(
        &self,
        invitation: &str,
        role: &str,
        ticket: Option<&str>,
    ) -> Result<Member, Error> {
        Ok(members::add(&self.opened, invitation, role, ticket).await?)
    }

    /// Removes the member `member`, by its id, from every open space it is a member of, and
    /// returns it as it was listed. Its role record stays. Only an admin removes.
    pub async fn member_remove(&self, member: &str) -> Result<Member, Error> {
        Ok(members::remove(&self.opened, member).await?)
    }

    /// The current members of the repository space, in member-id order, with their role records.
    pub async fn members(&self) -> Result<Vec<Member>, Error> {
        Ok(members::list(&self.opened).await?)
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
