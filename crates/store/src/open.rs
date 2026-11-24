//! Opening the device member's database (RFC 0001, "The local data on disk", "Reading and
//! writing"): once the repository's root is found, the data directory is checked to lie outside
//! it, and only then is anything created. The database lives under `device/` in the data
//! directory and opens from the committed model lock.
//!
//! The device's database syncs, while it is open, through the relay the device's settings name,
//! and works in island mode without one (RFC 0001, "Sync"). Opening never waits for sync; a
//! caller that must not touch the network, such as `nxs sync --status`, opens it
//! [`Network::Offline`]. A session's member has no relay setting yet, so it opens in island mode.

use std::fmt;
use std::io;
use std::path::PathBuf;

use nocentraldb::{Db, ErrorCode, OpenOptions};

use crate::config::ConfigError;
use crate::data_dir::DataDir;
use crate::device::{DeviceSettings, RelayUrl, SettingsError};
use crate::id::{NotASpaceId, SessionId};
use crate::member_select::{RunAs, SessionFiles};
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
    /// A space id in `nexus-flow.toml` is not one.
    SpaceId { file: PathBuf, error: NotASpaceId },
    /// The device's keys live in the platform's key store, and this machine has none that can
    /// be reached (NEX-224).
    KeyStoreUnavailable,
    /// The device's settings in the data directory could not be read.
    Settings(SettingsError),
    /// NoCentralDB refused to open the database or to create the space.
    Database { message: String },
    /// The session a command runs as is not running: its folder holds no database and keys.
    NoSession { session: SessionId, folder: PathBuf },
}

/// Whether an open database may reach the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// It syncs through the relay the device's settings name, if any.
    Relay,
    /// It stays off the network, whatever the settings name.
    Offline,
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
            InitError::SpaceId { file, error } => write!(f, "{}: {error}", file.display()),
            InitError::KeyStoreUnavailable => f.write_str(
                "this machine has no key store that nxs can reach, and nxs keeps the device's keys only there",
            ),
            InitError::Settings(e) => e.fmt(f),
            InitError::Database { message } => {
                write!(f, "the device's database: {message}")
            }
            InitError::NoSession { session, folder } => write!(
                f,
                "the session {session} is not running: {} holds no session database and keys; check NXS_SESSION or start the session with nxs session start",
                folder.display()
            ),
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

impl From<SettingsError> for InitError {
    fn from(e: SettingsError) -> Self {
        InitError::Settings(e)
    }
}

impl From<nocentraldb::Error> for InitError {
    fn from(e: nocentraldb::Error) -> Self {
        match e.code() {
            ErrorCode::KeyStoreUnavailable => InitError::KeyStoreUnavailable,
            _ => InitError::Database {
                message: e.to_string(),
            },
        }
    }
}

/// Opens the database of the member `run_as` names, for the repository at `root`, with the relay
/// it syncs through. A data directory inside the root is refused before anything is created.
pub(crate) async fn open_in(
    root: &RepoRoot,
    data: &DataDir,
    run_as: RunAs,
    network: Network,
) -> Result<(Db, Option<RelayUrl>), InitError> {
    root.check_data_dir(data)?;
    open_member(data, run_as, network).await
}

/// Opens the device member's database in `data`, creating it on first open, syncing through
/// `relay` when one is given and in island mode otherwise.
pub(crate) async fn open_db(
    data: &DataDir,
    keys: DeviceKeys,
    relay: Option<&RelayUrl>,
) -> Result<Db, InitError> {
    data.open().map_err(|error| InitError::DataDir {
        path: data.root().to_path_buf(),
        error,
    })?;
    let options = OpenOptions::new(model::LOCK).path(data.device());
    let options = match keys {
        DeviceKeys::KeyStore => options,
        DeviceKeys::File(file) => options.key_file(file),
    };
    let options = match relay {
        Some(relay) => options.relay(relay.as_str()),
        None => options,
    };
    Ok(Db::open(options).await?)
}

/// Opens the database of the member `run_as` names in `data`, with the relay it syncs through:
/// the device's settings name it for the device's member with [`Network::Relay`]; a session's
/// member and an offline open have none. A session that is not running is refused before
/// anything is created, the data directory included.
pub(crate) async fn open_member(
    data: &DataDir,
    run_as: RunAs,
    network: Network,
) -> Result<(Db, Option<RelayUrl>), InitError> {
    match run_as {
        RunAs::Device(keys) => {
            let relay = match network {
                Network::Relay => DeviceSettings::read(data)?.relay,
                Network::Offline => None,
            };
            let db = open_db(data, keys, relay.as_ref()).await?;
            Ok((db, relay))
        }
        RunAs::Session(session) => {
            let files = SessionFiles::of(data, &session);
            if !files.running() {
                return Err(InitError::NoSession {
                    session,
                    folder: files.folder,
                });
            }
            let options = OpenOptions::new(model::LOCK)
                .sub_agent()
                .path(files.database)
                .key_file(files.keys);
            Ok((Db::open(options).await?, None))
        }
    }
}
