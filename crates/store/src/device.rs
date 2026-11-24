//! The device member's own settings (RFC 0001, "Sync", **One relay per member database.**): the
//! relay the device's database syncs through, and when `nxs sync` last found nothing behind or
//! unconfirmed. They live in `device.toml` in the data directory, never in `nexus-flow.toml`:
//! which relay a machine syncs through is the machine's choice, and a URL may carry a host a team
//! keeps private.
//!
//! ```toml
//! relay = "wss://relay.example.org"   # optional: without it the database is in island mode
//! synced_at = 1795000000              # optional: seconds since the Unix epoch
//! ```
//!
//! The SDK reports no time of the last sync, so nxs records `synced_at` itself (NEX-102).

use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

use toml::{Table, Value};

use crate::data_dir::DataDir;

/// A relay's URL: `ws://` or `wss://` and a host, the only URLs NoCentralDB opens (NoCentralDB
/// RFC 0009, "The relay connection").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayUrl(String);

impl RelayUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Text that is not a relay's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotARelayUrl(pub String);

impl fmt::Display for NotARelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a relay URL: it must start with ws:// or wss:// and name a host",
            self.0
        )
    }
}

impl std::error::Error for NotARelayUrl {}

impl FromStr for RelayUrl {
    type Err = NotARelayUrl;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text
            .strip_prefix("wss://")
            .or_else(|| text.strip_prefix("ws://"));
        match rest {
            Some(rest) if !rest.is_empty() && !rest.starts_with('/') => {
                Ok(RelayUrl(text.to_owned()))
            }
            _ => Err(NotARelayUrl(text.to_owned())),
        }
    }
}

/// Why `device.toml` could not be read or written.
#[derive(Debug)]
pub struct SettingsError {
    pub file: PathBuf,
    pub problem: String,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.problem)
    }
}

impl std::error::Error for SettingsError {}

/// The device member's settings, from `device.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceSettings {
    /// The relay the database syncs through; `None` in island mode.
    pub relay: Option<RelayUrl>,
    /// When `nxs sync` last found nothing behind or unconfirmed, in seconds since the Unix
    /// epoch; `None` until it first did.
    pub synced_at: Option<u64>,
}

impl DeviceSettings {
    /// Reads the settings in `data`; a missing file holds none.
    pub fn read(data: &DataDir) -> Result<Self, SettingsError> {
        let file = data.settings();
        let fail = |problem: String| SettingsError {
            file: file.clone(),
            problem,
        };
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(fail(e.to_string())),
        };
        let table: Table = text
            .parse()
            .map_err(|e: toml::de::Error| fail(format!("not valid TOML: {}", e.message())))?;
        let mut settings = Self::default();
        for (key, value) in table {
            match (key.as_str(), value) {
                ("relay", Value::String(url)) => {
                    settings.relay =
                        Some(url.parse().map_err(|e: NotARelayUrl| fail(e.to_string()))?)
                }
                ("synced_at", Value::Integer(seconds)) if seconds >= 0 => {
                    settings.synced_at = Some(seconds as u64)
                }
                ("relay", _) => return Err(fail("relay must be a string".to_owned())),
                ("synced_at", _) => {
                    return Err(fail(
                        "synced_at must be a whole number of seconds".to_owned(),
                    ))
                }
                (other, _) => return Err(fail(format!("{other} is not a key nxs knows"))),
            }
        }
        Ok(settings)
    }

    /// Writes the settings into `data`, creating the directory if needed.
    pub fn write(&self, data: &DataDir) -> Result<(), SettingsError> {
        let file = data.settings();
        let fail = |e: io::Error| SettingsError {
            file: file.clone(),
            problem: e.to_string(),
        };
        let mut table = Table::new();
        if let Some(relay) = &self.relay {
            table.insert("relay".to_owned(), Value::String(relay.0.clone()));
        }
        if let Some(seconds) = self.synced_at {
            let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
            table.insert("synced_at".to_owned(), Value::Integer(seconds));
        }
        data.open().map_err(fail)?;
        fs::write(&file, table.to_string()).map_err(fail)
    }
}
