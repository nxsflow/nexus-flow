//! A v1 workspace, read without changing it (RFC 0008, "What v1 holds").
//!
//! The reader opens `.nxs/` itself and never through v1's `Store::open`, which writes on open. It
//! reads four things and nothing else: `replica.toml`, `config.toml`, the stream id in
//! `sync.toml`, and `db.sqlite`. `signing.key` is never opened, and nothing of `sync.toml` but its
//! stream id is kept.

mod db;
mod error;
mod files;

use std::path::{Path, PathBuf};

pub use error::Error;
pub use files::Replica;

/// A v1 workspace directory (a `.nxs/` folder), opened for reading.
#[derive(Debug)]
pub struct V1Workspace {
    dir: PathBuf,
    replica: Replica,
    plugin: String,
    name: Option<String>,
    stream_id: Option<String>,
    schema: i64,
}

impl V1Workspace {
    /// Opens the v1 workspace in `dir`, a `.nxs/` folder.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let replica = files::replica(dir)?;
        let config = files::config(dir)?;
        let stream_id = files::stream_id(dir)?;
        let conn = db::open(&dir.join(db::FILE))?;
        let schema = db::schema(&conn)?;
        Ok(V1Workspace {
            dir: dir.to_path_buf(),
            replica,
            plugin: config.plugin,
            name: config.name,
            stream_id,
            schema,
        })
    }

    /// The workspace directory it was opened from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The replica's identity, from `replica.toml`.
    pub fn replica(&self) -> &Replica {
        &self.replica
    }

    /// The flow plugin, resolved as v1 resolves it: `[flow] plugin`, else a top-level `plugin`,
    /// else `issue-tracker`.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// `[workspace] name` from `config.toml`, when it has one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The sync stream the workspace is bound to, when `sync.toml` exists.
    pub fn stream_id(&self) -> Option<&str> {
        self.stream_id.as_deref()
    }

    /// The database's schema version, its `PRAGMA user_version`.
    pub fn schema(&self) -> i64 {
        self.schema
    }
}
