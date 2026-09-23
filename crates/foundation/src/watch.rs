//! File-based change notification (E5 #9t7.3).
//!
//! A long-lived app must learn when ANOTHER writer — the `nxf` CLI agent, the MCP server, the
//! E4 sync client — mutates the workspace's SQLite file, so it can re-derive and re-render.
//! The mechanism is deliberately the lightest thing that works across processes and needs no
//! daemon: a background thread polls `PRAGMA data_version` (see [`Store::data_version`]) on its
//! OWN read connection. That pragma's value bumps whenever any *other* connection commits, so a
//! delta is exactly "someone else wrote." The thread sleeps between polls (`park_timeout`, no
//! busy-spin), lives only as long as the engine, and is woken to exit promptly on drop.
//!
//! The poller's connection is separate from the engine's store, so polling never contends with
//! the handle's read mutex. It is domain-agnostic — `data_version` is a substrate property, so the
//! watcher polls the foundation [`Store`] and knows nothing of any product's views.
//!
//! ## Contract
//!
//! [`Change`] is a **"someone wrote — re-read"** hint, not an exactly-once event log:
//!
//! - **Coalesced, not counted.** Several external commits between two polls collapse into one
//!   `Change`; the consumer's job is to re-read current state, not to replay deltas.
//! - **Do an initial read on subscribe.** The baseline `data_version` is sampled when the watcher
//!   starts, so a write committed between [`crate::engine::Engine::open`] and the first `subscribe`
//!   is folded into the baseline and never produces a `Change`.
//! - **Self-healing across a db reset.** If the workspace db is deleted/recreated/reset out from
//!   under the poller, the loop re-arms on the new file. Two triggers cover both shapes: a poll
//!   connection that goes bad (`data_version()` errors), and — for a same-path recreate that leaves
//!   the old connection pinned to the unlinked inode, still answering a stale value WITHOUT
//!   erroring — a change in the db file's identity (device+inode) checked each tick (#i8o). The
//!   [`Engine`](crate::engine::Engine)'s own read/write store re-points on that same swap
//!   (nexus-flow-2at); to keep that stat off the hot read path, the poll loop also *publishes* its
//!   per-tick identity ([`is_active`] + [`observed_id`]), which the engine's read path reuses while
//!   a subscriber keeps the watcher running (#uh3).
//!
//! [`is_active`]: Watcher::is_active
//! [`observed_id`]: Watcher::observed_id

use crate::error::{NxfError, Result};
use crate::store::Store;
use crate::workspace::Workspace;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A change event: an external writer committed to the workspace. Carries the new `data_version`
/// for diagnostics only (a 32-bit counter); the consumer's reaction is simply to re-read current
/// state (re-derive). A coalesced "go re-read" hint, not an exactly-once delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub data_version: i64,
}

/// Default poll cadence — responsive enough for an interactive desktop, idle enough to be
/// invisible (one `PRAGMA data_version` read per tick). Deliberately not a busy-spin. Tests drive a
/// shorter interval via [`crate::engine::Engine::open_with_poll_interval`].
pub const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// The shared change watcher: one background thread polling `data_version`, broadcasting [`Change`]
/// to every registered subscriber. Owned by the engine — it starts on the first
/// [`subscribe`](Watcher::subscribe) and is torn down on engine drop. No daemon.
pub(crate) struct Watcher {
    subscribers: Arc<Mutex<Vec<Sender<Change>>>>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
    db_path: String,
    site_id: i64,
    interval: Duration,
    active: Arc<AtomicBool>,
    observed_id: Arc<Mutex<Option<FileId>>>,
}

