//! flow's store: the [`nxs_foundation::store::Store`] op-log substrate plus flow's task vocabulary
//! folded over it. This newtype wraps the substrate (delegating the op-log methods — `apply`,
//! `export`, `refold`, …) and adds the task read/write helpers + the prefix-remap. flow's task
//! reducer is registered at open, so every op-log path folds `task` ops into the
//! `items`/`edge_adds`/`edge_removes`/`notes`/`note_tombstones` views (validated by the
//! differential oracle, tests/differential.rs).

use crate::model::{
    EdgeKind, ItemRow, LinkRelation, LinkWeight, MergeStrategy, Op, Status, ThreadLink, DOMAIN_TASK,
};
use crate::reducer::Reducer;
use crate::rewrite::rewrite_refs;
use crate::schema;
use crate::task_reducer::TaskReducer;
use nxs_foundation::store::Store as Substrate;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;

/// Separator inside an edge op's composite `target_id` (`from{SEP}to{SEP}kind`). `pub(crate)` so
/// the task reducer ([`crate::task_reducer`]) — which folds edge ops — shares the one definition.
pub(crate) const SEP: char = '\u{1f}';

/// flow's `view_watermarks.store_id` — the key under which its folded-through watermark is tracked
/// in the shared substrate (aye.36). Distinct from memory's so the two products fold the SAME log
/// into their own views and advance independently.
const FLOW_VIEWS: &str = "flow";

/// Item fields whose value is FREE TEXT that may cite a short-id (the dqj rewrite targets,
/// §8). `belongs_to` is excluded — its value is a *structural* id reference, remapped by
/// exact map lookup, not a free-text scan.
const FREETEXT_FIELDS: &[&str] = &[
    "title",
    "completion_criterion",
    "description",
    "design",
    "closing_comment",
];

/// An item's op-log-derived `(created_at, updated_at)` pair (6j6v.2kjy): the `wall_clock` of its
/// first / last op, each `None` when that op stamped none. Aliased so the singular + bulk reads
/// share one name (and to keep the bulk `BTreeMap` value under clippy's type-complexity bar).
pub type ItemTimestamps = (Option<String>, Option<String>);

/// How many item-read QUERIES a [`Store`] has run since it was opened (6j6v.g028) — the
/// instrument a test uses to assert the *shape* of a read path ("one bulk query, not N single
/// ones") instead of timing it. A wall-clock ratio can only infer that shape from how long the
/// machine took, so it reports the machine's load as readily as the code's; a query count is the
/// property itself, and reads the same on an idle laptop and a runner at load average 300.
///
/// Counts queries, not rows or calls: a `get_items` over 600 ids is TWO bulk queries (one per
/// ≤500-id chunk) and over zero ids is none. Take a snapshot before the read path, another after,
/// and compare with [`since`](Self::since).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemReadStats {
    /// Single-row [`Store::get_item`] queries.
    pub single: u64,
    /// Bulk [`Store::get_items`] queries — one per ≤500-id chunk.
    pub bulk: u64,
}

impl ItemReadStats {
    /// The queries run between an `earlier` snapshot and this one. Saturating, so a snapshot pair
    /// taken from two different stores reports zero rather than panicking.
    pub fn since(self, earlier: ItemReadStats) -> ItemReadStats {
        ItemReadStats {
            single: self.single.saturating_sub(earlier.single),
            bulk: self.bulk.saturating_sub(earlier.bulk),
        }
    }
}

/// flow's store over the shared substrate.
pub struct Store {
    inner: Substrate,
    /// Query-shape instrumentation ([`ItemReadStats`]). A `Cell` because the item reads take
    /// `&self`; `Store` is already `!Sync` (it holds a `rusqlite::Connection`), so this adds no
    /// constraint — and bumping a `Cell` is noise next to the SQLite query it counts.
    item_reads: std::cell::Cell<ItemReadStats>,
}

impl Store {
    pub fn open_in_memory(site: i64) -> Store {
        let mut inner = Substrate::open_in_memory(site);
        // flow's views live over the substrate's `ops` log; the task reducer folds into them.
        let upgraded = schema::apply_flow_views(inner.connection());
        inner.register_reducer(Box::new(TaskReducer));
        // Bind flow's folded-through watermark and refold if the shared log advanced out-of-band
        // (aye.36). A fresh in-memory log is empty and `upgraded` is always false (both views are
        // installed together), so this only binds the key here; the file-backed `open` is where a
        // multi-device pull or a pre-custom-fields upgrade actually triggers the refold.
        Self::materialize_views(&mut inner, upgraded);
        let mut store = Store {
            inner,
            item_reads: std::cell::Cell::default(),
        };
        store.migrate_belongs_to_to_parent_edges();
        store
    }

    /// File-backed store. Fallible: a missing-parent path, a corrupt/foreign file, or a workspace
    /// a newer incompatible nxs wrote returns the (handled) sqlite error from the substrate's skew
    /// gate — surfaced before flow touches the file, so a refused open writes nothing.
    pub fn open(path: &str, site: i64) -> rusqlite::Result<Store> {
        let mut inner = Substrate::open(path, site)?;
        let upgraded = schema::try_apply_flow_views(inner.connection())?;
        inner.register_reducer(Box::new(TaskReducer));
        Self::materialize_views(&mut inner, upgraded);
        let mut store = Store {
            inner,
            item_reads: std::cell::Cell::default(),
        };
        store.migrate_belongs_to_to_parent_edges();
        Ok(store)
    }

    /// Bind flow's views to the shared log after opening. Steady state is the O(1)
    /// [`refold_if_behind`](Substrate::refold_if_behind) — refold-when-behind (aye.36): if the shared
    /// log advanced past flow's folded-through watermark (a foundation-only sync pull landed `task`
    /// ops without folding them), rebuild the views once; otherwise reads stay on the pure view.
    /// When the open UPGRADED a pre-custom-fields workspace (the additive `custom_fields` minor,
    /// spec §7), [`force_refold`](Substrate::force_refold) instead — a `field set` op the pre-CF
    /// binary store-don't-folded sits BELOW the advanced watermark, so only an unconditional refold
    /// resurfaces it (mirrors `ChatStore::materialize_views`).
    fn materialize_views(inner: &mut Substrate, upgraded: bool) {
        if upgraded {
            inner.force_refold(FLOW_VIEWS);
        } else {
            inner.refold_if_behind(FLOW_VIEWS);
        }
    }

    // ---- substrate delegation -------------------------------------------------
    //
    // flow re-exposes the op-log surface its callers (sync, the read/write layers, the tests) use,
    // forwarding to the wrapped substrate. New domains/reducers register through here too.

    /// Register an additional [`Reducer`] (a future fact/message domain). flow's task reducer is
    /// already registered at open.
    pub fn register_reducer(&mut self, reducer: Box<dyn Reducer>) {
        self.inner.register_reducer(reducer);
    }

    /// Set the wall-clock timestamp stamped onto subsequently emitted LOCAL ops (display-only).
    pub fn set_wall_clock(&mut self, now: &str) {
        self.inner.set_wall_clock(now);
    }

    pub fn connection(&self) -> &Connection {
        self.inner.connection()
    }

    /// Snapshot of the item-read queries this store has run ([`ItemReadStats`], 6j6v.g028).
    pub fn item_read_stats(&self) -> ItemReadStats {
        self.item_reads.get()
    }

    /// Add the queries one read just ran to the running totals. `&self` (a `Cell`) because the
    /// reads themselves are `&self`.
    fn count_item_reads(&self, single: u64, bulk: u64) {
        let mut c = self.item_reads.get();
        c.single += single;
        c.bulk += bulk;
        self.item_reads.set(c);
    }

    /// The current Lamport clock (highest lamport emitted/observed).
    pub fn clock(&self) -> i64 {
        self.inner.clock()
    }

    pub fn op_count(&self) -> i64 {
        self.inner.op_count()
    }

    /// SQLite's `PRAGMA data_version` for this connection (the reactivity primitive; see
    /// `facade::watch`).
    pub fn data_version(&self) -> rusqlite::Result<i64> {
        self.inner.data_version()
    }

    /// Export the full op log (the sync payload).
    pub fn export(&self) -> Vec<Op> {
        self.inner.export()
    }

    /// Merge foreign ops; returns the ops stored-but-not-folded (forward-compat, §7).
    pub fn apply(&mut self, ops: &[Op]) -> Vec<Op> {
        self.inner.apply(ops)
    }

    /// Re-fold the durable log into the views (resurface, §7).
    pub fn refold(&mut self) {
        self.inner.refold();
    }

    // ---- who wrote an op, and whom this replica believes (6j6v.pzkb) -------------------------
    // Thin delegations to the substrate, where the key, the verdicts and the trust list live (see
    // `nxs_foundation::trust`). The sync engine and `nxs sync key|trust` drive this store.

    /// This replica's key id — what another replica trusts to believe it.
    pub fn key_id(&self) -> &str {
        self.inner.key_id()
    }

    /// The trust list, this replica's own key first.
    pub fn trusted_keys(&self) -> Vec<nxs_foundation::trust::TrustedKey> {
        self.inner.trusted_keys()
    }

    /// Put a key on the trust list; `true` when it was not on it before.
    pub fn trust_key(
        &mut self,
        key_id: &str,
        name: &str,
        now: &str,
    ) -> nxs_foundation::error::Result<bool> {
        self.inner.trust_key(key_id, name, now)
    }

    /// Take a key off the trust list; `true` when it was on it.
    pub fn distrust_key(&mut self, key_id: &str) -> nxs_foundation::error::Result<bool> {
        self.inner.distrust_key(key_id)
    }

    /// The keys that signed ops here without being trusted.
    pub fn untrusted_signers(&self) -> Vec<nxs_foundation::trust::UntrustedSigner> {
        self.inner.untrusted_signers()
    }

    /// Where one op came from, and whether an action may follow it.
    pub fn op_provenance(&self, op_id: &str) -> Option<nxs_foundation::trust::OpProvenance> {
        self.inner.op_provenance(op_id)
    }

    /// The check before an action: may an agent action follow this op here, now?
    pub fn acts_on(&self, op_id: &str) -> bool {
        self.inner.acts_on(op_id)
    }

    /// This store's image — the whole log and flow's folded views (6j6v.mxt2). See
    /// [`nxs_foundation::image`].
    pub fn image(&self) -> rusqlite::Result<nxs_foundation::image::Image> {
        self.inner.image()
    }

    /// Start this EMPTY store from an image (6j6v.mxt2): its log, and flow's views when this engine
    /// would have folded exactly those — otherwise folded again from that log.
    pub fn load_image(
        &mut self,
        image: &nxs_foundation::image::Image,
    ) -> Result<nxs_foundation::image::Loaded, nxs_foundation::image::ImageError> {
        self.inner.load_image(image)
    }

    fn conn(&self) -> &Connection {
        self.inner.connection()
    }

    /// Emit one LOCAL `task`-domain op through the substrate. flow is the task reducer: every op it
    /// originates rides the `task` domain (§4.1), so its emit always names [`DOMAIN_TASK`].
    fn emit_task(
        &mut self,
        target_kind: &str,
        target_id: &str,
        field: &str,
        op_type: &str,
        value: Option<String>,
        author: &str,
    ) -> String {
        self.inner.emit(
            DOMAIN_TASK,
            target_kind,
            target_id,
            field,
            op_type,
            value,
            author,
        )
    }

    // ---- flow write helpers ---------------------------------------------------

    /// Create an item with an opaque `ty` string (sp6.2). The core stores whatever type it is
    /// given — it has no fixed type set; the active plugin declares the valid set and the facade
    /// write seam enforces it before this is ever called.
    pub fn create_item(&mut self, id: &str, ty: &str, title: &str, author: &str) {
        // The id is an edge endpoint for every parent/dep/contributes edge it takes part in; a SEP
        // in it would later make those edges un-foldable (see `edge_target`). Reject it at the root.
        assert!(
            !id.contains(SEP),
            "item id must not contain the U+001F separator: {id:?}"
        );
        self.emit_task("item", id, "type", "set", Some(ty.to_string()), author);
        self.emit_task("item", id, "title", "set", Some(title.to_string()), author);
        self.emit_task(
            "item",
            id,
            "status",
            "set",
            Some(Status::Open.as_str().to_string()),
            author,
        );
    }

    /// Generic LWW set of a whitelisted item field — the `Lww` convenience over
    /// [`set_field_merge`](Self::set_field_merge). Byte-identical to the pre-w213 write path (it
    /// emits the bare `"set"` op_type), so every existing caller is unchanged.
    pub fn set_field(&mut self, id: &str, field: &str, value: Option<String>, author: &str) {
        self.set_field_merge(id, field, value, author, MergeStrategy::Lww);
    }

    /// Set a whitelisted item field under an explicit per-field merge strategy (w213). The strategy
    /// is stamped on the op (`op_type`) so the reducer dispatches on the op alone — the facade write
    /// seam resolves a field's strategy from the active plugin's `[merge]` table and passes it here.
    /// `CrdtText` is accepted and folds LWW today (reserved until the text-CRDT reducer, 4b39), so a
    /// notes plugin can declare a collaborative body now without data loss.
    pub fn set_field_merge(
        &mut self,
        id: &str,
        field: &str,
        value: Option<String>,
        author: &str,
        strategy: MergeStrategy,
    ) {
        assert!(
            schema::ITEM_LWW_FIELDS.contains(&field),
            "unknown field {field}"
        );
        self.emit_task(
            "item",
            id,
            field,
            strategy.item_set_op_type(),
            value,
            author,
        );
    }

    /// Set a plugin CUSTOM field under an explicit per-field merge strategy (plugin-custom-fields
    /// §4). Custom fields are the FIRST writer through the strategy-tagged path onto a NON-canonical
    /// field (`target_kind` `"field"`, the op's `field` column holding the custom NAME, not an
    /// `items` column) — so, unlike [`set_field_merge`](Self::set_field_merge), there is no
    /// `ITEM_LWW_FIELDS` assert: the core folds ANY well-formed `("field", set*)` op into the
    /// `custom_fields` view purely structurally (T1). A **clear** passes an empty (or `None`) value
    /// — the row remains with an empty `value` and reads as unset (§4.2). The facade write seam
    /// resolves the strategy from the active plugin's `[fields]` declaration and passes it here.
    pub fn set_custom_field_merge(
        &mut self,
        id: &str,
        field: &str,
        value: Option<String>,
        author: &str,
        strategy: MergeStrategy,
    ) {
        self.emit_task(
            "field",
            id,
            field,
            strategy.item_set_op_type(),
            value,
            author,
        );
    }

    pub fn delete_item(&mut self, id: &str, author: &str) {
        self.emit_task("item", id, "deleted", "set", Some("1".into()), author);
    }

    /// Returns soft-deleted (tombstoned) rows too — `deleted = Some("1")`; callers must
    /// check `.deleted` (the derivation layer already filters these in SQL).
    ///
    /// Fallible (#76u.14): a db error (e.g. `SQLITE_BUSY` past the busy-timeout, or a malformed
    /// row) is now surfaced (via [`OptionalExtension::optional`]) rather than `.ok()`-swallowed to
    /// `None` — so a long-lived read seam (MCP/embed) maps it to an `io` error instead of reporting
    /// a genuine db fault as a spurious `not_found`. `Ok(None)` still means the row is absent. The
    /// symmetric twin of memory's `get`, and of the `list_items`/`deps_of` Vec reads (#76u.13).
    pub fn get_item(&self, id: &str) -> rusqlite::Result<Option<ItemRow>> {
        self.count_item_reads(1, 0);
        self.conn()
            .query_row(
                // `belongs_to` is the `present_parent` projection of the `parent` OR-set edge
                // (sp6.3), not a stored column — every other field is read straight off `items`.
                "SELECT i.id, i.type, i.title, i.completion_criterion, i.description, i.design,
                        i.status, i.priority, i.due,
                        i.defer_until, i.assignee, pp.parent_id, i.closing_comment, i.deleted,
                        i.archived, i.closed_at
                 FROM items i LEFT JOIN present_parent pp ON pp.child_id = i.id
                 WHERE i.id=?1",
                [id],
                |r| {
                    Ok(ItemRow {
                        id: r.get(0)?,
                        item_type: r.get(1)?,
                        title: r.get(2)?,
                        completion_criterion: r.get(3)?,
                        description: r.get(4)?,
                        design: r.get(5)?,
                        status: r.get(6)?,
                        priority: r.get(7)?,
                        due: r.get(8)?,
                        defer_until: r.get(9)?,
                        assignee: r.get(10)?,
                        belongs_to: r.get(11)?,
                        closing_comment: r.get(12)?,
                        deleted: r.get(13)?,
                        archived: r.get(14)?,
                        closed_at: r.get(15)?,
                    })
                },
            )
            .optional()
    }

    /// Bulk sibling of [`get_item`] (xn8s): resolve MANY items in one query per 500-id chunk, so the
    /// `present_parent` view (the `belongs_to` projection) is materialized ONCE for the whole set
    /// instead of once per id. `read::next` resolves its ready set through this — N× `get_item` made
    /// the ready lane O(n·edges), its dominant cost; a single `IN (…)` join makes it O(edges + n).
    /// Returns the rows for the ids that exist, in unspecified order (a caller that needs a specific
    /// order re-orders); a missing id is simply absent, mirroring `get_item`'s `Ok(None)`. Chunked
    /// at 500 to stay under SQLite's bound-`?` cap, exactly like [`labels_of_bulk`](Self::labels_of_bulk).
    pub fn get_items(&self, ids: &[&str]) -> rusqlite::Result<Vec<ItemRow>> {
        const CHUNK: usize = 500;
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(CHUNK) {
            self.count_item_reads(0, 1);
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT i.id, i.type, i.title, i.completion_criterion, i.description, i.design,
                        i.status, i.priority, i.due,
                        i.defer_until, i.assignee, pp.parent_id, i.closing_comment, i.deleted,
                        i.archived, i.closed_at
                 FROM items i LEFT JOIN present_parent pp ON pp.child_id = i.id
                 WHERE i.id IN ({placeholders})"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok(ItemRow {
                    id: r.get(0)?,
                    item_type: r.get(1)?,
                    title: r.get(2)?,
                    completion_criterion: r.get(3)?,
                    description: r.get(4)?,
                    design: r.get(5)?,
                    status: r.get(6)?,
                    priority: r.get(7)?,
                    due: r.get(8)?,
                    defer_until: r.get(9)?,
                    assignee: r.get(10)?,
                    belongs_to: r.get(11)?,
                    closing_comment: r.get(12)?,
                    deleted: r.get(13)?,
                    archived: r.get(14)?,
                    closed_at: r.get(15)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// All materialized item rows, ordered by id (tombstoned rows included — callers
    /// filter on `.deleted`, mirroring `get_item`). Fallible (#76u.13): a db error (e.g.
    /// `SQLITE_BUSY` past the busy-timeout, or a malformed row) is surfaced, not `.unwrap()`-ed —
    /// so a long-lived read seam (MCP/embed) maps it to an `io` error instead of unwinding.
    pub fn list_items(&self) -> rusqlite::Result<Vec<ItemRow>> {
        let mut stmt = self.conn().prepare(
            // `belongs_to` is the `present_parent` projection (sp6.3); other fields off `items`.
            "SELECT i.id, i.type, i.title, i.completion_criterion, i.description, i.design,
                    i.status, i.priority, i.due,
                    i.defer_until, i.assignee, pp.parent_id, i.closing_comment, i.deleted,
                    i.archived, i.closed_at
             FROM items i LEFT JOIN present_parent pp ON pp.child_id = i.id
             ORDER BY i.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ItemRow {
                id: r.get(0)?,
                item_type: r.get(1)?,
                title: r.get(2)?,
                completion_criterion: r.get(3)?,
                description: r.get(4)?,
                design: r.get(5)?,
                status: r.get(6)?,
                priority: r.get(7)?,
                due: r.get(8)?,
                defer_until: r.get(9)?,
                assignee: r.get(10)?,
                belongs_to: r.get(11)?,
                closing_comment: r.get(12)?,
                deleted: r.get(13)?,
                archived: r.get(14)?,
                closed_at: r.get(15)?,
            })
        })?;
        rows.collect()
    }

