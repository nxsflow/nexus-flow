//! memory's store: the [`nxs_foundation::store::Store`] op-log substrate plus memory's `fact`
//! vocabulary folded over it (mirrors `nexus-flow-core`'s `Store`). This newtype wraps the
//! substrate (delegating the op-log surface — `apply`/`export`/`refold`/…) and adds the
//! remember/recall/memories/forget helpers. memory's fact reducer is registered at open, so every
//! op-log path folds `fact` ops into the `memories` view.
//!
//! memory is the task reducer's sibling, not its dependent: this store depends ONLY on the
//! foundation — no flow vocabulary (spec §4.4). Local ops always ride the `fact` domain.

use crate::fact_reducer::FactReducer;
use crate::model::{
    split_refs, MemoryRow, Scope, CATEGORY_UNSORTED, CONVENTIONAL_CATEGORIES, DOMAIN_FACT,
    FACT_FIELD, FACT_KIND, FIELD_CATEGORY, FIELD_INTRODUCTION, FIELD_MIGRATION, FIELD_ORDINAL,
    FIELD_REFS, FIELD_SCOPE, OP_FORGET, OP_SET,
};
use crate::schema;
use nxs_foundation::model::Op;
use nxs_foundation::store::Store as Substrate;
use rusqlite::{Connection, OptionalExtension};

/// memory's `view_watermarks.store_id` — the key under which its folded-through watermark is
/// tracked in the shared substrate (aye.36). Distinct from flow's so the two products fold the
/// SAME log into their own views and advance independently.
const MEMORY_VIEWS: &str = "memory";

/// memory's store over the shared substrate.
pub struct MemoryStore {
    inner: Substrate,
}

impl MemoryStore {
    pub fn open_in_memory(site: i64) -> MemoryStore {
        let mut inner = Substrate::open_in_memory(site);
        schema::apply_memory_views(inner.connection());
        inner.register_reducer(Box::new(FactReducer));
        // Bind memory's folded-through watermark and refold if the shared log advanced out-of-band
        // (aye.36). A fresh in-memory log is empty, so this only binds the key here; the file-backed
        // `open` is where a multi-device pull actually triggers the refold.
        inner.refold_if_behind(MEMORY_VIEWS);
        MemoryStore { inner }
    }

    /// File-backed store. Fallible: a missing-parent path, a corrupt/foreign file, or a workspace a
    /// newer incompatible nxs wrote returns the (handled) sqlite error from the substrate's skew
    /// gate — surfaced before memory touches the file.
    pub fn open(path: &str, site: i64) -> rusqlite::Result<MemoryStore> {
        let mut inner = Substrate::open(path, site)?;
        schema::try_apply_memory_views(inner.connection())?;
        inner.register_reducer(Box::new(FactReducer));
        Self::materialize_views(&mut inner);
        Ok(MemoryStore { inner })
    }

    /// Bind memory's view to the shared log after opening (aye.36; the upgrade half from the review
    /// of PR #381, nxf 6j6v.xbnh).
    ///
    /// Steady state is the O(1) [`refold_if_behind`](Substrate::refold_if_behind): if the shared log
    /// advanced past memory's folded-through watermark (a foundation-only sync pull landed `fact`
    /// ops without folding them), rebuild the view once; otherwise reads stay on the pure view.
    ///
    /// **When THIS open migrated the view's schema, refold unconditionally instead** — the same
    /// move `Store::materialize_views` makes in flow (`custom_fields`) and chat (the M2 deadline),
    /// for the same reason. A register column added by a migration retroactively makes a class of
    /// op foldable that the previous binary stored-don't-folded (§7), and those ops sit BELOW the
    /// watermark that binary already advanced. `refold_if_behind` sees itself as caught up and
    /// never revisits them, so a peer that received a foreign `introduction` (or `category`,
    /// `scope`, `refs`, `ordinal`) op before upgrading would lose it permanently. Memory had no
    /// such branch at all until now, which made that true for the v5 registers as well.
    ///
    /// Gated on the version the FILE carried, not on the presence of a column: `Substrate::open`
    /// migrates before it returns, so the column is already there by the time this runs — see
    /// [`Substrate::migrated_from`].
    fn materialize_views(inner: &mut Substrate) {
        if inner.migrated_from() < nxs_foundation::schema::SCHEMA_VERSION {
            inner.force_refold(MEMORY_VIEWS);
        } else {
            inner.refold_if_behind(MEMORY_VIEWS);
        }
    }

    // ---- substrate delegation -------------------------------------------------

    /// Set the wall-clock timestamp stamped onto subsequently emitted LOCAL ops (the `updated`
    /// display value; never load-bearing for ordering — that is the Lamport/site pair).
    pub fn set_wall_clock(&mut self, now: &str) {
        self.inner.set_wall_clock(now);
    }

    pub fn connection(&self) -> &Connection {
        self.inner.connection()
    }

    /// The current Lamport clock (highest lamport emitted/observed).
    pub fn clock(&self) -> i64 {
        self.inner.clock()
    }

    pub fn op_count(&self) -> i64 {
        self.inner.op_count()
    }

    /// SQLite's `PRAGMA data_version` for this connection (the reactivity primitive).
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

    /// Re-fold the durable log into the memories view (resurface, §7).
    pub fn refold(&mut self) {
        self.inner.refold();
    }

    /// Emit one LOCAL `fact`-domain op through the substrate. memory is the fact reducer: every op
    /// it originates rides the `fact` domain/kind/`body` field (spec §2.1).
    fn emit_fact(&mut self, key: &str, op_type: &str, value: Option<String>, author: &str) {
        self.inner.emit(
            DOMAIN_FACT,
            FACT_KIND,
            key,
            FACT_FIELD,
            op_type,
            value,
            author,
        );
    }

    // ---- memory write helpers -------------------------------------------------

    /// Upsert the memory at `key` to `body` (spec §5.1). Keep-if-beats LWW: a later remember under
    /// the same key wins in place; a remember after a forget revives. `key` is the resolved key —
    /// the CLI mints the content-hash auto-key ([`crate::key::auto_key`]) when none is given.
    pub fn remember(&mut self, key: &str, body: &str, author: &str) {
        self.emit_fact(key, OP_SET, Some(body.to_string()), author);
    }

