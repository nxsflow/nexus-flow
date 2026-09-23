//! flow's task reducer (spec §4.2/§4.4): folds `task`-domain ops into the item/edge/note views.
//! The first registered reducer — the substrate dispatches `task` ops here. The fold rules are
//! flow's E1 view-fold, unchanged: keep-if-beats LWW on item cells, observed-remove OR-set on
//! edges, grow-only tombstones on note redactions.
//!
//! Stateless: the views live in the substrate's db, so a unit struct suffices. (In a later phase
//! this file moves to the `nexus-flow` crate while the substrate + [`Reducer`] trait stay in the
//! foundation — the reducer is exactly the seam that makes that split clean.)

use crate::model::{EdgeKind, LinkRelation, LinkWeight, MergeStrategy, Op};
use crate::reducer::Reducer;
use crate::schema;
use crate::store::SEP;
use rusqlite::{params, Connection};

/// flow's reducer over the `task` domain.
pub struct TaskReducer;

impl TaskReducer {
    /// Parse an edge op's composite `target_id` (`from{SEP}to{SEP}kind`) — `None` if malformed.
    /// Used both to decide foldability and to fold, so no fold path can index-panic on bad input.
    fn parse_edge(op: &Op) -> Option<(String, String, String)> {
        let parts: Vec<&str> = op.target_id.split(SEP).collect();
        match parts.as_slice() {
            [from, to, kind] if EdgeKind::parse(kind).is_some() => {
                Some((from.to_string(), to.to_string(), kind.to_string()))
            }
            _ => None,
        }
    }

    /// Parse a label op's composite `target_id` (`item_id{SEP}label`) — `None` if malformed.
    /// Symmetric with [`parse_edge`](Self::parse_edge); an empty label is rejected so a bare
    /// separator or trailing-empty component can never fold into the OR-set.
    fn parse_label(op: &Op) -> Option<(String, String)> {
        let parts: Vec<&str> = op.target_id.split(SEP).collect();
        match parts.as_slice() {
            [item_id, label] if !label.is_empty() => Some((item_id.to_string(), label.to_string())),
            _ => None,
        }
    }

    /// Parse a thread-link op's composite `target_id`
    /// (`thread_id{SEP}item_id{SEP}relation{SEP}weight`) — `None` if malformed (nxf 6j6v.8dbe).
    /// Symmetric with [`parse_edge`](Self::parse_edge), including its "the attribute must be a
    /// RECOGNIZED token" discipline: an unknown relation/weight never folds, so a hostile or
    /// future-versioned peer cannot inject an attribute value the read paths would then have to
    /// cope with. Empty endpoints are rejected for [`parse_label`](Self::parse_label)'s reason.
    fn parse_thread_link(op: &Op) -> Option<(String, String, String, String)> {
        let parts: Vec<&str> = op.target_id.split(SEP).collect();
        match parts.as_slice() {
            [thread_id, item_id, relation, weight]
                if !thread_id.is_empty()
                    && !item_id.is_empty()
                    && LinkRelation::parse(relation).is_some()
                    && LinkWeight::parse(weight).is_some() =>
            {
                Some((
                    thread_id.to_string(),
                    item_id.to_string(),
                    relation.to_string(),
                    weight.to_string(),
                ))
            }
            _ => None,
        }
    }

    /// Fold an item field `set`, dispatching on the merge strategy the op encodes (w213). Today
    /// both strategies fold LWW: `Lww` is the register semantics; `CrdtText` is the reserved
    /// note-body strategy that falls back to LWW until the text-CRDT reducer lands (4b39), so
    /// declaring it is lossless. This match IS the seam 4b39 splits — it replaces the `CrdtText`
    /// arm with the real text-CRDT fold, no other call site changing.
    fn fold_item(conn: &Connection, op: &Op, strategy: MergeStrategy) {
        match strategy {
            MergeStrategy::Lww | MergeStrategy::CrdtText => Self::fold_item_lww(conn, op),
        }
    }