    /// T-remap (§8): swap this replica's prefix `p_old → p_new` everywhere it appears, **only the
    /// prefix component** of each short-id — suffixes stay, so within-replica uniqueness is
    /// untouched and nothing needs re-minting. Run as Step 0 of the first sync after the registry
    /// reassigns a colliding prefix, BEFORE any merge.
    ///
    /// What is rewritten, all from the op log (the views are then refolded from it via the task
    /// reducer):
    /// - **item / note** op `target_id` (the item id) — exact map lookup.
    /// - **edge** composite `target_id` (`from␟to␟kind`) — each endpoint by exact lookup.
    /// - **`belongs_to`** op value — the structural parent id, exact lookup.
    /// - **free-text** op values (`FREETEXT_FIELDS` + note bodies) — via the shared dqj engine
    ///   [`rewrite_refs`], so a cited short-id stays resolvable.
    ///
    /// Precondition: `p_old` is THIS replica's prefix, so every `p_old`-prefixed id is local.
    /// Idempotent: once no `p_old` id remains it is a no-op, so a crash-retried remap converges.
    pub fn remap_prefix(&mut self, p_old: &str, p_new: &str) {
        let ops = self.export();

        // Build the exact id map: every local item id `p_old.suffix → p_new.suffix`.
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for op in &ops {
            if op.target_kind == "item" {
                if let Some((prefix, suffix)) = op.target_id.split_once('.') {
                    if prefix == p_old {
                        map.entry(op.target_id.clone())
                            .or_insert_with(|| format!("{p_new}.{suffix}"));
                    }
                }
            }
        }
        if map.is_empty() {
            return; // nothing of ours to remap (fresh, or already remapped) — idempotent
        }

        // Compute the rewritten op for each log entry (op_id is stable — it is the dedup key and
        // carries no prefix).
        let rewritten: Vec<Op> = ops
            .iter()
            .map(|op| {
                let mut new = op.clone();
                new.target_id = Self::remap_target(op, &map);
                new.value = Self::remap_value(op, &map);
                new
            })
            .collect();

        // Rewrite the op log, then rebuild the views from it via the task reducer (the views are a
        // pure fold of the log): wipe flow's views and re-fold the rewritten ops — all in one
        // transaction, on the substrate's connection.
        let reducer = TaskReducer;
        let conn = self.inner.connection();
        conn.execute_batch("BEGIN").unwrap();
        let mut changed = Vec::new();
        for (orig, new) in ops.iter().zip(&rewritten) {
            if orig.target_id != new.target_id || orig.value != new.value {
                conn.execute(
                    "UPDATE ops SET target_id=?1, value=?2 WHERE op_id=?3",
                    params![new.target_id, new.value, new.op_id],
                )
                .unwrap();
                changed.push(new.clone());
            }
        }
        // A rewritten op is no longer the bytes it was signed over (6j6v.pzkb): this replica's own
        // are signed again — the precondition above says none of them has left it yet — and every
        // other one is judged again, so no verdict describes bytes the log no longer holds.
        self.inner.reseal(&changed);
        reducer.clear_views(conn);
        for op in &rewritten {
            // The bound every fold dispatch applies (6j6v.m19v): this rebuild is one too.
            if op.lamport_in_bound() && reducer.is_foldable(op) {
                reducer.fold(conn, op);
            }
        }
        conn.execute_batch("COMMIT").unwrap();
    }

    /// Remap an op's `target_id` under the id map: edge composites move each endpoint; a label
    /// composite moves its item-id endpoint (the label text is not an id); item/note targets move
    /// by exact lookup; anything unmapped is unchanged.
    fn remap_target(op: &Op, map: &BTreeMap<String, String>) -> String {
        if op.target_kind == "edge" {
            let parts: Vec<&str> = op.target_id.split(SEP).collect();
            if let [from, to, kind] = parts.as_slice() {
                let from = map.get(*from).map_or(*from, String::as_str);
                let to = map.get(*to).map_or(*to, String::as_str);
                return format!("{from}{SEP}{to}{SEP}{kind}");
            }
            return op.target_id.clone(); // malformed → leave as-is (matches parse_edge)
        }
        if op.target_kind == "label" {
            let parts: Vec<&str> = op.target_id.split(SEP).collect();
            if let [item_id, label] = parts.as_slice() {
                let item_id = map.get(*item_id).map_or(*item_id, String::as_str);
                return format!("{item_id}{SEP}{label}");
            }
            return op.target_id.clone(); // malformed → leave as-is (matches parse_label)
        }
        // A thread link moves ONLY its item endpoint (nxf 6j6v.8dbe). The thread id belongs to the
        // chat id space, which this remap knows nothing about and must not touch — that separation
        // is the whole reason the link is its own OR-set rather than an edge with a foreign
        // endpoint. The two attributes are not ids either, exactly as a label's text is not.
        if op.target_kind == "thread_link" {
            let parts: Vec<&str> = op.target_id.split(SEP).collect();
            if let [thread_id, item_id, relation, weight] = parts.as_slice() {
                let item_id = map.get(*item_id).map_or(*item_id, String::as_str);
                return format!("{thread_id}{SEP}{item_id}{SEP}{relation}{SEP}{weight}");
            }
            return op.target_id.clone(); // malformed → leave as-is (matches parse_thread_link)
        }
        map.get(&op.target_id)
            .cloned()
            .unwrap_or_else(|| op.target_id.clone())
    }

    /// Remap an op's `value`: `belongs_to` is a structural id (exact lookup); item free-text and
    /// note bodies go through the dqj rewrite engine; everything else is unchanged.
    fn remap_value(op: &Op, map: &BTreeMap<String, String>) -> Option<String> {
        let value = op.value.as_ref()?;
        let rewritten = match (op.target_kind.as_str(), op.field.as_str()) {
            ("item", "belongs_to") => map.get(value).cloned().unwrap_or_else(|| value.clone()),
            ("item", f) if FREETEXT_FIELDS.contains(&f) => rewrite_refs(value, map),
            ("note", "body") => rewrite_refs(value, map),
            _ => value.clone(),
        };
        Some(rewritten)
    }

    fn edge_target(from: &str, to: &str, kind: EdgeKind) -> String {
        // An endpoint id containing SEP would make the composite `target_id` split into >3 parts,
        // so the reducer's `from{SEP}to{SEP}kind` parse fails and the edge silently never folds
        // (Integrity review #2). Locally-minted ids never contain SEP; a SEP id reaching here is a
        // programming error (the facade existence-checks every user-supplied id before this), so we
        // fail loudly — matching `set_field`'s unknown-field assert — instead of dropping the edge.
        // (Merge-delivered malformed ops are NOT routed here; the reducer skips them, which is the
        // right posture — a hostile peer must not be able to panic the replica.)
        assert!(
            !from.contains(SEP) && !to.contains(SEP),
            "edge endpoint id must not contain the U+001F separator: {from:?} -> {to:?}"
        );
        format!("{from}{SEP}{to}{SEP}{}", kind.as_str())
    }

    /// Composite `target_id` for a label op (`item_id{SEP}label`). A SEP in either component would
    /// make the reducer's two-part parse fail and the label silently never fold — so reject it
    /// loudly here, exactly as [`edge_target`](Self::edge_target) does for edge endpoints. The
    /// facade validates user-supplied labels before this; a SEP reaching here is a programming bug.
    fn label_target(item_id: &str, label: &str) -> String {
        assert!(
            !item_id.contains(SEP) && !label.contains(SEP),
            "label item id and text must not contain the U+001F separator: {item_id:?} / {label:?}"
        );
        format!("{item_id}{SEP}{label}")
    }

    /// sp6.3 migration: convert legacy single-parent `belongs_to` register ops into `parent`
    /// OR-set edges, in place on the op-log. Run on open (the substrate migrates+stamps the schema
    /// version before flow sees it, so this is gated by **data shape**, not version). For each child
    /// that has a `belongs_to/set` op and **no present `parent` edge yet**, the LWW-winning op
    /// (max `(lamport, site)`) — if it set a non-null parent — is rewritten in place to a `parent`
    /// edge add, keeping its `op_id` so independently-migrating replicas produce the identical op
    /// and dedup on merge. Non-winning `belongs_to` ops are left inert (no longer foldable). The
    /// "no present parent edge" gate makes re-open a no-op (idempotent) and skips the refold when
    /// there is nothing to convert; a child whose winner was a clear is re-scanned but rewrites
    /// nothing. Lossless: every op is preserved; only the winner's shape changes.
    pub(crate) fn migrate_belongs_to_to_parent_edges(&mut self) {
        // (op_id, child, parent) for each child's winning, non-null belongs_to op with no edge yet.
        let rows: Vec<(String, String, String)> = {
            let conn = self.inner.connection();
            let mut stmt = conn
                .prepare(
                    // An op past the Lamport bound takes part in no fold (6j6v.m19v), so it is no
                    // register's winner here either — neither as the winner nor as what beats it.
                    "SELECT o1.op_id, o1.target_id, o1.value
                     FROM ops o1
                     WHERE o1.target_kind='item' AND o1.field='belongs_to' AND o1.op_type='set'
                       AND o1.value IS NOT NULL AND o1.lamport <= ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM ops o2
                           WHERE o2.target_kind='item' AND o2.field='belongs_to'
                             AND o2.op_type='set' AND o2.target_id=o1.target_id
                             AND o2.lamport <= ?1
                             AND (o2.lamport, o2.site) > (o1.lamport, o1.site))
                       AND NOT EXISTS (
                           SELECT 1 FROM present_edges pe
                           WHERE pe.from_id=o1.target_id AND pe.kind='parent')",
                )
                .unwrap();
            stmt.query_map([nxs_foundation::model::MAX_LAMPORT], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
        };
        if rows.is_empty() {
            return; // fresh DB, or already migrated — no rewrite, no refold
        }
        {
            let conn = self.inner.connection();
            for (op_id, child, parent) in &rows {
                let target = Self::edge_target(child, parent, EdgeKind::Parent);
                conn.execute(
                    "UPDATE ops SET target_kind='edge', target_id=?1, field='present',
                                    op_type='add', value=NULL
                     WHERE op_id=?2",
                    params![target, op_id],
                )
                .unwrap();
            }
        }
        // The rewritten ops no longer say what was signed (6j6v.pzkb) — only ever legacy ops in
        // practice, since `belongs_to` writes stopped at v4, but a verdict must never describe bytes
        // the log no longer holds.
        let rewritten: std::collections::HashSet<&str> =
            rows.iter().map(|(op_id, _, _)| op_id.as_str()).collect();
        let rewritten: Vec<Op> = self
            .inner
            .export()
            .into_iter()
            .filter(|op| rewritten.contains(op.op_id.as_str()))
            .collect();
        self.inner.reseal(&rewritten);
        // Rebuild the views from the rewritten log (the winning belongs_to ops are now parent
        // edges; the present_parent view projects them).
        self.inner.refold();
    }

    pub fn add_edge(&mut self, from: &str, to: &str, kind: EdgeKind, author: &str) {
        let target = Self::edge_target(from, to, kind);
        self.emit_task("edge", &target, "present", "add", None, author);
    }

