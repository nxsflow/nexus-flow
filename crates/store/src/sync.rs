//! Sync (RFC 0001, "Sync"): the device's database syncs while it is open, through the relay its
//! settings name. The SDK has no one-shot sync call, so [`sync`] opens the database and waits
//! until nothing is behind or unconfirmed, [`watch`] keeps it open until its caller stops it, and
//! [`status`] reads the local database without the network.
//!
//! The SDK reports no time of the last sync, so nxs records it in the device's settings whenever
//! it finds nothing behind or unconfirmed (NEX-102).
//!
//! The wait polls the SDK's reports on the calling thread: the SDK syncs on a thread of its own,
//! and nxs brings no async runtime.

use std::fmt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nocentraldb::Connection;

use crate::data_dir::DataDir;
use crate::device::{DeviceSettings, RelayUrl, SettingsError};
use crate::id::SpaceId;
use crate::open::{DeviceKeys, InitError, Network};
use crate::opened::{repository_on, Opened};

/// How often a wait looks at the SDK's reports.
const POLL: Duration = Duration::from_millis(100);

/// What `nxs sync --status` reports (RFC 0001, "Public surfaces this RFC brings").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    /// The device member's id, as 64 hex digits; `None` while the database holds none of the
    /// repository's spaces.
    pub member: Option<String>,
    /// The relay the database syncs through; `None` in island mode.
    pub relay: Option<RelayUrl>,
    /// This member's changes no relay has confirmed yet.
    pub unconfirmed: u64,
    /// When a sync last found nothing behind or unconfirmed, in seconds since the Unix epoch.
    pub synced_at: Option<u64>,
}

/// What a sync that timed out still waited for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Behind {
    /// The relay connection: `connected`, `connecting`, `offline`, `paused` or `none`.
    pub connection: &'static str,
    /// The SDK's last sync error, by its code; never content.
    pub error: Option<String>,
    /// This member's changes the relay has not confirmed.
    pub unconfirmed: u64,
    /// Each of the repository's spaces that is not synced yet.
    pub spaces: Vec<SpaceBehind>,
}

/// One space that is not synced yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceBehind {
    pub space: SpaceId,
    /// Whether the database holds any of it.
    pub held: bool,
    /// Whether a pull reached the relay's head since the database opened.
    pub initial_sync_complete: bool,
    /// How many relay positions the member heard of but has not folded.
    pub behind: u64,
}

impl fmt::Display for Behind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the relay connection is {}", self.connection)?;
        if let Some(error) = &self.error {
            write!(f, " ({error})")?;
        }
        write!(f, "; {} unconfirmed", self.unconfirmed)?;
        for space in &self.spaces {
            write!(f, "; space {}: ", space.space)?;
            if !space.held {
                f.write_str("not held yet")?;
            } else if !space.initial_sync_complete {
                write!(f, "initial sync not complete, {} behind", space.behind)?;
            } else {
                write!(f, "{} behind", space.behind)?;
            }
        }
        Ok(())
    }
}

/// Why sync did not finish.
#[derive(Debug)]
pub enum SyncError {
    /// The repository, its settings or the database could not be opened.
    Open(InitError),
    /// No relay is set: the database is in island mode and has nothing to sync with.
    NoRelay,
    /// The timeout passed while something was still behind or unconfirmed.
    TimedOut(Behind),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Open(e) => e.fmt(f),
            SyncError::NoRelay => f.write_str(
                "no relay is set, so the space stays on this device in island mode; nxs sync --relay <url> sets one",
            ),
            SyncError::TimedOut(behind) => write!(f, "sync timed out: {behind}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<InitError> for SyncError {
    fn from(e: InitError) -> Self {
        SyncError::Open(e)
    }
}

impl From<SettingsError> for SyncError {
    fn from(e: SettingsError) -> Self {
        SyncError::Open(InitError::Settings(e))
    }
}

/// Sets the relay the device's database syncs through, in its settings in `data`; the time of
/// the last sync is kept.
pub fn set_relay(data: &DataDir, relay: &RelayUrl) -> Result<(), SettingsError> {
    let mut settings = DeviceSettings::read(data)?;
    settings.relay = Some(relay.clone());
    settings.write(data)
}

/// The sync state of the device's database for the repository at or above `dir`, from the local
/// database alone: it is opened off the network.
pub async fn status(dir: &Path, data: &DataDir, keys: DeviceKeys) -> Result<SyncStatus, SyncError> {
    let opened = repository_on(dir, data, keys, Network::Offline).await?;
    report(&opened, data).await
}

/// Opens the device's database for the repository at or above `dir`, waits until nothing is
/// behind or unconfirmed, records the time, and reports the sync state. With `timeout`, a wait
/// that outlasts it ends in [`SyncError::TimedOut`] with what is still behind.
pub async fn sync(
    dir: &Path,
    data: &DataDir,
    keys: DeviceKeys,
    timeout: Option<Duration>,
) -> Result<SyncStatus, SyncError> {
    let mut opened = open(dir, data, keys).await?;
    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        match behind(&mut opened).await? {
            None => {
                record(data)?;
                return report(&opened, data).await;
            }
            Some(behind) if deadline.is_some_and(|d| Instant::now() >= d) => {
                return Err(SyncError::TimedOut(behind))
            }
            Some(_) => thread::sleep(POLL),
        }
    }
}

