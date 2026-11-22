//! The TOML files of a v1 workspace: `replica.toml`, `config.toml` and `sync.toml`.

use std::path::Path;

use serde::Deserialize;

use super::Error;

/// The replica's identity, as `replica.toml` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Replica {
    /// The replica's site in v1's op order.
    pub site_id: i64,
    /// The prefix of the item ids it mints.
    pub prefix: String,
    /// Its durable identity; empty in a `replica.toml` written before v1 had one.
    #[serde(default)]
    pub replica_uuid: String,
}

/// What the move takes from `config.toml`.
pub(super) struct Config {
    pub plugin: String,
    pub name: Option<String>,
}

/// v1's plugin when `config.toml` names none (`DEFAULT_PLUGIN` in v1's facade).
const DEFAULT_PLUGIN: &str = "issue-tracker";

#[derive(Deserialize, Default)]
struct RawConfig {
    /// A config written before `[flow]` names its plugin at the top level.
    plugin: Option<String>,
    flow: Option<RawFlow>,
    workspace: Option<RawWorkspace>,
}

#[derive(Deserialize)]
struct RawFlow {
    plugin: Option<String>,
}

#[derive(Deserialize)]
struct RawWorkspace {
    name: Option<String>,
}

/// The one key of `sync.toml` the move reads. The endpoint, which may carry a password, and the
/// sync cursors have no field here, so they are never kept.
#[derive(Deserialize)]
struct RawSync {
    stream_id: String,
}

pub(super) fn replica(dir: &Path) -> Result<Replica, Error> {
    let path = dir.join("replica.toml");
    let raw = read(&path)?.ok_or_else(|| missing(&path))?;
    parse(&path, &raw)
}

/// `config.toml`, read as v1 reads it: a missing file is the default config.
pub(super) fn config(dir: &Path) -> Result<Config, Error> {
    let path = dir.join("config.toml");
    let raw: RawConfig = match read(&path)? {
        Some(raw) => parse(&path, &raw)?,
        None => RawConfig::default(),
    };
    let plugin = raw
        .flow
        .and_then(|flow| flow.plugin)
        .or(raw.plugin)
        .unwrap_or_else(|| DEFAULT_PLUGIN.to_string());
    Ok(Config {
        plugin,
        name: raw.workspace.and_then(|w| w.name),
    })
}

/// The stream id in `sync.toml`, or `None` for a workspace never bound to a stream.
pub(super) fn stream_id(dir: &Path) -> Result<Option<String>, Error> {
    let path = dir.join("sync.toml");
    match read(&path)? {
        Some(raw) => Ok(Some(parse::<RawSync>(&path, &raw)?.stream_id)),
        None => Ok(None),
    }
}

/// The file's text, or `None` when it does not exist.
fn read(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn parse<T: serde::de::DeserializeOwned>(path: &Path, raw: &str) -> Result<T, Error> {
    toml::from_str(raw).map_err(|_| Error::Toml {
        path: path.to_path_buf(),
    })
}

fn missing(path: &Path) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source: std::io::ErrorKind::NotFound.into(),
    }
}
