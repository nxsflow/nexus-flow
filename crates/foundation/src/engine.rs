//! A long-lived, in-process engine handle (E5 #9t7.2) — the product-agnostic embedding API.
//!
//! Where the CLI and the MCP server open the workspace fresh per invocation, an embedding app holds
//! one [`Engine`] for its whole lifetime and reads through it many times. The handle owns the
//! resolved workspace + the product's store behind a `Mutex` (the store wraps a non-`Sync`
//! `rusqlite::Connection`, so the mutex is what makes the handle `Sync` and safe in async runtime
//! state). It is `Clone` (a cheap `Arc` bump) so it can be shared across tasks.
//!
//! It is **generic over the store** `S` and carries no product vocabulary or presentation: the
//! foundation supplies the lifecycle (lock + repoint-on-swap + change watch), and a product layers
//! its reads/writes + plugins over [`with_state`](Engine::with_state) / [`try_with_state`] /
//! [`with_state_mut`] (spec §4.4 — "Produkte legen Read-Schicht + Plugins darüber"). flow's layer
//! is `nexus_flow_facade::engine::FlowEngine`. The product also supplies the store factory the
//! engine (re)opens `S` with, so the foundation never needs to name a product's store type.

use crate::error::Result;
use crate::watch::{file_id, Change, FileId, Watcher, POLL_INTERVAL};
use crate::workspace::Workspace;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Opens (and on a same-path swap, reopens) the product's store `S` over a resolved workspace. The
/// product supplies this so the foundation engine stays domain-agnostic.
pub type OpenStoreFn<S> = Arc<dyn Fn(&Workspace) -> Result<S> + Send + Sync>;

/// The mutable state guarded by the handle's mutex: the resolved workspace, the open store, and the
/// identity of the db file that store is bound to (so a same-path delete+recreate can be detected
/// and the store re-pointed — nexus-flow-2at). `ws`/`store` are `pub` so a product's read/write
/// layer can reach both (the workspace carries the replica identity writes mint ids under).
pub struct State<S> {
    pub ws: Workspace,
    pub store: S,
    /// Identity (`device+inode` on Unix) of the file `store` was last opened on. `None` if the path
    /// was momentarily absent at open; reconciled on the next access once it reappears.
    db_id: Option<FileId>,
}

struct Inner<S> {
    state: Mutex<State<S>>,
    /// File-based change watcher (#9t7.3). Shared across clones; started lazily on the first
    /// `subscribe()` and torn down when the last `Engine` clone is dropped.
    watcher: Watcher,
    /// Reopen the store on a same-path swap (nexus-flow-2at) — supplied by the product.
    open_store: OpenStoreFn<S>,
}

/// A long-lived in-process handle owning workspace + store for the app's lifetime, generic over the
/// product's store `S`. `Clone` + `Send` + `Sync`: clone freely and share across async tasks.
pub struct Engine<S> {
    inner: Arc<Inner<S>>,
}