    fn fold_item_lww(conn: &Connection, op: &Op) {
        // SAFETY: is_foldable guarantees `op.field` is in ITEM_LWW_FIELDS; that whitelist is
        // exactly what makes the `format!`-interpolated column name injection-safe.
        debug_assert!(schema::ITEM_LWW_FIELDS.contains(&op.field.as_str()));
        // Single keep-if-beats upsert: insert the cell, or on id-conflict overwrite it only when
        // this op's (lamport, site) strictly beats the stored version. The row-value comparison
        // expresses the LWW tie-break (lamport, then site) in one statement.
        let f = &op.field;
        let sql = format!(
            "INSERT INTO items(id, {f}, {f}_v, {f}_site) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 {f}=excluded.{f}, {f}_v=excluded.{f}_v, {f}_site=excluded.{f}_site
             WHERE (excluded.{f}_v, excluded.{f}_site) > ({f}_v, {f}_site)"
        );
        conn.execute(&sql, params![op.target_id, op.value, op.lamport, op.site])
            .unwrap();
    }

    /// Fold a plugin custom field `set` (spec §4) — a keep-if-beats LWW upsert keyed on
    /// `(item_id, field)`, structurally identical to [`fold_item_lww`](Self::fold_item_lww) but on
    /// the pair. Both merge strategies (`Lww`, `CrdtText`) fold whole-value LWW today; `crdt-text`
    /// is a forward-compat marker (a future ticket may split it), so this deliberately does not
    /// branch on the strategy. A clear is an empty `value`, stored verbatim — the read layer (T4)
    /// treats an empty value as unset. Unlike `fold_item_lww` the field/value are BOUND params, not
    /// interpolated column names, so no whitelist is needed (any foreign field name folds — §7).
    fn fold_field(conn: &Connection, op: &Op) {
        conn.execute(
            "INSERT INTO custom_fields(item_id, field, value, value_v, value_site)
                 VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(item_id, field) DO UPDATE SET
                 value=excluded.value, value_v=excluded.value_v, value_site=excluded.value_site
             WHERE (excluded.value_v, excluded.value_site) > (value_v, value_site)",
            params![op.target_id, op.field, op.value, op.lamport, op.site],
        )
        .unwrap();
    }

    fn fold_edge_add(conn: &Connection, op: &Op) {
        let Some((from, to, kind)) = Self::parse_edge(op) else {
            return;
        };
        // tag = op_id (observed-remove)
        conn.execute(
            "INSERT OR IGNORE INTO edge_adds(from_id, to_id, kind, tag) VALUES(?1,?2,?3,?4)",
            params![from, to, kind, op.op_id],
        )
        .unwrap();
    }

    fn fold_edge_remove(conn: &Connection, op: &Op) {
        // value = SEP-joined observed add-tags this remove tombstones
        if let Some(tags) = &op.value {
            for tag in tags.split(SEP).filter(|t| !t.is_empty()) {
                conn.execute("INSERT OR IGNORE INTO edge_removes(tag) VALUES(?1)", [tag])
                    .unwrap();
            }
        }
    }

    fn fold_label_add(conn: &Connection, op: &Op) {
        let Some((item_id, label)) = Self::parse_label(op) else {
            return;
        };
        // tag = op_id (observed-remove), exactly as the edge OR-set.
        conn.execute(
            "INSERT OR IGNORE INTO label_adds(item_id, label, tag) VALUES(?1,?2,?3)",
            params![item_id, label, op.op_id],
        )
        .unwrap();
    }

    fn fold_label_remove(conn: &Connection, op: &Op) {
        // value = SEP-joined observed add-tags this remove tombstones (mirrors fold_edge_remove).
        if let Some(tags) = &op.value {
            for tag in tags.split(SEP).filter(|t| !t.is_empty()) {
                conn.execute("INSERT OR IGNORE INTO label_removes(tag) VALUES(?1)", [tag])
                    .unwrap();
            }
        }
    }

    fn fold_thread_link_add(conn: &Connection, op: &Op) {
        let Some((thread_id, item_id, relation, weight)) = Self::parse_thread_link(op) else {
            return;
        };
        // tag = op_id (observed-remove), exactly as the edge and label OR-sets.
        conn.execute(
            "INSERT OR IGNORE INTO thread_link_adds(thread_id, item_id, relation, weight, tag)
             VALUES(?1,?2,?3,?4,?5)",
            params![thread_id, item_id, relation, weight, op.op_id],
        )
        .unwrap();
    }

