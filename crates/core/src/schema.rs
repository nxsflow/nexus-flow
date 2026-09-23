//! flow's materialized views — the `items`/`edge_adds`/`edge_removes`/`notes`/`note_tombstones`
//! tables + the `present_edges` view that the task reducer folds into and derivation reads. The
//! `ops` log + schema-version spine are the substrate's; this module re-exports them so existing
//! `nexus_flow_core::schema::*` paths keep resolving, and owns only the flow-vocabulary DDL.

use rusqlite::Connection;

// The substrate's schema-version spine, re-exported (the compatibility axis is foundation-wide).
pub use nxs_foundation::schema::{
    assess_open, db_min_compatible, migrate, schema_version, SchemaCompat,
    MIN_COMPATIBLE_SCHEMA_VERSION, SCHEMA_VERSION,
};

/// Whitelisted LWW columns on `items` (guards the dynamic-column SQL in the store).
///
/// `belongs_to` was removed in sp6.3: hierarchy parenthood is no longer a single-parent LWW
/// register but a `parent` edge in the OR-set, projected to a single current parent by the
/// `present_parent` view. A migrated DB keeps a stale `belongs_to` column (harmless, never read);
/// a fresh DB has none. No write path sets it — `store.set_field` rejects any field not in this
/// list, so the register cannot be resurrected.
pub const ITEM_LWW_FIELDS: &[&str] = &[
    "type",
    "title",
    "completion_criterion",
    "description",
    "design",
    "status",
    "priority",
    "due",
    "defer_until",
    "assignee",
    "closing_comment",
    "deleted",
    // C1 (#916.1): the archive tombstone — a nullable ISO-8601 TIMESTAMP (not a bool), folded as
    // an LWW cell exactly like `deleted`. The instant is stored so the `archived` lane (C3) can
    // sort by archive-date descending. Adding it here extends the `items` DDL (fresh DBs) and the
    // generic fold path; existing DBs gain the column via the foundation v1→v2 migration.
    "archived",
    // C3 (#916.4): the close instant — a nullable ISO-8601 TIMESTAMP folded as an LWW cell, set by
    // `close` so the `closed` lane can sort by close-date descending (recency). Modelled exactly
    // like `archived`; fresh DBs get it from this DDL, existing DBs from the foundation v2→v3
    // migration.
    "closed_at",
];

/// Apply flow's materialized views, panicking on failure. For in-memory stores and tests. Returns
/// the same "upgraded a pre-custom-fields workspace" flag as [`try_apply_flow_views`].
pub fn apply_flow_views(conn: &Connection) -> bool {
    try_apply_flow_views(conn).expect("flow views")
}