impl Watcher {
    pub(crate) fn new(ws: &Workspace, interval: Duration) -> Watcher {
        Watcher {
            subscribers: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
            db_path: ws.db_path().to_string_lossy().into_owned(),
            site_id: ws.replica.site_id,
            interval,
            active: Arc::new(AtomicBool::new(false)),
            observed_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Whether the poll thread is running. While `true`, an engine read may reuse
    /// [`observed_id`](Watcher::observed_id) rather than issuing its own stat on the hot read path
    /// (#uh3); while `false` the engine must stat for itself.
    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// The db-file identity the poll loop last observed, for the engine to reuse in place of a
    /// per-access stat while [`is_active`](Watcher::is_active) (#uh3).
    pub(crate) fn observed_id(&self) -> Option<FileId> {
        *self.observed_id.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a subscriber, lazily starting the poll thread on the first call, and return the
    /// receiving end of its change channel. Callers should read current state once after
    /// subscribing (see the module contract).
    pub(crate) fn subscribe(&self) -> Result<Receiver<Change>> {
        self.ensure_running()?;
        let (tx, rx) = mpsc::channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        Ok(rx)
    }

    fn ensure_running(&self) -> Result<()> {
        let mut handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if handle.is_some() {
            return Ok(());
        }
        let store = open_poll_connection(&self.db_path, self.site_id)?;
        // Capture the poll connection's file identity HERE, on the subscribing thread, before
        // `subscribe()` returns — NOT lazily inside the spawned thread, so a same-path recreate
        // racing thread start-up cannot deafen the watcher (see #i8o).
        let current_id = file_id(&self.db_path);
        let baseline = store
            .data_version()
            .map_err(|e| NxfError::io(format!("watch: reading data_version: {e}")))?;
        let db_path = self.db_path.clone();
        let site_id = self.site_id;
        let subscribers = Arc::clone(&self.subscribers);
        let stop = Arc::clone(&self.stop);
        let interval = self.interval;
        let active = Arc::clone(&self.active);
        let observed_id = Arc::clone(&self.observed_id);
        let join = thread::Builder::new()
            .name("nxf-watch".into())
            .spawn(move || {
                poll_loop(
                    db_path,
                    site_id,
                    store,
                    baseline,
                    current_id,
                    subscribers,
                    stop,
                    interval,
                    active,
                    observed_id,
                )
            })
            .map_err(|e| NxfError::io(format!("watch: spawning poll thread: {e}")))?;
        *handle = Some(join);
        Ok(())
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            join.thread().unpark();
            let _ = join.join();
        }
    }
}

/// Open a poll connection on `db_path` with the same `busy_timeout` the workspace sets on its store
/// connections — so a `data_version` read that races a writer waits briefly rather than failing
/// with `SQLITE_BUSY`. WAL itself is a db-level property already set when the workspace was created,
/// so a fresh connection inherits it.
fn open_poll_connection(db_path: &str, site_id: i64) -> Result<Store> {
    let store = Store::open(db_path, site_id)
        .map_err(|e| NxfError::io(format!("watch: opening poll connection: {e}")))?;
    store
        .connection()
        .execute_batch("PRAGMA busy_timeout=5000;")
        .map_err(|e| NxfError::io(format!("watch: setting poll pragmas: {e}")))?;
    Ok(store)
}

/// A coarse identity for a file at a path: `(device, inode)` on Unix, `(mtime_nanos, len)`
/// elsewhere. Used to notice a same-path delete+recreate — see [`file_id`].
pub(crate) type FileId = (u64, u64);

/// A coarse identity for the file currently at `path` — enough to notice a same-path delete+recreate
/// (#i8o). On Unix that is `(device, inode)`. `None` if the path is momentarily absent
/// (mid-recreate). Shared with [`crate::engine`], whose read store re-points on the same signal.
#[cfg(unix)]
pub(crate) fn file_id(path: &str) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Non-Unix fallback: no stable inode, so approximate identity by `(mtime_nanos, len)`.
#[cfg(not(unix))]
pub(crate) fn file_id(path: &str) -> Option<FileId> {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path).ok().map(|m| {
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        (mtime, m.len())
    })
}

/// Broadcast a [`Change`] to every live subscriber, pruning the ones whose receiver was dropped.
fn broadcast(subscribers: &Arc<Mutex<Vec<Sender<Change>>>>, data_version: i64) {
    let change = Change { data_version };
    let mut subs = subscribers.lock().unwrap_or_else(|e| e.into_inner());
    subs.retain(|tx| tx.send(change).is_ok());
}

/// The poll loop: each tick reads `data_version`; on a delta, broadcast [`Change`]. Sleeps with
/// `park_timeout` so [`Watcher::drop`] can wake it immediately via `unpark`. Two re-arm triggers
/// cover a db deleted/recreated/reset out from under the poller (see the module contract / #i8o).
#[allow(clippy::too_many_arguments)]
fn poll_loop(
    db_path: String,
    site_id: i64,
    mut store: Store,
    mut last: i64,
    mut current_id: Option<FileId>,
    subscribers: Arc<Mutex<Vec<Sender<Change>>>>,
    stop: Arc<AtomicBool>,
    interval: Duration,
    active: Arc<AtomicBool>,
    observed_id: Arc<Mutex<Option<FileId>>>,
) {
    // Publish the starting identity and go active BEFORE the first park, so a read landing between
    // subscribe() and the first tick reuses a real identity rather than the `None` initial value
    // (#uh3). Order matters: any reader seeing `active == true` is guaranteed to see the published
    // `observed_id`.
    publish(&observed_id, current_id);
    active.store(true, Ordering::SeqCst);
    while !stop.load(Ordering::SeqCst) {
        thread::park_timeout(interval);
        if stop.load(Ordering::SeqCst) {
            break;
        }

        // (#i8o) A same-path recreate the Err branch can't catch: the old connection is pinned to
        // the now-unlinked inode and keeps answering `data_version()` with a stale value WITHOUT
        // erroring. When the file's identity at `db_path` changes, a different file lives there now
        // — reopen on it, re-baseline, and signal a re-read.
        let disk_id = file_id(&db_path);
        publish(&observed_id, disk_id);
        if disk_id.is_some() && disk_id != current_id {
            if let Ok(fresh) = open_poll_connection(&db_path, site_id) {
                if let Ok(v) = fresh.data_version() {
                    store = fresh;
                    current_id = disk_id;
                    last = v;
                    broadcast(&subscribers, v);
                    continue;
                }
            }
        }

        match store.data_version() {
            Ok(v) => {
                if v != last {
                    last = v;
                    broadcast(&subscribers, v);
                }
            }
            Err(_) => {
                // The connection failed outright — most likely the db was reset. Re-arm on the
                // current file ONLY if a file is actually present at the path (gate on `file_id`),
                // so reopening during the brief unlinked window does not conjure a fresh empty db.
                if file_id(&db_path).is_some() {
                    if let Ok(fresh) = open_poll_connection(&db_path, site_id) {
                        if let Ok(v) = fresh.data_version() {
                            store = fresh;
                            current_id = file_id(&db_path);
                            if v != last {
                                last = v;
                                broadcast(&subscribers, v);
                            }
                        }
                    }
                }
            }
        }
    }
    // The thread is exiting (engine dropped): stop offering an identity so any late read falls back
    // to its own stat rather than trusting a frozen value.
    active.store(false, Ordering::SeqCst);
}

/// Publish the watcher's most-recently observed file identity for the engine's read path to reuse
/// (#uh3). A tiny uncontended mutex — far cheaper than the `stat` syscall it spares each read.
fn publish(observed_id: &Arc<Mutex<Option<FileId>>>, id: Option<FileId>) {
    *observed_id.lock().unwrap_or_else(|e| e.into_inner()) = id;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::time::Instant;
    use tempfile::TempDir;

    /// Spin until `cond` holds or `budget` elapses — no fixed sleep that would flake or waste time.
    fn wait_until(budget: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        cond()
    }

    /// The foundation-owned regression net for the change watcher (a future memory/chat product
    /// inherits this generic watcher, not flow's facade tests): a commit from ANOTHER connection
    /// bumps `PRAGMA data_version`, which the poll loop observes and broadcasts as a [`Change`].
    #[test]
    fn watcher_broadcasts_a_change_when_another_connection_commits() {
        let tmp = TempDir::new().unwrap();
        let ws = crate::workspace::init(tmp.path(), &crate::workspace::WorkspaceConfig::default())
            .unwrap();
        let db = ws.db_path_str().unwrap();
        // Materialize the db (foundation `init` is store-agnostic) in WAL mode, so the poll
        // connection and the second writer open the same existing file and don't contend.
        let store = Store::open(&db, ws.replica.site_id).unwrap();
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL;")
            .unwrap();

        let watcher = Watcher::new(&ws, Duration::from_millis(10));
        let rx = watcher.subscribe().unwrap();

        // Commit from a SEPARATE connection — the poller's own connection sees the data_version
        // bump (data_version only changes for commits made by OTHER connections).
        let other = Store::open(&db, ws.replica.site_id).unwrap();
        other
            .connection()
            .execute_batch("CREATE TABLE poke(x); INSERT INTO poke VALUES (1);")
            .unwrap();

        assert!(
            wait_until(Duration::from_secs(2), || rx.try_recv().is_ok()),
            "an external commit produces a Change"
        );
    }
}