    fn fold_thread_link_remove(conn: &Connection, op: &Op) {
        // value = SEP-joined observed add-tags this remove tombstones (mirrors fold_edge_remove).
        if let Some(tags) = &op.value {
            for tag in tags.split(SEP).filter(|t| !t.is_empty()) {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_link_removes(tag) VALUES(?1)",
                    [tag],
                )
                .unwrap();
            }
        }
    }

    fn fold_note_add(conn: &Connection, op: &Op) {
        conn.execute(
            "INSERT OR IGNORE INTO notes(id, item_id, author, body) VALUES(?1,?2,?3,?4)",
            params![op.op_id, op.target_id, op.author, op.value],
        )
        .unwrap();
    }

    /// Redaction is a grow-only tombstone set keyed by note id (the redact op's `target_id` IS the
    /// note id). Order-independent: folding a redact before its `note_add` still tombstones the
    /// note, so any merge order converges (cf. §8).
    fn fold_note_redact(conn: &Connection, op: &Op) {
        conn.execute(
            "INSERT OR IGNORE INTO note_tombstones(note_id) VALUES(?1)",
            [&op.target_id],
        )
        .unwrap();
    }
}

impl Reducer for TaskReducer {
    fn domain(&self) -> &'static str {
        crate::model::DOMAIN_TASK
    }

    /// True iff this op has a shape the task reducer knows how to fold. A foreign op that fails
    /// this is stored but not folded (§7) — never panicked on, never dropped; `refold` can
    /// resurface it once a later version understands it.
    fn is_foldable(&self, op: &Op) -> bool {
        // w213: an item field `set` is foldable for any recognized merge strategy (encoded in the
        // op_type) whose field is whitelisted — `Lww`'s bare `"set"` and the reserved
        // `"set:crdt-text"` alike. Every other op gates on its (kind, op_type) shape.
        if op.target_kind == "item" {
            return MergeStrategy::from_item_set_op_type(&op.op_type).is_some()
                && schema::ITEM_LWW_FIELDS.contains(&op.field.as_str());
        }
        // Plugin custom field `set` (spec §4.1): foldable purely structurally — for any recognized
        // merge strategy (encoded in the op_type) — with NON-EMPTY `target_id` (the item id) and
        // `field` (the custom field name). The non-empty guards mirror `parse_edge`/`parse_label`:
        // a degenerate `field` op store-don't-folds instead of folding a junk (item_id, field) row.
        if op.target_kind == "field" {
            return MergeStrategy::from_item_set_op_type(&op.op_type).is_some()
                && !op.target_id.is_empty()
                && !op.field.is_empty();
        }
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            ("edge", "add") | ("edge", "remove") => Self::parse_edge(op).is_some(),
            ("label", "add") | ("label", "remove") => Self::parse_label(op).is_some(),
            ("thread_link", "add") => Self::parse_thread_link(op).is_some(),
            // A REMOVE carries the observed add-tags in `value`; its own `target_id` is only a label
            // for the pair, so it folds on shape alone — exactly as `("edge", "remove")` would if the
            // edge remove did not happen to share `parse_edge` with its add.
            ("thread_link", "remove") => true,
            ("note", "note_add") => true,
            ("note", "set") => op.field == "deleted",
            _ => false,
        }
    }

    /// Fold a foldable op into the views. The caller (substrate) guarantees foldability
    /// (`is_foldable`); any other shape is a bug, hence `unreachable!`.
    fn fold(&self, conn: &Connection, op: &Op) {
        // w213: an item field `set` carries its merge strategy in the op_type — decode and dispatch
        // it (is_foldable already vetted the field). Every other op matches on its (kind, op_type).
        if op.target_kind == "item" {
            if let Some(strategy) = MergeStrategy::from_item_set_op_type(&op.op_type) {
                return Self::fold_item(conn, op, strategy);
            }
        }
        // A plugin custom field `set` folds into `custom_fields` (is_foldable vetted the shape).
        if op.target_kind == "field" && MergeStrategy::from_item_set_op_type(&op.op_type).is_some()
        {
            return Self::fold_field(conn, op);
        }
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            ("edge", "add") => Self::fold_edge_add(conn, op),
            ("edge", "remove") => Self::fold_edge_remove(conn, op),
            ("label", "add") => Self::fold_label_add(conn, op),
            ("label", "remove") => Self::fold_label_remove(conn, op),
            ("thread_link", "add") => Self::fold_thread_link_add(conn, op),
            ("thread_link", "remove") => Self::fold_thread_link_remove(conn, op),
            ("note", "note_add") => Self::fold_note_add(conn, op),
            ("note", "set") => Self::fold_note_redact(conn, op),
            other => unreachable!("non-foldable op reached TaskReducer::fold(): {other:?}"),
        }
    }

    fn view_tables(&self) -> &'static [&'static str] {
        &[
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
        ]
    }
}
