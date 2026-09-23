//! memory's long-lived embedding handle (E5m/#3rx.2) — the mirror of flow's
//! `nexus_flow_facade::engine::Engine`, specialized to [`MemoryStore`].
//!
//! Where the `nxm` CLI opens the workspace fresh per invocation, an embedding app holds one
//! [`Engine`] for its whole lifetime and reads/writes through it many times. The handle owns the
//! resolved workspace + memory's store behind a `Mutex` (the store wraps a non-`Sync`
//! `rusqlite::Connection`, so the mutex is what makes the handle `Sync` and safe in async runtime
//! state). It is `Clone` (a cheap `Arc` bump) so it can be shared across tasks.
//!
//! The lifecycle (lock + repoint-on-swap + file-based change watch) is the product-agnostic
//! foundation [`Engine`](nxs_foundation::engine::Engine); this layer supplies memory's store factory
//! and exposes the [`crate::facade`] verbs over it. There is NO new semantics: the same `fact`
//! reducer, the same canonical [`MemoryRecord`], the same derivation as the `nxm` CLI (the seam
//! invariant, proven by the #3rx.3 differential).

use crate::error::Result;
use crate::facade::{self, Classification, IndexReport, MemoryRecord, PrimeReport};
use crate::migration::{ApplyOptions, MigrationPlan, MigrationReport, MigrationStatus};
use crate::project_doc::{self, Drift};
use crate::store::{MemoryQuery, MemoryStore};
use crate::workspace::{MemoryWorkspaceExt, Workspace};
use nxs_foundation::engine::{Engine as Handle, OpenStoreFn, State};
use nxs_foundation::watch::Change;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

/// A long-lived in-process handle owning workspace + memory store for the app's lifetime.
/// `Clone` + `Send` + `Sync`: clone freely and share across async tasks. Reads are short and
/// lock-internal; nothing is held across `.await`.
#[derive(Clone)]
pub struct Engine {
    handle: Handle<MemoryStore>,
}

/// memory's store factory for the foundation handle: open the memory store (fact reducer + the
/// `memories` view) over a resolved workspace, in WAL mode.
fn memory_store_factory() -> OpenStoreFn<MemoryStore> {
    Arc::new(|ws: &Workspace| ws.open_memory_store())
}

impl Engine {
    /// Open the workspace `--db`/discovery resolves from `start` and open memory's store over it,
    /// holding both for the handle's lifetime. Fails (`no_workspace`/`io`) if there is no workspace
    /// or the store cannot be opened.
    pub fn open(db: Option<&str>, start: &Path) -> Result<Engine> {
        Self::open_with_poll_interval(db, start, nxs_foundation::watch::POLL_INTERVAL)
    }

    /// Like [`open`](Engine::open) but with an explicit change-watcher poll interval. This is a
    /// **test seam** — production always uses [`open`] (the default cadence).
    #[doc(hidden)]
    pub fn open_with_poll_interval(
        db: Option<&str>,
        start: &Path,
        poll_interval: Duration,
    ) -> Result<Engine> {
        let handle =
            Handle::open_with_poll_interval(db, start, poll_interval, memory_store_factory())?;
        Ok(Engine { handle })
    }

    /// Subscribe to external-write notifications (#3rx.2, mirror of #9t7.3). Returns a receiver that
    /// yields a [`Change`] whenever ANOTHER writer commits to this workspace — the app's cue to
    /// re-read the memory bestand.
    pub fn subscribe(&self) -> Result<Receiver<Change>> {
        self.handle.subscribe()
    }

    // ---- reads -------------------------------------------------------------

    /// Recall one active memory by key, or `not_found`.
    pub fn recall(&self, key: &str) -> Result<MemoryRecord> {
        self.handle
            .try_with_state(|s| facade::recall(&s.store, key))
    }

    /// List the active memories a [`MemoryQuery`] selects, as records. The default query is the
    /// key-sorted, unfiltered read; the query also carries the substring search, the
    /// category/scope filters and the reading-order switch (6j6v.e0z6). Fallible (#76u.13): a db
    /// error in the underlying store read surfaces as `io` instead of unwinding the calling task —
    /// the mirror of flow's `Engine::list` hardening (jo9).
    pub fn memories(&self, query: &MemoryQuery) -> Result<Vec<MemoryRecord>> {
        self.handle
            .try_with_state(|s| facade::memories(&s.store, query))
    }