/// Apply flow's materialized views (the `_v`/`_site` columns carry the LWW Lamport metadata per
/// cell). Created over the substrate's `ops` log; the task reducer refreshes them.
///
/// Returns `true` when it **upgraded** a pre-custom-fields workspace — one whose `items` view
/// already exists but which has no `custom_fields` view yet (spec §7, the additive `custom_fields`
/// minor). The gate is computed BEFORE the CREATE batch installs `custom_fields`: a fresh db has
/// neither table (→ `false`, the O(1) reopen path); a re-open under this binary has both
/// (→ `false`); only an OLD (pre-CF) workspace has `items` but not `custom_fields` (→ `true`). The
/// caller (`Store::open`) uses the flag to force a one-time refold so any `field set` op that
/// store-don't-folded under the pre-CF binary surfaces into `custom_fields` (mirrors nexus-chat's
/// M2 deadline minor — see `ChatStore::open`).
pub fn try_apply_flow_views(conn: &Connection) -> rusqlite::Result<bool> {
    // The upgrade gate, read BEFORE the CREATE batch below installs `custom_fields`: a pre-CF
    // workspace has `items` but not `custom_fields`; a fresh db has neither; a reopen has both.
    let items_existed = table_exists(conn, "items")?;
    let custom_fields_existed = table_exists(conn, "custom_fields")?;
    let mut cols = String::new();
    for f in ITEM_LWW_FIELDS {
        cols.push_str(&format!(
            "{f} TEXT, {f}_v INTEGER DEFAULT 0, {f}_site INTEGER DEFAULT 0,\n"
        ));
    }
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS items(
             id TEXT PRIMARY KEY,
             {cols}
             _placeholder INTEGER DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS edge_adds(
             from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
             tag TEXT PRIMARY KEY
         );
         CREATE TABLE IF NOT EXISTS edge_removes(tag TEXT PRIMARY KEY);

         -- Labels (h89s.1): a SECOND observed-remove OR-set, modelled exactly like the edge one
         -- above. An add's `tag` is its op_id; a remove tombstones the observed add-tags. A label is
         -- an opaque user string attached to an item — user vocabulary, distinct from the plugin's
         -- display type. `present_labels` is adds-minus-removes.
         CREATE TABLE IF NOT EXISTS label_adds(
             item_id TEXT NOT NULL, label TEXT NOT NULL,
             tag TEXT PRIMARY KEY
         );
         CREATE TABLE IF NOT EXISTS label_removes(tag TEXT PRIMARY KEY);

         -- Thread links (nxf 6j6v.8dbe): a THIRD observed-remove OR-set, modelled exactly like the
         -- edge and label ones above — the element is a (thread, item) pair carrying two independent
         -- attributes, `relation` (worked_on/cited) and `weight` (bearing/passing).
         --
         -- Its own OR-set and NOT a `mentions` edge with a namespaced endpoint, because
         -- `present_edges` is item↔item by INVARIANT, not by convention: `invariant.rs`'s
         -- `dangling_edge_violations` flags every present-edge endpoint of kind<>'parent' that does
         -- not resolve to a live item, so a thread endpoint there would be a permanent violation —
         -- or would force weakening a check that today catches genuine dangling references. The
         -- prefix-remap pass (`Store::remap_target`) is the second reason: it rewrites BOTH endpoints
         -- of an `edge` composite, and a thread id is not in the item id space. Lifting the thread
         -- into the item space instead would put a non-work object on the board — visible in `list`,
         -- countable as a child, needing a plugin-declared type, a candidate in `ready`.
         CREATE TABLE IF NOT EXISTS thread_link_adds(
             thread_id TEXT NOT NULL, item_id TEXT NOT NULL,
             relation  TEXT NOT NULL, weight  TEXT NOT NULL,
             tag TEXT PRIMARY KEY
         );
         CREATE TABLE IF NOT EXISTS thread_link_removes(tag TEXT PRIMARY KEY);

         CREATE TABLE IF NOT EXISTS notes(
             id TEXT PRIMARY KEY,
             item_id TEXT NOT NULL,
             author TEXT,
             body TEXT
         );
         CREATE TABLE IF NOT EXISTS note_tombstones(note_id TEXT PRIMARY KEY);

         -- Plugin custom fields (6j6v epic): a folded EAV view over the op-log, ONE row per
         -- (item, custom field) that has ever been set. `value` is TEXT like every canonical LWW
         -- cell; the `value_v`/`value_site` pair carries the LWW Lamport metadata the keep-if-beats
         -- fold compares on. Kept OFF `items` on purpose so `items`/`ItemRow`/`CanonicalItem` stay
         -- byte-identical (spec §2, TB-CF-1). The PK indexes `item_id` leftmost, so T4's bulk
         -- `WHERE item_id IN (…)` read is covered with no extra index.
         CREATE TABLE IF NOT EXISTS custom_fields(
             item_id TEXT NOT NULL, field TEXT NOT NULL, value TEXT,
             value_v INTEGER DEFAULT 0, value_site INTEGER DEFAULT 0,
             PRIMARY KEY(item_id, field)
         );

         CREATE VIEW IF NOT EXISTS present_edges AS
             SELECT DISTINCT from_id, to_id, kind FROM edge_adds a
             WHERE NOT EXISTS (SELECT 1 FROM edge_removes r WHERE r.tag = a.tag);

         -- Labels (h89s.1): the present (adds-minus-removes) label set per item.
         CREATE VIEW IF NOT EXISTS present_labels AS
             SELECT DISTINCT item_id, label FROM label_adds a
             WHERE NOT EXISTS (SELECT 1 FROM label_removes r WHERE r.tag = a.tag);

         -- Thread links (nxf 6j6v.8dbe): the present links, with ONE (relation, weight) per
         -- (thread, item) pair. Among a pair's present adds the winner is the one whose add-op has
         -- the max (lamport, site, tag) — the SAME projection idiom `present_parent` below uses, and
         -- for the same reason: it is a pure, convergent function of the converged add set.
         --
         -- This is what makes a link MOVABLE, which the ticket requires and a plain OR-set element
         -- would not give: re-attaching the same pair with a different weight is an ordinary later
         -- add that wins causally, so a link can firm up (passing -> bearing) or soften without a
         -- remove-then-add dance and without inventing per-attribute LWW registers. Detaching is the
         -- normal observed-remove: it tombstones every live add for the pair, whatever its
         -- attributes, so the pair leaves this view entirely.
         CREATE VIEW IF NOT EXISTS present_thread_links AS
             SELECT a.thread_id, a.item_id, a.relation, a.weight
             FROM thread_link_adds a JOIN ops o ON o.op_id = a.tag
             WHERE NOT EXISTS (SELECT 1 FROM thread_link_removes r WHERE r.tag = a.tag)
               AND NOT EXISTS (
                   SELECT 1 FROM thread_link_adds a2 JOIN ops o2 ON o2.op_id = a2.tag
                   WHERE a2.thread_id = a.thread_id AND a2.item_id = a.item_id
                     AND NOT EXISTS (SELECT 1 FROM thread_link_removes r2 WHERE r2.tag = a2.tag)
                     AND (o2.lamport, o2.site, a2.tag) > (o.lamport, o.site, a.tag));

         -- sp6.3: the single current parent per child, projected from the `parent` OR-set edge.
         -- Among a child's present `parent` edges, the winner is the one whose add-op has the max
         -- (lamport, site, to_id) — the LWW-equivalent of the former `belongs_to` register, and a
         -- pure, convergent function of the converged edge set. Under the ②a single-parent write
         -- shim there is normally exactly one; the tiebreak keeps it deterministic in the transient
         -- multi-parent state a concurrent merge can produce. `get_item`/`list_items` read
         -- `parent_id AS belongs_to` from here, and `invariant` joins it — so every belongs_to
         -- reader stays unchanged while the substrate becomes n:m-capable.
         CREATE VIEW IF NOT EXISTS present_parent AS
             SELECT a.from_id AS child_id, a.to_id AS parent_id
             FROM edge_adds a JOIN ops o ON o.op_id = a.tag
             WHERE a.kind = 'parent'
               AND NOT EXISTS (SELECT 1 FROM edge_removes r WHERE r.tag = a.tag)
               AND NOT EXISTS (
                   SELECT 1 FROM edge_adds a2 JOIN ops o2 ON o2.op_id = a2.tag
                   WHERE a2.from_id = a.from_id AND a2.kind = 'parent'
                     AND NOT EXISTS (SELECT 1 FROM edge_removes r2 WHERE r2.tag = a2.tag)
                     AND (o2.lamport, o2.site, a2.to_id) > (o.lamport, o.site, a.to_id));

         -- Per-item read indexes (perf, 1w5v). The projection tables PRIMARY-KEY on the op-id `tag`,
         -- so the hot per-item lookups scanned the WHOLE table on every call: `labels_of` (via the
         -- `present_labels` view's `WHERE item_id=?`), `notes_of`/`show` (via `notes.item_id`), and
         -- `deps_of` (via `present_edges`'s `from_id`). Decorating a board lane runs these once PER
         -- item — `next_to_value`/`list_to_value` append the per-row labels join and `deps_with_status`
         -- the per-row deps join — so the full scans dominated the read cost. These turn each into an
         -- index lookup; `edge_adds_from` also serves the `present_parent` tiebreak, which self-joins
         -- on `from_id`. `edge_adds(to_id)` indexes the reverse direction — the native dependents read
         -- (`dependents_count`/`children_of`, both `WHERE to_id=?`).
         -- Additive + IF NOT EXISTS, and the flow schema is re-applied on every `Store::open`, so
         -- every existing workspace gains them on next open — no schema-version bump.
         CREATE INDEX IF NOT EXISTS label_adds_item ON label_adds(item_id);
         CREATE INDEX IF NOT EXISTS notes_item ON notes(item_id);
         CREATE INDEX IF NOT EXISTS edge_adds_from ON edge_adds(from_id);
         CREATE INDEX IF NOT EXISTS edge_adds_to ON edge_adds(to_id);
         -- Both directions of the thread-link OR-set (nxf 6j6v.8dbe): `WHERE item_id=?` decorates a
         -- board lane (once per row in `next`/`list`), `WHERE thread_id=?` enumerates one thread's
         -- items. Both are per-item/per-thread lookups on the hot read path, so neither may be a
         -- full scan — same reasoning as `label_adds_item` above.
         CREATE INDEX IF NOT EXISTS thread_link_adds_item ON thread_link_adds(item_id);
         CREATE INDEX IF NOT EXISTS thread_link_adds_thread ON thread_link_adds(thread_id);"
    ))?;
    Ok(items_existed && !custom_fields_existed)
}

/// Whether a table exists — used to tell a fresh db (where the CREATE batch installs `custom_fields`
/// and there is nothing to resurface) from a pre-custom-fields workspace whose `items` view predates
/// the `custom_fields` view and must force a one-time refold.
fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn schema_creates_all_objects() {
        // The substrate creates `ops`; flow creates its views over it. Together they are the full
        // flow schema a `Store` opens.
        let conn = Connection::open_in_memory().unwrap();
        nxs_foundation::schema::apply(&conn);
        apply_flow_views(&conn);
        let names = table_names(&conn);
        for expected in [
            "ops",
            "items",
            "edge_adds",
            "edge_removes",
            "label_adds",
            "label_removes",
            "notes",
            "note_tombstones",
            "custom_fields",
            "thread_link_adds",
            "thread_link_removes",
            "present_edges",
            "present_labels",
            "present_parent",
            "present_thread_links",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn fresh_db_reports_no_custom_fields_upgrade() {
        // A fresh workspace has neither `items` nor `custom_fields` before the CREATE batch, so the
        // batch installs both and nothing needs resurfacing — the O(1) reopen path (no force-refold).
        let conn = Connection::open_in_memory().unwrap();
        assert!(
            !try_apply_flow_views(&conn).unwrap(),
            "fresh db: no pre-CF upgrade"
        );
        assert!(
            !try_apply_flow_views(&conn).unwrap(),
            "re-apply is idempotent: both tables now exist → still no upgrade"
        );
    }

    #[test]
    fn a_pre_custom_fields_workspace_reports_an_upgrade() {
        // The additive `custom_fields` minor (spec §7): an OLD workspace has the `items` view but no
        // `custom_fields` view. Applying the views installs `custom_fields` AND reports `true` so the
        // caller force-refolds once, resurfacing any `field set` op the pre-CF binary skipped.
        let conn = Connection::open_in_memory().unwrap();
        // Simulate the pre-CF shape: `items` present, `custom_fields` absent. The minimal `items`
        // shape is enough — the gate keys on table existence, not columns.
        conn.execute_batch("CREATE TABLE items(id TEXT PRIMARY KEY);")
            .unwrap();
        assert!(
            !table_exists(&conn, "custom_fields").unwrap(),
            "precondition: no custom_fields view yet"
        );
        assert!(
            try_apply_flow_views(&conn).unwrap(),
            "items present, custom_fields absent → upgrade"
        );
        assert!(table_exists(&conn, "custom_fields").unwrap());
        assert!(
            !try_apply_flow_views(&conn).unwrap(),
            "second apply: both tables present → no re-upgrade (O(1) reopen)"
        );
    }

    #[test]
    fn per_item_read_indexes_exist_and_are_used() {
        // 1w5v: the hot per-item lookups (`labels_of`, `notes_of`, `deps_of`, `dependents_count`)
        // must be index lookups, not full table scans of the op-keyed projection tables. Assert the
        // indexes exist AND that the planner actually uses each on its real production read (a missing
        // index would plan a `SCAN`; the index plans a `SEARCH ... USING INDEX <name>`).
        let conn = Connection::open_in_memory().unwrap();
        nxs_foundation::schema::apply(&conn);
        apply_flow_views(&conn);

        let mut idx_stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' ORDER BY name")
            .unwrap();
        let index_names: Vec<String> = idx_stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for idx in [
            "label_adds_item",
            "notes_item",
            "edge_adds_from",
            "edge_adds_to",
            "thread_link_adds_item",
            "thread_link_adds_thread",
        ] {
            assert!(
                index_names.contains(&idx.to_string()),
                "missing index {idx}"
            );
        }

        // Each of the four indexes must actually be USED by its real production read — a missing
        // index plans a full `SCAN`, the index plans a `SEARCH ... USING INDEX <name>`. The SQL
        // below is verbatim from the cited method, so a future join change that regresses one of
        // these reads back to a full scan reds here instead of only slowing down silently.
        let plan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> String {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            stmt.query_map(params, |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .join(" | ")
        };

        // `labels_of` — the per-row labels join in `next_to_value`/`list_to_value`.
        let p = plan(
            "SELECT label FROM present_labels WHERE item_id=?1 ORDER BY label",
            &[&"x"],
        );
        assert!(p.contains("label_adds_item"), "labels_of plan: {p}");

        // `labels_of_bulk` (1w5v) — the ONE-query lane decoration. The `item_id IN (…)` must ride
        // the same index (a SEARCH per listed id), not regress to a full SCAN of label_adds; that is
        // what makes N→1 a real per-row win and not just one big scan.
        let p = plan(
            "SELECT item_id, label FROM present_labels WHERE item_id IN (?1,?2) \
             ORDER BY item_id, label",
            &[&"x", &"y"],
        );
        assert!(p.contains("label_adds_item"), "labels_of_bulk plan: {p}");

        // `custom_fields_of_bulk` (6j6v.ekf5; ky26-review Test Quality #2) — the ONE-query lane
        // decoration for plugin custom fields. The `item_id IN (…)` must ride the `custom_fields`
        // PRIMARY KEY `(item_id, field)` (item_id leftmost) as a SEARCH, NOT regress to a full SCAN —
        // that is what keeps the populated-fields lane read bulk (one indexed query) rather than an
        // N+1 over `custom_fields_of`. A missing/regressed index would plan a `SCAN custom_fields`.
        let p = plan(
            "SELECT item_id, field, value FROM custom_fields \
             WHERE item_id IN (?1,?2) AND value <> '' ORDER BY item_id, field",
            &[&"x", &"y"],
        );
        assert!(
            p.contains("SEARCH custom_fields") && !p.contains("SCAN custom_fields"),
            "custom_fields_of_bulk must SEARCH the (item_id, field) PK, not SCAN: {p}"
        );

        // `notes_of` — the `show` notes read.
        let p = plan(
            "SELECT n.id, n.body FROM notes n JOIN ops o ON o.op_id = n.id
             WHERE n.item_id=?1
               AND n.id NOT IN (SELECT note_id FROM note_tombstones)
             ORDER BY o.lamport, o.site",
            &[&"x"],
        );
        assert!(p.contains("notes_item"), "notes_of plan: {p}");

        // `deps_of`/`targets_of_result` — the per-dep read behind `deps_with_status`.
        let p = plan(
            "SELECT to_id FROM present_edges WHERE from_id=?1 AND kind=?2 ORDER BY to_id",
            &[&"x", &"dep"],
        );
        assert!(p.contains("edge_adds_from"), "deps_of plan: {p}");

        // `dependents_count` — the reverse fan-out read (`WHERE to_id=?`).
        let p = plan(
            "SELECT COUNT(*) FROM present_edges d JOIN items i ON i.id=d.from_id
               WHERE d.to_id=?1 AND d.kind='dep'
                 AND COALESCE(i.deleted,'0')<>'1' AND i.status<>'closed'",
            &[&"x"],
        );
        assert!(p.contains("edge_adds_to"), "dependents_count plan: {p}");

        // `thread_links_of_item` (nxf 6j6v.8dbe) — the per-row conversation decoration on a board
        // lane, run once per item in `next`/`list`, so a full scan of the whole link set here is the
        // exact regression `label_adds_item` exists to prevent one lane over.
        let p = plan(
            "SELECT thread_id, relation, weight FROM present_thread_links \
             WHERE item_id=?1 ORDER BY thread_id",
            &[&"x"],
        );
        assert!(
            p.contains("thread_link_adds_item"),
            "thread_links_of_item plan: {p}"
        );

        // `items_of_thread` — the other direction: one thread enumerating its board items.
        let p = plan(
            "SELECT item_id, relation, weight FROM present_thread_links \
             WHERE thread_id=?1 ORDER BY item_id",
            &[&"m-1"],
        );
        assert!(
            p.contains("thread_link_adds_thread"),
            "items_of_thread plan: {p}"
        );
    }

    #[test]
    fn flow_views_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply_flow_views(&conn);
        apply_flow_views(&conn); // must not panic on second apply
    }
}
