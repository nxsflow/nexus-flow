//! flow's task reducer (spec §4.2/§4.4): folds `task`-domain ops into the item/edge/note views.
//! The first registered reducer — the substrate dispatches `task` ops here. The fold rules are
//! flow's E1 view-fold, unchanged: keep-if-beats LWW on item cells, observed-remove OR-set on
//! edges, grow-only tombstones on note redactions.
//!
//! Stateless: the views live in the substrate's db, so a unit struct suffices. (In a later phase
//! this file moves to the `nexus-flow` crate while the substrate + [`Reducer`] trait stay in the
//! foundation — the reducer is exactly the seam that makes that split clean.)

use crate::model::{
    is_valid_chunk_field, is_valid_chunk_id, EdgeKind, LinkRelation, LinkWeight, MergeStrategy, Op,
    CHUNK_PAYLOAD_SEP, MAX_CHUNK_BYTES,
};
use crate::reducer::Reducer;
use crate::schema;
use crate::store::SEP;
use nxs_foundation::change::{cells, Change, Version, Wins};

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

    /// The changes of an item field `set`, dispatching on the merge strategy the op encodes (w213).
    /// Today both strategies fold LWW: `Lww` is the register semantics; `CrdtText` is the reserved
    /// note-body strategy that falls back to LWW until the text-CRDT reducer lands (4b39), so
    /// declaring it is lossless. This match IS the seam 4b39 splits — it replaces the `CrdtText`
    /// arm with the real text-CRDT fold, no other call site changing.
    fn item_changes(op: &Op, strategy: MergeStrategy) -> Vec<Change> {
        match strategy {
            MergeStrategy::Lww | MergeStrategy::CrdtText => vec![Self::item_lww(op)],
        }
    }

    /// One keep-if-beats LWW cell of `items`: the cell and its `_v`/`_site` version move together,
    /// and only an op whose `(lamport, site)` strictly beats the stored version writes.
    fn item_lww(op: &Op) -> Change {
        // The column name comes from the whitelist, never from the op: `is_foldable` guarantees
        // `op.field` is on it, and the applier interpolates column names.
        let f = schema::ITEM_LWW_FIELDS
            .iter()
            .find(|f| **f == op.field)
            .expect("is_foldable vetted the field");
        Change::register(
            "items",
            cells([("id", op.target_id.as_str().into())]),
            cells([(f, op.value.clone().into())]),
            (&format!("{f}_v"), &format!("{f}_site")),
            version(op),
            Wins::Higher,
        )
    }

    /// A plugin custom field `set` (spec §4) — a keep-if-beats LWW register keyed on `(item_id,
    /// field)`, structurally identical to [`item_lww`](Self::item_lww) but on the pair. Both merge
    /// strategies (`Lww`, `CrdtText`) fold whole-value LWW today; `crdt-text` is a forward-compat
    /// marker (a future ticket may split it), so this deliberately does not branch on the strategy.
    /// A clear is an empty `value`, stored verbatim — the read layer (T4) treats an empty value as
    /// unset. The field name is a KEY value here, not a column, so any foreign field name folds (§7).
    fn field_change(op: &Op) -> Change {
        Change::register(
            "custom_fields",
            cells([
                ("item_id", op.target_id.as_str().into()),
                ("field", op.field.as_str().into()),
            ]),
            cells([("value", op.value.clone().into())]),
            ("value_v", "value_site"),
            version(op),
            Wins::Higher,
        )
    }

    /// An item's creation and last change (6j6v.2kjy), folded instead of read off the log
    /// (6j6v.vvw6): the `wall_clock` of the op with the LOWEST `(lamport, site)` among the ops that
    /// target the item, and that of the HIGHEST. Every op whose `target_id` is the item emits these
    /// — its own cells, its custom fields, its worklog notes; edges and labels target a composite
    /// id and do not. An unstamped op carries `''`, kept as it is: the read treats it as "no
    /// timestamp", exactly as it did reading the log.
    ///
    /// Their own table, not two more cells on `items`: a note or a custom field may arrive before
    /// the item's own ops do, and a row in `items` is an item on the board.
    fn timestamp_changes(op: &Op) -> [Change; 2] {
        let key = || cells([("item_id", op.target_id.as_str().into())]);
        [
            Change::register(
                "item_timestamps",
                key(),
                cells([("created_at", op.wall_clock.as_str().into())]),
                ("created_v", "created_site"),
                version(op),
                Wins::Lower,
            ),
            Change::register(
                "item_timestamps",
                key(),
                cells([("updated_at", op.wall_clock.as_str().into())]),
                ("updated_v", "updated_site"),
                version(op),
                Wins::Higher,
            ),
        ]
    }

    /// An OR-set add: one row per add op, tagged with its op id (observed-remove), carrying the add
    /// op's coordinate so the views that pick one element among several (`present_parent`,
    /// `present_thread_links`) need nothing but the row (6j6v.vvw6).
    fn edge_add(op: &Op) -> Vec<Change> {
        let Some((from, to, kind)) = Self::parse_edge(op) else {
            return Vec::new();
        };
        vec![Change::put(
            "edge_adds",
            cells([("tag", op.op_id.as_str().into())]),
            cells([
                ("from_id", from.into()),
                ("to_id", to.into()),
                ("kind", kind.into()),
                ("lamport", op.lamport.into()),
                ("site", op.site.into()),
            ]),
        )]
    }

    /// An observed remove: `value` holds the SEP-joined add tags it tombstones, each a grow-only
    /// row of `table`.
    fn tombstones(op: &Op, table: &'static str) -> Vec<Change> {
        op.value
            .as_deref()
            .map(|tags| {
                tags.split(SEP)
                    .filter(|t| !t.is_empty())
                    .map(|tag| Change::put(table, cells([("tag", tag.into())]), Vec::new()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn label_add(op: &Op) -> Vec<Change> {
        let Some((item_id, label)) = Self::parse_label(op) else {
            return Vec::new();
        };
        // tag = op_id (observed-remove), exactly as the edge OR-set.
        vec![Change::put(
            "label_adds",
            cells([("tag", op.op_id.as_str().into())]),
            cells([("item_id", item_id.into()), ("label", label.into())]),
        )]
    }

    fn thread_link_add(op: &Op) -> Vec<Change> {
        let Some((thread_id, item_id, relation, weight)) = Self::parse_thread_link(op) else {
            return Vec::new();
        };
        // tag = op_id (observed-remove), exactly as the edge and label OR-sets — and, like the
        // edge add, with its coordinate for the projection that picks one link per pair.
        vec![Change::put(
            "thread_link_adds",
            cells([("tag", op.op_id.as_str().into())]),
            cells([
                ("thread_id", thread_id.into()),
                ("item_id", item_id.into()),
                ("relation", relation.into()),
                ("weight", weight.into()),
                ("lamport", op.lamport.into()),
                ("site", op.site.into()),
            ]),
        )]
    }

    fn note_add(op: &Op) -> Change {
        Change::put(
            "notes",
            cells([("id", op.op_id.as_str().into())]),
            cells([
                ("item_id", op.target_id.as_str().into()),
                ("author", op.author.as_str().into()),
                ("body", op.value.clone().into()),
                ("lamport", op.lamport.into()),
                ("site", op.site.into()),
                ("created_at", op.wall_clock.as_str().into()),
            ]),
        )
    }

    /// A chunk op's shape (6j6v.c0kn): the item in `target_id`, the chunk field in `field`, a
    /// `value` within [`MAX_CHUNK_BYTES`]. For a supersede, the replaced ids and the payload, split
    /// at the first [`CHUNK_PAYLOAD_SEP`]; for an append, the payload whole. `None` for anything
    /// else, which is then stored and never folded — the same on every replica and on a server, so
    /// an oversized chunk from a writer that skipped the seam's check cannot diverge them.
    fn parse_chunk(op: &Op) -> Option<(Vec<&str>, &str)> {
        let value = op.value.as_deref()?;
        if op.target_id.is_empty()
            || op.target_id.contains(SEP)
            || !is_valid_chunk_field(&op.field)
            || value.len() > MAX_CHUNK_BYTES
        {
            return None;
        }
        match op.op_type.as_str() {
            "append" => Some((Vec::new(), value)),
            "supersede" => {
                let (ids, payload) = value.split_once(CHUNK_PAYLOAD_SEP)?;
                let ids: Vec<&str> = ids.split(SEP).collect();
                ids.iter()
                    .all(|id| is_valid_chunk_id(id))
                    .then_some((ids, payload))
            }
            _ => None,
        }
    }

    /// A chunk append or supersede: the carried chunk joins the item field's grow set under the
    /// op's id, and each id a supersede names joins the grow-only replaced set. Order-independent:
    /// a supersede folded before a chunk it names still hides it, so any merge order converges.
    fn chunk_changes(op: &Op) -> Vec<Change> {
        let Some((replaced, payload)) = Self::parse_chunk(op) else {
            return Vec::new();
        };
        let (item, field) = (op.target_id.as_str(), op.field.as_str());
        let mut changes = vec![Change::put(
            "chunks",
            cells([
                ("item_id", item.into()),
                ("field", field.into()),
                ("id", op.op_id.as_str().into()),
            ]),
            cells([
                ("author", op.author.as_str().into()),
                ("body", payload.into()),
                ("lamport", op.lamport.into()),
                ("site", op.site.into()),
                ("created_at", op.wall_clock.as_str().into()),
            ]),
        )];
        changes.extend(replaced.into_iter().map(|id| {
            Change::put(
                "chunk_superseded",
                cells([
                    ("item_id", item.into()),
                    ("field", field.into()),
                    ("chunk_id", id.into()),
                ]),
                Vec::new(),
            )
        }));
        changes
    }

    /// Redaction is a grow-only tombstone set keyed by note id (the redact op's `target_id` IS the
    /// note id). Order-independent: folding a redact before its `note_add` still tombstones the
    /// note, so any merge order converges (cf. §8).
    fn note_redact(op: &Op) -> Change {
        Change::put(
            "note_tombstones",
            cells([("note_id", op.target_id.as_str().into())]),
            Vec::new(),
        )
    }
}

/// An op's coordinate, the version its registers compare on.
fn version(op: &Op) -> Version {
    Version {
        lamport: op.lamport,
        site: op.site,
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
            ("chunk", "append") | ("chunk", "supersede") => Self::parse_chunk(op).is_some(),
            _ => false,
        }
    }

    /// The changes a foldable op makes to the views. The caller (substrate) guarantees
    /// foldability (`is_foldable`); any other shape is a bug, hence `unreachable!`.
    fn changes(&self, op: &Op) -> Vec<Change> {
        // w213: an item field `set` carries its merge strategy in the op_type — decode and dispatch
        // it (is_foldable already vetted the field). Every other op matches on its (kind, op_type).
        let mut changes = if op.target_kind == "item" {
            let strategy = MergeStrategy::from_item_set_op_type(&op.op_type)
                .expect("is_foldable vetted the strategy");
            Self::item_changes(op, strategy)
        } else if op.target_kind == "field" {
            // A plugin custom field `set` folds into `custom_fields` (is_foldable vetted the shape).
            vec![Self::field_change(op)]
        } else {
            match (op.target_kind.as_str(), op.op_type.as_str()) {
                ("edge", "add") => Self::edge_add(op),
                ("edge", "remove") => Self::tombstones(op, "edge_removes"),
                ("label", "add") => Self::label_add(op),
                ("label", "remove") => Self::tombstones(op, "label_removes"),
                ("thread_link", "add") => Self::thread_link_add(op),
                ("thread_link", "remove") => Self::tombstones(op, "thread_link_removes"),
                ("note", "note_add") => vec![Self::note_add(op)],
                ("note", "set") => vec![Self::note_redact(op)],
                // A chunk does not date its item: the engine does not read it, and a writer that
                // streams many small chunks would otherwise move the item on every keystroke.
                ("chunk", _) => Self::chunk_changes(op),
                other => unreachable!("non-foldable op reached TaskReducer::changes(): {other:?}"),
            }
        };
        // The ops whose target IS an item date it: its own cells, its custom fields, its notes.
        if matches!(op.target_kind.as_str(), "item" | "field")
            || (op.target_kind == "note" && op.op_type == "note_add")
        {
            changes.extend(Self::timestamp_changes(op));
        }
        changes
    }

    /// 1 (6j6v.vvw6): the OR-set adds and the notes carry their op's coordinate, and an item's
    /// instants are folded — all filled only by folding the log again.
    /// 2 (6j6v.c0kn): chunk ops fold, and a store that held them unfolded folds them once.
    fn fold_revision(&self) -> i64 {
        2
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
            "chunks",
            "chunk_superseded",
            "custom_fields",
            "thread_link_adds",
            "thread_link_removes",
            "item_timestamps",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::change::{Cell, Effect};

    fn op(target_kind: &str, target_id: &str, field: &str, op_type: &str) -> Op {
        Op {
            op_id: "op-1".into(),
            lamport: 7,
            site: 3,
            domain: crate::model::DOMAIN_TASK.into(),
            target_kind: target_kind.into(),
            target_id: target_id.into(),
            field: field.into(),
            op_type: op_type.into(),
            value: None,
            author: "u".into(),
            wall_clock: "2026-10-06T00:00:00Z".into(),
            key_id: None,
            sig: None,
        }
    }

    fn cell<'a>(change: &'a Change, column: &str) -> &'a Cell {
        let cells = match &change.effect {
            Effect::Ensure { cells } | Effect::Put { cells } | Effect::Register { cells, .. } => {
                cells
            }
        };
        &cells.iter().find(|(c, _)| c == column).expect(column).1
    }

    #[test]
    fn the_adds_and_the_note_carry_their_op_coordinate_and_the_note_its_wall_clock() {
        // What `present_parent`, `present_thread_links` and the note reads order by now that they
        // no longer join the log (6j6v.vvw6).
        let edge = op("edge", &format!("a{SEP}b{SEP}parent"), "present", "add");
        let link = op(
            "thread_link",
            &format!("t-1{SEP}a{SEP}worked_on{SEP}bearing"),
            "present",
            "add",
        );
        let note = op("note", "a", "body", "note_add");
        for o in [&edge, &link, &note] {
            let changes = TaskReducer.changes(o);
            let row = &changes[0];
            assert_eq!(cell(row, "lamport"), &Cell::Int(7), "{}", o.target_kind);
            assert_eq!(cell(row, "site"), &Cell::Int(3), "{}", o.target_kind);
        }
        assert_eq!(
            cell(&TaskReducer.changes(&note)[0], "created_at"),
            &Cell::Text("2026-10-06T00:00:00Z".into())
        );
    }

    #[test]
    fn only_the_ops_whose_target_is_the_item_date_it() {
        let dates = |o: &Op| {
            TaskReducer
                .changes(o)
                .iter()
                .filter(|c| c.table == "item_timestamps")
                .count()
        };
        let mut set = op("item", "a", "title", "set");
        set.value = Some("T".into());
        assert_eq!(dates(&set), 2, "an item cell");
        assert_eq!(dates(&op("field", "a", "size", "set")), 2, "a custom field");
        assert_eq!(dates(&op("note", "a", "body", "note_add")), 2, "a note");
        assert_eq!(
            dates(&op("note", "n-1", "deleted", "set")),
            0,
            "a redaction targets the note"
        );
        assert_eq!(
            dates(&op("edge", &format!("a{SEP}b{SEP}dep"), "present", "add")),
            0,
            "an edge targets a composite"
        );
    }
}
