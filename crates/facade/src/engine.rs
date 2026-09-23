//! flow's long-lived embedding handle (E5 #9t7.2): the product layer over the foundation's
//! product-agnostic [`Engine`](nxs_foundation::engine::Engine) (spec §4.4 — "Produkte legen
//! Read-Schicht + Plugins darüber").
//!
//! The foundation handle owns the lifecycle (the `Arc<Mutex<Store>>`, the change watch, the
//! repoint-on-swap) and is generic over the store + carries NO plugin. This `Engine` specializes it
//! to flow's [`Store`](nexus_flow_core::store::Store), supplies the store factory, and layers flow's
//! read/write compute + the active plugin config (vocabulary + ranking) on top. The methods are a
//! thin concurrency-safe wrapper over [`crate::read`]/[`crate::write`] — no new semantics, the same
//! derivation and canonical records as the CLI.

use crate::error::Result;
use crate::plugin::{self, PluginConfig};
use crate::read::{self, BlockedItem, PrimeReport, ShowRecord};
use crate::validate;
use crate::workspace::{Workspace, WorkspaceExt};
use crate::write::{self, NewItem};
use nexus_flow_core::model::{ItemRow, LinkRelation, LinkWeight, ThreadLink};
use nexus_flow_core::store::Store;
use nxs_foundation::engine::{Engine as Handle, OpenStoreFn};
use nxs_foundation::watch::Change;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

/// A long-lived in-process handle owning workspace + flow store + the active plugin for the app's
/// lifetime. `Clone` + `Send` + `Sync`: clone freely and share across async tasks. Reads are short
/// and lock-internal; nothing is held across `.await`.
#[derive(Clone)]
pub struct Engine {
    handle: Handle<Store>,
    /// The plugin config the workspace selected (vocabulary + ranking), read once at open — flow's
    /// presentation axis, layered over the product-agnostic foundation handle.
    cfg: PluginConfig,
}

/// flow's store factory for the foundation handle: open the flow store (task reducer + flow views)
/// over a resolved workspace, in WAL mode.
fn flow_store_factory() -> OpenStoreFn<Store> {
    Arc::new(|ws: &Workspace| ws.open_store())
}