    /// Remove = tombstone every currently-live add-tag for this element (observed-remove).
    pub fn remove_edge(&mut self, from: &str, to: &str, kind: EdgeKind, author: &str) {
        let tags: Vec<String> = {
            let mut stmt = self
                .conn()
                .prepare(
                    "SELECT tag FROM edge_adds a
                     WHERE from_id=?1 AND to_id=?2 AND kind=?3
                       AND NOT EXISTS (SELECT 1 FROM edge_removes r WHERE r.tag=a.tag)",
                )
                .unwrap();
            stmt.query_map(params![from, to, kind.as_str()], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        }; // stmt dropped here, releasing the borrow on the connection
        if tags.is_empty() {
            return;
        }
        let target = Self::edge_target(from, to, kind);
        let joined = tags.join(&SEP.to_string());
        self.emit_task("edge", &target, "present", "remove", Some(joined), author);
    }

    // ---- thread links (nxf 6j6v.8dbe): a third OR-set, over (thread, item) --------

    /// Composite `target_id` for a thread-link op (`thread{SEP}item{SEP}relation{SEP}weight`). A SEP
    /// in either id would make the reducer's four-part parse fail and the link silently never fold —
    /// so reject it loudly here, exactly as [`edge_target`](Self::edge_target) and
    /// [`label_target`](Self::label_target) do. The two attributes are `&'static str` from the
    /// enums' own `as_str`, so they cannot contain one.
    fn thread_link_target(
        thread_id: &str,
        item_id: &str,
        relation: LinkRelation,
        weight: LinkWeight,
    ) -> String {
        assert!(
            !thread_id.contains(SEP) && !item_id.contains(SEP),
            "thread link ids must not contain the U+001F separator: {thread_id:?} / {item_id:?}"
        );
        format!(
            "{thread_id}{SEP}{item_id}{SEP}{}{SEP}{}",
            relation.as_str(),
            weight.as_str()
        )
    }

    /// Attach `thread_id` to `item_id` with a relation and a weight (OR-set add; the add-tag is the
    /// op_id).
    ///
    /// Re-attaching the SAME pair with different attributes is not an error and needs no prior
    /// detach: `present_thread_links` projects the causally-latest add per pair, so this is how a
    /// link firms up or softens. Attaching may happen at any point in a thread's life — nothing here
    /// forces an early binding.
    pub fn add_thread_link(
        &mut self,
        thread_id: &str,
        item_id: &str,
        relation: LinkRelation,
        weight: LinkWeight,
        author: &str,
    ) {
        let target = Self::thread_link_target(thread_id, item_id, relation, weight);
        self.emit_task("thread_link", &target, "present", "add", None, author);
    }

    /// Detach `thread_id` from `item_id` = tombstone every currently-live add-tag for the pair
    /// (observed-remove), mirroring [`remove_edge`](Self::remove_edge).
    ///
    /// Deliberately keyed on the PAIR alone, not on the attributes: detaching means "this thread is
    /// no longer linked to this item", and a caller should not have to know which relation/weight the
    /// link currently carries to be able to take it back. A no-op when the pair is already unlinked.
    ///
    /// The remove op's own `target_id` carries a canonical attribute suffix so it reads as the pair's
    /// op in the log; the fold ignores it entirely and works from the observed tags in `value`,
    /// exactly as the edge and label removes do.
    pub fn remove_thread_link(&mut self, thread_id: &str, item_id: &str, author: &str) {
        let tags: Vec<String> = {
            let mut stmt = self
                .conn()
                .prepare(
                    "SELECT tag FROM thread_link_adds a
                     WHERE thread_id=?1 AND item_id=?2
                       AND NOT EXISTS (SELECT 1 FROM thread_link_removes r WHERE r.tag=a.tag)",
                )
                .unwrap();
            stmt.query_map(params![thread_id, item_id], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        }; // stmt dropped here, releasing the borrow on the connection
        if tags.is_empty() {
            return;
        }
        let target = Self::thread_link_target(
            thread_id,
            item_id,
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
        );
        let joined = tags.join(&SEP.to_string());
        self.emit_task(
            "thread_link",
            &target,
            "present",
            "remove",
            Some(joined),
            author,
        );
    }

    /// The threads presently linked to `item_id`, sorted by thread id (deterministic). Fallible for
    /// the read-compute facade (a db fault maps to `io`), mirroring [`labels_of`](Self::labels_of).
    pub fn thread_links_of_item(&self, item_id: &str) -> rusqlite::Result<Vec<ThreadLink>> {
        let mut stmt = self.conn().prepare(
            "SELECT thread_id, relation, weight FROM present_thread_links
             WHERE item_id=?1 ORDER BY thread_id",
        )?;
        let rows = stmt.query_map([item_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        Self::collect_links(rows, |thread_id, relation, weight| ThreadLink {
            thread_id,
            item_id: item_id.to_string(),
            relation,
            weight,
        })
    }

    /// The board items `thread_id` is presently linked to, sorted by item id (deterministic) — the
    /// other direction of the same n:m set, so a thread can enumerate what it is about.
    pub fn items_of_thread(&self, thread_id: &str) -> rusqlite::Result<Vec<ThreadLink>> {
        let mut stmt = self.conn().prepare(
            "SELECT item_id, relation, weight FROM present_thread_links
             WHERE thread_id=?1 ORDER BY item_id",
        )?;
        let rows = stmt.query_map([thread_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        Self::collect_links(rows, |item_id, relation, weight| ThreadLink {
            thread_id: thread_id.to_string(),
            item_id,
            relation,
            weight,
        })
    }

    /// How many `bearing` threads each of `item_ids` carries, in ONE query (nxf 6j6v.8dbe) — the
    /// bulk sibling of [`thread_links_of_item`](Self::thread_links_of_item) for decorating a whole
    /// lane without N round-trips, exactly as [`labels_of_bulk`](Self::labels_of_bulk) is for labels.
    /// An id with no bearing links is simply ABSENT from the map (a miss means zero), so the caller's
    /// sparse-key rendering needs no filtering of its own.
    ///
    /// Counts `bearing` alone: see `facade::read::bearing_thread_count` for why the weight gate
    /// belongs on this path and not in the caller.
    pub fn bearing_thread_counts(
        &self,
        item_ids: &[&str],
    ) -> rusqlite::Result<BTreeMap<String, usize>> {
        // SQLite caps the bound-`?` count per statement; chunk defensively, mirroring
        // `labels_of_bulk`'s identical guard, so a lane far larger than any real board still works.
        let mut out = BTreeMap::new();
        for chunk in item_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT item_id, COUNT(*) FROM present_thread_links
                 WHERE item_id IN ({placeholders}) AND weight=?
                 GROUP BY item_id"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let mut params: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            let bearing = LinkWeight::Bearing.as_str();
            params.push(&bearing);
            let rows = stmt.query_map(params.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
            })?;
            for row in rows {
                let (id, n) = row?;
                out.insert(id, n);
            }
        }
        Ok(out)
    }

    /// Shared row→[`ThreadLink`] collection for the two directions above. An unparseable attribute
    /// cannot occur (the reducer's `parse_thread_link` only folds recognized tokens), so a row that
    /// somehow carries one is SKIPPED rather than defaulted: inventing a relation would be worse
    /// than reporting one fewer link, and defaulting is how a corrupt value becomes a fact.
    fn collect_links<F>(
        rows: rusqlite::MappedRows<
            '_,
            impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String)>,
        >,
        build: F,
    ) -> rusqlite::Result<Vec<ThreadLink>>
    where
        F: Fn(String, LinkRelation, LinkWeight) -> ThreadLink,
    {
        let mut out = Vec::new();
        for row in rows {
            let (id, relation, weight) = row?;
            let (Some(relation), Some(weight)) =
                (LinkRelation::parse(&relation), LinkWeight::parse(&weight))
            else {
                continue;
            };
            out.push(build(id, relation, weight));
        }
        Ok(out)
    }

    // ---- labels (h89s.1): a second observed-remove OR-set over (item, label) ------

    /// Attach a user label to an item (OR-set add; the add-tag is the op_id). Idempotent in effect
    /// — re-adding a present label leaves one present label, exactly as a repeated edge add.
    pub fn add_label(&mut self, item_id: &str, label: &str, author: &str) {
        let target = Self::label_target(item_id, label);
        self.emit_task("label", &target, "present", "add", None, author);
    }

    /// Detach a label from an item = tombstone every currently-live add-tag for `(item, label)`
    /// (observed-remove), mirroring [`remove_edge`](Self::remove_edge). A no-op when the label is
    /// already absent.
    pub fn remove_label(&mut self, item_id: &str, label: &str, author: &str) {
        let tags: Vec<String> = {
            let mut stmt = self
                .conn()
                .prepare(
                    "SELECT tag FROM label_adds a
                     WHERE item_id=?1 AND label=?2
                       AND NOT EXISTS (SELECT 1 FROM label_removes r WHERE r.tag=a.tag)",
                )
                .unwrap();
            stmt.query_map(params![item_id, label], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        }; // stmt dropped here, releasing the borrow on the connection
        if tags.is_empty() {
            return;
        }
        let target = Self::label_target(item_id, label);
        let joined = tags.join(&SEP.to_string());
        self.emit_task("label", &target, "present", "remove", Some(joined), author);
    }

    /// Present labels of an item (sorted, deterministic). Fallible (#76u.13): reached by the
    /// read-compute facade, so a db error is surfaced for the seam to map to `io`, mirroring
    /// [`deps_of`](Self::deps_of).
    pub fn labels_of(&self, item_id: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT label FROM present_labels WHERE item_id=?1 ORDER BY label")?;
        let rows = stmt.query_map([item_id], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Present labels of many items in ONE query (1w5v) — the bulk sibling of [`labels_of`] for
    /// decorating a whole lane without N round-trips (the app-bridge `with_labels` seam). Returns
    /// `item_id → sorted labels` for every id that has at least one present label; an id with no
    /// present labels is simply absent from the map (a miss means "no labels", exactly as
    /// `labels_of` returning empty). Reads the same `present_labels` view, so an observed-remove
    /// drops out here too and each id's labels are byte-identical to `labels_of` — the parity the
    /// lane projection relies on. Deterministic: `ORDER BY item_id, label` groups the rows so each
    /// id's labels arrive contiguous and sorted. Fallible for the read-compute facade (a db fault
    /// maps to `io`), mirroring [`labels_of`].
    pub fn labels_of_bulk(
        &self,
        item_ids: &[&str],
    ) -> rusqlite::Result<BTreeMap<String, Vec<String>>> {
        // SQLite caps the bound-`?` count per statement; chunk defensively so an id set far larger
        // than a lane still resolves in a handful of queries instead of failing or degrading to N.
        // A lane (~10² ids) is a single chunk = one round-trip. `item_id IN (?,…)` rides the
        // `label_adds_item` index (SEARCH, not SCAN — PR #186).
        const CHUNK: usize = 500;
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for chunk in item_ids.chunks(CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT item_id, label FROM present_labels WHERE item_id IN ({placeholders}) \
                 ORDER BY item_id, label"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, label) = row?;
                out.entry(id).or_default().push(label);
            }
        }
        Ok(out)
    }

    // ---- plugin custom fields (6j6v.ekf5, T4): the read side of the folded `custom_fields` view --

    /// The set custom-field values of ONE item (plugin-custom-fields §2.3), as `field → value`
    /// sorted by field (deterministic). Reads the folded `custom_fields` view. An **empty** value is
    /// a clear (§4.2) and reads as UNSET, so `value <> ''` excludes it — a caller sees only fields
    /// that currently hold a value. The core is meaning-free: this returns EVERY set field the item
    /// carries, foreign/undeclared fields included; narrowing to the ACTIVE plugin's declared set is
    /// the facade read layer's job (§7). Fallible for the read-compute facade (a db fault maps to
    /// `io`), mirroring [`labels_of`](Self::labels_of). The single-item sibling of
    /// [`custom_fields_of_bulk`](Self::custom_fields_of_bulk), used by `show`.
    pub fn custom_fields_of(&self, item_id: &str) -> rusqlite::Result<BTreeMap<String, String>> {
        let mut stmt = self.conn().prepare(
            "SELECT field, value FROM custom_fields WHERE item_id=?1 AND value <> '' ORDER BY field",
        )?;
        let rows = stmt.query_map([item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// Set custom-field values of MANY items in ONE query per 500-id chunk (plugin-custom-fields §6)
    /// — the bulk sibling of [`custom_fields_of`](Self::custom_fields_of) for decorating a whole lane
    /// (`next`/`list`) with **no N+1**, exactly as [`labels_of_bulk`](Self::labels_of_bulk) does for
    /// labels. Returns `item_id → {field → value}` for every id that has ≥1 NON-empty custom value;
    /// an id with none is absent from the map (a miss means "no custom values"). `value <> ''`
    /// excludes a cleared field (§4.2), so each id's inner map is byte-identical to `custom_fields_of`
    /// — the parity the lane projection relies on. Deterministic: `ORDER BY item_id, field` groups
    /// each id's rows contiguous and field-sorted (the `BTreeMap` also normalises order). Chunked at
    /// 500 to stay under SQLite's bound-`?` cap; a lane (~10² ids) is a single chunk = one round-trip.
    /// The `custom_fields` PK indexes `item_id` leftmost, so `item_id IN (…)` is a SEARCH, not a SCAN.
    pub fn custom_fields_of_bulk(
        &self,
        item_ids: &[&str],
    ) -> rusqlite::Result<BTreeMap<String, BTreeMap<String, String>>> {
        const CHUNK: usize = 500;
        let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for chunk in item_ids.chunks(CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT item_id, field, value FROM custom_fields \
                 WHERE item_id IN ({placeholders}) AND value <> '' \
                 ORDER BY item_id, field"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, field, value) = row?;
                out.entry(id).or_default().insert(field, value);
            }
        }
        Ok(out)
    }

    /// The op-log-derived `(created_at, updated_at)` for an item (6j6v.2kjy): created_at is the
    /// `wall_clock` stamped on the item's FIRST op (its create), updated_at that of its LAST op —
    /// any op whose `target_id` is the item (its own cells, its custom fields, its worklog notes;
    /// NOT edges/labels, whose op target is a composite id). An end with no stamped `wall_clock` (a
    /// legacy pre-wall_clock write, or a direct un-stamped seed) is `None`, so a fully un-stamped
    /// item — or a missing one — is `(None, None)`. Fallible: a db fault surfaces as `io`.
    pub fn item_timestamps_of(&self, item_id: &str) -> rusqlite::Result<ItemTimestamps> {
        Ok(self
            .item_timestamps_of_bulk(&[item_id])?
            .remove(item_id)
            .unwrap_or((None, None)))
    }

    /// `(created_at, updated_at)` for MANY items in ONE query per 500-id chunk (6j6v.2kjy) — the
    /// bulk sibling of [`item_timestamps_of`](Self::item_timestamps_of) for decorating a whole lane
    /// with **no N+1**, like [`custom_fields_of_bulk`](Self::custom_fields_of_bulk). Returns
    /// `item_id → (created, updated)` for every id with ≥1 op carrying a non-empty `wall_clock`; an
    /// id with none — or a missing id — is absent from the map (a miss means "no timestamps"), the
    /// same sparse contract the label/custom bulk reads follow. `FIRST_VALUE`/`LAST_VALUE` over the
    /// canonical `(lamport, site)` op order pick the create/last-change instants; the window frame is
    /// spelled out so `LAST_VALUE` sees the whole partition, not just up to the current row. Chunked
    /// at 500 to stay under SQLite's bound-`?` cap; the `ops_target` index makes `target_id IN (…)`
    /// a SEARCH, not a SCAN.
    pub fn item_timestamps_of_bulk(
        &self,
        item_ids: &[&str],
    ) -> rusqlite::Result<BTreeMap<String, ItemTimestamps>> {
        const CHUNK: usize = 500;
        let mut out: BTreeMap<String, ItemTimestamps> = BTreeMap::new();
        for chunk in item_ids.chunks(CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            // An op past the Lamport bound takes part in no fold (6j6v.m19v), so it is neither an
            // item's creation nor its last change — it would otherwise always be the latter.
            let max = nxs_foundation::model::MAX_LAMPORT;
            let sql = format!(
                "SELECT DISTINCT target_id, \
                    FIRST_VALUE(wall_clock) OVER w AS created, \
                    LAST_VALUE(wall_clock) OVER w AS updated \
                 FROM ops \
                 WHERE target_id IN ({placeholders}) AND lamport <= {max} \
                 WINDOW w AS ( \
                     PARTITION BY target_id ORDER BY lamport, site \
                     ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING \
                 )"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?;
            for row in rows {
                let (id, created, updated) = row?;
                // An empty wall_clock ('' — the unstamped default) reads as "no timestamp".
                let norm = |v: Option<String>| v.filter(|s| !s.is_empty());
                let (created, updated) = (norm(created), norm(updated));
                if created.is_some() || updated.is_some() {
                    out.insert(id, (created, updated));
                }
            }
        }
        Ok(out)
    }

    /// Present `dep` targets of `from` (used by derivation; sorted, deterministic). Fallible
    /// (#76u.13): reached by the read-compute facade (`show`/`blocked`), so a db error is surfaced
    /// for the seam to map to `io`, not `.unwrap()`-ed.
    pub fn deps_of(&self, from: &str) -> rusqlite::Result<Vec<String>> {
        self.targets_of_result(from, EdgeKind::Dep)
    }

    /// Present `mentions` targets of `from` — the short-ids `from` cites in its free text. Infallible
    /// (the one-shot path); the long-lived read seam takes [`mentions_of_result`](Self::mentions_of_result).
    pub fn mentions_of(&self, from: &str) -> Vec<String> {
        self.targets_of(from, EdgeKind::Mentions)
    }

    /// Present `mentions` targets of `from`, fallible — the read-seam variant (#76u.14/07a.7): reached
    /// by the read-compute facade (`mention list` → `flow_mention_list`), so a db error is surfaced for
    /// the seam to map to `io` instead of `.expect()`-panicking on the long-lived server path. Mirrors
    /// [`contributes_to_of`](Self::contributes_to_of).
    pub fn mentions_of_result(&self, from: &str) -> rusqlite::Result<Vec<String>> {
        self.targets_of_result(from, EdgeKind::Mentions)
    }

    /// Present `contributes-to` targets of `from` — the items `from` contributes to (n:m). Fallible
    /// (#76u.13): reached by the read-compute facade (`show`), so a db error is surfaced for the
    /// seam to map to `io`.
    pub fn contributes_to_of(&self, from: &str) -> rusqlite::Result<Vec<String>> {
        self.targets_of_result(from, EdgeKind::ContributesTo)
    }

    /// Present `parent` targets of `child` — the child's parent(s) in the OR-set (sp6.3). The
    /// substrate is n:m-capable; under the ②a single-parent write shim this normally yields one.
    /// The single *current* parent (the `belongs_to` projection) is the `present_parent` view's
    /// winner; this returns the full present set (used by the differential oracle / later n:m work).
    ///
    /// Infallible OR-set projection, retained for one-shot LOCAL paths (the `nxf import` edge walk in
    /// `beads_import`) and the differential oracle / tests. Every LONG-LIVED read/write seam now takes
    /// the fallible [`parents_of_result`](Self::parents_of_result) instead — the effective-lane read
    /// joins and claim-up (07a.7) as well as the write-validation walks (cycle/cardinality/depth on
    /// `create`/`update --parent`, and retype revalidation) — so a db error on that server path maps
    /// to `io` rather than `.expect()`-unwinding, mirroring the dep-edge `deps_of` (#76u.14/07a.7).
    pub fn parents_of(&self, child: &str) -> Vec<String> {
        self.targets_of(child, EdgeKind::Parent)
    }

    /// The fallible parent read (#76u.14): the write-validation walk (`reaches_via_parent`/
    /// `check_parent`/`clear_parent`) reads the parent edge on the long-lived server, so it surfaces
    /// a db error as `Err` (mapped to `io` at the seam) instead of `.expect()`-panicking — the
    /// symmetric twin of the dep-cycle `reaches` walk, which already takes `deps_of` (fallible).
    pub fn parents_of_result(&self, child: &str) -> rusqlite::Result<Vec<String>> {
        self.targets_of_result(child, EdgeKind::Parent)
    }

    /// Present `parent`-edge children of `parent` — items whose `parent` edge points at `parent`
    /// (sorted, deterministic). The inverse of [`parents_of`](Self::parents_of). Infallible companion
    /// to [`children_of_result`](Self::children_of_result); as of 07a.7 the facade's relationship-
    /// matrix depth walk + retype revalidation took the fallible variant (a db error there maps to
    /// `io` instead of the `.expect()` below unwinding), leaving this for the differential oracle /
    /// tests and any future one-shot local caller.
    pub fn children_of(&self, parent: &str) -> Vec<String> {
        self.children_of_result(parent).expect("present_edges read")
    }

    /// The fallible core behind [`children_of`](Self::children_of) (#76u.14): the write-validation
    /// depth walk (`height_below`) reads it on the long-lived server, so a db error surfaces as `Err`
    /// (mapped to `io`) instead of unwinding.
    pub fn children_of_result(&self, parent: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn().prepare(
            "SELECT from_id FROM present_edges WHERE to_id=?1 AND kind='parent' ORDER BY from_id",
        )?;
        let rows = stmt.query_map([parent], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Re-point `child` to a single `parent` (sp6.3). Removes every existing `parent` edge first
    /// (observed-remove), then adds the new one — the cardinality-1 path. Validity (live, allowed
    /// type pair, cardinality, depth, not-self) is enforced at the facade write seam before this is
    /// called; the core stays meaning-free.
    pub fn set_parent(&mut self, child: &str, parent: &str, author: &str) -> rusqlite::Result<()> {
        self.clear_parent(child, author)?;
        self.add_edge(child, parent, EdgeKind::Parent, author);
        Ok(())
    }

    /// Add `parent` as an ADDITIONAL parent of `child` without disturbing existing ones (sp6.6: the
    /// n-cardinality path; single-parent re-point uses [`set_parent`](Self::set_parent)). The facade
    /// enforces the cardinality limit before calling this.
    pub fn add_parent(&mut self, child: &str, parent: &str, author: &str) {
        self.add_edge(child, parent, EdgeKind::Parent, author);
    }

    /// Remove every present `parent` edge of `child` (observed-remove), leaving it an orphan.
    /// Fallible (#76u.14): reads the current parent set first, which can surface a db error rather
    /// than panicking — propagated by `set_parent` and the facade write seam.
    pub fn clear_parent(&mut self, child: &str, author: &str) -> rusqlite::Result<()> {
        for parent in self.parents_of_result(child)? {
            self.remove_edge(child, &parent, EdgeKind::Parent, author);
        }
        Ok(())
    }

    /// Present `to` endpoints of `from` for one edge `kind` (sorted, deterministic). Infallible: the
    /// remaining caller (`mentions_of`) runs against a freshly-folded, locally-consistent store where
    /// a db error is a genuine bug — a panic there is acceptable (the one-shot path). The long-lived
    /// read/write seams take the fallible [`targets_of_result`](Self::targets_of_result) via
    /// `deps_of`/`contributes_to_of`/`parents_of` so a db error maps to `io` instead (#76u.13/.14).
    pub fn targets_of(&self, from: &str, kind: EdgeKind) -> Vec<String> {
        self.targets_of_result(from, kind)
            .expect("present_edges read")
    }

    /// The fallible core behind [`targets_of`](Self::targets_of): a present-edge query that surfaces
    /// a db error instead of unwinding. The read-seam edge reads (`deps_of`/`contributes_to_of`)
    /// return this directly so the seam can map a db failure to `io` (#76u.13).
    fn targets_of_result(&self, from: &str, kind: EdgeKind) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn().prepare(
            "SELECT to_id FROM present_edges WHERE from_id=?1 AND kind=?2 ORDER BY to_id",
        )?;
        let rows = stmt.query_map(params![from, kind.as_str()], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Append an immutable note. Returns the note id.
    pub fn add_note(&mut self, item_id: &str, body: &str, author: &str) -> String {
        self.emit_task(
            "note",
            item_id,
            "body",
            "note_add",
            Some(body.to_string()),
            author,
        )
    }

    /// Tombstone a note (LWW `deleted`). The note row stays; it is hidden from reads.
    pub fn redact_note(&mut self, note_id: &str, author: &str) {
        self.emit_task("note", note_id, "deleted", "set", Some("1".into()), author);
    }

    /// Non-redacted notes for an item, ordered by the canonical op order `(lamport, site)` —
    /// replica-independent, consistent with history/export. Fallible (#76u.13): reached by the
    /// read-compute facade (`show`/`search`), so a db error is surfaced for the seam to map to `io`.
    pub fn notes_of(&self, item_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn().prepare(
            "SELECT n.id, n.body FROM notes n JOIN ops o ON o.op_id = n.id
             WHERE n.item_id=?1
               AND n.id NOT IN (SELECT note_id FROM note_tombstones)
             ORDER BY o.lamport, o.site",
        )?;
        let rows = stmt.query_map([item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// [`notes_of`](Self::notes_of) plus each note's `created_at` — the `wall_clock` the note-add op
    /// stamped (6j6v.2kjy), the SAME op-log source item timestamps use, so a note's date is consistent
    /// with its item's (and deterministic under a pinned `now`) rather than the note ULID's real
    /// mint-instant. `None` when the note-add op carried no `wall_clock` (a legacy/un-stamped write).
    /// Same non-redacted set + canonical `(lamport, site)` order as `notes_of`.
    pub fn notes_with_created_of(
        &self,
        item_id: &str,
    ) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
        let mut stmt = self.conn().prepare(
            "SELECT n.id, n.body, o.wall_clock FROM notes n JOIN ops o ON o.op_id = n.id
             WHERE n.item_id=?1
               AND n.id NOT IN (SELECT note_id FROM note_tombstones)
             ORDER BY o.lamport, o.site",
        )?;
        let rows = stmt.query_map([item_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                // '' (the unstamped default) reads as "no timestamp", like item_timestamps_of.
                r.get::<_, Option<String>>(2)?.filter(|s| !s.is_empty()),
            ))
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MergeStrategy;

    #[test]
    fn get_items_bulk_matches_per_item_get_item() {
        // xn8s: read::next resolved its ready set with N× get_item, and EACH get_item re-materializes
        // the present_parent view (the belongs_to projection) → O(n·edges), the dominant read::next
        // cost. get_items resolves the whole set in ONE query (present_parent materialized once) and
        // must return the SAME rows as per-item get_item (present ids only; a missing id is absent).
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.p", "project", "Epic", "t");
        s.create_item("c1.a", "task", "A", "t");
        s.create_item("c1.b", "task", "B", "t");
        s.add_edge("c1.a", "c1.p", EdgeKind::Parent, "t"); // a.belongs_to = c1.p; b has none
        let ids = ["c1.a", "c1.b", "c1.missing", "c1.p"];

        let mut bulk = s.get_items(&ids).unwrap();
        let mut per_item: Vec<ItemRow> = ids
            .iter()
            .filter_map(|id| s.get_item(id).unwrap())
            .collect();
        bulk.sort_by(|x, y| x.id.cmp(&y.id));
        per_item.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(
            bulk, per_item,
            "bulk get_items matches per-item get_item, missing id absent"
        );
        assert_eq!(
            bulk.iter()
                .find(|i| i.id == "c1.a")
                .unwrap()
                .belongs_to
                .as_deref(),
            Some("c1.p"),
            "the present_parent (belongs_to) projection comes through the bulk join"
        );
    }

    #[test]
    fn item_read_stats_counts_the_queries_each_item_read_runs() {
        // 6j6v.g028: the guard that read::next resolves its ready set in BULK used to be a wall-clock
        // ratio, which measures the machine's load as much as the code. The property it wants is
        // algorithmic — "one query, not N" — so the store counts the item-read QUERIES it runs and a
        // test asserts the shape directly. A `get_item` is one single-row query; a `get_items` is one
        // query per ≤500-id chunk, whatever the id count.
        let mut s = Store::open_in_memory(1);
        for i in 0..3 {
            s.create_item(&format!("c1.{i}"), "task", "t", "t");
        }
        let before = s.item_read_stats();
        let _ = s.get_item("c1.0").unwrap();
        let _ = s.get_item("c1.1").unwrap();
        let _ = s.get_items(&["c1.0", "c1.1", "c1.2"]).unwrap();
        assert_eq!(
            s.item_read_stats().since(before),
            ItemReadStats { single: 2, bulk: 1 },
            "two per-item reads and ONE bulk read for the three-id set"
        );
    }

    #[test]
    fn item_read_stats_counts_one_bulk_query_per_chunk() {
        // The bulk counter counts QUERIES, not calls: 550 ids cross the 500-id chunk boundary, so
        // `get_items` runs two — and an empty id slice runs none (it short-circuits). Without this
        // the guard could not tell "one bulk read" from "one call that fanned out".
        let mut s = Store::open_in_memory(1);
        let ids: Vec<String> = (0..550).map(|i| format!("c1.{i:04}")).collect();
        for id in &ids {
            s.create_item(id, "task", "t", "t");
        }
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let before = s.item_read_stats();
        let _ = s.get_items(&refs).unwrap();
        let _ = s.get_items(&[]).unwrap();
        assert_eq!(
            s.item_read_stats().since(before),
            ItemReadStats { single: 0, bulk: 2 },
            "one query per 500-id chunk; the empty slice runs none"
        );
    }

    #[test]
    fn get_items_empty_input_is_an_empty_result_no_query() {
        // PR-review Test Quality #2: an empty id slice must short-circuit to an empty Vec — never
        // emit a `WHERE id IN ()` (a syntax error). `chunks(500)` over `[]` yields no chunk, so no
        // statement is prepared.
        let s = Store::open_in_memory(1);
        assert!(s.get_items(&[]).unwrap().is_empty());
    }

    #[test]
    fn get_items_spans_multiple_chunks() {
        // PR-review Test Quality #2 (mirrors `labels_of_bulk_spans_multiple_chunks`): 550 ids cross
        // the 500-id chunk boundary, so the result must union both chunks — every existing id back
        // exactly once, nothing dropped or duplicated at the seam.
        let mut s = Store::open_in_memory(1);
        let n = 550;
        let ids: Vec<String> = (0..n).map(|i| format!("c1.{i:04}")).collect();
        for id in &ids {
            s.create_item(id, "task", "t", "t");
        }
        let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let got = s.get_items(&refs).unwrap();
        let seen: std::collections::BTreeSet<&str> = got.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            got.len(),
            n,
            "every id across both chunks resolved exactly once"
        );
        assert_eq!(seen.len(), n, "no duplicate rows across the chunk merge");
        assert!(
            ids.iter().all(|id| seen.contains(id.as_str())),
            "no id dropped at the chunk seam"
        );
    }

    #[test]
    fn crdt_text_field_set_folds_lww_until_the_text_reducer_lands() {
        // w213: a field declared crdt-text rides a strategy-tagged op, but until the text-CRDT
        // reducer (4b39) the fold falls back to LWW — declaring crdt-text today is safe and
        // lossless. The op is foldable and the value materializes into `items` exactly like a plain
        // LWW set; a later write beats the earlier one on the Lamport clock.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.N", "note", "a note", "t");
        s.set_field_merge(
            "c1.N",
            "description",
            Some("first".into()),
            "t",
            MergeStrategy::CrdtText,
        );
        assert_eq!(
            s.get_item("c1.N").unwrap().unwrap().description.as_deref(),
            Some("first")
        );
        s.set_field_merge(
            "c1.N",
            "description",
            Some("second".into()),
            "t",
            MergeStrategy::CrdtText,
        );
        assert_eq!(
            s.get_item("c1.N").unwrap().unwrap().description.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn lww_and_crdt_text_writes_converge_on_one_field() {
        // The strategy is carried on the op, so the reducer dispatches purely on op shape. Mixed
        // lww/crdt-text writes to one field still fold deterministically (both LWW today), and the
        // higher Lamport wins regardless of which strategy tagged it.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.N", "note", "a note", "t");
        s.set_field("c1.N", "description", Some("plain".into()), "t");
        s.set_field_merge(
            "c1.N",
            "description",
            Some("tagged".into()),
            "t",
            MergeStrategy::CrdtText,
        );
        assert_eq!(
            s.get_item("c1.N").unwrap().unwrap().description.as_deref(),
            Some("tagged")
        );
    }

    #[test]
    fn crdt_text_field_converges_across_two_replicas() {
        // w213 review: the merge-strategy fold dispatch must CONVERGE across replicas, not just fold
        // correctly in one. Two replicas concurrently set a crdt-text field; after exchanging ops
        // both land the same (lamport, site) winner, exactly like a plain LWW cell (crdt-text folds
        // LWW until 4b39). Covers the multi-replica gap the single-replica merge tests left open.
        let mut a = Store::open_in_memory(1);
        a.create_item("g.X", "note", "n", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.set_field_merge(
            "g.X",
            "description",
            Some("from-A".into()),
            "x",
            MergeStrategy::CrdtText,
        );
        b.set_field_merge(
            "g.X",
            "description",
            Some("from-B".into()),
            "x",
            MergeStrategy::CrdtText,
        );
        // Pin the precondition the winner assertion relies on: both writes are CONCURRENT at the
        // same lamport, so `site` is the only tiebreak.
        assert_eq!(
            a.clock(),
            b.clock(),
            "both crdt-text writes are concurrent at the same lamport"
        );

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        let da = a.get_item("g.X").unwrap().unwrap().description;
        let db = b.get_item("g.X").unwrap().unwrap().description;
        assert_eq!(da, db, "replicas converge on one crdt-text value");
        assert_eq!(
            da.as_deref(),
            Some("from-B"),
            "site 2 > site 1 at equal lamport"
        );
    }

    #[test]
    fn mixed_lww_and_crdt_text_strategies_converge_across_replicas() {
        // The strategy rides the op (op_type), so a replica folding a synced op needs NO sender
        // plugin config: replica A tags its write lww, replica B tags crdt-text on the SAME field,
        // and both converge on the same (lamport, site) winner. This is the convergence guarantee
        // w213's op-carried strategy buys — verified across replicas, not just single-store.
        let mut a = Store::open_in_memory(1);
        a.create_item("g.X", "note", "n", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.set_field_merge(
            "g.X",
            "description",
            Some("lww-side".into()),
            "x",
            MergeStrategy::Lww,
        );
        b.set_field_merge(
            "g.X",
            "description",
            Some("crdt-side".into()),
            "x",
            MergeStrategy::CrdtText,
        );
        assert_eq!(
            a.clock(),
            b.clock(),
            "concurrent writes at the same lamport"
        );

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);
        assert_eq!(
            a.get_item("g.X").unwrap().unwrap().description,
            b.get_item("g.X").unwrap().unwrap().description,
            "mixed-strategy writes converge regardless of which strategy tagged the op"
        );
        assert_eq!(
            a.get_item("g.X").unwrap().unwrap().description.as_deref(),
            Some("crdt-side"),
            "site 2 wins the tie"
        );
    }

    #[test]
    fn set_field_merge_lww_is_identical_to_set_field() {
        // Lww via the strategy-aware seam emits the bare "set" op — byte-identical to `set_field`,
        // so no bundled plugin's behaviour (or its op history) changes.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.set_field_merge(
            "c1.A",
            "title",
            Some("renamed".into()),
            "t",
            MergeStrategy::Lww,
        );
        let op_types: Vec<String> = s
            .export()
            .iter()
            .filter(|o| o.target_id == "c1.A" && o.field == "title")
            .map(|o| o.op_type.clone())
            .collect();
        assert!(
            op_types.iter().all(|t| t == "set"),
            "an lww set emits the bare \"set\" op_type, got {op_types:?}"
        );
    }

    #[test]
    fn get_item_surfaces_a_db_read_failure_as_err_not_a_spurious_not_found() {
        // #76u.14: get_item is `rusqlite::Result<Option<ItemRow>>` via `.optional()`, so a genuine db
        // fault is `Err` — distinct from `Ok(None)` for a truly-absent row. The pre-#76u.14 `.ok()`
        // swallow (or a revert to it) would report a busy/corrupt db as a spurious `not_found`; the
        // facade maps this `Err` to `io` instead. Assert BOTH arms so a revert can't stay green.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "X", "t");
        assert!(
            matches!(s.get_item("c1.X"), Ok(Some(_))),
            "a present row is Ok(Some)"
        );
        assert!(
            matches!(s.get_item("c1.MISSING"), Ok(None)),
            "a truly-absent row is Ok(None), not Err"
        );
        s.connection()
            .execute_batch("DROP TABLE items;")
            .expect("drop items to force a read error");
        assert!(
            s.get_item("c1.X").is_err(),
            "a db read failure is Err (mapped to io at the seam), never a spurious Ok(None)/not_found"
        );
    }

    #[test]
    fn the_substrate_dispatches_fold_by_op_domain() {
        // aye.6: the flow store delegates op-log writes to the substrate, which dispatches each op
        // to the reducer registered for its `domain` (§4.2). A reducer registered for a foreign
        // domain folds that domain's ops — proving dispatch is registry-driven, not hardcoded to
        // flow's task fold.
        use crate::reducer::Reducer;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct DemoReducer {
            folds: Arc<AtomicUsize>,
        }
        impl Reducer for DemoReducer {
            fn domain(&self) -> &'static str {
                "demo"
            }
            fn is_foldable(&self, _op: &Op) -> bool {
                true
            }
            fn fold(&self, _conn: &Connection, _op: &Op) {
                self.folds.fetch_add(1, Ordering::SeqCst);
            }
            fn clear_views(&self, _conn: &Connection) {}
        }

        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));

        let demo = Op {
            op_id: "01J0DEMOOP00000000000000A".into(),
            lamport: 1,
            site: 9,
            domain: "demo".into(),
            target_kind: "gizmo".into(),
            target_id: "g.1".into(),
            field: "x".into(),
            op_type: "set".into(),
            value: None,
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        let deferred = s.apply(std::slice::from_ref(&demo));
        assert!(
            deferred.is_empty(),
            "the demo reducer folded its domain's op (not deferred)"
        );
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "fold was dispatched to the reducer registered for the op's domain"
        );

        let orphan = Op {
            op_id: "01J0ORPHANOP000000000000B".into(),
            domain: "nobody".into(),
            ..demo.clone()
        };
        let deferred = s.apply(std::slice::from_ref(&orphan));
        assert_eq!(
            deferred,
            vec![orphan],
            "an unregistered domain defers (no reducer to fold it)"
        );
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "no extra fold for the orphan"
        );
    }

    #[test]
    fn data_version_is_stable_across_own_commits() {
        let mut s = Store::open_in_memory(1);
        let v0 = s.data_version().unwrap();
        s.create_item("c1.X", "task", "T", "a");
        s.set_field("c1.X", "priority", Some("0".into()), "a");
        assert_eq!(
            s.data_version().unwrap(),
            v0,
            "own commits never bump data_version"
        );
    }

    /// Build a raw foreign Op for merge-path tests (bypasses the typed `emit` builders, so it can
    /// carry deliberately malformed shapes).
    fn raw_op(target_kind: &str, target_id: &str, field: &str, op_type: &str) -> Op {
        Op {
            op_id: "x.1".into(),
            lamport: 1,
            site: 9,
            domain: "task".into(),
            target_kind: target_kind.into(),
            target_id: target_id.into(),
            field: field.into(),
            op_type: op_type.into(),
            value: None,
            author: "t".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    #[test]
    fn apply_stores_but_does_not_fold_unknown_op_shape() {
        let mut s = Store::open_in_memory(1);
        let op = raw_op("item", "c1.X", "title", "frobnicate");
        let deferred = s.apply(&[op]);
        assert_eq!(
            deferred.len(),
            1,
            "unknown op is reported as stored-not-folded"
        );
        assert_eq!(
            s.op_count(),
            1,
            "the op IS persisted (the resurface path stays open)"
        );
        assert!(
            s.get_item("c1.X").unwrap().is_none(),
            "but it is not folded into the views"
        );

        s.create_item("c1.X", "task", "First", "alice");
        assert!(s.get_item("c1.X").unwrap().is_some());
    }

    #[test]
    fn apply_stores_but_does_not_fold_malformed_edge_target() {
        let mut s = Store::open_in_memory(1);
        let op = raw_op("edge", "garbage", "present", "add");
        let deferred = s.apply(&[op]);
        assert_eq!(deferred.len(), 1);
        assert_eq!(s.op_count(), 1, "stored, not dropped");

        s.create_item("c1.X", "task", "First", "alice");
        assert!(s.get_item("c1.X").unwrap().is_some());
    }

    #[test]
    fn apply_stores_but_does_not_fold_unknown_item_field() {
        let mut s = Store::open_in_memory(1);
        let op = raw_op("item", "c1.X", "nonsense", "set");
        let deferred = s.apply(&[op]);
        assert_eq!(deferred.len(), 1);
        assert_eq!(s.op_count(), 1, "stored, not dropped");
        assert!(
            s.get_item("c1.X").unwrap().is_none(),
            "unknown field is not materialized"
        );
    }

    // ---- plugin custom fields (T1, 6j6v.sfyy): the `("field", set*)` fold ---------
    //
    // There is no write helper yet (that's T3), so these drive the fold through `apply` of
    // hand-built `field` ops and read back the `custom_fields` view directly.

    /// Build a raw plugin custom-field `set` op — the `field`-kind op (spec §4.1) whose `field`
    /// column holds the custom field's NAME and whose `op_type` carries the merge strategy.
    fn field_op(
        op_id: &str,
        lamport: i64,
        site: i64,
        item: &str,
        field: &str,
        op_type: &str,
        value: Option<&str>,
    ) -> Op {
        Op {
            op_id: op_id.into(),
            lamport,
            site,
            domain: "task".into(),
            target_kind: "field".into(),
            target_id: item.into(),
            field: field.into(),
            op_type: op_type.into(),
            value: value.map(Into::into),
            author: "t".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    /// The folded value of one `(item, field)` custom field, or `None` if no row (or a NULL value).
    fn custom_field(s: &Store, item: &str, field: &str) -> Option<String> {
        s.connection()
            .query_row(
                "SELECT value FROM custom_fields WHERE item_id=?1 AND field=?2",
                params![item, field],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .unwrap()
            .flatten()
    }

    /// Total row count in the `custom_fields` view (used to assert a deferred op folded NOTHING).
    fn custom_fields_count(s: &Store) -> i64 {
        s.connection()
            .query_row("SELECT COUNT(*) FROM custom_fields", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn set_custom_field_merge_emits_a_field_set_that_folds_into_custom_fields() {
        // T3: the `field`-kind emit helper is the write-path entry the facade drives. An Lww write
        // emits a bare `("field","set")` op and a CrdtText write emits `("field","set:crdt-text")`
        // — both fold into `custom_fields` keyed on (item_id, field), no ITEM_LWW_FIELDS assert
        // (the field is NON-canonical). A clear (empty value) leaves the row with an empty value.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "file", "F", "t");
        s.set_custom_field_merge(
            "c1.X",
            "uri",
            Some(".nxs/files/report.md".into()),
            "t",
            MergeStrategy::Lww,
        );
        s.set_custom_field_merge(
            "c1.X",
            "body",
            Some("draft".into()),
            "t",
            MergeStrategy::CrdtText,
        );

        assert_eq!(
            custom_field(&s, "c1.X", "uri").as_deref(),
            Some(".nxs/files/report.md"),
            "an lww custom set folds into custom_fields"
        );
        assert_eq!(
            custom_field(&s, "c1.X", "body").as_deref(),
            Some("draft"),
            "a crdt-text custom set folds too (LWW today)"
        );
        // The op shapes: lww is the bare "set", crdt-text carries "set:crdt-text", both target_kind "field".
        let ops: Vec<(String, String, String)> = s
            .export()
            .into_iter()
            .filter(|o| o.target_kind == "field")
            .map(|o| {
                (
                    o.field.clone(),
                    o.op_type.clone(),
                    o.value.clone().unwrap_or_default(),
                )
            })
            .collect();
        assert!(
            ops.contains(&("uri".into(), "set".into(), ".nxs/files/report.md".into())),
            "lww uri set emits the bare set op: {ops:?}"
        );
        assert!(
            ops.contains(&("body".into(), "set:crdt-text".into(), "draft".into())),
            "crdt-text body set carries the strategy marker: {ops:?}"
        );

        // Clear = an empty-value field set; the row remains with an empty value.
        s.set_custom_field_merge("c1.X", "uri", Some(String::new()), "t", MergeStrategy::Lww);
        assert_eq!(
            custom_field(&s, "c1.X", "uri").as_deref(),
            Some(""),
            "clearing stores an empty value (LWW), not a row deletion"
        );
        assert_eq!(
            custom_fields_count(&s),
            2,
            "the cleared row remains (uri + body)"
        );
    }

    #[test]
    fn a_field_set_op_folds_into_the_custom_fields_view() {
        let mut s = Store::open_in_memory(1);
        let deferred = s.apply(&[field_op(
            "f.1",
            1,
            1,
            "c1.X",
            "uri",
            "set",
            Some(".nxs/files/report.md"),
        )]);
        assert!(
            deferred.is_empty(),
            "a well-formed field set folds (not deferred)"
        );
        assert_eq!(
            custom_field(&s, "c1.X", "uri").as_deref(),
            Some(".nxs/files/report.md"),
            "the value materializes into custom_fields keyed on (item_id, field)"
        );
    }

    #[test]
    fn a_custom_field_converges_reorder_independent() {
        // DoD: two replicas set the SAME (item, field) custom field CONCURRENTLY (equal lamport, so
        // `site` is the sole tiebreak). Delivering the two ops in EITHER order lands the same winner
        // (higher (lamport, site)) — the fold is a keep-if-beats LWW register, order-independent.
        let a_op = field_op("f.A", 5, 1, "c1.X", "uri", "set", Some("from-A"));
        let b_op = field_op("f.B", 5, 2, "c1.X", "uri", "set", Some("from-B"));
        assert_eq!(
            a_op.lamport, b_op.lamport,
            "precondition: concurrent writes at equal lamport, site decides"
        );

        let mut r1 = Store::open_in_memory(1);
        r1.apply(&[a_op.clone(), b_op.clone()]);
        let mut r2 = Store::open_in_memory(2);
        r2.apply(&[b_op.clone(), a_op.clone()]); // reversed delivery order

        assert_eq!(
            custom_field(&r1, "c1.X", "uri").as_deref(),
            Some("from-B"),
            "site 2 > site 1 at equal lamport"
        );
        assert_eq!(
            custom_field(&r1, "c1.X", "uri"),
            custom_field(&r2, "c1.X", "uri"),
            "both delivery orders converge on the same winner"
        );
    }

    #[test]
    fn a_field_op_with_an_unknown_strategy_defers_store_dont_fold() {
        // Forward-compat (§7): a `field` op whose op_type carries a strategy THIS binary does not
        // understand (`set:or-set`) is stored-not-folded — the op stays in the log (op_count == 1),
        // no `custom_fields` row, no panic, no loss. A well-formed `("field","set")` op DOES fold.
        let mut s = Store::open_in_memory(1);
        let deferred = s.apply(&[field_op(
            "f.1",
            1,
            1,
            "c1.X",
            "uri",
            "set:or-set",
            Some("x"),
        )]);
        assert_eq!(
            deferred.len(),
            1,
            "the unknown-strategy field op is deferred"
        );
        assert_eq!(
            s.op_count(),
            1,
            "but it IS persisted (the resurface path stays open)"
        );
        assert_eq!(custom_fields_count(&s), 0, "nothing folded into the view");

        let deferred = s.apply(&[field_op("f.2", 2, 1, "c1.X", "uri", "set", Some("ok"))]);
        assert!(deferred.is_empty(), "a well-formed field set folds");
        assert_eq!(custom_field(&s, "c1.X", "uri").as_deref(), Some("ok"));
    }

    #[test]
    fn a_malformed_field_op_defers() {
        // The non-empty guards (mirroring parse_edge/parse_label): an empty field name or empty item
        // id must store-don't-fold rather than fold a junk (item_id, field) row.
        let mut s = Store::open_in_memory(1);
        let empty_field = s.apply(&[field_op("f.1", 1, 1, "c1.X", "", "set", Some("x"))]);
        assert_eq!(empty_field.len(), 1, "empty field name defers");
        let empty_item = s.apply(&[field_op("f.2", 2, 1, "", "uri", "set", Some("x"))]);
        assert_eq!(empty_item.len(), 1, "empty target_id defers");
        assert_eq!(s.op_count(), 2, "both persisted, not dropped");
        assert_eq!(custom_fields_count(&s), 0, "no junk row folded");
    }

    #[test]
    fn a_foreign_custom_field_folds_and_round_trips() {
        // §7: a `field set` with an arbitrary name (no plugin declares it — the core has no plugin)
        // still folds structurally and survives an export → new-replica apply with the same value.
        let mut a = Store::open_in_memory(1);
        a.apply(&[field_op(
            "f.1",
            1,
            1,
            "c1.X",
            "made_up_field",
            "set",
            Some("carried"),
        )]);
        assert_eq!(
            custom_field(&a, "c1.X", "made_up_field").as_deref(),
            Some("carried")
        );

        let mut b = Store::open_in_memory(2);
        let deferred = b.apply(&a.export());
        assert!(
            deferred.is_empty(),
            "the foreign field op folds on the new replica too"
        );
        assert_eq!(
            custom_field(&b, "c1.X", "made_up_field").as_deref(),
            Some("carried"),
            "value round-trips across replicas"
        );
    }

    #[test]
    fn a_crdt_text_custom_field_folds_lww() {
        // TB-CF-5: a `("field","set:crdt-text")` op folds identically to `set` (whole-value LWW)
        // today — crdt-text is a forward-compat marker, not yet a char-level merge. A later write
        // beats an earlier one on (lamport, site).
        let mut s = Store::open_in_memory(1);
        let d = s.apply(&[field_op(
            "f.1",
            1,
            1,
            "c1.X",
            "body",
            "set:crdt-text",
            Some("first"),
        )]);
        assert!(d.is_empty(), "a crdt-text field op folds");
        assert_eq!(custom_field(&s, "c1.X", "body").as_deref(), Some("first"));
        s.apply(&[field_op(
            "f.2",
            2,
            1,
            "c1.X",
            "body",
            "set:crdt-text",
            Some("second"),
        )]);
        assert_eq!(
            custom_field(&s, "c1.X", "body").as_deref(),
            Some("second"),
            "the later write wins LWW"
        );
    }

    #[test]
    fn clearing_a_custom_field_stores_an_empty_value_via_lww() {
        // A clear is a `field set` with an empty value (spec §4.2): an LWW-by-value on the scalar
        // register. The row REMAINS with an empty `value` (the read layer, T4, treats empty as
        // unset) — it is not a row deletion.
        let mut s = Store::open_in_memory(1);
        s.apply(&[field_op("f.1", 1, 1, "c1.X", "uri", "set", Some("u1"))]);
        s.apply(&[field_op("f.2", 2, 1, "c1.X", "uri", "set", Some(""))]);
        assert_eq!(
            custom_field(&s, "c1.X", "uri").as_deref(),
            Some(""),
            "the later empty-value set wins LWW and clears to empty"
        );
        assert_eq!(
            custom_fields_count(&s),
            1,
            "the row remains (empty value, not a delete)"
        );
    }

    #[test]
    fn opening_a_pre_custom_fields_workspace_force_refolds_a_deferred_field_op() {
        // The additive `custom_fields` minor (spec §7): a pre-CF workspace has the `items` view but
        // NO `custom_fields` view, and a synced `field set` op the pre-CF binary store-don't-folded —
        // yet it advanced the flow watermark PAST that op (it folded everything else). A plain
        // refold_if_behind on reopen would be a no-op (caught up), so the value would never fold. The
        // view-schema bump (custom_fields add + force_refold) must resurface it. Mirrors chat's
        // `opening_an_m1_workspace_force_refolds_a_deferred_deadline_op`.
        use nxs_foundation::store::Store as Substrate;

        let path = std::env::temp_dir().join(format!("nxf-cf-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();

        {
            // Build the pre-CF state directly in the shared db.
            let sub = Substrate::open(p, 1).unwrap();
            let conn = sub.connection();
            // Install the flow views, then DROP custom_fields → a pre-CF shape (items present, no
            // custom_fields view) so the next flow open's gate reports an upgrade.
            schema::apply_flow_views(conn);
            conn.execute_batch("DROP TABLE custom_fields;").unwrap();
            // A `field set` op sitting UNFOLDED in the log (the pre-CF binary could not fold it).
            conn.execute(
                "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                 op_type, value, author, wall_clock)
                 VALUES('op-cf', 3, 1, 'task', 'field', 'c1.X', 'uri', 'set',
                        '.nxs/files/report.md', 'a', '')",
                [],
            )
            .unwrap();
            // The pre-CF binary advanced the flow watermark to the log boundary (it folded
            // everything foldable), so a plain refold_if_behind on the next open is a no-op.
            conn.execute(
                "INSERT INTO view_watermarks(store_id, folded_through)
                 VALUES('flow', (SELECT MAX(rowid) FROM ops))",
                [],
            )
            .unwrap();
        }

        // The upgrade open: try_apply_flow_views re-adds custom_fields and reports upgraded=true, so
        // force_refold folds the op that refold_if_behind alone would have left unfolded forever.
        let s = Store::open(p, 1).unwrap();
        assert_eq!(
            custom_field(&s, "c1.X", "uri").as_deref(),
            Some(".nxs/files/report.md"),
            "the deferred field op is resurfaced by the view-schema bump"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refold_resurfaces_persisted_ops_into_the_views() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "T", "a");
        s.set_field("c1.X", "status", Some("closed".into()), "a");
        s.add_note("c1.X", "n", "a");
        let before = s.get_item("c1.X").unwrap();
        let notes_before = s.notes_of("c1.X").unwrap();

        s.connection()
            .execute_batch(
                "DELETE FROM items; DELETE FROM edge_adds; DELETE FROM edge_removes;
                 DELETE FROM notes; DELETE FROM note_tombstones;",
            )
            .unwrap();
        assert!(
            s.get_item("c1.X").unwrap().is_none(),
            "views are empty after the wipe"
        );

        s.refold();
        assert_eq!(
            s.get_item("c1.X").unwrap(),
            before,
            "item rebuilt purely from the log"
        );
        assert_eq!(
            s.notes_of("c1.X").unwrap(),
            notes_before,
            "notes rebuilt purely from the log"
        );
    }

    #[test]
    fn refold_surfaces_a_log_resident_op_that_was_never_folded() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "T", "a");

        s.connection()
            .execute(
                "INSERT INTO ops
                   (op_id, lamport, site, target_kind, target_id, field, op_type, value, author, wall_clock)
                 VALUES ('resurf.1', 999, 1, 'item', 'c1.X', 'title', 'set', 'resurfaced', 'a', '')",
                [],
            )
            .unwrap();
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().title.as_deref(),
            Some("T"),
            "the log-resident op is not yet folded"
        );

        s.refold();
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().title.as_deref(),
            Some("resurfaced"),
            "refold surfaces the previously-unfolded op"
        );
    }

    #[test]
    fn apply_accepts_a_valid_foreign_op() {
        let mut s = Store::open_in_memory(1);
        let mut op = raw_op("item", "c1.X", "title", "set");
        op.value = Some("hello".into());
        let rejected = s.apply(&[op]);
        assert!(rejected.is_empty(), "well-formed op is accepted");
        assert_eq!(s.op_count(), 1);
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().title.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn creating_an_item_appends_ops_and_advances_lamport() {
        let mut s = Store::open_in_memory(1);
        assert_eq!(s.op_count(), 0);
        s.create_item("c1.X", "task", "First", "alice");
        assert_eq!(s.op_count(), 3, "type + title + status");
        assert_eq!(s.clock(), 3, "lamport advanced once per op");
    }

    #[test]
    fn create_item_stores_an_arbitrary_type_verbatim() {
        // sp6.2: the core is type-agnostic — it no longer constrains items to a fixed
        // {project,task} enum. Any opaque type string is folded verbatim; the *valid* set is
        // enforced by the plugin seam (the facade), never by the core.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "milestone", "M", "alice");
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().item_type.as_deref(),
            Some("milestone"),
            "the core stores any type string verbatim — no fixed enum"
        );
    }

    #[test]
    #[should_panic(expected = "U+001F separator")]
    fn create_item_rejects_an_id_containing_the_edge_separator() {
        // Integrity review #2: a SEP (U+001F) in an id would later make every edge it joins
        // un-foldable (the composite splits into >3 parts). The write primitive rejects it loudly
        // at the root instead of letting the edge silently vanish.
        let mut s = Store::open_in_memory(1);
        s.create_item("bad\u{1f}id", "task", "x", "t");
    }

    #[test]
    #[should_panic(expected = "U+001F separator")]
    fn add_edge_rejects_an_endpoint_containing_the_separator() {
        // The same guard at the edge-encoding boundary, covering a SEP id supplied directly to the
        // pub edge API rather than via `create_item`.
        let mut s = Store::open_in_memory(1);
        s.create_item("a", "task", "a", "t");
        s.add_parent("a", "b\u{1f}c", "t");
    }

    // ---- thread links (nxf 6j6v.8dbe) --------------------------------------------------------

    #[test]
    fn a_thread_link_is_n_to_m_in_both_directions() {
        // Non-negotiable per the ticket: a thread concerns several items, an item several threads.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.create_item("c1.B", "task", "B", "t");
        s.add_thread_link(
            "m-1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "t",
        );
        s.add_thread_link("m-1", "c1.B", LinkRelation::Cited, LinkWeight::Passing, "t");
        s.add_thread_link("m-2", "c1.A", LinkRelation::Cited, LinkWeight::Bearing, "t");

        let of_thread: Vec<_> = s
            .items_of_thread("m-1")
            .unwrap()
            .into_iter()
            .map(|l| (l.item_id, l.relation, l.weight))
            .collect();
        assert_eq!(
            of_thread,
            vec![
                (
                    "c1.A".to_string(),
                    LinkRelation::WorkedOn,
                    LinkWeight::Bearing
                ),
                ("c1.B".to_string(), LinkRelation::Cited, LinkWeight::Passing),
            ]
        );
        let of_item: Vec<_> = s
            .thread_links_of_item("c1.A")
            .unwrap()
            .into_iter()
            .map(|l| l.thread_id)
            .collect();
        assert_eq!(of_item, vec!["m-1", "m-2"]);
    }

    #[test]
    fn the_two_attributes_are_independent_so_all_four_combinations_survive() {
        // The ticket is explicit that relation and weight are freely combinable: a thread can
        // load-bearingly CITE an item, or in-passing WORK ON one. Nothing may collapse the pair.
        let mut s = Store::open_in_memory(1);
        let combos = [
            (LinkRelation::WorkedOn, LinkWeight::Bearing),
            (LinkRelation::WorkedOn, LinkWeight::Passing),
            (LinkRelation::Cited, LinkWeight::Bearing),
            (LinkRelation::Cited, LinkWeight::Passing),
        ];
        for (i, (relation, weight)) in combos.iter().enumerate() {
            let item = format!("c1.{i}");
            s.create_item(&item, "task", "x", "t");
            s.add_thread_link("m-1", &item, *relation, *weight, "t");
        }
        let got: Vec<_> = s
            .items_of_thread("m-1")
            .unwrap()
            .into_iter()
            .map(|l| (l.relation, l.weight))
            .collect();
        assert_eq!(got, combos.to_vec());
    }

    #[test]
    fn a_link_can_firm_up_and_be_taken_back() {
        // "Movable": unlike a memory, a link may come into being, firm up, and be retracted. Firming
        // up is an ordinary later add that wins causally — no detach-then-reattach, and exactly ONE
        // link for the pair remains present throughout.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.add_thread_link("m-1", "c1.A", LinkRelation::Cited, LinkWeight::Passing, "t");
        s.add_thread_link("m-1", "c1.A", LinkRelation::Cited, LinkWeight::Bearing, "t");

        let links = s.thread_links_of_item("c1.A").unwrap();
        assert_eq!(links.len(), 1, "one present link per pair, got {links:?}");
        assert_eq!(links[0].weight, LinkWeight::Bearing, "the later add wins");

        // Retracting keys on the PAIR alone — the caller need not know the current attributes.
        s.remove_thread_link("m-1", "c1.A", "t");
        assert!(s.thread_links_of_item("c1.A").unwrap().is_empty());
        assert!(s.items_of_thread("m-1").unwrap().is_empty());
        // And re-attaching afterwards works: a retraction is not a permanent tombstone on the pair.
        s.add_thread_link(
            "m-1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "t",
        );
        assert_eq!(s.thread_links_of_item("c1.A").unwrap().len(), 1);
    }

    #[test]
    fn concurrent_attaches_of_the_same_pair_converge_on_one_winner() {
        // Two replicas attach the SAME pair with different attributes, unaware of each other. After
        // the merge both must agree on ONE present link — the `present_thread_links` max-(lamport,
        // site, tag) projection, the same tiebreak `present_parent` uses.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.A", "task", "A", "t");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.add_thread_link(
            "m-1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "t",
        );
        b.add_thread_link("m-1", "c1.A", LinkRelation::Cited, LinkWeight::Passing, "t");
        let (from_a, from_b) = (a.export(), b.export());
        a.apply(&from_b);
        b.apply(&from_a);

        let la = a.thread_links_of_item("c1.A").unwrap();
        let lb = b.thread_links_of_item("c1.A").unwrap();
        assert_eq!(la.len(), 1, "exactly one present link after merge: {la:?}");
        assert_eq!(la, lb, "both replicas agree on the winner");
    }

    #[test]
    fn a_thread_link_never_gates_derivation_and_never_dangles() {
        // The link carries no blocking semantics whatsoever — `ready`/`blocked` walk `dep` alone —
        // and, because it is NOT in `present_edges`, its thread endpoint is not a candidate for the
        // dangling-edge invariant. Both halves are the reason it is its own OR-set.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.add_thread_link(
            "m-1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "t",
        );
        assert_eq!(
            crate::derive::ready(s.connection(), "2026-08-02T00:00:00Z").unwrap(),
            vec!["c1.A".to_string()],
            "a linked item is as ready as it ever was"
        );
        assert!(
            crate::invariant::reference_violations(s.connection()).is_empty(),
            "a thread endpoint must not read as a dangling edge"
        );
    }

    #[test]
    #[should_panic(expected = "U+001F separator")]
    fn add_thread_link_rejects_an_id_containing_the_separator() {
        // The same guard the edge and label encoders carry: a SEP would make the reducer's four-part
        // parse fail and the link would silently never fold.
        let mut s = Store::open_in_memory(1);
        s.add_thread_link(
            "m\u{1f}1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "t",
        );
    }

    #[test]
    fn file_store_recovers_lamport_clock_across_reopen() {
        let path = std::env::temp_dir().join(format!("nxf-test-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();
        {
            let mut s = Store::open(p, 1).unwrap();
            s.create_item("c1.X", "task", "X", "a"); // lamports 1..=3
            s.set_field("c1.X", "title", Some("v1".into()), "a"); // lamport 4
        } // dropped — clock state lives only in the db
        {
            let mut s = Store::open(p, 1).unwrap(); // must recover clock from MAX(lamport)=4
            s.set_field("c1.X", "title", Some("v2".into()), "a"); // must get lamport 5 > 4
            assert_eq!(
                s.get_item("c1.X").unwrap().unwrap().title.as_deref(),
                Some("v2"),
                "post-reopen write must win LWW; if clock reset to 0 it would lose to v1"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refold_on_open_materializes_task_ops_from_a_foundation_only_pull() {
        // aye.36: nxs sync moves the shared op-log but, foundation-only, folds no product views. A
        // `task` op that lands out-of-band must materialize into flow's views the next time the
        // flow store opens — refold-when-behind, driven by the folded-through watermark.
        use nxs_foundation::store::Store as Substrate;

        let path = std::env::temp_dir().join(format!("nxf-aye36-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();

        // Device-local item, folded inline by the flow store.
        {
            let mut s = Store::open(p, 1).unwrap();
            s.create_item("c1.X", "task", "X", "a");
        }
        // A foundation-only pull: a RAW substrate store (no task reducer) appends a foreign task op.
        // It lands in the shared log unfolded and does not touch the flow store's watermark.
        {
            let mut sub = Substrate::open(p, 2).unwrap();
            let foreign = Op {
                op_id: "aye36.Y".into(),
                lamport: 9,
                site: 2,
                domain: crate::model::DOMAIN_TASK.into(),
                target_kind: "item".into(),
                target_id: "c1.Y".into(),
                field: "title".into(),
                op_type: "set".into(),
                value: Some("from-device-A".into()),
                author: "a".into(),
                wall_clock: String::new(),
                key_id: None,
                sig: None,
            };
            let deferred = sub.apply(std::slice::from_ref(&foreign));
            assert_eq!(
                deferred.len(),
                1,
                "the raw substrate defers the task op (no reducer)"
            );
        }
        // The next flow open is behind the log → refold-on-open folds c1.Y in, keeping c1.X.
        {
            let s = Store::open(p, 1).unwrap();
            assert_eq!(
                s.get_item("c1.Y").unwrap().and_then(|i| i.title).as_deref(),
                Some("from-device-A"),
                "the out-of-band task op is materialized on the next flow open"
            );
            assert!(
                s.get_item("c1.X").unwrap().is_some(),
                "the local item survives the refold"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn last_writer_wins_by_lamport_then_site() {
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.X", "task", "First", "alice");
        a.set_field("c1.X", "title", Some("from-A".into()), "alice");

        let mut b = Store::open_in_memory(2);
        b.apply(&a.export()); // b learns the item
        b.set_field("c1.X", "title", Some("from-B".into()), "bob");

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        let ta = a.get_item("c1.X").unwrap().unwrap().title;
        let tb = b.get_item("c1.X").unwrap().unwrap().title;
        assert_eq!(ta, tb, "replicas converge on one title");
        assert_eq!(ta.as_deref(), Some("from-B"), "higher lamport wins");
        assert_eq!(tb.as_deref(), Some("from-B"));
    }

    #[test]
    fn equal_lamport_breaks_tie_by_site() {
        let mut a = Store::open_in_memory(1);
        a.create_item("g.X", "task", "t", "x");
        a.set_field("g.X", "title", Some("from-1".into()), "x"); // lamport 4, site 1

        let mut b = Store::open_in_memory(2);
        b.create_item("g.X", "task", "t", "x");
        b.set_field("g.X", "title", Some("from-2".into()), "x"); // lamport 4, site 2

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);
        assert_eq!(
            a.get_item("g.X").unwrap().unwrap().title.as_deref(),
            Some("from-2"),
            "site 2 > site 1 at equal lamport"
        );
        assert_eq!(
            b.get_item("g.X").unwrap().unwrap().title.as_deref(),
            Some("from-2")
        );
    }

    #[test]
    fn delete_sets_tombstone() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "First", "alice");
        s.delete_item("c1.X", "alice");
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().deleted.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn archived_stores_a_timestamp_with_lww_semantics() {
        // C1 (#916.1): `archived` is a nullable TIMESTAMP tombstone (analogous to `deleted` but
        // carrying the archive instant for later date-descending sort), folded as an LWW cell.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "First", "alice");
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().archived,
            None,
            "a fresh item is not archived"
        );
        s.set_field(
            "c1.X",
            "archived",
            Some("2026-06-23T10:00:00Z".into()),
            "alice",
        );
        assert_eq!(
            s.get_item("c1.X").unwrap().unwrap().archived.as_deref(),
            Some("2026-06-23T10:00:00Z"),
            "the archive timestamp is materialized (not a bare bool)"
        );
    }

    #[test]
    fn archived_converges_under_concurrent_lww() {
        // Two replicas archive the same item at different instants; the (lamport, site) winner is
        // the same on both — `archived` rides the identical keep-if-beats LWW as every other cell.
        let mut a = Store::open_in_memory(1);
        a.create_item("g.X", "task", "t", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.set_field("g.X", "archived", Some("from-1".into()), "x"); // lamport 4, site 1
        b.set_field("g.X", "archived", Some("from-2".into()), "x"); // lamport 4, site 2

        // Pin the precondition the winner assertion below relies on: both archived writes are
        // CONCURRENT at the SAME lamport, so `site` is the only tiebreak. Asserted, not just
        // commented — a clock shift that made the lamports unequal would otherwise let the higher
        // lamport win and pass this test for the wrong reason (review TQ#4).
        assert_eq!(
            a.clock(),
            b.clock(),
            "both replicas emit the archived write at the same lamport (concurrent)"
        );

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);
        assert_eq!(
            a.get_item("g.X").unwrap().unwrap().archived.as_deref(),
            Some("from-2"),
            "site 2 > site 1 at equal lamport"
        );
        assert_eq!(
            b.get_item("g.X").unwrap().unwrap().archived.as_deref(),
            Some("from-2")
        );
    }

    #[test]
    fn concurrent_add_and_remove_keeps_the_edge() {
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.A", "task", "A", "alice");
        a.create_item("c1.B", "task", "B", "alice");
        a.add_edge("c1.A", "c1.B", EdgeKind::Dep, "alice"); // add #1

        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.remove_edge("c1.A", "c1.B", EdgeKind::Dep, "alice");
        b.add_edge("c1.A", "c1.B", EdgeKind::Dep, "bob"); // add #2 (unseen by a's remove)

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        assert_eq!(
            a.deps_of("c1.A").unwrap(),
            vec!["c1.B".to_string()],
            "add wins on a"
        );
        assert_eq!(
            b.deps_of("c1.A").unwrap(),
            vec!["c1.B".to_string()],
            "add wins on b"
        );
    }

    // ---- labels (h89s.1): a second observed-remove OR-set, analogous to edges ----

    #[test]
    fn add_label_then_labels_of_returns_it_sorted_and_deduped() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "X", "t");
        s.add_label("c1.X", "urgent", "t");
        s.add_label("c1.X", "backend", "t");
        s.add_label("c1.X", "urgent", "t"); // duplicate add — OR-set stays one present label
        assert_eq!(
            s.labels_of("c1.X").unwrap(),
            vec!["backend".to_string(), "urgent".to_string()],
            "labels are sorted and de-duplicated"
        );
    }

    #[test]
    fn remove_label_tombstones_every_observed_add() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "X", "t");
        s.add_label("c1.X", "urgent", "t");
        s.add_label("c1.X", "urgent", "t"); // two adds of the same label
        s.remove_label("c1.X", "urgent", "t"); // observed-remove tombstones both live tags
        assert!(
            s.labels_of("c1.X").unwrap().is_empty(),
            "the label is gone after an observed-remove"
        );
    }

    #[test]
    fn labels_are_scoped_to_their_item() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.create_item("c1.B", "task", "B", "t");
        s.add_label("c1.A", "shared", "t");
        s.add_label("c1.B", "shared", "t");
        s.remove_label("c1.A", "shared", "t");
        assert!(s.labels_of("c1.A").unwrap().is_empty());
        assert_eq!(
            s.labels_of("c1.B").unwrap(),
            vec!["shared".to_string()],
            "removing A's label leaves B's — the OR-set is keyed by (item, label)"
        );
    }

    #[test]
    fn labels_of_bulk_matches_per_item_reads_in_one_pass() {
        // The bulk sibling of `labels_of` (1w5v): decorate a whole lane in ONE query instead of N.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.create_item("c1.B", "task", "B", "t");
        s.create_item("c1.C", "task", "C", "t"); // C has no labels
        s.add_label("c1.A", "urgent", "t");
        s.add_label("c1.A", "backend", "t"); // two labels — must come back sorted, like `labels_of`
        s.add_label("c1.B", "urgent", "t");

        let bulk = s.labels_of_bulk(&["c1.A", "c1.B", "c1.C"]).unwrap();

        assert_eq!(
            bulk.get("c1.A"),
            Some(&vec!["backend".to_string(), "urgent".to_string()]),
            "labels sorted per item, exactly as labels_of"
        );
        assert_eq!(bulk.get("c1.B"), Some(&vec!["urgent".to_string()]));
        assert_eq!(
            bulk.get("c1.C"),
            None,
            "an item with no present labels is simply absent from the map"
        );

        // Bulk agrees with the singular read for every id (the property a lane decoration relies on).
        for id in ["c1.A", "c1.B", "c1.C"] {
            let bulk_labels = bulk.get(id).cloned().unwrap_or_default();
            assert_eq!(
                bulk_labels,
                s.labels_of(id).unwrap(),
                "bulk == singular for {id}"
            );
        }

        // Empty input is a valid no-op — no query, empty map (the caller may hand an empty lane).
        assert!(s.labels_of_bulk(&[]).unwrap().is_empty());
    }

    #[test]
    fn labels_of_bulk_ignores_removed_labels_like_the_view() {
        // Bulk reads the same `present_labels` view, so an observed-remove drops out of the batch too.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "t");
        s.add_label("c1.A", "gone", "t");
        s.add_label("c1.A", "kept", "t");
        s.remove_label("c1.A", "gone", "t");
        let bulk = s.labels_of_bulk(&["c1.A"]).unwrap();
        assert_eq!(bulk.get("c1.A"), Some(&vec!["kept".to_string()]));
    }

    #[test]
    fn labels_of_bulk_spans_multiple_chunks() {
        // `labels_of_bulk` chunks the `IN (…)` at 500 ids to stay under SQLite's bound-param cap
        // (PR-review Test Quality #1 / Code Quality #2). Exercise the >1-chunk MERGE path: 550 ids
        // cross the boundary, so the result must union both chunks with nothing dropped or duplicated.
        let mut s = Store::open_in_memory(1);
        let n = 550;
        let ids: Vec<String> = (0..n).map(|i| format!("c1.{i:04}")).collect();
        for id in &ids {
            s.create_item(id, "task", "t", "t");
            s.add_label(id, "tag", "t");
        }
        let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let map = s.labels_of_bulk(&refs).unwrap();
        assert_eq!(
            map.len(),
            n,
            "every id across both chunks is present exactly once"
        );
        for id in &ids {
            assert_eq!(
                map.get(id),
                Some(&vec!["tag".to_string()]),
                "id {id} labels survive the chunk merge"
            );
        }
    }

    // ---- plugin custom fields (6j6v.ekf5, T4): the read side of `custom_fields` ------

    #[test]
    fn custom_fields_of_bulk_matches_per_item_and_excludes_empty() {
        // The bulk sibling of `custom_fields_of` (§6): decorate a whole lane in ONE query. A CLEARED
        // field (empty value, §4.2) reads as unset, so it is excluded from BOTH reads; an item with
        // no set custom value is simply absent from the bulk map (like a label-less item).
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "file", "A", "t");
        s.create_item("c1.B", "project", "B", "t");
        s.create_item("c1.C", "task", "C", "t"); // no custom values
        s.set_custom_field_merge(
            "c1.A",
            "uri",
            Some(".nxs/a.md".into()),
            "t",
            MergeStrategy::Lww,
        );
        s.set_custom_field_merge(
            "c1.A",
            "note",
            Some("draft".into()),
            "t",
            MergeStrategy::Lww,
        );
        s.set_custom_field_merge(
            "c1.B",
            "stage",
            Some("active".into()),
            "t",
            MergeStrategy::Lww,
        );
        // A field set then CLEARED (empty value) must not surface.
        s.set_custom_field_merge("c1.B", "gone", Some("x".into()), "t", MergeStrategy::Lww);
        s.set_custom_field_merge("c1.B", "gone", Some(String::new()), "t", MergeStrategy::Lww);

        let bulk = s.custom_fields_of_bulk(&["c1.A", "c1.B", "c1.C"]).unwrap();

        assert_eq!(
            bulk.get("c1.A"),
            Some(&BTreeMap::from([
                ("note".to_string(), "draft".to_string()),
                ("uri".to_string(), ".nxs/a.md".to_string()),
            ])),
            "fields sorted per item, non-empty only"
        );
        assert_eq!(
            bulk.get("c1.B"),
            Some(&BTreeMap::from([(
                "stage".to_string(),
                "active".to_string()
            )])),
            "the cleared `gone` field is excluded, `stage` remains"
        );
        assert_eq!(
            bulk.get("c1.C"),
            None,
            "an item with no set custom value is absent from the map"
        );

        // Bulk agrees with the singular read for every id (the lane-decoration invariant).
        for id in ["c1.A", "c1.B", "c1.C"] {
            let bulk_row = bulk.get(id).cloned().unwrap_or_default();
            assert_eq!(
                bulk_row,
                s.custom_fields_of(id).unwrap(),
                "bulk == singular for {id}"
            );
        }

        // Empty input is a valid no-op — no query, empty map.
        assert!(s.custom_fields_of_bulk(&[]).unwrap().is_empty());
    }

    #[test]
    fn custom_fields_of_bulk_spans_multiple_chunks() {
        // Chunks the `IN (…)` at 500 ids to stay under SQLite's bound-param cap (mirrors
        // `labels_of_bulk_spans_multiple_chunks`): 550 ids cross the boundary, so the result must
        // union both chunks with nothing dropped or duplicated.
        let mut s = Store::open_in_memory(1);
        let n = 550;
        let ids: Vec<String> = (0..n).map(|i| format!("c1.{i:04}")).collect();
        for id in &ids {
            s.create_item(id, "file", "t", "t");
            s.set_custom_field_merge(id, "uri", Some("u".into()), "t", MergeStrategy::Lww);
        }
        let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let map = s.custom_fields_of_bulk(&refs).unwrap();
        assert_eq!(
            map.len(),
            n,
            "every id across both chunks is present exactly once"
        );
        for id in &ids {
            assert_eq!(
                map.get(id),
                Some(&BTreeMap::from([("uri".to_string(), "u".to_string())])),
                "id {id} survives the chunk merge"
            );
        }
    }

    #[test]
    #[ignore = "perf micro-benchmark (1w5v): `cargo test -p nexus-flow-core --release \
                bulk_labels_timing -- --ignored --nocapture`"]
    fn bulk_labels_timing_n_queries_to_one() {
        // Isolates exactly what 1w5v changes: decorating a lane's labels via N per-item `labels_of`
        // (one SQL prepare+exec each) vs a single `labels_of_bulk`. In-memory, so this measures the
        // per-statement dispatch overhead the batch removes; on disk each avoided query also skips
        // an I/O round-trip, so the real-world win is larger (see the ticket's live measurements).
        use std::time::Instant;
        const N: usize = 300;
        const REPS: u32 = 20;
        let mut s = Store::open_in_memory(1);
        let ids: Vec<String> = (0..N).map(|i| format!("ab12.{i:04}")).collect();
        for id in &ids {
            s.create_item(id, "task", "t", "t");
            s.add_label(id, "urgent", "t");
            s.add_label(id, "backend", "t");
            s.add_label(id, "p1", "t");
        }
        let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();

        // Warm both paths so we time steady state, not first-touch.
        let _ = s.labels_of_bulk(&id_refs).unwrap();
        for id in &id_refs {
            let _ = s.labels_of(id).unwrap();
        }

        let t0 = Instant::now();
        for _ in 0..REPS {
            for id in &id_refs {
                let _ = s.labels_of(id).unwrap(); // OLD lane decoration: N queries
            }
        }
        let per_item = t0.elapsed() / REPS;

        let t1 = Instant::now();
        for _ in 0..REPS {
            let _ = s.labels_of_bulk(&id_refs).unwrap(); // NEW lane decoration: ONE query
        }
        let bulk = t1.elapsed() / REPS;

        // Same load, same answer: bulk agrees with the per-item reads for every id.
        let map = s.labels_of_bulk(&id_refs).unwrap();
        for id in &id_refs {
            assert_eq!(
                map.get(*id).cloned().unwrap_or_default(),
                s.labels_of(id).unwrap()
            );
        }

        eprintln!(
            "\n1w5v labels read — lane of {N} items, in-memory, avg of {REPS}:\n  \
             per-item (N={N} queries): {per_item:?}\n  \
             bulk     (1 query)      : {bulk:?}\n  \
             speedup: {:.1}x\n",
            per_item.as_secs_f64() / bulk.as_secs_f64().max(1e-9)
        );
    }

    #[test]
    fn concurrent_add_and_remove_keeps_the_label() {
        // The OR-set add-wins rule, exactly as for edges: a remove only tombstones the add-tags it
        // has observed, so a concurrent unseen add survives the merge.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.X", "task", "X", "alice");
        a.add_label("c1.X", "urgent", "alice"); // add #1

        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.remove_label("c1.X", "urgent", "alice");
        b.add_label("c1.X", "urgent", "bob"); // add #2 (unseen by a's remove)

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        assert_eq!(
            a.labels_of("c1.X").unwrap(),
            vec!["urgent".to_string()],
            "add wins on a"
        );
        assert_eq!(
            b.labels_of("c1.X").unwrap(),
            vec!["urgent".to_string()],
            "add wins on b"
        );
    }

    #[test]
    fn remap_prefix_rewrites_label_item_ids() {
        // §8 prefix remap: a label's composite target_id carries the item short-id, so the remap
        // must move it — and clear_views must wipe the label tables before the rewritten log is
        // re-folded, else the stale (old-id, same-tag) row would survive the INSERT-OR-IGNORE.
        let mut s = Store::open_in_memory(1);
        s.create_item("old.X", "task", "X", "a");
        s.add_label("old.X", "urgent", "a");
        s.remap_prefix("old", "new");
        assert!(s.get_item("old.X").unwrap().is_none(), "the old id is gone");
        assert_eq!(
            s.labels_of("new.X").unwrap(),
            vec!["urgent".to_string()],
            "the label follows the remapped item id"
        );
        assert!(
            s.labels_of("old.X").unwrap().is_empty(),
            "no stale label lingers under the old id"
        );
    }

    #[test]
    #[should_panic(expected = "U+001F separator")]
    fn add_label_rejects_a_label_containing_the_separator() {
        // A SEP in the label would make the composite target_id split into >2 parts, so the reducer
        // could not parse it back — reject loudly at the write seam, mirroring the edge guard.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "x", "t");
        s.add_label("c1.X", "a\u{1f}b", "t");
    }

    #[test]
    fn set_parent_projects_into_belongs_to_via_the_parent_edge() {
        // sp6.3: parenthood is a `parent` edge in the OR-set, and `get_item().belongs_to` is the
        // `present_parent` projection of it (the single winning parent). No belongs_to LWW register.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "project", "Parent", "x");
        s.create_item("c1.Q", "project", "Other parent", "x");
        s.create_item("c1.T", "task", "Child", "x");
        assert_eq!(
            s.get_item("c1.T").unwrap().unwrap().belongs_to,
            None,
            "orphan by default"
        );

        s.set_parent("c1.T", "c1.P", "x").unwrap();
        assert_eq!(
            s.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
            Some("c1.P"),
            "the parent edge projects into belongs_to"
        );
        assert_eq!(s.parents_of("c1.T"), vec!["c1.P".to_string()]);

        // Re-point: single-parent shim removes the old edge before adding the new one.
        s.set_parent("c1.T", "c1.Q", "x").unwrap();
        assert_eq!(
            s.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
            Some("c1.Q"),
            "re-point replaces the parent (single-parent enforced at write time)"
        );
        assert_eq!(
            s.parents_of("c1.T"),
            vec!["c1.Q".to_string()],
            "exactly one parent edge remains after a re-point"
        );

        // Clear: no parent edge, belongs_to back to None.
        s.clear_parent("c1.T", "x").unwrap();
        assert_eq!(s.get_item("c1.T").unwrap().unwrap().belongs_to, None);
        assert!(s.parents_of("c1.T").is_empty());
    }

    /// `TaskReducer::view_tables` is the WHOLE list of flow's views (6j6v.mxt2): after a fold that
    /// touches every kind of op flow understands, clearing the views must leave no table holding
    /// anything but the log and the watermarks. A view table missing from the list would be one a
    /// snapshot silently leaves behind while its watermark claims it folded — this is where that
    /// shows up, the day somebody adds a table.
    #[test]
    fn clearing_the_views_leaves_nothing_the_fold_wrote() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.create_item("c1.B", "task", "B", "x");
        s.set_field("c1.A", "description", Some("d".into()), "x");
        s.set_custom_field_merge(
            "c1.A",
            "estimate",
            Some("3".into()),
            "x",
            MergeStrategy::Lww,
        );
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        s.remove_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        s.add_label("c1.A", "l", "x");
        s.remove_label("c1.A", "l", "x");
        let note = s.add_note("c1.A", "n", "x");
        s.redact_note(&note, "x");
        s.add_thread_link(
            "t-1",
            "c1.A",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "x",
        );
        s.remove_thread_link("t-1", "c1.A", "x");

        let tables: Vec<String> = s
            .connection()
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let count = |s: &Store, t: &str| -> i64 {
            s.connection()
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        };
        let written: Vec<&String> = tables.iter().filter(|t| count(&s, t) > 0).collect();
        for view in TaskReducer.view_tables() {
            assert!(
                written.iter().any(|t| t == view),
                "the fold never wrote {view} — widen this test's ops so the list is exercised"
            );
        }
        TaskReducer.clear_views(s.connection());
        for t in &tables {
            if nxs_foundation::schema::SUBSTRATE_TABLES.contains(&t.as_str()) {
                continue;
            }
            assert_eq!(
                count(&s, t),
                0,
                "{t} still holds folded rows after clear_views — add it to TaskReducer::view_tables"
            );
        }
    }

    /// Every maintenance path the engine runs keeps the log whole (nxf 6j6v.hehx #3): no op is
    /// lost, and none changes what it IS — id, `(lamport, site)` coordinate, author. The substrate's
    /// trigger refuses a violation outright; this walks the paths that exist today and says, per
    /// path, which one broke the promise if one ever does.
    #[test]
    fn no_maintenance_path_loses_an_op_or_changes_what_one_is() {
        type Identity = (String, i64, i64, String);
        fn identities(s: &Store) -> std::collections::BTreeSet<Identity> {
            s.export()
                .into_iter()
                .map(|o| (o.op_id, o.lamport, o.site, o.author))
                .collect()
        }
        fn step(path: &str, s: &Store, seen: &mut std::collections::BTreeSet<Identity>) {
            let now = identities(s);
            let lost: Vec<_> = seen.difference(&now).collect();
            assert!(lost.is_empty(), "{path} lost or rewrote {lost:?}");
            *seen = now;
        }

        let mut s = Store::open_in_memory(1);
        s.create_item("aaaa.1", "task", "T", "x");
        s.create_item("aaaa.2", "task", "U", "x");
        s.set_field("aaaa.2", "description", Some("see aaaa.1".into()), "x");
        s.add_edge("aaaa.2", "aaaa.1", EdgeKind::Dep, "x");
        s.add_label("aaaa.1", "l", "x");
        s.add_note("aaaa.1", "about aaaa.2", "x");
        let mut seen = identities(&s);

        // A foreign op, a legacy `belongs_to` register the migration will rewrite, and a
        // re-delivery of everything already held.
        let foreign = |op_id: &str, lamport: i64, field: &str, value: &str| Op {
            op_id: op_id.into(),
            lamport,
            site: 2,
            domain: DOMAIN_TASK.into(),
            target_kind: "item".into(),
            target_id: "aaaa.2".into(),
            field: field.into(),
            op_type: "set".into(),
            value: Some(value.into()),
            author: "y".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        s.apply(&[
            foreign("f.title", 50, "title", "U2"),
            foreign("f.legacy", 51, "belongs_to", "aaaa.1"),
        ]);
        step("apply", &s, &mut seen);
        let everything = s.export();
        s.apply(&everything);
        step("re-delivery", &s, &mut seen);
        s.refold();
        step("refold", &s, &mut seen);
        s.migrate_belongs_to_to_parent_edges();
        step("the belongs_to migration", &s, &mut seen);
        assert_eq!(s.parents_of("aaaa.2"), vec!["aaaa.1".to_string()]);
        s.remap_prefix("aaaa", "bbbb");
        step("the prefix remap", &s, &mut seen);
        assert!(s.get_item("bbbb.2").unwrap().is_some(), "the remap did run");
    }

    #[test]
    fn migration_converts_the_winning_belongs_to_op_into_a_parent_edge() {
        // A pre-sp6.3 (v3) log used `item/belongs_to/set` LWW ops for parenthood. On open the flow
        // store rewrites the LWW-winning one in place into a `parent` edge; superseded ops stay
        // inert (no longer foldable). Idempotent + lossless.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "project", "P", "x");
        s.create_item("c1.Q", "project", "Q", "x");
        s.create_item("c1.T", "task", "T", "x");

        // Legacy register ops: set parent P (lamport 10), re-point to Q (lamport 20 — the winner).
        let belongs = |op_id: &str, lamport: i64, parent: &str| Op {
            op_id: op_id.into(),
            lamport,
            site: 1,
            domain: DOMAIN_TASK.into(),
            target_kind: "item".into(),
            target_id: "c1.T".into(),
            field: "belongs_to".into(),
            op_type: "set".into(),
            value: Some(parent.into()),
            author: "x".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        s.apply(&[
            belongs("legacy.P", 10, "c1.P"),
            belongs("legacy.Q", 20, "c1.Q"),
        ]);
        // Not yet migrated: `belongs_to` ops are no longer foldable, so no parent is projected.
        assert_eq!(s.get_item("c1.T").unwrap().unwrap().belongs_to, None);
        let ops_before = s.op_count();

        s.migrate_belongs_to_to_parent_edges();
        assert_eq!(
            s.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
            Some("c1.Q"),
            "the LWW winner (Q) becomes the parent edge; the superseded P op is inert"
        );
        assert_eq!(
            s.parents_of("c1.T"),
            vec!["c1.Q".to_string()],
            "exactly one parent edge after migration"
        );
        assert_eq!(
            s.op_count(),
            ops_before,
            "the rewrite is in place — no op added or dropped (lossless)"
        );

        // Idempotent: the child now has a parent edge, so a re-run rewrites nothing.
        s.migrate_belongs_to_to_parent_edges();
        assert_eq!(s.parents_of("c1.T"), vec!["c1.Q".to_string()]);
        assert_eq!(s.op_count(), ops_before);
    }

    /// The legacy migration picks the `belongs_to` register's winner by `(lamport, site)` straight
    /// off the log, so it is a fold of its own — and an op past the Lamport bound takes part in no
    /// fold (6j6v.m19v; review of PR #489, Test Quality #3). A planted one naming another parent
    /// neither wins nor displaces the real winner.
    #[test]
    fn a_belongs_to_op_past_the_lamport_bound_is_never_the_migrations_winner() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "project", "P", "x");
        s.create_item("c1.Q", "project", "Q", "x");
        s.create_item("c1.T", "task", "T", "x");
        let belongs = |op_id: &str, lamport: i64, parent: &str| Op {
            op_id: op_id.into(),
            lamport,
            site: 1,
            domain: DOMAIN_TASK.into(),
            target_kind: "item".into(),
            target_id: "c1.T".into(),
            field: "belongs_to".into(),
            op_type: "set".into(),
            value: Some(parent.into()),
            author: "x".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        s.apply(&[
            belongs("legacy.Q", 20, "c1.Q"),
            belongs("planted.P", i64::MAX, "c1.P"),
        ]);
        s.migrate_belongs_to_to_parent_edges();
        assert_eq!(s.parents_of("c1.T"), vec!["c1.Q".to_string()]);
    }

    #[test]
    fn migration_auto_triggers_on_file_backed_open_and_survives_export_reimport() {
        // Test review #1: the migration must fire on the REAL `Store::open` path, not only when
        // called directly. Seed a v3 belongs_to log in a file, reopen, and assert the parent edge
        // appears with NO explicit migrate call — then export→reimport to confirm convergence and
        // op-id dedup on a peer.
        let path = std::env::temp_dir().join(format!("nxf-migrate-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();

        let belongs = |op_id: &str, lamport: i64, parent: &str| Op {
            op_id: op_id.into(),
            lamport,
            site: 1,
            domain: DOMAIN_TASK.into(),
            target_kind: "item".into(),
            target_id: "c1.T".into(),
            field: "belongs_to".into(),
            op_type: "set".into(),
            value: Some(parent.into()),
            author: "x".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };

        // Scope 1: write a v3-shaped log (legacy belongs_to LWW ops) to the file and close it. The
        // first open migrates an empty log (no-op); the belongs_to ops applied after stay un-migrated
        // on disk.
        {
            let mut s = Store::open(p, 1).unwrap();
            s.create_item("c1.P", "project", "P", "x");
            s.create_item("c1.Q", "project", "Q", "x");
            s.create_item("c1.T", "task", "T", "x");
            s.apply(&[
                belongs("legacy.P", 10, "c1.P"),
                belongs("legacy.Q", 20, "c1.Q"),
            ]);
            assert_eq!(
                s.get_item("c1.T").unwrap().unwrap().belongs_to,
                None,
                "un-migrated on disk: belongs_to ops are inert until migration"
            );
        }

        // Scope 2: reopen via the real path — migration runs INSIDE `open`, no direct call.
        let exported = {
            let s = Store::open(p, 1).unwrap();
            assert_eq!(
                s.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
                Some("c1.Q"),
                "open auto-migrated the LWW winner (Q) into a parent edge"
            );
            assert_eq!(s.parents_of("c1.T"), vec!["c1.Q".to_string()]);
            s.export()
        };
        let _ = std::fs::remove_file(&path);

        // Convergence: a peer applying the exported log materializes the same parent edge.
        let mut peer = Store::open_in_memory(2);
        peer.apply(&exported);
        assert_eq!(
            peer.parents_of("c1.T"),
            vec!["c1.Q".to_string()],
            "the migrated edge converges on a peer"
        );
        let after_first = peer.op_count();

        // Op-id dedup: re-applying the identical log is idempotent — no duplicate ops or edges.
        peer.apply(&exported);
        assert_eq!(
            peer.op_count(),
            after_first,
            "re-applying the same ops adds nothing (op-id dedup)"
        );
        assert_eq!(
            peer.parents_of("c1.T"),
            vec!["c1.Q".to_string()],
            "still exactly one parent after the re-apply"
        );
    }

    #[test]
    fn concurrent_parent_add_and_remove_keeps_the_edge() {
        // sp6.3: the `parent` edge rides the same observed-remove OR-set as `dep` — a concurrent
        // add (unseen by the other replica's remove) survives the merge, on both replicas.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.P", "project", "P", "alice");
        a.create_item("c1.T", "task", "T", "alice");
        a.set_parent("c1.T", "c1.P", "alice").unwrap(); // add #1
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.clear_parent("c1.T", "alice").unwrap(); // remove (observes add #1)
        b.set_parent("c1.T", "c1.P", "bob").unwrap(); // add #2, unseen by a's remove

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        assert_eq!(
            a.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
            Some("c1.P"),
            "the unseen add wins on a"
        );
        assert_eq!(
            b.get_item("c1.T").unwrap().unwrap().belongs_to.as_deref(),
            Some("c1.P"),
            "…and on b — replicas converge"
        );
    }

    #[test]
    fn concurrent_re_point_converges_to_one_deterministic_parent() {
        // The key new convergence property (sp6.3): two replicas concurrently re-point the SAME
        // child to DIFFERENT parents. The OR-set keeps both parent edges (transient multi-parent),
        // but the `present_parent` projection picks ONE deterministic winner by (lamport, site,
        // to_id) — so both replicas agree on `belongs_to` even though the substrate holds two edges.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.P", "project", "P", "x");
        a.create_item("c1.Q", "project", "Q", "x");
        a.create_item("c1.T", "task", "T", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.set_parent("c1.T", "c1.P", "alice").unwrap(); // re-point to P (lamport L, site 1)
        b.set_parent("c1.T", "c1.Q", "bob").unwrap(); // concurrent re-point to Q (lamport L, site 2)

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        // Both replicas converge on the SAME single projected parent…
        let pa = a.get_item("c1.T").unwrap().unwrap().belongs_to;
        let pb = b.get_item("c1.T").unwrap().unwrap().belongs_to;
        assert_eq!(pa, pb, "replicas converge on one projected parent");
        assert_eq!(
            pa.as_deref(),
            Some("c1.Q"),
            "site 2 (bob) beats site 1 at equal lamport — deterministic winner"
        );
        // …while the substrate is genuinely n:m-capable: both parent edges are present.
        assert_eq!(a.parents_of("c1.T"), b.parents_of("c1.T"));
        assert_eq!(
            a.parents_of("c1.T"),
            vec!["c1.P".to_string(), "c1.Q".to_string()],
            "both concurrent parent edges survive in the OR-set"
        );
    }

    #[test]
    fn remove_of_a_solitary_edge_clears_it() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "alice");
        s.create_item("c1.B", "task", "B", "alice");
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "alice");
        s.remove_edge("c1.A", "c1.B", EdgeKind::Dep, "alice");
        assert!(s.deps_of("c1.A").unwrap().is_empty());
    }

    #[test]
    fn notes_are_append_only_and_ordered() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "X", "alice");
        let n1 = s.add_note("c1.X", "started", "alice");
        let n2 = s.add_note("c1.X", "halfway", "alice");
        let notes = s.notes_of("c1.X").unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].0, n1);
        assert_eq!(notes[1].0, n2);
        assert_eq!(notes[0].1, "started");
    }

    #[test]
    fn redacted_notes_are_hidden() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "X", "alice");
        let n1 = s.add_note("c1.X", "oops secret", "alice");
        s.redact_note(&n1, "alice");
        assert!(s.notes_of("c1.X").unwrap().is_empty());
    }

    #[test]
    fn note_redaction_survives_reverse_order_delivery() {
        let mut src = Store::open_in_memory(1);
        src.create_item("c1.X", "task", "X", "alice");
        let n1 = src.add_note("c1.X", "oops secret", "alice");
        src.redact_note(&n1, "alice");

        let all = src.export();
        let item_ops: Vec<Op> = all
            .iter()
            .filter(|o| o.target_kind == "item")
            .cloned()
            .collect();
        let note_ops_reversed: Vec<Op> = all
            .iter()
            .filter(|o| o.target_kind == "note")
            .rev()
            .cloned()
            .collect();

        let mut dst = Store::open_in_memory(2);
        dst.apply(&item_ops);
        dst.apply(&note_ops_reversed);

        assert!(
            dst.notes_of("c1.X").unwrap().is_empty(),
            "redaction must survive reversed delivery"
        );
    }

    #[test]
    fn contributes_to_edge_does_not_block_or_appear_as_dep() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.create_item("c1.B", "task", "B", "x");
        s.add_edge("c1.A", "c1.B", EdgeKind::ContributesTo, "x");
        assert!(
            s.deps_of("c1.A").unwrap().is_empty(),
            "contributes_to is not a dep"
        );
        assert!(crate::derive::ready(s.connection(), "2026-06-08T00:00:00Z")
            .unwrap()
            .contains(&"c1.A".to_string()));
    }

    #[test]
    fn mentions_edge_does_not_block_appear_as_dep_or_form_a_cycle() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.create_item("c1.B", "task", "B", "x");
        s.add_edge("c1.A", "c1.B", EdgeKind::Mentions, "x");
        s.add_edge("c1.B", "c1.A", EdgeKind::Mentions, "x"); // mutual reference, not a dep cycle

        assert!(
            s.deps_of("c1.A").unwrap().is_empty(),
            "mentions is not a dep"
        );
        assert!(crate::invariant::cyclic_nodes(s.connection()).is_empty());
        let ready = crate::derive::ready(s.connection(), "2026-06-08T00:00:00Z").unwrap();
        assert!(ready.contains(&"c1.A".to_string()) && ready.contains(&"c1.B".to_string()));
        assert!(crate::derive::blocked(s.connection()).unwrap().is_empty());
    }

    #[test]
    fn remap_prefix_swaps_only_the_prefix_across_ops_views_and_freetext() {
        let mut s = Store::open_in_memory(1);
        s.create_item("aaaa.0001", "project", "Parent", "x");
        s.create_item("aaaa.0002", "task", "Child", "x");
        s.set_parent("aaaa.0002", "aaaa.0001", "x").unwrap();
        s.set_field(
            "aaaa.0002",
            "description",
            Some("done when aaaa.0001 ships".into()),
            "x",
        );
        s.add_edge("aaaa.0002", "aaaa.0001", EdgeKind::Dep, "x"); // child deps on parent
        s.add_edge("aaaa.0002", "aaaa.0001", EdgeKind::Mentions, "x");
        s.add_note("aaaa.0002", "see aaaa.0001 for context", "x");
        s.create_item("zzzz.9999", "task", "Foreign", "y");

        let ops_before = s.op_count();
        let clock_before = s.clock();

        s.remap_prefix("aaaa", "kp3z");

        assert!(
            s.get_item("aaaa.0001").unwrap().is_none()
                && s.get_item("aaaa.0002").unwrap().is_none()
        );
        assert!(
            s.get_item("kp3z.0001").unwrap().is_some()
                && s.get_item("kp3z.0002").unwrap().is_some()
        );
        assert!(
            s.get_item("zzzz.9999").unwrap().is_some(),
            "foreign id untouched"
        );
        assert_eq!(
            s.get_item("kp3z.0002")
                .unwrap()
                .unwrap()
                .belongs_to
                .as_deref(),
            Some("kp3z.0001")
        );
        assert_eq!(
            s.deps_of("kp3z.0002").unwrap(),
            vec!["kp3z.0001".to_string()]
        );
        assert_eq!(s.mentions_of("kp3z.0002"), vec!["kp3z.0001".to_string()]);
        assert_eq!(
            s.get_item("kp3z.0002")
                .unwrap()
                .unwrap()
                .description
                .as_deref(),
            Some("done when kp3z.0001 ships")
        );
        assert_eq!(
            s.notes_of("kp3z.0002").unwrap()[0].1,
            "see kp3z.0001 for context"
        );
        assert!(
            !s.export().iter().any(|o| o.target_id.contains("aaaa.")
                || o.value.as_deref().is_some_and(|v| v.contains("aaaa."))),
            "no aaaa.* short-id remains anywhere in the log"
        );
        assert_eq!(s.op_count(), ops_before, "remap adds/drops no ops");
        assert_eq!(s.clock(), clock_before, "remap does not advance the clock");
    }

    /// 6j6v.m19v's acceptance on flow's own registers: a foreign op with a Lamport number near
    /// `i64::MAX` — what anybody who can reach a relay that authenticates nobody can plant — makes
    /// the next local change neither panic nor lose, and it is kept in the log, where history
    /// shows it.
    #[test]
    fn a_foreign_op_near_the_top_of_the_clock_neither_crashes_nor_beats_the_next_local_change() {
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-09-22T10:00:00Z");
        s.create_item("aaaa.0001", "task", "Mine", "x");
        let title = s.export().into_iter().find(|o| o.field == "title").unwrap();
        for (n, lamport) in [i64::MAX, i64::MAX - 1].into_iter().enumerate() {
            s.apply(&[Op {
                op_id: format!("hostile-{n}"),
                lamport,
                site: 9,
                value: Some("planted".into()),
                wall_clock: "2099-01-01T00:00:00Z".into(),
                key_id: None,
                sig: None,
                ..title.clone()
            }]);
        }
        s.set_wall_clock("2026-09-22T11:00:00Z");
        s.set_field("aaaa.0001", "title", Some("changed".into()), "x");
        let item = s.get_item("aaaa.0001").unwrap().unwrap();
        assert_eq!(
            item.title.as_deref(),
            Some("changed"),
            "the next local change wins"
        );
        assert_eq!(
            s.item_timestamps_of("aaaa.0001").unwrap().1.as_deref(),
            Some("2026-09-22T11:00:00Z"),
            "and it, not the planted op, is the item's last change"
        );
        assert_eq!(
            crate::history::item_history(s.connection(), "aaaa.0001")
                .iter()
                .filter(|c| c.value.as_deref() == Some("planted"))
                .count(),
            2,
            "both planted ops are kept, and history shows them"
        );
    }

    /// The remap rewrites ops inside the log (6j6v.pzkb): this replica's own are signed again over
    /// what they now say, so they keep verifying — and a received op the remap had to rewrite is no
    /// longer what its author signed, so it loses its verdict rather than keeping a false one.
    #[test]
    fn remap_prefix_signs_its_own_rewritten_ops_again_and_rejudges_the_rest() {
        use nxs_foundation::signing::Provenance;
        let mut s = Store::open_in_memory(1);
        s.create_item("aaaa.0001", "task", "Mine", "x");
        let mut peer = Store::open_in_memory(2);
        peer.create_item("zzzz.0001", "task", "cites aaaa.0001", "y");
        s.apply(&peer.export());
        s.trust_key(peer.key_id(), "peer", "").unwrap();

        s.remap_prefix("aaaa", "kp3z");

        for op in s.export() {
            let seen = s.op_provenance(&op.op_id).unwrap();
            if op.site == 1 {
                assert_eq!(seen.provenance, Provenance::Own);
                assert!(nxs_foundation::signing::verify(
                    s.key_id(),
                    op.sig.as_deref().unwrap(),
                    &op.canonical_bytes()
                ));
            } else if op.value.as_deref() == Some("cites kp3z.0001") {
                assert_eq!(
                    seen.provenance,
                    Provenance::Invalid,
                    "rewritten, so not what was signed"
                );
                assert!(!seen.acts);
            } else {
                assert_eq!(
                    seen.provenance,
                    Provenance::Verified,
                    "untouched ops keep their verdict"
                );
            }
        }
    }

    #[test]
    fn remap_prefix_is_idempotent_and_a_noop_when_no_local_prefix_present() {
        let mut s = Store::open_in_memory(1);
        s.create_item("aaaa.0001", "task", "T", "x");
        s.remap_prefix("aaaa", "kp3z");
        let after_first = s.get_item("kp3z.0001").unwrap();
        s.remap_prefix("aaaa", "kp3z");
        assert_eq!(s.get_item("kp3z.0001").unwrap(), after_first);
        let snapshot = s.get_item("kp3z.0001").unwrap();
        s.remap_prefix("qqqq", "rrrr");
        assert_eq!(s.get_item("kp3z.0001").unwrap(), snapshot);
    }

    #[test]
    fn merge_is_order_independent() {
        let mut src = Store::open_in_memory(1);
        src.create_item("c1.A", "task", "A", "alice");
        src.create_item("c1.B", "task", "B", "alice");
        src.set_field("c1.A", "status", Some("closed".into()), "alice");
        src.add_edge("c1.A", "c1.B", EdgeKind::Dep, "alice");
        src.add_edge("c1.A", "c1.B", EdgeKind::ContributesTo, "alice");
        src.remove_edge("c1.A", "c1.B", EdgeKind::ContributesTo, "alice");
        src.add_note("c1.A", "a note", "alice");

        let ops = src.export();
        let reversed: Vec<Op> = ops.iter().rev().cloned().collect();

        let mut d1 = Store::open_in_memory(2);
        d1.apply(&ops);
        let mut d2 = Store::open_in_memory(3);
        d2.apply(&reversed);

        for id in ["c1.A", "c1.B"] {
            assert_eq!(
                d1.get_item(id).unwrap(),
                d2.get_item(id).unwrap(),
                "item {id} diverged"
            );
            assert_eq!(
                d1.notes_of(id).unwrap(),
                d2.notes_of(id).unwrap(),
                "notes {id} diverged"
            );
        }
        assert_eq!(
            d1.deps_of("c1.A").unwrap(),
            d2.deps_of("c1.A").unwrap(),
            "deps of c1.A diverged"
        );
    }

    #[test]
    fn description_and_design_round_trip_as_lww_longtext_fields() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "T", "a");
        s.set_field("c1.X", "description", Some("why + goal".into()), "a");
        s.set_field("c1.X", "design", Some("path to the goal".into()), "a");
        let item = s.get_item("c1.X").unwrap().unwrap();
        assert_eq!(item.description.as_deref(), Some("why + goal"));
        assert_eq!(item.design.as_deref(), Some("path to the goal"));
    }

    /// Swap every op each side holds with the other — the two directions of one sync.
    fn exchange(a: &mut Store, b: &mut Store) {
        let (ea, eb) = (a.export(), b.export());
        a.apply(&eb);
        b.apply(&ea);
    }

    fn description(s: &Store, id: &str) -> Option<String> {
        s.get_item(id).unwrap().unwrap().description
    }

    /// Both halves of the order, each where it is the ONLY thing deciding (review of PR #482: the
    /// earlier body let b see a's write first, so b won on lamport and the site half was never
    /// reached).
    #[test]
    fn description_converges_last_writer_wins_by_lamport_then_site() {
        // Lamport first. B has SEEN A's write and writes after it, so B's write carries the higher
        // lamport — and wins although B sits on the LOWER site, where a site-first rule picks A.
        let mut a = Store::open_in_memory(2);
        a.create_item("c1.X", "task", "T", "a");
        a.set_field("c1.X", "description", Some("from-A".into()), "alice");
        let mut b = Store::open_in_memory(1);
        b.apply(&a.export());
        b.set_field("c1.X", "description", Some("from-B".into()), "bob");
        exchange(&mut a, &mut b);
        assert_eq!(description(&a, "c1.X"), description(&b, "c1.X"));
        assert_eq!(
            description(&a, "c1.X").as_deref(),
            Some("from-B"),
            "the higher lamport wins, whatever the sites"
        );

        // Site second. Two writes that did NOT see each other, at the SAME lamport: only the site
        // can decide, and each replica meets the two in the opposite order — so arrival order
        // would leave them disagreeing.
        let mut c = Store::open_in_memory(1);
        c.create_item("c1.Y", "task", "T", "c");
        let mut d = Store::open_in_memory(2);
        d.apply(&c.export());
        assert_eq!(
            c.clock(),
            d.clock(),
            "precondition: both stand at one lamport"
        );
        c.set_field("c1.Y", "description", Some("from-site-1".into()), "carol");
        d.set_field("c1.Y", "description", Some("from-site-2".into()), "dave");
        assert_eq!(
            c.clock(),
            d.clock(),
            "precondition: the two writes carry one lamport"
        );
        exchange(&mut c, &mut d);
        assert_eq!(description(&c, "c1.Y"), description(&d, "c1.Y"));
        assert_eq!(
            description(&c, "c1.Y").as_deref(),
            Some("from-site-2"),
            "at an equal lamport the higher site wins"
        );
    }

    /// "Different fields both survive" at the fold itself (review of PR #482, Test Quality #3).
    /// It was asserted directly only end to end; the property suite compares permutations against
    /// a reference computed by the same fold, so a regression to ROW-level last-writer-wins would
    /// still read as convergent there. Here the concurrent edit of the OTHER field must not erase
    /// the first — on both replicas, whichever the order.
    #[test]
    fn concurrent_edits_of_different_fields_of_one_item_both_survive() {
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.Z", "task", "T", "a");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());
        a.set_field("c1.Z", "description", Some("desc-from-A".into()), "alice");
        b.set_field("c1.Z", "design", Some("design-from-B".into()), "bob");
        exchange(&mut a, &mut b);
        for s in [&a, &b] {
            let item = s.get_item("c1.Z").unwrap().unwrap();
            assert_eq!(item.description.as_deref(), Some("desc-from-A"));
            assert_eq!(item.design.as_deref(), Some("design-from-B"));
        }
    }

    #[test]
    fn local_ops_are_stamped_with_the_task_domain() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.X", "task", "T", "a");
        s.set_field("c1.X", "priority", Some("0".into()), "a");
        s.add_note("c1.X", "n", "a");
        let exported = s.export();
        assert!(!exported.is_empty());
        assert!(
            exported
                .iter()
                .all(|o| o.domain == crate::model::DOMAIN_TASK),
            "every locally emitted op is stamped with the task domain"
        );
    }

    #[test]
    fn foreign_non_task_domain_ops_round_trip_and_defer() {
        let mut s = Store::open_in_memory(1);
        let foreign = Op {
            op_id: "01J0FAKEULID0000000000000".into(),
            lamport: 5,
            site: 2,
            domain: "fact".into(),
            target_kind: "fact".into(),
            target_id: "f.1".into(),
            field: "body".into(),
            op_type: "set".into(),
            value: Some("hello".into()),
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        let deferred = s.apply(std::slice::from_ref(&foreign));
        assert_eq!(
            deferred,
            vec![foreign.clone()],
            "unknown shape is deferred, not folded"
        );
        assert!(
            s.export()
                .iter()
                .any(|o| o.domain == "fact" && o.op_id == foreign.op_id),
            "the foreign op is stored with its domain preserved"
        );

        let before = s.op_count();
        s.refold();
        assert_eq!(s.op_count(), before, "refold drops no ops");
        assert!(
            s.export()
                .iter()
                .any(|o| o.domain == "fact" && o.op_id == foreign.op_id),
            "the foreign op survives a refold with its domain preserved"
        );
    }

    #[test]
    fn design_citation_is_remapped_on_prefix_swap() {
        let mut s = Store::open_in_memory(1);
        s.create_item("aaaa.0001", "project", "Parent", "x");
        s.create_item("aaaa.0002", "task", "Child", "x");
        s.set_field(
            "aaaa.0002",
            "design",
            Some("mirror the approach in aaaa.0001".into()),
            "x",
        );
        s.set_field(
            "aaaa.0002",
            "description",
            Some("supersedes aaaa.0001".into()),
            "x",
        );
        s.remap_prefix("aaaa", "kp3z");
        let item = s.get_item("kp3z.0002").unwrap().unwrap();
        assert_eq!(
            item.design.as_deref(),
            Some("mirror the approach in kp3z.0001")
        );
        assert_eq!(item.description.as_deref(), Some("supersedes kp3z.0001"));
    }

    #[test]
    fn open_fails_loud_on_a_workspace_written_by_a_newer_incompatible_nxs() {
        let path = std::env::temp_dir().join(format!("nxf-skew-fail-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();
        Store::open(p, 1).unwrap(); // a normal workspace at our version
        {
            let conn = Connection::open(p).unwrap();
            let newer = schema::SCHEMA_VERSION + 1;
            conn.pragma_update(None, "user_version", newer).unwrap();
            conn.pragma_update(None, "application_id", newer).unwrap();
        }
        match Store::open(p, 1) {
            Ok(_) => panic!("a workspace from a newer, incompatible nxs must not open"),
            Err(e) => assert!(
                e.to_string().contains("upgrade"),
                "fail-loud open surfaces an actionable message: {e}"
            ),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_succeeds_degraded_on_a_newer_additive_workspace() {
        let path = std::env::temp_dir().join(format!("nxf-skew-degr-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();
        Store::open(p, 1).unwrap();
        {
            let conn = Connection::open(p).unwrap();
            conn.pragma_update(None, "user_version", schema::SCHEMA_VERSION + 1)
                .unwrap(); // floor (application_id) stays 0
        }
        let mut s = Store::open(p, 1).expect("degraded workspace must still open");
        s.create_item("c1.X", "task", "T", "a");
        assert!(
            s.get_item("c1.X").unwrap().is_some(),
            "degraded workspace stays writable on the known axes"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_refuses_a_foreign_file_without_writing_any_nxs_table() {
        let path = std::env::temp_dir().join(format!("nxf-foreign-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();
        {
            let conn = Connection::open(p).unwrap();
            conn.execute_batch("CREATE TABLE foreign_marker(x);")
                .unwrap();
            let newer = schema::SCHEMA_VERSION + 1;
            conn.pragma_update(None, "user_version", newer).unwrap();
            conn.pragma_update(None, "application_id", newer).unwrap();
        }
        match Store::open(p, 1) {
            Ok(_) => panic!("a foreign, incompatible file must not open"),
            Err(e) => assert!(e.to_string().contains("upgrade"), "fail-loud: {e}"),
        }
        let conn = Connection::open(p).unwrap();
        let nxs_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ops'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            nxs_tables, 0,
            "a refused foreign file must get no nxs tables"
        );
        let _ = std::fs::remove_file(&path);
    }

    // ---- op-log-derived item timestamps (6j6v.2kjy) -------------------------

    #[test]
    fn item_timestamps_derive_created_from_first_op_and_updated_from_last() {
        // created_at/updated_at are NOT stored cells — they derive from the op-log's `wall_clock`
        // (the `now` each write stamped). created_at is the wall_clock of the item's FIRST op (its
        // create); updated_at that of its LAST op (any later change targeting the item).
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-07-10T08:00:00Z");
        s.create_item("c1.a", "task", "A", "t"); // first op → created_at
        s.set_wall_clock("2026-07-12T09:30:00Z");
        s.set_field("c1.a", "priority", Some("1".into()), "t"); // later op → updated_at

        let (created, updated) = s.item_timestamps_of("c1.a").unwrap();
        assert_eq!(created.as_deref(), Some("2026-07-10T08:00:00Z"));
        assert_eq!(updated.as_deref(), Some("2026-07-12T09:30:00Z"));
    }

    #[test]
    fn item_timestamps_created_equals_updated_when_never_changed() {
        // A create with no later op ⇒ created_at == updated_at (both the create instant).
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-07-11T00:00:00Z");
        s.create_item("c1.b", "task", "B", "t");
        let (created, updated) = s.item_timestamps_of("c1.b").unwrap();
        assert_eq!(created.as_deref(), Some("2026-07-11T00:00:00Z"));
        assert_eq!(updated.as_deref(), Some("2026-07-11T00:00:00Z"));
    }

    #[test]
    fn item_timestamps_are_scoped_per_item_and_include_notes_not_edges() {
        // updated_at tracks ops whose `target_id` is the item: its own cells, its custom fields, and
        // its worklog notes (all keyed by the plain item id) — but NOT edges/labels, whose op
        // `target_id` is a composite (`from{SEP}to{SEP}kind`), so a later dep add never bumps it.
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-07-10T00:00:00Z");
        s.create_item("c1.a", "task", "A", "t");
        s.create_item("c1.b", "task", "B", "t");
        // A dep add (edge, composite target) at a LATER wall_clock must NOT count as an a-update.
        s.set_wall_clock("2026-07-20T00:00:00Z");
        s.add_edge("c1.a", "c1.b", EdgeKind::Dep, "t");
        // A worklog note on `a` (target_id = the item) at 07-15 DOES count as an update.
        s.set_wall_clock("2026-07-15T00:00:00Z");
        s.add_note("c1.a", "progress", "t");

        let (created, updated) = s.item_timestamps_of("c1.a").unwrap();
        assert_eq!(created.as_deref(), Some("2026-07-10T00:00:00Z"));
        assert_eq!(
            updated.as_deref(),
            Some("2026-07-15T00:00:00Z"),
            "the note (07-15) bumps updated_at; the later edge add (07-20) does not"
        );
    }

    #[test]
    fn item_timestamps_absent_when_no_wall_clock_was_stamped() {
        // A store that never set a wall_clock (a legacy pre-wall_clock op, or a direct test seed)
        // yields (None, None) — the read layer then omits the sparse created_at/updated_at keys.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.a", "task", "A", "t"); // no set_wall_clock ⇒ wall_clock is ''
        assert_eq!(s.item_timestamps_of("c1.a").unwrap(), (None, None));
        // A missing item is likewise (None, None), never an error.
        assert_eq!(s.item_timestamps_of("c1.zzzz").unwrap(), (None, None));
    }

    #[test]
    fn item_timestamps_bulk_matches_singular_and_omits_absent_ids() {
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-07-10T00:00:00Z");
        s.create_item("c1.a", "task", "A", "t");
        s.set_wall_clock("2026-07-12T00:00:00Z");
        s.set_field("c1.a", "status", Some("closed".into()), "t");
        s.set_wall_clock("2026-07-11T00:00:00Z");
        s.create_item("c1.b", "task", "B", "t");
        // c1.c seeded WITHOUT a wall_clock ⇒ (None, None) ⇒ absent from the bulk map.
        s.set_wall_clock("");
        s.create_item("c1.c", "task", "C", "t");

        let bulk = s
            .item_timestamps_of_bulk(&["c1.a", "c1.b", "c1.c", "c1.missing"])
            .unwrap();
        // Bulk == singular for each id it carries (the property the lane decoration relies on).
        assert_eq!(
            bulk.get("c1.a").cloned(),
            Some(s.item_timestamps_of("c1.a").unwrap())
        );
        assert_eq!(
            bulk.get("c1.b").cloned(),
            Some(s.item_timestamps_of("c1.b").unwrap())
        );
        assert_eq!(
            bulk.get("c1.a"),
            Some(&(
                Some("2026-07-10T00:00:00Z".to_string()),
                Some("2026-07-12T00:00:00Z".to_string()),
            )),
        );
        // No-wall_clock and missing ids are simply absent (a miss means "no timestamps"), not errors.
        assert_eq!(
            bulk.get("c1.c"),
            None,
            "an id with no stamped wall_clock is absent"
        );
        assert_eq!(
            bulk.get("c1.missing"),
            None,
            "a missing id is tolerated, absent"
        );
    }

    #[test]
    fn item_timestamps_bulk_empty_input_is_empty_no_query() {
        // An empty id slice must short-circuit to an empty map — never emit a `WHERE ... IN ()`.
        let s = Store::open_in_memory(1);
        assert!(s.item_timestamps_of_bulk(&[]).unwrap().is_empty());
    }

    #[test]
    fn item_timestamps_bulk_spans_multiple_chunks() {
        // Mirrors `get_items_spans_multiple_chunks`/`labels_of_bulk_spans_multiple_chunks`: 550 ids
        // cross the 500-id chunk boundary, so the result must union both chunks — every stamped id
        // back exactly once with its timestamps, nothing dropped or duplicated at the seam.
        let mut s = Store::open_in_memory(1);
        let n = 550;
        let ids: Vec<String> = (0..n).map(|i| format!("c1.{i:04}")).collect();
        for id in &ids {
            s.set_wall_clock("2026-07-10T00:00:00Z");
            s.create_item(id, "task", "t", "t");
        }
        let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let got = s.item_timestamps_of_bulk(&refs).unwrap();
        assert_eq!(
            got.len(),
            n,
            "every id across both chunks is present exactly once"
        );
        assert_eq!(
            got.get("c1.0000"),
            got.get("c1.0549"),
            "an id in the first chunk and one past the 500 boundary both resolve"
        );
        assert_eq!(
            got.get("c1.0500"),
            Some(&(
                Some("2026-07-10T00:00:00Z".to_string()),
                Some("2026-07-10T00:00:00Z".to_string()),
            )),
            "the first id past the chunk boundary carries its stamped timestamps"
        );
    }

    #[test]
    fn item_timestamps_follow_lamport_order_not_wall_clock_magnitude() {
        // `wall_clock` is display-only, NOT the ordering key — the `(lamport, site)` op order is. Prove
        // created/updated derive from the FIRST/LAST op in that causal order even when a LATER op
        // carries an EARLIER calendar wall_clock (clock skew / a backdated `now`). A naive
        // MIN/MAX(wall_clock) would invert both ends: it passes every monotonic test but fails here.
        let mut s = Store::open_in_memory(1);
        s.set_wall_clock("2026-07-10T00:00:00Z"); // the create (lowest lamport)
        s.create_item("c1.a", "task", "A", "t");
        s.set_wall_clock("2026-07-01T00:00:00Z"); // EARLIER calendar time, but a LATER op (higher lamport)
        s.set_field("c1.a", "priority", Some("1".into()), "t");

        let (created, updated) = s.item_timestamps_of("c1.a").unwrap();
        assert_eq!(
            created.as_deref(),
            Some("2026-07-10T00:00:00Z"),
            "created = the FIRST op by lamport, not min(wall_clock)"
        );
        assert_eq!(
            updated.as_deref(),
            Some("2026-07-01T00:00:00Z"),
            "updated = the LAST op by lamport, not max(wall_clock)"
        );
    }
}