    /// The memories each of `ids` carries — the retrieval rule's board-item half (6j6v.srpg):
    /// item-scoped memories bucketed by the board item their `refs` name, ids with none omitted.
    /// The counterpart of [`prime`](Engine::prime), which serves the other half (everything that is
    /// NOT item-scoped). An app rendering a work item reads this instead of re-deriving the rule.
    pub fn memories_about(
        &self,
        ids: &[&str],
    ) -> Result<std::collections::BTreeMap<String, Vec<MemoryRecord>>> {
        self.handle
            .try_with_state(|s| facade::memories_about(&s.store, ids))
    }

    /// The `prime` session-bootstrap record (nxf 6j6v.wph0) — the memory rule, the recovery hint,
    /// the command reference and every active memory, as data. The mirror of flow's
    /// `Engine::prime`: an embedding host renders the SAME record the `nxm` CLI does
    /// ([`render_markdown`](PrimeReport::render_markdown) for the SessionStart block,
    /// [`to_value`](PrimeReport::to_value) for its JSON form) instead of rebuilding the assembly on
    /// its own side.
    pub fn prime(&self) -> Result<PrimeReport> {
        self.handle.try_with_state(|s| facade::prime(&s.store))
    }

    /// The whole memory index (nxf 6j6v.5jm3): every memory the session start replays, one written
    /// line each, unbounded in count — the full form of the index [`prime`](Self::prime) renders
    /// within a byte budget.
    ///
    /// An embedding host that composes its own session start reads this when its own surface has
    /// room for the whole directory, or serves it as the "show me everything" the bounded block
    /// points at. The selection and order are [`Engine::prime`]'s own, so the two never disagree
    /// about what a session start is made of.
    pub fn index(&self) -> Result<IndexReport> {
        self.handle.try_with_state(|s| facade::index(&s.store))
    }

    // ---- writes (now + actor explicit) -------------------------------------

    /// Remember a fact and return its record. `key` is the explicit key, or `None` for the
    /// content-hash auto-key; `class` optionally files it as it is written (6j6v.e0z6). The store is
    /// mutated under the handle's state lock — and no model is consulted, so this stays offline and
    /// deterministic like every other write.
    pub fn remember(
        &self,
        now: &str,
        actor: &str,
        key: Option<&str>,
        text: &str,
        class: &Classification,
    ) -> Result<MemoryRecord> {
        self.handle.with_state_mut(|s| {
            let rec = facade::remember(&mut s.store, now, actor, key, text, class)?;
            project_after_write(s);
            Ok(rec)
        })
    }

    /// File an existing memory — category, reach and/or references — without touching its text
    /// (6j6v.e0z6). Returns the reclassified record.
    pub fn classify(
        &self,
        now: &str,
        actor: &str,
        key: &str,
        class: &Classification,
    ) -> Result<MemoryRecord> {
        self.handle.with_state_mut(|s| {
            let rec = facade::classify(&mut s.store, now, actor, key, class)?;
            project_after_write(s);
            Ok(rec)
        })
    }

    /// Store `keys` as the reading order (6j6v.e0z6) — position 1, 2, … in the given sequence — and
    /// return the records in that order. The deliberate, rarely-run counterpart to the write path:
    /// the caller has already decided the sequence, the engine only records it.
    pub fn reorder(&self, now: &str, actor: &str, keys: &[String]) -> Result<Vec<MemoryRecord>> {
        self.handle.with_state_mut(|s| {
            let records = facade::reorder(&mut s.store, now, actor, keys)?;
            project_after_write(s);
            Ok(records)
        })
    }

    /// Forget a memory by key (a reversible tombstone) and return the tombstone record.
    pub fn forget(&self, now: &str, actor: &str, key: &str) -> Result<MemoryRecord> {
        self.handle.with_state_mut(|s| {
            let rec = facade::forget(&mut s.store, now, actor, key)?;
            project_after_write(s);
            Ok(rec)
        })
    }

    // ---- the judging migration (6j6v.9yaj) ---------------------------------

    /// Build the migration plan for this handle's workspace: the memories still filed as `unsorted`
    /// plus every section of the hand-written context documents at the workspace root, each carrying
    /// the status quo as its decision.
    ///
    /// Reading only. The judgement is a separate step — [`crate::migration::ask_judge`] for a model,
    /// or a person editing the document — and [`migrate_apply`](Engine::migrate_apply) executes what
    /// comes back, so an embedding app can put its own review UI between the two exactly where the
    /// CLI puts a file.
    pub fn migrate_plan(&self) -> Result<MigrationPlan> {
        self.handle
            .try_with_state(|s| facade::migrate_plan(&s.store, doc_root(s)))
    }