impl<S> Clone for Engine<S> {
    fn clone(&self) -> Self {
        Engine {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> Engine<S> {
    /// Open the workspace that `--db`/discovery resolves from `start` and open the product's store
    /// over it via `open_store`, holding both for the handle's lifetime. Fails
    /// (`no_workspace`/`io`/…) if there is no workspace or the store cannot be opened.
    pub fn open(db: Option<&str>, start: &Path, open_store: OpenStoreFn<S>) -> Result<Engine<S>> {
        Self::open_with_poll_interval(db, start, POLL_INTERVAL, open_store)
    }

    /// Like [`open`](Engine::open) but with an explicit change-watcher poll interval. A **test
    /// seam** — production always uses [`open`] (the default cadence); tests drive the watcher with
    /// a short interval to assert across several completed poll cycles quickly.
    #[doc(hidden)]
    pub fn open_with_poll_interval(
        db: Option<&str>,
        start: &Path,
        poll_interval: Duration,
        open_store: OpenStoreFn<S>,
    ) -> Result<Engine<S>> {
        let ws = Workspace::resolve(db, start)?;
        let store = open_store(&ws)?;
        let db_id = file_id(&ws.db_path().to_string_lossy());
        let watcher = Watcher::new(&ws, poll_interval);
        Ok(Engine {
            inner: Arc::new(Inner {
                state: Mutex::new(State { ws, store, db_id }),
                watcher,
                open_store,
            }),
        })
    }

    /// Subscribe to external-write notifications (#9t7.3). Returns a receiver that yields a
    /// [`Change`] whenever ANOTHER writer commits to this workspace — the app's cue to re-read and
    /// re-render. The watcher starts on the first call and lives until the last `Engine` clone is
    /// dropped; there is no daemon. May fail (`io`) if the poll connection cannot be opened.
    pub fn subscribe(&self) -> Result<Receiver<Change>> {
        self.inner.watcher.subscribe()
    }

    /// Run `f` against the locked state, **recovering** from a poisoned lock rather than cascading
    /// the panic. This is a process-lifetime `Arc` handle, so re-panicking would permanently brick
    /// every clone — a self-inflicted DoS; recovering is far more robust for a long-lived embedding
    /// app (#9t7.2 review, Integrity #1). Recovery stays sound because the only `State` mutation a
    /// read can trigger is [`repoint_if_swapped`]'s coherent store swap (both `store` and `db_id`
    /// move together, infallibly), so a panic in the read `f` itself leaves nothing torn.
    ///
    /// The **infallible** read path: a failed re-point is discarded (best-effort, retried next
    /// access) because this signature carries no `Result` to surface it. A read that *can* fail uses
    /// [`try_with_state`](Engine::try_with_state).
    pub fn with_state<T>(&self, f: impl FnOnce(&State<S>) -> T) -> T {
        let mut guard = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let _ = ensure_current_read(&mut guard, &self.inner.watcher, &self.inner.open_store);
        f(&guard)
    }

    /// Like [`with_state`](Engine::with_state) but for the **fallible** reads: a persistent
    /// [`ensure_current_read`] re-point failure is propagated as `io` (before `f` runs) instead of
    /// silently serving rows from the old, unlinked inode (review I&R #3).
    pub fn try_with_state<T>(&self, f: impl FnOnce(&State<S>) -> Result<T>) -> Result<T> {
        let mut guard = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        ensure_current_read(&mut guard, &self.inner.watcher, &self.inner.open_store)?;
        f(&guard)
    }

    /// Like [`with_state`](Engine::with_state) but for the write methods, which need `&mut State`.
    /// A persistent [`ensure_current_write`] re-point failure is propagated before the write runs —
    /// writing through a store pinned to a dead inode would be silently lost (review I&R #3).
    pub fn with_state_mut<T>(&self, f: impl FnOnce(&mut State<S>) -> Result<T>) -> Result<T> {
        let mut guard = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        ensure_current_write(&mut guard, &self.inner.open_store)?;
        f(&mut guard)
    }

    /// The resolved workspace (db path, replica identity). Used by the change watcher and by a
    /// consumer that needs the on-disk location.
    pub fn workspace(&self) -> Workspace {
        self.with_state(|s| s.ws.clone())
    }
}

/// Re-point the store at the file now living at the workspace db path when a same-path
/// delete+recreate has swapped it underneath the handle (nexus-flow-2at). The change watcher (#i8o)
/// already re-arms its OWN poll connection on such a swap — but the handle's read/write store stayed
/// pinned to the old, now-unlinked inode, which keeps answering STALE rows WITHOUT erroring. Both
/// entry points mirror the watcher's identity check (`watch::file_id`, `device+inode` on Unix).
///
/// Reads and writes source that identity DIFFERENTLY (#uh3): [`ensure_current_read`] lifts the stat
/// off the hot read path (reusing the watcher's per-tick identity while a subscriber keeps it
/// polling); [`ensure_current_write`] ALWAYS stats, for an IMMEDIATE re-point (a write must never
/// land on the old unlinked inode). Failure handling is deliberate (review I&R #3): the reopen is
/// retried on every access (`db_id` only advances on success, so a transient failure self-heals),
/// but a persistent failure is surfaced, not swallowed, by the fallible read/write paths.
fn ensure_current_read<S>(
    state: &mut State<S>,
    watcher: &Watcher,
    open_store: &OpenStoreFn<S>,
) -> Result<()> {
    let disk_id = if watcher.is_active() {
        watcher.observed_id()
    } else {
        engine_stat(&state.ws.db_path().to_string_lossy())
    };
    repoint_if_swapped(state, disk_id, open_store)
}

/// The write-path identity check: always stat for an immediate re-point (see [`ensure_current_read`]
/// for why writes do not piggyback on the watcher cadence).
fn ensure_current_write<S>(state: &mut State<S>, open_store: &OpenStoreFn<S>) -> Result<()> {
    let disk_id = file_id(&state.ws.db_path().to_string_lossy());
    repoint_if_swapped(state, disk_id, open_store)
}

/// Reopen the store on the file now at the db path iff its identity differs from the one the store
/// is bound to. Shared tail of both [`ensure_current_read`] and [`ensure_current_write`]. The swap
/// is coherent — `store` and `db_id` move together, infallibly — so a panic afterwards leaves no
/// torn state. A `None` disk id (path absent right now) is left alone, as the watcher does.
fn repoint_if_swapped<S>(
    state: &mut State<S>,
    disk_id: Option<FileId>,
    open_store: &OpenStoreFn<S>,
) -> Result<()> {
    if disk_id.is_some() && disk_id != state.db_id {
        let store = open_store(&state.ws)?;
        state.store = store;
        state.db_id = disk_id;
    }
    Ok(())
}

/// Stat the db path for its identity on the read path's OWN behalf — the fallback taken only when no
/// watcher is polling to supply it (#uh3). Factored out so a test can assert the hot path does not
/// reach here while the watcher is active.
fn engine_stat(path: &str) -> Option<FileId> {
    #[cfg(test)]
    HOT_PATH_STATS.with(|c| c.set(c.get() + 1));
    file_id(path)
}

// Counts the engine's own per-access stats on the calling thread, so a unit test can pin that the
// hot read path stats itself only when there is no watcher to lean on (#uh3). Test-only.
#[cfg(test)]
thread_local! {
    static HOT_PATH_STATS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NxfError;
    use crate::store::Store;
    use std::time::Instant;
    use tempfile::TempDir;

    /// A store factory over the foundation substrate store — the foundation tests the generic handle
    /// against its own store; a product (flow) supplies its own factory in production.
    fn open_substrate_store() -> OpenStoreFn<Store> {
        Arc::new(|ws: &Workspace| -> Result<Store> {
            let path = ws.db_path_str()?;
            let store = Store::open(&path, ws.replica.site_id)
                .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))?;
            store
                .connection()
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
            Ok(store)
        })
    }

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

    /// nexus-flow-uh3: while a subscriber keeps the change watcher polling, the engine's hot read
    /// path reuses the watcher's observed identity instead of issuing its OWN `stat` on every locked
    /// read.
    #[test]
    fn subscribed_reads_do_not_stat_on_the_hot_path() {
        let tmp = TempDir::new().unwrap();
        crate::workspace::init(tmp.path(), &crate::workspace::WorkspaceConfig::default()).unwrap();
        let engine = Engine::open_with_poll_interval(
            None,
            tmp.path(),
            Duration::from_millis(20),
            open_substrate_store(),
        )
        .unwrap();
        let _rx = engine.subscribe().unwrap();
        assert!(
            wait_until(Duration::from_secs(2), || engine.inner.watcher.is_active()),
            "the poll thread is active after subscribe, so reads take the piggyback branch"
        );

        HOT_PATH_STATS.with(|c| c.set(0));
        for _ in 0..50 {
            engine.with_state(|_| ());
        }
        assert_eq!(
            HOT_PATH_STATS.with(|c| c.get()),
            0,
            "no per-access stat while the watcher supplies the file identity"
        );
    }

    /// The mirror: with NO subscriber the watcher never polls, so the read path must fall back to
    /// its own per-access stat.
    #[test]
    fn unsubscribed_reads_stat_for_themselves() {
        let tmp = TempDir::new().unwrap();
        crate::workspace::init(tmp.path(), &crate::workspace::WorkspaceConfig::default()).unwrap();
        let engine = Engine::open(None, tmp.path(), open_substrate_store()).unwrap();
        assert!(
            !engine.inner.watcher.is_active(),
            "no poll thread without a subscriber"
        );

        HOT_PATH_STATS.with(|c| c.set(0));
        for _ in 0..10 {
            engine.with_state(|_| ());
        }
        assert_eq!(
            HOT_PATH_STATS.with(|c| c.get()),
            10,
            "each read stats for itself when there is no watcher to lean on"
        );
    }

    /// A reader that panics while holding the state lock must NOT permanently brick the shared
    /// handle: a subsequent read on a clone recovers the poisoned lock and succeeds.
    #[test]
    fn reads_recover_after_a_poisoned_lock() {
        let tmp = TempDir::new().unwrap();
        crate::workspace::init(tmp.path(), &crate::workspace::WorkspaceConfig::default()).unwrap();
        let engine = Engine::open(None, tmp.path(), open_substrate_store()).unwrap();

        let poisoner = engine.clone();
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            poisoner.with_state(|_| panic!("poison the lock"));
        }));
        std::panic::set_hook(prev_hook);
        assert!(res.is_err(), "the poisoning read panicked as set up");

        // The lock is now poisoned; reads on another clone still work (recovered, not cascaded).
        engine.with_state(|s| assert!(s.store.op_count() == 0));
        engine
            .try_with_state(|_| Ok::<(), NxfError>(()))
            .expect("still answers normally");
    }
}