    /// Forget the memory at `key`: a reversible tombstone (body→NULL, active→0). A later remember
    /// (higher lamport) revives it (spec §3.1). The store-level forget is pure — it emits the op
    /// unconditionally; the CLI checks existence for a friendly `not_found`.
    pub fn forget(&mut self, key: &str, author: &str) {
        self.emit_fact(key, OP_FORGET, None, author);
    }

    /// Set the memory's category — the section it belongs to (6j6v.e0z6). Its own LWW register, so
    /// this never disturbs the body, `author` or `updated`: the fact did not change, only where it
    /// is filed. Pure like [`forget`](Self::forget) — the facade validates the slug.
    pub fn set_category(&mut self, key: &str, category: &str, author: &str) {
        self.emit_register(key, FIELD_CATEGORY, category.to_string(), author);
    }

    /// Set how far the memory reaches (6j6v.e0z6). Storing and addressing the reach is the engine's
    /// job; DECIDING which memory deserves which reach is the product's, so nothing here judges.
    pub fn set_scope(&mut self, key: &str, scope: Scope, author: &str) {
        self.emit_register(key, FIELD_SCOPE, scope.as_str().to_string(), author);
    }

    /// Set the board items the memory is about (6j6v.e0z6). `refs` must already be canonical
    /// ([`crate::model::normalize_refs`]) — that is what lets two writers naming the same items in
    /// different orders converge instead of fighting over the register.
    pub fn set_refs(&mut self, key: &str, refs: &[String], author: &str) {
        self.emit_register(key, FIELD_REFS, refs.join(","), author);
    }

    /// Set the memory's explicit reading position (6j6v.e0z6) — the deliberate, rarely-run
    /// `reorder`'s only write. Everything else leaves the ordinal alone, so an unordered memory
    /// keeps sorting by insertion order.
    pub fn set_ordinal(&mut self, key: &str, ordinal: i64, author: &str) {
        self.emit_register(key, FIELD_ORDINAL, ordinal.to_string(), author);
    }

    /// Set the memory's written introduction (6j6v.xbnh) — the ONE line the session bootstrap
    /// replays for it. Its own register, so rewriting the introduction leaves the body, `author` and
    /// `updated` alone: the fact did not change, only the line that points at it. Pure like the
    /// classification setters — the facade validates the length.
    pub fn set_introduction(&mut self, key: &str, introduction: &str, author: &str) {
        self.emit_register(key, FIELD_INTRODUCTION, introduction.to_string(), author);
    }

    /// Leave the judging migration's mark on this stream (6j6v.9yaj).
    ///
    /// A `fact` op on the reserved [`MARK_TARGET`](crate::migration::MARK_TARGET) carrying the
    /// [`FIELD_MIGRATION`] field — which no register column answers to, so the reducer stores it and
    /// never folds it (§7). The mark is therefore durable, syncs to every device on the stream, and
    /// materializes nothing: a second device can see that the run happened without the bookkeeping
    /// appearing as a memory in `prime` or in the generated document.
    pub fn mark_migration(&mut self, value: &str, author: &str) {
        self.inner.emit_unfolded(
            DOMAIN_FACT,
            FACT_KIND,
            crate::migration::MARK_TARGET,
            FIELD_MIGRATION,
            OP_SET,
            Some(value.to_string()),
            author,
        );
    }

    /// The latest migration mark on this stream, or `None` if no run ever happened.
    ///
    /// Read straight off the durable `ops` log rather than a view, because the mark deliberately has
    /// no view: `(lamport, site)` descending is the same total order the keep-if-beats registers
    /// use, so two devices that both marked converge on the same answer to "who ran it".
    pub fn migration_mark(&self) -> rusqlite::Result<Option<String>> {
        self.conn()
            .query_row(
                // Within the Lamport bound (6j6v.m19v): an op past it takes part in no fold, and
                // this read is the fold of a register that has no view.
                "SELECT value FROM ops
                 WHERE domain=?1 AND target_kind=?2 AND target_id=?3 AND field=?4
                   AND lamport <= ?5
                 ORDER BY lamport DESC, site DESC LIMIT 1",
                rusqlite::params![
                    DOMAIN_FACT,
                    FACT_KIND,
                    crate::migration::MARK_TARGET,
                    FIELD_MIGRATION,
                    nxs_foundation::model::MAX_LAMPORT
                ],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
    }

    /// Emit one non-body register op. Same domain/kind as every other fact op; only the
    /// `field` selects which register it writes.
    fn emit_register(&mut self, key: &str, field: &str, value: String, author: &str) {
        self.inner.emit(
            DOMAIN_FACT,
            FACT_KIND,
            key,
            field,
            OP_SET,
            Some(value),
            author,
        );
    }

    // ---- memory reads ---------------------------------------------------------

    /// The raw memory row at `key`, INCLUDING an inactive (forgotten) tombstone — the existence
    /// probe `forget` checks. `recall`/`memories` filter to active rows. Fallible (#76u.13): a real
    /// db error is now surfaced (via [`OptionalExtension::optional`]), distinct from a genuinely
    /// absent row (`Ok(None)`) — resolving the prior `.ok()` swallow that mapped BOTH to "not found"
    /// (jo9). The seam maps the db error to `io`; an absent key still reads as `not_found`.
    pub fn get(&self, key: &str) -> rusqlite::Result<Option<MemoryRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM memories WHERE key=?1"),
                [key],
                Self::row,
            )
            .optional()
    }

    /// The active memory at `key`, or nothing (forgotten or never-remembered). Fallible (#76u.13):
    /// propagates a db error from [`get`](Self::get) rather than swallowing it.
    pub fn recall(&self, key: &str) -> rusqlite::Result<Option<MemoryRow>> {
        Ok(self.get(key)?.filter(|m| m.active))
    }

    /// The active memories a [`MemoryQuery`] selects (spec §5.1).
    ///
    /// `category`/`scope` narrow in SQL (exact match on the stored register). `search` is filtered
    /// in Rust: the store is a deliberately small, hand-curated set (research §1/§7), so an exact,
    /// escaping-free substring match over the fetched rows is both correct and clear. The order is
    /// `key` unless the caller asks for [reading order](MemoryQuery::ordered).
    pub fn memories(&self, query: &MemoryQuery) -> rusqlite::Result<Vec<MemoryRow>> {
        let mut sql = format!("SELECT {COLUMNS} FROM memories WHERE active=1");
        let mut binds: Vec<String> = Vec::new();
        if let Some(category) = &query.category {
            binds.push(category.clone());
            sql.push_str(&format!(" AND category=?{}", binds.len()));
        }
        if let Some(scope) = query.scope {
            binds.push(scope.as_str().to_string());
            sql.push_str(&format!(" AND scope=?{}", binds.len()));
        }
        // The reading order binds the category names it ranks by, so it is built AFTER the filters
        // and appends to the same list — the `?N` indices stay in step with `binds` either way.
        let order = if query.ordered {
            reading_order(&mut binds)
        } else {
            "key".to_string()
        };
        sql.push_str(&format!(" ORDER BY {order}"));

        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(binds.iter()), Self::row)?
            .collect::<rusqlite::Result<Vec<MemoryRow>>>()?;
        Ok(match query.search.as_deref().map(str::to_lowercase) {
            None => rows,
            Some(q) => rows
                .into_iter()
                .filter(|m| {
                    m.key.to_lowercase().contains(&q)
                        || m.body.as_deref().unwrap_or("").to_lowercase().contains(&q)
                })
                .collect(),
        })
    }

    fn conn(&self) -> &Connection {
        self.inner.connection()
    }

    /// Map a `memories` row to a [`MemoryRow`] (the [`COLUMNS`] projection, in order).
    fn row(r: &rusqlite::Row) -> rusqlite::Result<MemoryRow> {
        Ok(MemoryRow {
            key: r.get(0)?,
            body: r.get(1)?,
            author: r.get(2)?,
            updated: r.get(3)?,
            active: r.get::<_, i64>(4)? != 0,
            category: r.get(5)?,
            scope: r.get(6)?,
            refs: split_refs(&r.get::<_, String>(7)?),
            ordinal: r.get(8)?,
            introduction: r.get(9)?,
        })
    }
}