    /// Execute a decided plan (see [`facade::migrate_apply`]) and regenerate the projection, exactly
    /// as every other write through this handle does.
    pub fn migrate_apply(
        &self,
        now: &str,
        actor: &str,
        plan: &MigrationPlan,
        options: &ApplyOptions,
    ) -> Result<MigrationReport> {
        self.handle.with_state_mut(|s| {
            let root =
                s.ws.dir
                    .parent()
                    .unwrap_or(s.ws.dir.as_path())
                    .to_path_buf();
            let report = facade::migrate_apply(&mut s.store, now, actor, &root, plan, options)?;
            // A dry run wrote nothing, so there is nothing to project; regenerating anyway would
            // turn "compute the report" into a filesystem write behind the caller's back.
            if !report.dry_run {
                project_after_write(s);
            }
            Ok(report)
        })
    }

    /// This workspace's migration state — what is still unfiled, and the mark of a run that already
    /// happened on this stream. The same read `prime` reports an open move from.
    pub fn migrate_status(&self) -> Result<MigrationStatus> {
        self.handle
            .try_with_state(|s| facade::migrate_status(&s.store))
    }

    // ---- the project-memory document (6j6v.8q88) ---------------------------

    /// The project-memory document as this workspace's store projects it — the exact bytes
    /// `NEXUS_MEMORY.md` holds, which is also what `nxm doc` prints. An embedding host that renders
    /// its own context surface reads this instead of re-assembling the projection on its own side,
    /// the same reason [`prime`](Engine::prime) exists.
    ///
    /// Reading it never writes: the file is regenerated by WRITES (and by a sync pass that pulled
    /// something), so that a repository nobody has opened for two weeks does not show a two-week-old
    /// state to the one reader the file exists for.
    pub fn doc(&self) -> Result<String> {
        self.handle
            .try_with_state(|s| project_doc::render(&s.store))
    }

    /// Whether the `NEXUS_MEMORY.md` on disk still matches the store — the drift guard behind
    /// `nxm doc --check`. A hand edit, or a write whose regeneration never ran (a sync daemon that
    /// was not running in this checkout), shows up here as [`Drift::Drifted`]/[`Drift::Missing`].
    ///
    /// **This is also how a host learns that a write's own projection failed** (see
    /// [`project_after_write`]): a projection that could not land leaves the file stale, absent, or
    /// unreadable, so every such state is reported here as an `Err` or a non-[`Drift::InSync`]
    /// verdict — never as silence. Pinned by `tests/embed.rs`'s
    /// `a_write_still_succeeds_when_the_projection_cannot_be_written`, so the claim is a guarantee
    /// rather than a note. A host that surfaces staleness at all should call this after a write
    /// burst, or on whatever cadence it already re-reads the bestand.
    pub fn doc_check(&self) -> Result<Drift> {
        self.handle
            .try_with_state(|s| project_doc::check(&s.store, doc_root(s)))
    }

    /// The resolved workspace (db path, replica identity).
    pub fn workspace(&self) -> Workspace {
        self.handle.workspace()
    }
}

/// Regenerate the project-memory document after a write through this handle, so an embedding app
/// keeps the file current exactly as the `nxm` CLI does.
///
/// Deliberately best-effort: the memory itself is already committed by the time this runs, so
/// turning an IO failure here into the write's error would report a write that DID happen as one
/// that did not.
///
/// **The error is dropped, not lost** (PR #287 review, Integrity #1). The `nxm` CLI can print a
/// `note:` to stderr because it owns the process; a library embedded in someone else's app must
/// not, and this crate deliberately carries no `log`/`tracing` dependency. The signal a host reads
/// instead is [`Engine::doc_check`]: a projection that could not land leaves the document stale,
/// absent or unreadable, and every one of those states is a non-`InSync` verdict there. So the
/// failure is discoverable on the seam the guard already exists on, rather than through a channel
/// the host would have to be told to watch — and a test pins that, so it cannot quietly stop being
/// true.
fn project_after_write(s: &State<MemoryStore>) {
    let _ = project_doc::regenerate(&s.store, doc_root(s));
}

/// The workspace ROOT the document lives in — `ws.dir` IS the `.nxs/` directory, so the file is its
/// parent's, next to `AGENTS.md`.
fn doc_root(s: &State<MemoryStore>) -> &Path {
    s.ws.dir.parent().unwrap_or(s.ws.dir.as_path())
}
