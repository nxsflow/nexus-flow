//! `nxs sync` (RFC 0001, "Sync"): `--relay <url>` sets the device's relay in the data directory;
//! `--status` reports the sync state from the local database without the network; `--watch`
//! keeps the database open, and so syncing, until the process is stopped; otherwise it waits until
//! nothing is behind or unconfirmed, bounded by `--timeout <seconds>`.

use std::path::Path;
use std::thread;
use std::time::Duration;

use clap::Args;
use nexus_flow_seam::{
    set_relay, DataDirError, Engine, Error, Options, RelayUrl, SyncError, SyncStatus,
};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::{CliError, Kind};
use crate::output::Render;

/// How long `--watch` waits in one look before it looks again, and rests once synced.
const WATCH_STEP: Duration = Duration::from_secs(1);

/// The flags of `nxs sync`.
#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Keep the database open, and so syncing, until stopped.
    #[arg(long, conflicts_with_all = ["status", "timeout"])]
    watch: bool,
    /// Report the sync state from the local database, without the network.
    #[arg(long, conflicts_with = "timeout")]
    status: bool,
    /// Stop waiting after this many seconds, report what is still behind, and fail.
    #[arg(long, value_name = "seconds")]
    timeout: Option<u64>,
    /// Set the relay this device syncs through, in its data directory.
    #[arg(long, value_name = "url")]
    relay: Option<String>,
}

/// What `nxs sync` prints (RFC 0001, "Public surfaces this RFC brings").
#[derive(Debug, Serialize)]
pub struct Status {
    /// The device member's id; `null` while the database holds none of the repository's spaces.
    member: Option<String>,
    /// The relay the device syncs through; `null` in island mode.
    relay: Option<String>,
    /// This member's changes no relay has confirmed yet.
    unconfirmed: u64,
    /// When a sync last found nothing behind or unconfirmed, in RFC 3339 UTC; `null` until then.
    synced_at: Option<String>,
}

/// Runs `nxs sync` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    args: SyncArgs,
) -> Result<Status, CliError> {
    let options = options()?;
    if let Some(url) = &args.relay {
        let relay: RelayUrl = url
            .parse()
            .map_err(|e: nexus_flow_seam::NotARelayUrl| CliError {
                kind: Kind::Validation,
                msg: e.to_string(),
            })?;
        set_relay(&options, &relay)?;
    }
    if args.status {
        return Ok(block_on(Engine::sync_status_with(dir, options))?.into());
    }
    let mut engine = block_on(Engine::open_with(dir, options))?;
    if args.watch {
        return Err(watch(&mut engine).into());
    }
    let timeout = args.timeout.map(Duration::from_secs);
    Ok(block_on(engine.sync(timeout))?.into())
}

/// Keeps `engine` open and syncing until the process is stopped; returns only an error.
fn watch(engine: &mut Engine) -> Error {
    loop {
        match block_on(engine.sync(Some(WATCH_STEP))) {
            Ok(_) => thread::sleep(WATCH_STEP),
            Err(Error::Sync(SyncError::TimedOut(_))) => {}
            Err(e) => return e,
        }
    }
}

impl From<SyncStatus> for Status {
    fn from(status: SyncStatus) -> Self {
        Status {
            member: status.member,
            relay: status.relay.map(|r| r.to_string()),
            unconfirmed: status.unconfirmed,
            synced_at: status.synced_at.map(rfc3339),
        }
    }
}

impl Render for Status {
    fn markdown(&self) -> String {
        let member = match &self.member {
            Some(member) => format!("member `{member}`"),
            None => "no member yet: the database holds none of the repository's spaces".to_owned(),
        };
        let relay = match &self.relay {
            Some(relay) => format!("relay `{relay}`"),
            None => "no relay: island mode".to_owned(),
        };
        let unconfirmed = format!("{} unconfirmed", self.unconfirmed);
        let synced = match &self.synced_at {
            Some(at) => format!("last synced at {at}"),
            None => "never synced".to_owned(),
        };
        [member, relay, unconfirmed, synced]
            .iter()
            .map(|line| format!("- {line}\n"))
            .collect()
    }
}

/// `seconds` since the Unix epoch as an RFC 3339 time in UTC, to the second.
fn rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