/// The projection every read shares — the column order [`MemoryStore::row`] decodes.
const COLUMNS: &str =
    "key, body, author, updated, active, category, scope, refs, ordinal, introduction";

/// Reading order (6j6v.e0z6, completed by 6j6v.643z): **the category first, then the position
/// within it.** An [ordinal](crate::model::FIELD_ORDINAL) has always been a position *within a
/// category*, so ranking by it alone read only half the stored order — an `introduction` nobody had
/// reordered would fall behind a `rules` entry that carried an ordinal.
///
/// The category rank is the argument a memory document makes:
/// [`CONVENTIONAL_CATEGORIES`] first, in their declared order (what the workspace *is*, then the
/// idea that carries it, then the hard rules), then every other category alphabetically, and
/// [`CATEGORY_UNSORTED`] last — what nobody has filed yet belongs at the end, not wherever its slug
/// happens to sort.
///
/// Within a category: explicitly placed memories first, in the position `reorder` gave them, then
/// everything never reordered in insertion order. `created_v`/`created_site` are the earliest op's
/// `(lamport, site)`; `key` is the final tiebreak, so the order is total and stable.
///
/// **Why the tail is the alphabet and not the ordinals** (6j6v.6v4n, decided): behind the
/// conventional head the order is a lookup key, not a judgement — `agent-tooling` precedes
/// `build-test-gotchas` because `a` precedes `b`. Deriving the rank from the positions instead (a
/// category ranked by its members' lowest ordinal) is tempting, because a workspace whose whole
/// bestand was placed by ONE `reorder` call carries a global sequence that looks like a statement
/// about its categories. It is not one, and taking it for one would be a bug:
///
/// * `reorder` numbers exactly the keys it is handed, `1..n`, and leaves every other position alone.
///   Two calls therefore both start at 1 — reachable through the public verb, and pinned by
///   `two_memories_sharing_an_ordinal_still_read_in_a_stable_order`. Ordinals minted by different
///   calls are not comparable, so a cross-category comparison would rank sections by how the user
///   happened to batch their calls.
/// * An ordinal has always been a position *within* a category
///   (`one_reorder_across_categories_places_each_memory_within_its_own_section` pins that the
///   ranking never compares them across sections). Deriving the section order from them would
///   contradict the very invariant that makes a global numbering safe.
/// * And it is undefined the moment a category holds nothing but unplaced memories, which is the
///   normal state of a category somebody just invented.
///
/// So the alphabet stays: deterministic, explainable in one sentence, and stable under every write.
/// Recording a genuine judgement about section order needs a position *per category* — a register
/// no memory can carry — and no consumer has asked for one. `the_order_between_categories_is_the_
/// alphabet_and_not_the_positions` is the fence around this decision.
///
/// Returned as SQL with the ranked category names appended to `binds` as parameters rather than
/// interpolated — the ORDER BY carries data, and data belongs in a bind even when today's source is
/// a `&'static str` constant.
fn reading_order(binds: &mut Vec<String>) -> String {
    let mut arms = String::new();
    for (rank, category) in CONVENTIONAL_CATEGORIES.iter().enumerate() {
        binds.push((*category).to_string());
        arms.push_str(&format!(" WHEN ?{} THEN {rank}", binds.len()));
    }
    // Everything unconventional shares one rank and is separated by `category` below; `unsorted`
    // gets its own, one past it.
    let other = CONVENTIONAL_CATEGORIES.len();
    binds.push(CATEGORY_UNSORTED.to_string());
    arms.push_str(&format!(" WHEN ?{} THEN {}", binds.len(), other + 1));
    format!(
        "CASE category{arms} ELSE {other} END, category, \
         ordinal IS NULL, ordinal, created_v, created_site, key"
    )
}