impl Engine {
    /// Open the workspace `--db`/discovery resolves from `start`, load its plugin config, and open
    /// the flow store — holding all three for the handle's lifetime. Fails
    /// (`no_workspace`/`io`/`validation`) if there is no workspace, the db cannot be opened, or the
    /// selected plugin is unknown.
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
        // Resolve the workspace to read its plugin selection (the same resolution the foundation
        // handle does); a malformed/unknown plugin fails here before the handle opens.
        let ws = Workspace::resolve(db, start)?;
        let cfg = plugin::load(&crate::workspace::flow_plugin(&ws.config))?;
        let handle =
            Handle::open_with_poll_interval(db, start, poll_interval, flow_store_factory())?;
        Ok(Engine { handle, cfg })
    }

    /// Subscribe to external-write notifications (#9t7.3). Returns a receiver that yields a
    /// [`Change`] whenever ANOTHER writer commits to this workspace — the app's cue to re-read.
    pub fn subscribe(&self) -> Result<Receiver<Change>> {
        self.handle.subscribe()
    }

    /// Blocked items, each with its open blockers, in the shared default order (rank, C2 #916.2).
    /// Fallible (#76u.13): a db error in the blocker reads surfaces as `io` instead of unwinding.
    pub fn blocked(&self) -> Result<Vec<BlockedItem>> {
        self.handle
            .try_with_state(|s| read::blocked(&self.cfg, &s.store, None))
    }

    /// The `next` recommendation: the actionable candidates (`ready ∪ in_progress` — claimed work
    /// is always included), in the finish-first tiered default order (C4 #916.3,
    /// `docs/specs/next-finish-first-tiers.md`). A malformed `now` is `validation`-rejected here
    /// (#9t7.9), as for [`prime`](Engine::prime).
    pub fn next(&self, now: &str) -> Result<Vec<ItemRow>> {
        validate::iso_date(now)?;
        self.handle
            .try_with_state(|s| read::next(&self.cfg, &s.store, now, None))
    }

    /// The `deferred` lane: open, unblocked, acyclic work whose `defer_until` is still in the
    /// future, in the shared default order (defer-date ascending, C3 #916.4). `now` is the defer
    /// boundary; a malformed `now` is `validation`-rejected here (#9t7.9).
    pub fn deferred(&self, now: &str) -> Result<Vec<ItemRow>> {
        validate::iso_date(now)?;
        self.handle
            .try_with_state(|s| read::deferred(&self.cfg, &s.store, now, None))
    }

    /// The `closed` lane: closed, non-archived items in the shared default order (close-date
    /// descending — recency, C3 #916.4). Fallible (#76u.13): a db error surfaces as `io`.
    pub fn closed(&self) -> Result<Vec<ItemRow>> {
        self.handle
            .try_with_state(|s| read::closed(&self.cfg, &s.store, None))
    }

    /// The `archived` lane: every archived item (any status) in the shared default order
    /// (archive-date descending — recency, C3 #916.4). Fallible (#76u.13): a db error surfaces as `io`.
    pub fn archived(&self) -> Result<Vec<ItemRow>> {
        self.handle
            .try_with_state(|s| read::archived(&self.cfg, &s.store, None))
    }

    /// Lane-ranked substring search (C6 #916.6): matches grouped by lane priority (next+ip →
    /// blocked → deferred → closed [→ archived]), each group in its natural in-lane order — the
    /// SAME implementation the CLI's `nxf search` runs. `now` is the defer boundary; a malformed
    /// `now` is `validation`-rejected here (#9t7.9). Archive is excluded unless `include_archived`
    /// (appends it) or `archived_only` (searches only it).
    pub fn search(
        &self,
        now: &str,
        query: &str,
        status: Option<&str>,
        item_type: Option<&str>,
        include_archived: bool,
        archived_only: bool,
    ) -> Result<Vec<ItemRow>> {
        validate::iso_date(now)?;
        self.handle.try_with_state(|s| {
            read::search(
                &self.cfg,
                &s.store,
                now,
                query,
                read::SearchArgs {
                    status,
                    item_type,
                    include_archived,
                    archived_only,
                    sort: None,
                },
            )
        })
    }

    /// Full detail of one item, or `not_found` if it is missing/deleted.
    pub fn show(&self, id: &str) -> Result<ShowRecord> {
        self.handle.try_with_state(|s| read::show(&s.store, id))
    }

    /// Every live item, optionally filtered by core status / type, in the shared default order
    /// (rank, C2 #916.2). Fallible (#76u.13): a db error in the underlying `list_items` read
    /// surfaces as `io` instead of unwinding a long-lived embed task — the symmetric flow half of
    /// memory's `memories` hardening (jo9).
    pub fn list(&self, status: Option<&str>, item_type: Option<&str>) -> Result<Vec<ItemRow>> {
        self.handle
            .try_with_state(|s| read::list(&self.cfg, &s.store, status, item_type, None))
    }

    /// The `prime` session-bootstrap record. A malformed `now` is `validation`-rejected here
    /// (#9t7.9). `bound` (is the workspace bound to a sync stream?) gates the sync hint; the
    /// embedding host passes its own sync state.
    pub fn prime(&self, now: &str, bound: bool) -> Result<PrimeReport> {
        validate::iso_date(now)?;
        self.handle
            .try_with_state(|s| read::prime(&self.cfg, &s.store, now, bound))
    }

    /// The plugin config the workspace selected — for a consumer that renders the typed records
    /// (vocabulary, ranking, presentation) itself.
    pub fn plugin_config(&self) -> PluginConfig {
        self.cfg.clone()
    }

    /// The resolved workspace (db path, replica identity).
    pub fn workspace(&self) -> Workspace {
        self.handle.workspace()
    }

    // ---- write methods (#9t7.5) --------------------------------------------
    //
    // Each takes `now` + `actor` as explicit parameters and routes through the shared
    // [`crate::write`] layer — the same ops, derivation, and validation as the CLI and MCP. The
    // store is mutated under the foundation handle's state lock; `now`/`actor` are the caller's.

    /// Create an item and return its materialized record. The id is minted under the workspace's
    /// replica prefix. Validation / dangling-reference failures write nothing.
    ///
    /// `NewItem::priority` is the plugin LABEL (`"P0".."P4"`), not the read-form ordinal reads emit
    /// — translate at your seam if you store the ordinal (see [`NewItem`](crate::write::NewItem)).
    pub fn create(
        &self,
        now: &str,
        actor: &str,
        ty: &str,
        title: &str,
        new: NewItem,
    ) -> Result<ItemRow> {
        self.handle.with_state_mut(|s| {
            write::create(
                &mut s.store,
                &self.cfg,
                &s.ws.replica.prefix,
                now,
                actor,
                ty,
                title,
                new,
            )
        })
    }

    /// Update item fields via `field=value` assignments (validated as a batch before any write).
    /// `priority=<label>` takes the plugin LABEL (`P0..P4`), not the read-form ordinal — translate
    /// at your seam if you store the ordinal (see [`NewItem`](crate::write::NewItem)).
    pub fn update(&self, now: &str, actor: &str, id: &str, sets: &[String]) -> Result<ItemRow> {
        self.handle
            .with_state_mut(|s| write::update(&mut s.store, &self.cfg, now, actor, id, sets))
    }

    /// Claim an item: mark it in progress, optionally assigning it.
    pub fn claim(
        &self,
        now: &str,
        actor: &str,
        id: &str,
        assignee: Option<&str>,
    ) -> Result<ItemRow> {
        self.handle
            .with_state_mut(|s| write::claim(&mut s.store, now, actor, id, assignee))
    }

    /// Close an item with a mandatory closing comment.
    pub fn close(&self, now: &str, actor: &str, id: &str, reason: Option<&str>) -> Result<ItemRow> {
        self.handle
            .with_state_mut(|s| write::close(&mut s.store, now, actor, id, reason))
    }

    /// Add a `from -> to` dependency (from depends on to); rejected at write time if it would
    /// create a cycle.
    pub fn dep_add(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::dep_add(&mut s.store, now, actor, from, to))
    }

    /// Remove the `from -> to` dependency.
    pub fn dep_remove(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::dep_remove(&mut s.store, now, actor, from, to))
    }

    /// Record a `from -> to` reference edge (free-text short-id citation; never blocks).
    pub fn mention_add(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::mention_add(&mut s.store, now, actor, from, to))
    }

    /// Remove the `from -> to` reference edge.
    pub fn mention_remove(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::mention_remove(&mut s.store, now, actor, from, to))
    }

    /// Record a `from -> to` contributes-to edge (n:m; never blocks).
    pub fn contributes_add(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::contributes_add(&mut s.store, now, actor, from, to))
    }

    /// Remove the `from -> to` contributes-to edge.
    pub fn contributes_remove(&self, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::contributes_remove(&mut s.store, now, actor, from, to))
    }

    /// Append an immutable worklog note; returns the minted note id.
    pub fn note_add(&self, now: &str, actor: &str, id: &str, text: &str) -> Result<String> {
        self.handle
            .with_state_mut(|s| write::note_add(&mut s.store, now, actor, id, text))
    }

    /// Archive a batch of roots, cascading DOWN each fully-closed subtree (C5 #916.5). Partial:
    /// atomic per root, independent between roots; the result reports each root's outcome plus the
    /// full set of ids actually archived (incl. cascaded).
    pub fn archive(&self, now: &str, actor: &str, ids: &[String]) -> Result<write::BatchResult> {
        self.handle
            .with_state_mut(|s| write::archive(&mut s.store, now, actor, ids))
    }

    /// Unarchive a batch of roots, cascading UP only (surface each item's ancestor chain, never
    /// its children/siblings — C5 #916.5). Same partial-batch result shape as [`archive`](Engine::archive).
    pub fn unarchive(&self, now: &str, actor: &str, ids: &[String]) -> Result<write::BatchResult> {
        self.handle
            .with_state_mut(|s| write::unarchive(&mut s.store, now, actor, ids))
    }

    // ---- who wrote an op, and whom this workspace believes (6j6v.pzkb) ------
    //
    // Every op this workspace writes is signed with its replica key; every op it receives is
    // verified and the verdict kept beside it; an agent action follows an op only when it is this
    // replica's own or verified by a key on the local trust list. These are the embedding twins of
    // `nxs sync key|trust|verify` — the same substrate calls, so an app and the CLI cannot disagree
    // about whom the workspace believes. Nothing here syncs: the trust list is this replica's.

    /// This replica's key id (`ed25519:…`) — what another replica trusts to believe this one.
    pub fn key_id(&self) -> String {
        self.handle.with_state(|s| s.store.key_id().to_string())
    }

    /// The trust list, this replica's own key first.
    pub fn trusted_keys(&self) -> Vec<nxs_foundation::trust::TrustedKey> {
        self.handle.with_state(|s| s.store.trusted_keys())
    }

    /// Trust `key_id` under `name`, as of `now`: the ops it signed and signs may carry agent
    /// actions here. `true` when it was not trusted before. A key id that names no Ed25519 key, or
    /// a name that is not one line of at most 64 characters, is a `validation` error.
    pub fn trust_key(&self, now: &str, key_id: &str, name: &str) -> Result<bool> {
        self.handle
            .with_state_mut(|s| s.store.trust_key(key_id, name, now))
    }

    /// Stop trusting `key_id` — effective at once. `true` when it was trusted. This replica's own
    /// key is a `validation` error.
    pub fn distrust_key(&self, key_id: &str) -> Result<bool> {
        self.handle.with_state_mut(|s| s.store.distrust_key(key_id))
    }

    /// The keys that signed ops in this workspace's log without being trusted, with the authors
    /// their ops claim — what an app shows next to "trust this key?".
    pub fn untrusted_signers(&self) -> Vec<nxs_foundation::trust::UntrustedSigner> {
        self.handle.with_state(|s| s.store.untrusted_signers())
    }

    /// Where the op `op_id` came from and whether an agent action may follow it. `not_found` for an
    /// op this workspace's log does not hold.
    pub fn op_provenance(&self, op_id: &str) -> Result<nxs_foundation::trust::OpProvenance> {
        self.handle.try_with_state(|s| {
            s.store.op_provenance(op_id).ok_or_else(|| {
                nxs_foundation::error::NxfError::not_found(format!("no op {op_id} in this log"))
            })
        })
    }

    /// **The check before an action**: may an agent action follow the op `op_id` here, now? Only
    /// for this replica's own ops and those verified by a trusted key — `false` for everything
    /// else, an op this log does not hold included.
    pub fn acts_on(&self, op_id: &str) -> bool {
        self.handle.with_state(|s| s.store.acts_on(op_id))
    }

    // ---- user labels (h89s) ------------------------------------------------
    //
    // The h89s OR-set was wired for the CLI/MCP/migration as store-level free functions but never
    // surfaced on this handle — the only surface an embedding consumer (app-foundations
    // engine-bridge → manufakt.io / nexflow.it) has. These mirror the dep/mention wrappers (writes
    // under `with_state_mut`, reads under `try_with_state`). Labels are an observed-remove VIEW, not
    // an `ItemRow`/`ShowRecord` field (the canonical-record byte-identity contract), so
    // labels-on-records ride the JSON `*_value` projections — the exact shape the CLI/MCP emit —
    // rather than mutating the typed records.

    /// Attach a user label to an item (idempotent OR-set add). `not_found` if the item is
    /// missing/deleted; an empty or separator-bearing label is a `validation` error.
    pub fn label_add(&self, now: &str, actor: &str, id: &str, label: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::label_add(&mut s.store, now, actor, id, label))
    }

    /// Detach a user label from an item (observed-remove; removing an absent label is a no-op).
    pub fn label_remove(&self, now: &str, actor: &str, id: &str, label: &str) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::label_remove(&mut s.store, now, actor, id, label))
    }

    /// An item's present user labels, sorted; `not_found` if the item is missing/deleted, `io` on a
    /// db fault (mirrors [`show`](Engine::show)).
    pub fn labels(&self, id: &str) -> Result<Vec<String>> {
        self.handle.try_with_state(|s| read::labels(&s.store, id))
    }

    /// Present labels for MANY items in ONE query (1w5v) — the bulk sibling of [`labels`](Engine::labels)
    /// for decorating a whole lane (the app-bridge `with_labels` seam) without N round-trips. Returns
    /// `id → sorted labels` for ids carrying ≥1 label; an id with none — or a stale/missing id — is
    /// absent from the map. Unlike [`labels`](Engine::labels) this does NOT `not_found`: a lane hands
    /// ids it already read, and a miss just means "no labels". `io` on a db fault.
    pub fn labels_bulk(&self, ids: &[&str]) -> Result<BTreeMap<String, Vec<String>>> {
        self.handle
            .try_with_state(|s| read::labels_bulk(&s.store, ids))
    }

    /// Filter an already-read item set to those carrying `label` — the `--label` filter behind
    /// `list`/`next`, reachable through the handle (kept off their signatures so the read surface
    /// stays additive).
    pub fn with_label(&self, items: Vec<ItemRow>, label: &str) -> Result<Vec<ItemRow>> {
        self.handle
            .try_with_state(|s| read::with_label(&s.store, items, label))
    }

    /// [`show`](Engine::show) as the canonical `--json` object WITH the item's user labels attached
    /// (a sparse `labels` key, present only when labelled) — the label-bearing detail read for a
    /// consumer that reads records through the handle. `not_found` if the item is missing/deleted.
    pub fn show_value(&self, id: &str) -> Result<Value> {
        self.handle
            .try_with_state(|s| read::show_value_with_custom(&self.cfg, &s.store, id))
    }

    /// [`list`](Engine::list) as the canonical `--json` array with each record's user labels attached
    /// (a sparse `labels` key per record). Same filter/order as [`list`](Engine::list).
    pub fn list_value(&self, status: Option<&str>, item_type: Option<&str>) -> Result<Value> {
        self.handle.try_with_state(|s| {
            let items = read::list(&self.cfg, &s.store, status, item_type, None)?;
            read::list_to_value_with_custom(&self.cfg, &s.store, &items)
        })
    }

    /// [`next`](Engine::next) as the canonical `--json` array with each record's user labels attached
    /// (a sparse `labels` key per record). A malformed `now` is `validation`-rejected, as for
    /// [`next`](Engine::next).
    pub fn next_value(&self, now: &str) -> Result<Value> {
        validate::iso_date(now)?;
        self.handle.try_with_state(|s| {
            let items = read::next(&self.cfg, &s.store, now, None)?;
            read::next_to_value_with_custom(&self.cfg, &s.store, &items)
        })
    }

    // ---- custom-carrying lane / bulk value reads (6j6v.1hh7) ----------------
    //
    // The `blocked`/`deferred`/`closed`/`archived` lanes carry the sparse plugin-`custom` join on
    // the CLI/MCP `--json` surface (the `read::*_with_custom` free fns), but the embedding Engine
    // carried `custom` only on `show`/`list`/`next`. These add the missing lane `_value` reads plus
    // a bulk custom read, so an app-foundations consumer (41j0.5rnj) mirrors the full `--json`
    // custom coverage in-process — notably WITHOUT an N+1 `show_value` on the non-live archived
    // lane. Additive (`facade changed`, not breaking): the typed `blocked`/`deferred`/`closed`/
    // `archived` reads and `PrimeReport` are untouched; `custom` rides the same sparse JSON key it
    // does everywhere else.

    /// The [`closed`](Engine::closed) lane as the canonical `--json` array with the sparse declared
    /// `custom` map attached per record — the embedding sibling of the CLI's `closed --json`.
    /// Byte-identical to the plain lane when the active plugin declares no `[fields]`.
    pub fn closed_value(&self) -> Result<Value> {
        self.handle.try_with_state(|s| {
            let items = read::closed(&self.cfg, &s.store, None)?;
            read::items_in_order_value_with_custom(&self.cfg, &s.store, &items)
        })
    }

    /// The [`deferred`](Engine::deferred) lane as the canonical `--json` array with the sparse
    /// declared `custom` map attached per record. A malformed `now` is `validation`-rejected here,
    /// as for [`deferred`](Engine::deferred).
    pub fn deferred_value(&self, now: &str) -> Result<Value> {
        validate::iso_date(now)?;
        self.handle.try_with_state(|s| {
            let items = read::deferred(&self.cfg, &s.store, now, None)?;
            read::items_in_order_value_with_custom(&self.cfg, &s.store, &items)
        })
    }

    /// The [`archived`](Engine::archived) lane as the canonical `--json` array with the sparse
    /// declared `custom` map attached per record.
    pub fn archived_value(&self) -> Result<Value> {
        self.handle.try_with_state(|s| {
            let items = read::archived(&self.cfg, &s.store, None)?;
            read::items_in_order_value_with_custom(&self.cfg, &s.store, &items)
        })
    }

    /// The [`blocked`](Engine::blocked) lane as the canonical `--json` array with each record's open
    /// `blockers` list AND the sparse declared `custom` map attached — the embedding sibling of the
    /// CLI's `blocked --json`.
    pub fn blocked_value(&self) -> Result<Value> {
        self.handle.try_with_state(|s| {
            let rows = read::blocked(&self.cfg, &s.store, None)?;
            read::blocked_to_value_with_custom(&self.cfg, &s.store, &rows)
        })
    }

    /// The sparse declared `custom` object per id: `id → {field: value, …}` for each id carrying ≥1
    /// declared custom value (an id with none — or a stale/missing id — is absent). The bulk sibling
    /// of the lane `_value` reads, for joining `custom` onto a row set the handle does not itself
    /// project with `custom` — notably [`prime`](Engine::prime)'s `next` list (whose
    /// [`PrimeReport`](crate::read::PrimeReport) deliberately carries no custom, so the CLI joins it
    /// the same way behind `prime --json`). Empty map when the plugin declares no `[fields]`.
    pub fn custom_fields_bulk(&self, ids: &[&str]) -> Result<BTreeMap<String, Value>> {
        self.handle
            .try_with_state(|s| read::declared_custom_by_id(&self.cfg, &s.store, ids))
    }

    // ---- thread links (6j6v.8dbe) ------------------------------------------
    //
    // The chat-thread↔item edge shipped as store-level free functions plus the `nxf thread` CLI,
    // but never on this handle — and an embedding consumer (app-foundations' engine-bridge →
    // manufakt.io / nexflow.it) has no other door: the `Handle<Store>` above is PRIVATE, so
    // `with_state_mut`/`try_with_state` are unreachable from outside and the `&Store`/`&mut Store`
    // the free functions want cannot be obtained at all (6j6v.72zn). Same cut as the h89s labels
    // one section up, same fix: thin wrappers, no new semantics.
    //
    // Note what is deliberately NOT here — a thread check. Only the ITEM endpoint is
    // existence-checked, in `write::thread_link_add`; the thread lives in the chat store's id
    // space, which flow neither owns nor can read, so adjudicating it here would make threads from
    // a foreign workspace unlinkable and take the edge's point away. The types (`ThreadLink`,
    // `LinkRelation`, `LinkWeight`) are re-exported at the crate root, so a consumer that links the
    // facade but not `nexus-flow-core` can name them in its own signature.

    /// Attach `thread` to `item` with a relation and a weight. Idempotent in effect and re-callable
    /// at any point in the thread's life: re-attaching the same pair with different attributes is
    /// how a link firms up (`passing` → `bearing`), not an error. `not_found` if the ITEM is
    /// missing/deleted; the thread is only normalized, never resolved (a blank or
    /// separator-bearing thread id is a `validation` error).
    pub fn thread_link_add(
        &self,
        now: &str,
        actor: &str,
        thread: &str,
        item: &str,
        relation: LinkRelation,
        weight: LinkWeight,
    ) -> Result<()> {
        self.handle.with_state_mut(|s| {
            write::thread_link_add(&mut s.store, now, actor, thread, item, relation, weight)
        })
    }

    /// Detach `thread` from `item` (observed-remove), whatever attributes the link currently
    /// carries. The item is validated exactly as in
    /// [`thread_link_add`](Engine::thread_link_add), so a typo is a loud `not_found` rather than a
    /// silently-successful no-op (mirrors [`dep_remove`](Engine::dep_remove)).
    pub fn thread_link_remove(
        &self,
        now: &str,
        actor: &str,
        thread: &str,
        item: &str,
    ) -> Result<()> {
        self.handle
            .with_state_mut(|s| write::thread_link_remove(&mut s.store, now, actor, thread, item))
    }

    /// The chat threads presently linked to `item`, sorted by thread id. `not_found` if the item is
    /// missing/deleted (explicit lookup, as [`show`](Engine::show)); `io` on a db fault.
    pub fn thread_links(&self, item: &str) -> Result<Vec<ThreadLink>> {
        self.handle
            .try_with_state(|s| read::thread_links(&s.store, item))
    }

    /// The board items `thread` is presently linked to, sorted by item id — the other direction, so
    /// a thread can enumerate what it is about. A thread flow has never seen simply has no links
    /// and reads back EMPTY, exactly as a linkless known one does; it is never rejected.
    pub fn thread_items(&self, thread: &str) -> Result<Vec<ThreadLink>> {
        self.handle
            .try_with_state(|s| read::thread_items(&s.store, thread))
    }
}