/// Opens the device's database for the repository at or above `dir` and keeps it open, and so
/// syncing, until `stop` returns `true`; `stop` is asked after every look at the SDK's reports.
/// Each time nothing is behind or unconfirmed, the time is recorded.
pub async fn watch(
    dir: &Path,
    data: &DataDir,
    keys: DeviceKeys,
    mut stop: impl FnMut() -> bool,
) -> Result<(), SyncError> {
    let mut opened = open(dir, data, keys).await?;
    let mut recorded = None;
    loop {
        if behind(&mut opened).await?.is_none() {
            let now = now();
            if recorded != Some(now) {
                record(data)?;
                recorded = Some(now);
            }
        }
        if stop() {
            return Ok(());
        }
        thread::sleep(POLL);
    }
}

/// Opens the database on the relay; without one there is nothing to sync with.
async fn open(dir: &Path, data: &DataDir, keys: DeviceKeys) -> Result<Opened, SyncError> {
    if DeviceSettings::read(data)?.relay.is_none() {
        return Err(SyncError::NoRelay);
    }
    Ok(repository_on(dir, data, keys, Network::Relay).await?)
}

/// What is still behind or unconfirmed; `None` when nothing is.
async fn behind(opened: &mut Opened) -> Result<Option<Behind>, InitError> {
    opened.refresh().await?;
    let sync = opened.db().sync().current();
    let spaces: Vec<SpaceBehind> = opened
        .spaces()
        .iter()
        .map(|(id, space)| {
            let status = space.as_ref().map(|space| space.status().current());
            SpaceBehind {
                space: *id,
                held: space.is_some(),
                initial_sync_complete: status.as_ref().is_some_and(|s| s.initial_sync_complete),
                behind: status.map_or(0, |s| s.behind),
            }
        })
        .filter(|s| !s.held || !s.initial_sync_complete || s.behind > 0)
        .collect();
    let connected = sync.connection == Connection::Connected;
    if connected && sync.unconfirmed == 0 && spaces.is_empty() {
        return Ok(None);
    }
    Ok(Some(Behind {
        connection: connection(sync.connection),
        error: sync.error,
        unconfirmed: sync.unconfirmed,
        spaces,
    }))
}

/// The sync state of `opened`, with the settings in `data`.
async fn report(opened: &Opened, data: &DataDir) -> Result<SyncStatus, SyncError> {
    let settings = DeviceSettings::read(data)?;
    Ok(SyncStatus {
        member: member(opened).await?,
        relay: settings.relay,
        unconfirmed: opened.db().sync().current().unconfirmed,
        synced_at: settings.synced_at,
    })
}

/// The device member's id, from the first open space the database holds.
async fn member(opened: &Opened) -> Result<Option<String>, InitError> {
    for space in opened
        .spaces()
        .iter()
        .filter_map(|(_, space)| space.as_ref())
    {
        let members = space.members().list().await?;
        if let Some(me) = members.iter().find(|m| m.is_self) {
            return Ok(Some(me.id.to_string()));
        }
    }
    Ok(None)
}

/// Records now as the time of the last sync.
fn record(data: &DataDir) -> Result<(), SettingsError> {
    let mut settings = DeviceSettings::read(data)?;
    settings.synced_at = Some(now());
    settings.write(data)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn connection(connection: Connection) -> &'static str {
    match connection {
        Connection::Connected => "connected",
        Connection::Connecting => "connecting",
        Connection::Offline => "offline",
        Connection::Paused => "paused",
        Connection::None => "none",
        _ => "unknown",
    }
}