/// What a memory read selects and how it is ordered (6j6v.e0z6). The default — no filter, key order
/// — is exactly the read `recall`/`memories`/`prime` always performed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryQuery {
    /// Case-insensitive substring over key **and** body (parity with bd).
    pub search: Option<String>,
    /// Only memories filed under this category.
    pub category: Option<String>,
    /// Only memories with this reach.
    pub scope: Option<Scope>,
    /// Sort by the stored [reading order](reading_order) — category, then position within it —
    /// instead of by key.
    pub ordered: bool,
}

impl MemoryQuery {
    /// The plain substring search — the shape `nxm memories <search>` and `memory_search` use.
    pub fn search(term: impl Into<String>) -> MemoryQuery {
        MemoryQuery {
            search: Some(term.into()),
            ..MemoryQuery::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DOMAIN_FACT, FACT_FIELD, FACT_KIND, OP_SET};
    use nxs_foundation::model::Op;

    /// The migration mark is read straight off the log, newest `(lamport, site)` first — a fold of a
    /// register with no view — so an op past the Lamport bound is no mark (6j6v.m19v; review of PR
    /// #489, Test Quality #3): a planted "somebody already ran it" cannot stop this device's run.
    #[test]
    fn a_mark_past_the_lamport_bound_is_not_the_migration_mark() {
        let mut s = MemoryStore::open_in_memory(1);
        s.mark_migration("{\"actor\":\"this device\"}", "me");
        s.apply(&[Op {
            op_id: "planted".into(),
            lamport: i64::MAX,
            site: 9,
            domain: DOMAIN_FACT.into(),
            target_kind: FACT_KIND.into(),
            target_id: crate::migration::MARK_TARGET.into(),
            field: FIELD_MIGRATION.into(),
            op_type: OP_SET.into(),
            value: Some("{\"actor\":\"planted\"}".into()),
            author: "mallory".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }]);
        assert_eq!(
            s.migration_mark().unwrap().as_deref(),
            Some("{\"actor\":\"this device\"}")
        );
    }

    /// Every table a memory store keeps is `FactReducer`'s view — the one a snapshot would carry
    /// and `clear_views` empties (6j6v.mxt2, review of PR #487, Code Quality #4). A table added to
    /// the schema is red here until somebody decides whether another replica may receive it.
    #[test]
    fn every_table_in_a_memory_store_is_a_view_of_the_fact_reducer() {
        use nxs_foundation::reducer::Reducer as _;
        let store = MemoryStore::open_in_memory(1);
        let tables: Vec<String> = store
            .connection()
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let views = crate::fact_reducer::FactReducer.view_tables();
        for t in &tables {
            assert!(
                nxs_foundation::schema::SUBSTRATE_TABLES.contains(&t.as_str())
                    || views.contains(&t.as_str()),
                "{t} is not a view of the fact reducer — add it to view_tables or say why not"
            );
        }
        assert!(views.iter().all(|v| tables.iter().any(|t| t == v)));
    }

    fn task_op(id: &str) -> Op {
        // A foreign (flow) op — the cross-domain-isolation probe.
        Op {
            op_id: format!("task-{id}"),
            lamport: 1,
            site: 9,
            domain: "task".into(),
            target_kind: "item".into(),
            target_id: id.into(),
            field: "title".into(),
            op_type: "set".into(),
            value: Some("a task".into()),
            author: "bob".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    #[test]
    fn remember_then_recall_returns_the_active_body() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("auth-jwt", "auth uses JWT not sessions", "alice");
        let m = s.recall("auth-jwt").unwrap().expect("recall finds it");
        assert_eq!(m.body.as_deref(), Some("auth uses JWT not sessions"));
        assert!(m.active);
        assert_eq!(m.author, "alice");
    }

    #[test]
    fn remembering_a_stable_key_updates_in_place() {
        // bd parity (research §4b): the SAME key overwrites in place — one entity, not a duplicate.
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("auth-jwt", "auth uses JWT", "alice");
        s.remember(
            "auth-jwt",
            "auth uses JWT (HS256), refresh in Redis",
            "alice",
        );
        assert_eq!(
            s.recall("auth-jwt").unwrap().unwrap().body.as_deref(),
            Some("auth uses JWT (HS256), refresh in Redis"),
            "later write wins the LWW register"
        );
        assert_eq!(
            s.memories(&MemoryQuery::default()).unwrap().len(),
            1,
            "one entity under the stable key"
        );
    }

    #[test]
    fn forget_hides_from_recall_and_memories_but_keeps_the_tombstone() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "secret", "alice");
        s.forget("k", "alice");
        assert!(
            s.recall("k").unwrap().is_none(),
            "forgotten → recall finds nothing"
        );
        assert!(
            s.memories(&MemoryQuery::default()).unwrap().is_empty(),
            "forgotten → not listed"
        );
        // The tombstone row survives (revive carrier) but is inactive.
        let raw = s
            .get("k")
            .unwrap()
            .expect("the tombstone row is still present");
        assert!(!raw.active);
        assert_eq!(raw.body, None, "forget NULLed the body (spec §3.2)");
    }

