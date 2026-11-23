//! A v1 workspace, read without changing it (RFC 0008, "What v1 holds").
//!
//! The reader opens `.nxs/` itself and never through v1's `Store::open`, which writes on open. It
//! reads four things and nothing else: `replica.toml`, `config.toml`, the stream id in
//! `sync.toml`, and `db.sqlite`. `signing.key` is never opened, and nothing of `sync.toml` but its
//! stream id is kept. The one write is [`V1Workspace::mark`], which the move calls before it
//! replays anything (see [`crate::marker`]).

mod db;
mod error;
mod files;
mod op;

use std::path::{Path, PathBuf};

pub use error::{Error, Refusal};
pub use files::Replica;
pub use op::V1Op;

pub(crate) use db::{refuse_in_use, FILE as DB_FILE};

use rusqlite::Connection;

use crate::marker::{self, Mark, Marker};

/// The one plugin whose workspaces move (decided by the owner, NEX-81).
const SUPPORTED_PLUGIN: &str = "issue-tracker";

/// The one schema of v1's database the reader knows.
const SCHEMA: i64 = 9;

/// A v1 workspace directory (a `.nxs/` folder), opened for reading.
pub struct V1Workspace {
    dir: PathBuf,
    conn: Connection,
    unchanged: db::Unchanged,
    replica: Replica,
    plugin: String,
    name: Option<String>,
    stream_id: Option<String>,
    schema: i64,
}

impl std::fmt::Debug for V1Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1Workspace")
            .field("dir", &self.dir)
            .field("replica", &self.replica)
            .field("plugin", &self.plugin)
            .field("name", &self.name)
            .field("stream_id", &self.stream_id)
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl V1Workspace {
    /// Opens the v1 workspace in `dir`, a `.nxs/` folder, or refuses it. The checks run in the
    /// order of [`Refusal`]'s variants, and none of them writes: a refused workspace is as it was.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let db_path = dir.join(db::FILE);
        if !db_path.is_file() {
            return Err(Error::Refused(Refusal::NothingToMigrate));
        }
        let unchanged = db::Unchanged::take(&db_path)?;
        let replica = files::replica(dir)?;
        let config = files::config(dir)?;
        if config.plugin != SUPPORTED_PLUGIN {
            return Err(Error::Refused(Refusal::UnsupportedPlugin {
                plugin: config.plugin,
            }));
        }
        let stream_id = files::stream_id(dir)?;
        let conn = db::open(&db_path)?;
        let schema = db::schema(&conn)?;
        if schema < SCHEMA {
            return Err(Error::Refused(Refusal::V1FormatOld { schema }));
        }
        if schema > SCHEMA {
            return Err(Error::Refused(Refusal::V1FormatUnknown { schema }));
        }
        if db::is_based(&conn)? {
            return Err(Error::Refused(Refusal::V1BasedStore));
        }
        unchanged.check()?;
        Ok(V1Workspace {
            dir: dir.to_path_buf(),
            conn,
            unchanged,
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

    /// Every op of the log, chat's included, in v1's order: `lamport`, then `site`, then `op_id`.
    ///
    /// The database is read without a lock, so a v1 process that opened it since [`open`]
    /// could have changed it under the read: the read is refused with [`Refusal::V1InUse`] then,
    /// whatever it returned.
    ///
    /// [`open`]: Self::open
    pub fn ops(&self) -> Result<Vec<V1Op>, Error> {
        let ops = db::ops(&self.conn);
        self.unchanged.check()?;
        ops
    }

    /// Marks the workspace as moved before the run replays it (RFC 0008, "The marker"): writes
    /// `migrated.toml` and raises the database's header to [`marker::RAISED`], so every v1 binary
    /// refuses the workspace from then on. Refused with [`Refusal::V1InUse`] while a v1 process
    /// holds the database, or after one changed it since [`open`](Self::open). The ops still read
    /// afterwards, since the header is the only part of the database that changed.
    pub fn mark(&mut self, mark: &Mark) -> Result<Marker, marker::Error> {
        self.unchanged.check()?;
        let marker = marker::write(&self.dir, mark)?;
        self.unchanged = db::Unchanged::take(&self.dir.join(db::FILE))?;
        Ok(marker)
    }
}
