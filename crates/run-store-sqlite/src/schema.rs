//! The tables of `run.sqlite` (§7). No table, key or file is shared with the old engine.
//!
//! Instants are stored as unix milliseconds, so `fire_at <= now` is an integer comparison. `body` holds
//! the whole event in its wire form, not only its body: `inbox` has no column for `v`, and what a newer
//! writer put on the envelope has to survive a round trip. The other columns are written from the same
//! value in the same statement, and only `body` is read back into an event.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use nexus_run_contracts::{StoreError, StoreResult};

use crate::db;

const SCHEMA_VERSION: i64 = 1;

const TABLES: &str = "
    CREATE TABLE IF NOT EXISTS events (
        run    TEXT    NOT NULL,
        seq    INTEGER NOT NULL,
        at     INTEGER NOT NULL,
        v      INTEGER NOT NULL,
        key    TEXT    NOT NULL,
        origin TEXT    NOT NULL,
        body   TEXT    NOT NULL,
        PRIMARY KEY (run, seq),
        UNIQUE (run, key)
    );
    CREATE TABLE IF NOT EXISTS inbox (
        run    TEXT    NOT NULL,
        key    TEXT    NOT NULL,
        at     INTEGER NOT NULL,
        origin TEXT    NOT NULL,
        body   TEXT    NOT NULL,
        PRIMARY KEY (run, key)
    );
    CREATE TABLE IF NOT EXISTS timers (
        run     TEXT    NOT NULL,
        timer   TEXT    NOT NULL,
        fire_at INTEGER NOT NULL,
        purpose TEXT    NOT NULL,
        state   TEXT    NOT NULL,
        PRIMARY KEY (run, timer)
    );
    CREATE INDEX IF NOT EXISTS timers_due ON timers (state, fire_at);
    CREATE TABLE IF NOT EXISTS kv (
        key   TEXT PRIMARY KEY,
        value BLOB NOT NULL
    );
    CREATE TABLE IF NOT EXISTS runs (
        run             TEXT PRIMARY KEY,
        workspace_id    TEXT    NOT NULL,
        finished        INTEGER NOT NULL,
        last_seq        INTEGER NOT NULL,
        max_v           INTEGER NOT NULL,
        commissioned_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS schema (version INTEGER NOT NULL);
";

/// Creates the tables on a new file. A file already at this version takes no write lock, so an open
/// beside a busy writer does not wait for it; a file from a newer version is refused.
pub(crate) fn ensure(conn: &mut Connection) -> StoreResult<()> {
    let version = match read_version(conn)? {
        Some(version) => version,
        None => {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            tx.execute_batch(TABLES).map_err(db)?;
            tx.execute(
                "INSERT INTO schema (version) SELECT ?1 WHERE NOT EXISTS (SELECT 1 FROM schema)",
                [SCHEMA_VERSION],
            )
            .map_err(db)?;
            tx.commit().map_err(db)?;
            SCHEMA_VERSION
        }
    };
    if version > SCHEMA_VERSION {
        return Err(StoreError::Backend {
            what: format!(
                "run.sqlite is at schema {version}, and this version knows {SCHEMA_VERSION}"
            ),
        });
    }
    Ok(())
}

fn read_version(conn: &Connection) -> StoreResult<Option<i64>> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    if exists.is_none() {
        return Ok(None);
    }
    conn.query_row("SELECT max(version) FROM schema", [], |row| row.get(0))
        .map_err(db)
}