    #[test]
    fn forgetting_an_already_forgotten_key_stays_inactive() {
        // Spec §5.1 names the inactive case explicitly: forgetting again is a no-op on visible
        // state — the memory stays forgotten (recall/memories empty), never resurrected.
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "v", "alice");
        s.forget("k", "alice");
        s.forget("k", "alice"); // already forgotten
        assert!(
            s.recall("k").unwrap().is_none(),
            "still forgotten after a second forget"
        );
        assert!(s.memories(&MemoryQuery::default()).unwrap().is_empty());
        let raw = s.get("k").unwrap().expect("tombstone row present");
        assert!(!raw.active && raw.body.is_none());
    }

    #[test]
    fn re_remember_after_forget_revives() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "v1", "alice");
        s.forget("k", "alice");
        s.remember("k", "v2", "alice");
        let m = s.recall("k").unwrap().expect("revived");
        assert_eq!(m.body.as_deref(), Some("v2"));
        assert!(
            m.active,
            "a later remember (higher lamport) revives (spec §3.1)"
        );
    }

    #[test]
    fn memories_are_key_sorted_and_substring_searchable() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember(
            "dolt-phantoms",
            "Dolt phantom DBs hide in three places",
            "alice",
        );
        s.remember("auth-jwt", "auth uses JWT", "alice");
        s.remember("race", "always run tests with -race", "alice");

        let keys: Vec<String> = s
            .memories(&MemoryQuery::default())
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        assert_eq!(
            keys,
            ["auth-jwt", "dolt-phantoms", "race"],
            "deterministic key order"
        );

        // Substring over key AND body, case-insensitive (research §2).
        let hits: Vec<String> = s
            .memories(&MemoryQuery::search("DOLT"))
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        assert_eq!(
            hits,
            ["dolt-phantoms"],
            "matches the key case-insensitively"
        );
        let hits: Vec<String> = s
            .memories(&MemoryQuery::search("jwt"))
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        assert_eq!(hits, ["auth-jwt"], "matches the body");
    }

    #[test]
    fn updated_carries_the_wall_clock_of_the_write() {
        let mut s = MemoryStore::open_in_memory(1);
        s.set_wall_clock("2026-06-20T10:00:00Z");
        s.remember("k", "v", "alice");
        assert_eq!(
            s.recall("k").unwrap().unwrap().updated,
            "2026-06-20T10:00:00Z"
        );
    }

    #[test]
    fn lww_converges_by_lamport_then_site() {
        let mut a = MemoryStore::open_in_memory(1);
        a.remember("k", "from-A", "alice");
        let mut b = MemoryStore::open_in_memory(2);
        b.apply(&a.export()); // b learns the memory
        b.remember("k", "from-B", "bob"); // higher lamport

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);
        assert_eq!(
            a.recall("k").unwrap().unwrap().body,
            b.recall("k").unwrap().unwrap().body
        );
        assert_eq!(
            a.recall("k").unwrap().unwrap().body.as_deref(),
            Some("from-B"),
            "higher lamport wins"
        );
    }

    #[test]
    fn merge_of_concurrent_remember_and_forget_is_order_independent() {
        // The §3.2 convergence case: a forget and a concurrent remember must converge to the SAME
        // state under any delivery order (this is exactly what forget→NULL buys).
        let mut src = MemoryStore::open_in_memory(1);
        src.remember("k", "v1", "a");
        src.remember("k", "v2", "a");
        src.forget("k", "a");
        let ops = src.export();
        let reversed: Vec<Op> = ops.iter().rev().cloned().collect();

        let mut d1 = MemoryStore::open_in_memory(2);
        d1.apply(&ops);
        let mut d2 = MemoryStore::open_in_memory(3);
        d2.apply(&reversed);
        assert_eq!(
            d1.get("k").unwrap(),
            d2.get("k").unwrap(),
            "diverged across delivery order"
        );
    }

    // ---- classification + order (6j6v.e0z6) --------------------------------

    fn keys(rows: &[MemoryRow]) -> Vec<&str> {
        rows.iter().map(|m| m.key.as_str()).collect()
    }

    #[test]
    fn classification_survives_a_body_update_and_a_forget_revive() {
        // Where a memory is filed is not part of what it SAYS: rewording it, forgetting it and
        // remembering it again all leave the classification standing.
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("auth", "v1", "alice");
        s.set_category("auth", "rules", "alice");
        s.set_scope("auth", Scope::Global, "alice");
        s.set_refs("auth", &["6j6v.e0z6".to_string()], "alice");

        s.remember("auth", "v2", "alice");
        s.forget("auth", "alice");
        s.remember("auth", "v3", "alice");

        let m = s.recall("auth").unwrap().unwrap();
        assert_eq!(m.body.as_deref(), Some("v3"));
        assert_eq!(m.category, "rules");
        assert_eq!(m.scope, "global");
        assert_eq!(m.refs, ["6j6v.e0z6"]);
    }

    #[test]
    fn classifying_does_not_move_the_author_or_the_updated_stamp() {
        // `updated` dates the FACT. Filing a memory is not a change to the fact, so the stamp — the
        // one a reader uses to judge how current the knowledge is — must not drift.
        let mut s = MemoryStore::open_in_memory(1);
        s.set_wall_clock("2026-06-20T10:00:00Z");
        s.remember("auth", "uses JWT", "alice");
        s.set_wall_clock("2026-08-03T12:00:00Z");
        s.set_category("auth", "rules", "bob");

        let m = s.recall("auth").unwrap().unwrap();
        assert_eq!(
            m.updated, "2026-06-20T10:00:00Z",
            "the fact is as old as it was"
        );
        assert_eq!(m.author, "alice", "the fact's author, not the filer");
        assert_eq!(m.category, "rules");
    }

    #[test]
    fn reads_can_narrow_by_category_and_by_reach() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("intro", "what this workspace is", "alice");
        s.set_category("intro", "introduction", "alice");
        s.remember("race", "always run tests with -race", "alice");
        s.set_category("race", "rules", "alice");
        s.set_scope("race", Scope::Global, "alice");
        s.remember("loose", "not filed yet", "alice");

        let by_category = MemoryQuery {
            category: Some("rules".into()),
            ..MemoryQuery::default()
        };
        assert_eq!(keys(&s.memories(&by_category).unwrap()), ["race"]);

        let by_scope = MemoryQuery {
            scope: Some(Scope::Project),
            ..MemoryQuery::default()
        };
        assert_eq!(
            keys(&s.memories(&by_scope).unwrap()),
            ["intro", "loose"],
            "project is the reach an unclassified memory already had"
        );

        // The reserved default is countable — the signal the later judging migration works from.
        let unsorted = MemoryQuery {
            category: Some(crate::model::CATEGORY_UNSORTED.into()),
            ..MemoryQuery::default()
        };
        assert_eq!(keys(&s.memories(&unsorted).unwrap()), ["loose"]);
    }

    #[test]
    fn reading_order_places_reordered_memories_first_then_insertion_order() {
        // The whole point of a stored order: a curated document reads in the order someone chose,
        // and anything nobody has placed yet appends in the order it was written — never
        // alphabetically, which is what `key` order would impose.
        let mut s = MemoryStore::open_in_memory(1);
        for key in ["zeta", "alpha", "mid", "omega"] {
            s.remember(key, "body", "alice");
        }
        s.set_ordinal("omega", 1, "alice");
        s.set_ordinal("mid", 2, "alice");

        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        assert_eq!(
            keys(&s.memories(&ordered).unwrap()),
            ["omega", "mid", "zeta", "alpha"],
            "placed first in their positions, then the unplaced in insertion order"
        );
        assert_eq!(
            keys(&s.memories(&MemoryQuery::default()).unwrap()),
            ["alpha", "mid", "omega", "zeta"],
            "the default read is untouched: still key order"
        );
    }

    #[test]
    fn reading_order_leads_with_the_conventional_categories_and_ends_with_the_unfiled() {
        // 6j6v.643z: an ordinal is a position WITHIN a category, so ranking by it alone read only
        // half the stored order. The whole order is the argument a memory document makes — what the
        // workspace IS, then the idea that carries it, then the hard rules — with everything else
        // alphabetical behind it and whatever nobody filed yet at the very end.
        let mut s = MemoryStore::open_in_memory(1);
        for (key, category) in [
            ("loose", None),
            ("gate", Some("rules")),
            ("crates", Some("architecture")),
            ("what", Some("introduction")),
            ("cut", Some("release-process")),
            ("flake", Some("build-test")),
            ("branch", Some("rules")),
        ] {
            s.remember(key, "body", "alice");
            if let Some(category) = category {
                s.set_category(key, category, "alice");
            }
        }
        // A position within `rules`, and one inside a category that ranks BEHIND two unordered ones
        // — the case a flat ordinal read got wrong.
        s.set_ordinal("branch", 1, "alice");
        s.set_ordinal("gate", 2, "alice");

        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        assert_eq!(
            keys(&s.memories(&ordered).unwrap()),
            [
                "what",   // introduction
                "crates", // architecture
                "branch", "gate",  // rules, in the positions `reorder` gave them
                "flake", // build-test — the rest, alphabetically by category
                "cut",   // release-process
                "loose", // unsorted, last whatever its slug
            ],
            "conventional categories lead, the rest sort by category name, unsorted ends"
        );
    }

    #[test]
    fn the_order_between_categories_is_the_alphabet_and_not_the_positions() {
        // 6j6v.6v4n, the decision this test fences. Behind the conventional head the section order
        // is a lookup key; it deliberately does NOT follow the ordinals, however much a single
        // global numbering looks like it should. Here `zebra` carries the two lowest positions in
        // the store and `alpha` none at all — a rank derived from "the lowest ordinal among a
        // category's members" would lead with `zebra`, and would then answer differently depending
        // on how the caller batched their `reorder` calls, since each call numbers from 1.
        let mut s = MemoryStore::open_in_memory(1);
        for (key, category) in [
            ("z-first", "zebra"),
            ("z-second", "zebra"),
            ("a-only", "alpha"),
        ] {
            s.remember(key, "body", "alice");
            s.set_category(key, category, "alice");
        }
        s.set_ordinal("z-first", 1, "alice");
        s.set_ordinal("z-second", 2, "alice");

        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        assert_eq!(
            keys(&s.memories(&ordered).unwrap()),
            ["a-only", "z-first", "z-second"],
            "`alpha` leads on its name alone, despite holding no position at all"
        );

        // And placing the last unplaced memory — the one write that WOULD move a derived rank —
        // leaves the section order exactly where it was.
        s.set_ordinal("a-only", 9, "alice");
        assert_eq!(
            keys(&s.memories(&ordered).unwrap()),
            ["a-only", "z-first", "z-second"],
            "a position is a place within a section, never a claim about the section"
        );
    }

    #[test]
    fn a_narrowed_read_can_be_ordered_too_and_the_filter_binds_stay_in_step() {
        // PR #291 review, Test Quality #1. `nxm memories --category <c> --ordered` is a documented
        // combination, and it is the one place where the filters' `?N` binds and the ones
        // [`reading_order`] appends share a list. Get the arithmetic wrong by one and the read
        // silently narrows on a category name used as an ORDER BY rank — or fails outright. Both
        // filters are set here BECAUSE two preceding binds is the case that goes wrong.
        let mut s = MemoryStore::open_in_memory(1);
        for (key, category, scope) in [
            ("gate", "rules", Scope::Project),
            ("branch", "rules", Scope::Project),
            ("shared", "rules", Scope::Global),
            ("crates", "architecture", Scope::Project),
        ] {
            s.remember(key, "body", "alice");
            s.set_category(key, category, "alice");
            s.set_scope(key, scope, "alice");
        }
        s.set_ordinal("branch", 1, "alice");

        let narrowed_and_ordered = MemoryQuery {
            category: Some("rules".into()),
            scope: Some(Scope::Project),
            ordered: true,
            ..MemoryQuery::default()
        };
        assert_eq!(
            keys(&s.memories(&narrowed_and_ordered).unwrap()),
            ["branch", "gate"],
            "both filters still narrow (no `shared`, no `crates`) AND the order is the reading one"
        );
        // The same narrowing without the order: the SET must be identical, so a difference can only
        // ever be the sequence.
        let narrowed = MemoryQuery {
            ordered: false,
            ..narrowed_and_ordered.clone()
        };
        assert_eq!(keys(&s.memories(&narrowed).unwrap()), ["branch", "gate"]);
        // …and with the placement inverted, the two reads genuinely disagree — proof that
        // `ordered` is doing work here rather than coinciding with the alphabet.
        s.set_ordinal("gate", 0, "alice");
        assert_eq!(
            keys(&s.memories(&narrowed_and_ordered).unwrap()),
            ["gate", "branch"]
        );
        assert_eq!(keys(&s.memories(&narrowed).unwrap()), ["branch", "gate"]);
    }

    #[test]
    fn the_ordered_read_converges_across_replicas_whatever_the_delivery_order() {
        // PR #291 review, Test Quality #2 / Integrity #2. The doc comments claim the two-level order
        // is made of stored facts and therefore byte-stable across replicas. `reading_order_survives
        // _a_refold` proves that for ONE replica's log; this proves it for two that filed and placed
        // memories independently — and for a third that receives the identical ops in the opposite
        // order. This project's bar is that a convergence property is tested, not assumed.
        let mut a = MemoryStore::open_in_memory(1);
        for key in ["intro", "gate", "branch", "flake", "cut", "loose"] {
            a.remember(key, "body", "alice");
        }
        let mut b = MemoryStore::open_in_memory(2);
        b.apply(&a.export());

        // Now the two file and place CONCURRENTLY, each unaware of the other's writes.
        a.set_category("intro", "introduction", "alice");
        a.set_category("gate", "rules", "alice");
        a.set_category("branch", "rules", "alice");
        a.set_ordinal("branch", 1, "alice");
        a.set_ordinal("gate", 2, "alice");
        b.set_category("flake", "build-test", "bob");
        b.set_category("cut", "release-process", "bob");
        b.set_ordinal("cut", 1, "bob");

        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        let expected = [
            "intro", // introduction
            "branch", "gate",  // rules, in A's positions
            "flake", // build-test
            "cut",   // release-process
            "loose", // unsorted, never filed by either
        ];
        assert_eq!(keys(&a.memories(&ordered).unwrap()), expected);
        assert_eq!(
            keys(&b.memories(&ordered).unwrap()),
            expected,
            "the replica that never saw the other's writes first reads the same document"
        );

        // A third replica receiving the SAME ops in the opposite order agrees as well — the order is
        // a property of the log's content, not of when each op happened to arrive.
        let mut c = MemoryStore::open_in_memory(3);
        let reversed: Vec<Op> = ea.iter().chain(eb.iter()).rev().cloned().collect();
        c.apply(&reversed);
        assert_eq!(keys(&c.memories(&ordered).unwrap()), expected);
    }

    #[test]
    fn reading_order_survives_a_refold() {
        // Insertion order lives in the log, not just in the view: wipe the view, rebuild it from the
        // ops, and the order is the same. This is what makes the order a fact rather than an
        // artefact of how the rows happened to be written.
        let mut s = MemoryStore::open_in_memory(1);
        for key in ["zeta", "alpha", "mid"] {
            s.remember(key, "body", "alice");
        }
        s.set_ordinal("mid", 1, "alice");
        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        let before = keys(&s.memories(&ordered).unwrap())
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>();
        s.refold();
        assert_eq!(keys(&s.memories(&ordered).unwrap()), before);
        assert_eq!(before, ["mid", "zeta", "alpha"]);
    }

    #[test]
    fn classification_ops_ride_the_fact_domain_on_their_own_fields() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "v", "alice");
        s.set_category("k", "rules", "alice");
        s.set_scope("k", Scope::Item, "alice");
        s.set_refs("k", &["a".to_string(), "b".to_string()], "alice");
        s.set_ordinal("k", 3, "alice");

        let ops = s.export();
        assert!(
            ops.iter()
                .all(|o| o.domain == DOMAIN_FACT && o.target_kind == FACT_KIND),
            "every op still rides the fact domain/kind"
        );
        let fields: Vec<&str> = ops.iter().map(|o| o.field.as_str()).collect();
        assert_eq!(fields, ["body", "category", "scope", "refs", "ordinal"]);
        let refs_op = ops.iter().find(|o| o.field == "refs").unwrap();
        assert_eq!(refs_op.value.as_deref(), Some("a,b"), "the canonical list");
    }

    // ---- cross-domain isolation (aye.20 acceptance) ------------------------

    #[test]
    fn a_task_op_is_stored_but_not_folded_into_memories() {
        // The fact reducer folds NO task ops; the foundation store-don't-folds the foreign domain
        // (spec §4.2/§7). The op IS persisted (never dropped) but never touches the memories view.
        let mut s = MemoryStore::open_in_memory(1);
        let deferred = s.apply(&[task_op("t.1")]);
        assert_eq!(
            deferred.len(),
            1,
            "the task op is deferred (no fact reducer for it)"
        );
        assert_eq!(s.op_count(), 1, "but it IS persisted in the shared log");
        assert!(
            s.memories(&MemoryQuery::default()).unwrap().is_empty(),
            "no cross-contamination of memories"
        );
        assert!(s.get("t.1").unwrap().is_none());
    }

    #[test]
    fn an_unknown_fact_shape_is_stored_not_folded() {
        // §7 within the fact domain: a fact-domain op the current build cannot fold is kept, not
        // dropped — refoldable once a later version understands it.
        let mut s = MemoryStore::open_in_memory(1);
        let weird = Op {
            op_id: "f-weird".into(),
            lamport: 1,
            site: 1,
            domain: DOMAIN_FACT.into(),
            target_kind: FACT_KIND.into(),
            target_id: "k".into(),
            field: FACT_FIELD.into(),
            op_type: "future_op".into(),
            value: Some("v".into()),
            author: "a".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        let deferred = s.apply(std::slice::from_ref(&weird));
        assert_eq!(deferred.len(), 1, "unknown fact shape deferred");
        assert_eq!(s.op_count(), 1, "stored, not dropped");
        assert!(s.get("k").unwrap().is_none(), "not materialized");
    }

    #[test]
    fn local_ops_are_stamped_with_the_fact_domain() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "v", "a");
        s.forget("k", "a");
        let ops = s.export();
        assert!(!ops.is_empty());
        assert!(
            ops.iter().all(|o| o.domain == DOMAIN_FACT
                && o.target_kind == FACT_KIND
                && o.field == FACT_FIELD),
            "every emitted memory op rides the fact domain/kind/field"
        );
        assert_eq!(
            ops.iter().filter(|o| o.op_type == OP_SET).count(),
            1,
            "one remember"
        );
    }

    #[test]
    fn refold_rebuilds_memories_purely_from_the_log() {
        let mut s = MemoryStore::open_in_memory(1);
        s.remember("k", "v", "a");
        let before = s.get("k").unwrap();
        s.connection()
            .execute_batch("DELETE FROM memories;")
            .unwrap();
        assert!(s.get("k").unwrap().is_none(), "view wiped");
        s.refold();
        assert_eq!(
            s.get("k").unwrap(),
            before,
            "rebuilt from the durable op-log"
        );
    }

    /// A foreign `fact` op as it would ride the sync wire — built by hand so the test can land it
    /// in a shared log via a raw substrate (no fact reducer), mirroring a foundation-only pull.
    fn fact_op(op_id: &str, key: &str, body: &str) -> Op {
        Op {
            op_id: op_id.into(),
            lamport: 7,
            site: 2,
            domain: DOMAIN_FACT.into(),
            target_kind: FACT_KIND.into(),
            target_id: key.into(),
            field: FACT_FIELD.into(),
            op_type: OP_SET.into(),
            value: Some(body.into()),
            author: "device-A".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    #[test]
    fn refold_on_open_materializes_fact_ops_from_a_foundation_only_pull() {
        // aye.36: nxs sync moves the shared op-log but, foundation-only, folds no product views. A
        // fact op that lands out-of-band must materialize into `memories` the next time the memory
        // store opens — refold-when-behind, driven by the folded-through watermark.
        use nxs_foundation::store::Store as Substrate;

        let path = std::env::temp_dir().join(format!("nxm-aye36-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();

        // Device-local memory, folded inline by the memory store.
        {
            let mut m = MemoryStore::open(p, 1).unwrap();
            m.remember("k0", "v0", "alice");
        }
        // A foundation-only pull: a RAW substrate store (no fact reducer) appends a foreign fact op.
        // It lands in the shared log unfolded and does not touch the memory store's watermark.
        {
            let mut sub = Substrate::open(p, 2).unwrap();
            let deferred = sub.apply(std::slice::from_ref(&fact_op(
                "f.k1",
                "k1",
                "from-device-A",
            )));
            assert_eq!(
                deferred.len(),
                1,
                "the raw substrate defers the fact op (no reducer)"
            );
        }
        // The next memory open is behind the log → refold-on-open folds k1 in, keeping k0.
        {
            let m = MemoryStore::open(p, 1).unwrap();
            assert_eq!(
                m.recall("k1").unwrap().unwrap().body.as_deref(),
                Some("from-device-A"),
                "the out-of-band fact op is materialized on the next memory open"
            );
            assert_eq!(m.recall("k0").unwrap().unwrap().body.as_deref(), Some("v0"));
        }
        // Idempotent: a further open with no new ops leaves the view unchanged.
        {
            let m = MemoryStore::open(p, 1).unwrap();
            assert_eq!(
                m.memories(&MemoryQuery::default()).unwrap().len(),
                2,
                "stable across a needless reopen"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_round_trips_a_memory_across_reopen() {
        let path = std::env::temp_dir().join(format!("nxm-test-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();
        {
            let mut s = MemoryStore::open(p, 1).unwrap();
            s.remember("k", "v1", "a");
        }
        {
            let mut s = MemoryStore::open(p, 1).unwrap();
            assert_eq!(
                s.recall("k").unwrap().unwrap().body.as_deref(),
                Some("v1"),
                "persisted"
            );
            s.remember("k", "v2", "a"); // clock recovered → this must win
            assert_eq!(s.recall("k").unwrap().unwrap().body.as_deref(), Some("v2"));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_a_pre_v6_workspace_force_refolds_a_deferred_introduction_op() {
        // **The bug class flow and chat already have a test for, reaching memory** (review of PR
        // #381, Test Quality #2). A peer on a pre-6j6v.xbnh binary receives a foreign
        // `introduction` op through a sync pull. It cannot fold it — no such register exists in its
        // build — so the op is store-don't-fold (§7), and the peer's watermark advances past it
        // anyway (it folded everything else in that pull). The peer then upgrades. A plain
        // `refold_if_behind` sees itself as caught up and never revisits the op, so the written
        // introduction is lost from that peer's view FOREVER while every other replica shows it.
        //
        // `MemoryStore::materialize_views` is what closes it: this open migrated the file's schema,
        // so it refolds unconditionally.
        use nxs_foundation::store::Store as Substrate;

        let path = std::env::temp_dir().join(format!("nxm-v6-{}.db", ulid::Ulid::new()));
        let p = path.to_str().unwrap();

        {
            // Build the pre-v6 state directly in the shared db: the substrate at v5, memory's view
            // WITHOUT the introduction column, a body op folded, and the introduction op sitting
            // unfolded below an already-advanced watermark.
            let sub = Substrate::open(p, 1).unwrap();
            let conn = sub.connection();
            schema::apply_memory_views(conn);
            for column in ["introduction", "introduction_v", "introduction_site"] {
                conn.execute_batch(&format!("ALTER TABLE memories DROP COLUMN {column};"))
                    .unwrap();
            }
            conn.pragma_update(None, "user_version", 5).unwrap();
            conn.execute(
                "INSERT INTO memories(key, body, author, updated, active, v, site)
                 VALUES('auth', 'auth uses JWT', 'peer-a', '', 1, 1, 2)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                 op_type, value, author, wall_clock)
                 VALUES('op-intro', 2, 2, 'fact', 'fact', 'auth', 'introduction', 'set',
                        'auth is JWT, not sessions', 'peer-a', '')",
                [],
            )
            .unwrap();
            // The pre-v6 binary folded everything it understood and advanced the watermark to the
            // log boundary — which is what makes `refold_if_behind` alone a no-op here.
            conn.execute(
                "INSERT INTO view_watermarks(store_id, folded_through)
                 VALUES('memory', (SELECT MAX(rowid) FROM ops))",
                [],
            )
            .unwrap();
        }

        let s = MemoryStore::open(p, 1).unwrap();
        assert_eq!(
            s.recall("auth").unwrap().unwrap().introduction.as_deref(),
            Some("auth is JWT, not sessions"),
            "the deferred introduction op is resurfaced by the schema bump"
        );
        // …and the steady-state reopen does NOT refold again: it migrated nothing, so it takes the
        // cheap branch. Asserted through behaviour rather than a counter — the value survives.
        let s = MemoryStore::open(p, 1).unwrap();
        assert_eq!(
            s.recall("auth").unwrap().unwrap().introduction.as_deref(),
            Some("auth is JWT, not sessions")
        );
        let _ = std::fs::remove_file(&path);
    }
}
