//! The coordination store of the nxc run engine on SQLite: `.nxs/run.sqlite`, beside the old engine's
//! `db.sqlite` and sharing nothing with it. The specification is §7 of epic 6j6v.zjyv.
//!
//! One connection per process, in WAL, with the `busy_timeout` its opener asks for: 100 ms for the host,
//! below its 200 ms tick budget, and 5 s for the CLI bridge. A held lock answers `Busy` and never blocks
//! past that. This store declares no bounds of its own, and no write here depends on one of another
//! backend.

mod append;
mod lease;
mod schema;
mod timers;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use nexus_run_contracts::{
    Appended, Cas, Enqueued, Epoch, Event, Key, RunId, RunRow, RunStore, Seq, StoreError,
    StoreLimits, StoreResult, Timer, Timestamp,
};

pub use lease::{Lease, LEASE_EXPIRY};

const EFFECT_PREFIX: &str = "effect/";

/// [`RunStore`] on one SQLite file. Open it once per process; the tests that open it twice are playing
/// two hosts.
#[derive(Debug)]
pub struct SqliteRunStore {
    conn: Mutex<Connection>,
}

impl SqliteRunStore {
    /// Opens the file, creating it and its tables when missing. `busy_timeout` is how long any call of
    /// this store waits for another writer before it answers `Busy`.
    pub fn open(path: &Path, busy_timeout: Duration) -> StoreResult<Self> {
        let mut conn = Connection::open(path).map_err(db)?;
        conn.busy_timeout(busy_timeout).map_err(db)?;
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(db)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::Backend {
                what: format!("run.sqlite cannot use WAL here: the journal mode stays {mode}"),
            });
        }
        schema::ensure(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// A panic while the lock was held rolled its transaction back when it dropped, so the connection
    /// is still good.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A locked database is `Busy`, whichever statement met it. No error text of SQLite carries a bound
/// value, so `what` is safe to log.
pub(crate) fn db(error: rusqlite::Error) -> StoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StoreError::Busy,
        _ => StoreError::Backend {
            what: error.to_string(),
        },
    }
}

impl RunStore for SqliteRunStore {
    fn enqueue(&self, run: &RunId, event: &Event, cas: &[Cas]) -> StoreResult<Enqueued> {
        append::enqueue(&mut self.conn(), run, event, cas)
    }

    fn append(
        &self,
        run: &RunId,
        expected_seq: Seq,
        fence: Epoch,
        events: &[Event],
        consume: &[Key],
        cas: &[Cas],
    ) -> StoreResult<Appended> {
        append::append(
            &mut self.conn(),
            run,
            expected_seq,
            fence,
            events,
            consume,
            cas,
        )
    }

    fn next_input(&self, run: &RunId) -> StoreResult<Option<Event>> {
        let row: Option<(String, String)> = self
            .conn()
            .query_row(
                "SELECT key, body FROM inbox WHERE run = ?1 ORDER BY rowid LIMIT 1",
                [run.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        row.map(|(key, body)| {
            serde_json::from_str(&body).map_err(|_| StoreError::Backend {
                what: format!("inbox row {key} of {run} does not decode"),
            })
        })
        .transpose()
    }

    fn due_timers(&self, _now: Timestamp) -> StoreResult<Vec<Timer>> {
        Ok(Vec::new())
    }

    fn compare_and_set(
        &self,
        key: &str,
        from: Option<&[u8]>,
        to: Option<&[u8]>,
    ) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = append::immediate(&mut conn)?;
        append::apply_cas(
            &tx,
            &Cas {
                key: key.to_owned(),
                from: from.map(<[u8]>::to_vec),
                to: to.map(<[u8]>::to_vec),
            },
        )?;
        tx.commit().map_err(db)
    }

    fn get(&self, key: &str) -> StoreResult<Option<Vec<u8>>> {
        self.conn()
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(db)
    }

    fn open_runs(&self) -> StoreResult<Vec<RunRow>> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT run, workspace_id, last_seq, max_v, commissioned_at FROM runs
                 WHERE finished = 0 ORDER BY commissioned_at, run",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Seq>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(db)?;
        rows.map(|row| {
            let (run, workspace_id, last_seq, max_v, commissioned_at) = row.map_err(db)?;
            Ok(RunRow {
                run: stored_run_id(&run)?,
                workspace_id,
                finished: false,
                last_seq,
                max_v,
                commissioned_at: Timestamp::from_unix_millis(commissioned_at),
            })
        })
        .collect()
    }

    fn load(&self, run: &RunId, from_seq: Seq) -> StoreResult<Vec<Event>> {
        let conn = self.conn();
        let mut statement = conn
            .prepare("SELECT seq, body FROM events WHERE run = ?1 AND seq > ?2 ORDER BY seq")
            .map_err(db)?;
        let rows = statement
            .query_map(params![run.as_str(), from_seq], |row| {
                Ok((row.get::<_, Seq>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db)?;
        rows.map(|row| {
            let (seq, body) = row.map_err(db)?;
            serde_json::from_str(&body).map_err(|_| StoreError::Backend {
                what: format!("event {seq} of {run} does not decode"),
            })
        })
        .collect()
    }

    fn limits(&self) -> StoreLimits {
        StoreLimits::default()
    }
}

/// A run id the store wrote itself, so one that does not read is a damaged file.
fn stored_run_id(text: &str) -> StoreResult<RunId> {
    RunId::parse(text).ok_or_else(|| StoreError::Backend {
        what: "a run row holds no run id".to_owned(),
    })
}
