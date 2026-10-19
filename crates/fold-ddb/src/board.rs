//! The board's indexes, kept beside the folded rows, and the reads that use them
//! (6j6v.vvw6 points 4 and 5; `typed`, 6j6v.xkr7).
//!
//! # `active`
//!
//! Sparse: its key attribute ([`ACTIVE`]) is set on an active ticket — live, not archived, `open` or
//! `in_progress` ([`nexus_flow_core::model::is_active`]) — and on a present `dep` or `parent` edge
//! at least one of whose ends is an active ticket, and on nothing else. One Query on it is
//! [`select`]: exactly the tickets and edges `nexus_flow_core::graph::select` hands the library
//! locally, so the lanes come out the same.
//!
//! A ticket's activity is a function of its own row, so it is set whenever an op touches the row.
//! An edge's flag depends on both ends: when a ticket's activity flips, the fold rewrites the flag
//! of every edge touching it, found through an adjacency entry per end (`.adj#<ticket>#<tag>`,
//! written when the edge's add is folded). That fan-out costs one Query plus, per edge, three
//! reads and at most one write — and it happens only on a flip: a close, an archive, a reopen,
//! a delete.
//!
//! Removes are observed, as locally: an edge add whose tag has a remove is not present. An add
//! folded after its remove looks for the remove first, so the order ops arrive in does not decide.
//!
//! # `dated`
//!
//! Sparse too: a closed ticket that is neither archived nor deleted sits in partition
//! `<stream>#closed` under its `closed_at`; an archived ticket that is not deleted in
//! `<stream>#archived` under its `archived` instant, the id appended ([`dated_key`]) so tickets of
//! one instant keep one order. A ticket without the instant sits under `"0"`, which sorts after
//! every date — last, as the local lanes put it. [`dated`] reads one lane newest first.
//!
//! # `typed`
//!
//! Sparse as well (6j6v.xkr7): a live item — not deleted, archived or not — with a type sits in
//! partition `<stream>#<type>` ([`typed_partition`]) under its own sort key, so ONE Query lists the
//! live items of a type in id order: the `list` scope of a type the derived ranking keys rank by
//! (docs/specs/derived-ranking-keys.md §6). Like the `active` flag of a ticket it is a function of
//! the row alone, set whenever an op touches the row ([`refresh_item`]) — after the row, so a run
//! that dies in between sets it when it folds the op again. Generic: every typed item is listed,
//! whatever the plugin ranks, at one attribute per item row. A row's `typed` attribute never
//! names another row, so a retype or a delete moves nothing else.
//!
//! # Who owns a label or a thread link
//!
//! `label_adds` and `thread_link_adds` key on the add's tag alone, so nothing in them is found by
//! ticket. Beside each add the fold keeps one entry under the ticket — `.ladj#<ticket>#<tag>` for a
//! label, `.tadj#<ticket>#<tag>` for a thread link — and a reader of the item records
//! ([`crate::records`]) reads one ticket's labels or links with ONE Query on that prefix and no
//! further read: the entry copies what the reader needs of the add (the label; the thread, weight
//! and coordinate of a link) and says whether the add is removed ([`REMOVED`]).
//!
//! Presence is derived like the index flags: when an add or its remove is folded, the fold reads
//! both rows and writes the entry from them, so the order the two arrive in does not decide — an
//! add folded after its remove finds the remove, a remove folded after its add rewrites the entry.
//! A remove whose add has not arrived writes nothing; the add writes the entry when it comes. That
//! costs two `GetItem`s and at most one write per add or remove, and [`reindex`] rebuilds every
//! entry from the rows. The entries came with index revision 1 ([`crate::fold::INDEX_REVISION`]);
//! a stream folded before has none and is refolded.
//!
//! Revision 2 (6j6v.15ed) files a ticket's outgoing `contributes_to` edges the same way,
//! `.cadj#<ticket>#<tag>` under the edge's `from_id`, copying its `to_id`, for the container filter.
//! Only `contributes_to` adds get one: folding another edge's add costs nothing more, and folding
//! an edge remove one `GetItem` of its add, to learn its kind.
//!
//! # What an adjacency entry says
//!
//! Revision 3 (6j6v.1c5s) makes the `.adj#<ticket>#<tag>` entries self-describing, as the entries
//! under a ticket are: each copies the edge add's `from_id`, `to_id`, `kind`, `lamport` and `site`
//! and says whether the add is removed ([`REMOVED`]). Every edge add has one under each end,
//! whatever its kind and whatever the ends' activity, so ONE Query on a ticket's `.adj#` prefix
//! reads every edge touching it, in and out, with its presence — what the derived ranking keys read
//! ([`crate::records::with_ranking`]): inbound `contributes_to`, `mentions` both ways, the
//! `parent` edges a child's current parent is picked from. The entries keep their place and key;
//! only their attributes grow. Folding an edge add reads its remove (a `dep` or `parent` add read it
//! already, for its flag); folding an edge remove now reads the add and the remove and rewrites
//! both entries — two `GetItem`s and two writes, one when the edge is a loop.
//!
//! # Reverse references
//!
//! Revision 4 (6j6v.9tx3) keeps what a hosted `list --ref <field>=<id>` reads
//! ([`crate::records::referrers`]): for every custom value an item holds that
//! [`ref_place_ok`](nexus_flow_core::model::ref_place_ok) admits — its field's single value, or a
//! present entry of a list field — a hint `.radj#<value>#<field>#<item>`
//! ([`ref_adjacency_key`]), and beside it a mirror `.rsrc#<item>#<field>#<value>`
//! ([`ref_source_key`]), so the values hinted for one place are one Query. ONE ROW PER HINTED VALUE
//! on either side, so no bookkeeping row grows with a place. The rule is syntactic — the fold knows
//! no declarations and needs none — so [`reindex`] and a snapshot import apply it from the rows
//! alone, and the hints are a pure FUNCTION OF THE ROWS: after every op that touches a place,
//! [`reconcile_refs`] makes them exactly what its rows warrant, pruning a retargeted, cleared or
//! removed value, and `reindex` rebuilds and prunes them all. Neither key is among
//! [`derived_keys`]: a place the predicate admits always fits a sort key (968 bytes at most, the
//! `layout` unit test), and a place it does not admit gets no hint — so a hint is never a reason to
//! refuse an op.
//!
//! # One writer per stream
//!
//! The rows a reducer describes converge whatever the order and however many folders write them.
//! The index flags are DERIVED — read a row, then write a flag — so two folders of the same
//! stream running at once could leave a flag that answers an older state. A stream is folded by
//! one run at a time (the fold run's queue is per stream); [`reindex`] recomputes every flag
//! from the rows if that was ever broken.

use crate::layout::{
    adjacency_key, adjacency_prefix, contributes_adjacency_key, label_adjacency_key,
    link_adjacency_key, ref_adjacency_key, ref_source_key, ref_source_prefix, row_key,
    table_prefix, typed_partition, ACTIVE, DATED, DATED_AT, MAX_KEY_BYTES, SK, TYPED,
};
use crate::table::{text, Dated, Outcome, Row, Table};
use crate::write::{attribute_bytes, Cond, Write};
use nexus_flow_core::graph::{Board, Edge, Ticket};
use nexus_flow_core::model::{ref_place_ok, EdgeKind};
use nxs_foundation::change::{Cell, Change, Effect};
use std::collections::{BTreeMap, BTreeSet};

fn effect_cells(change: &Change) -> &[(String, Cell)] {
    match &change.effect {
        Effect::Ensure { cells } | Effect::Put { cells } | Effect::Register { cells, .. } => cells,
    }
}

/// The key cell of a `Change` on `items` / `edge_adds` / `edge_removes`.
fn key_text<'a>(change: &'a Change, column: &str) -> Option<&'a str> {
    change.key.iter().find_map(|(c, v)| match v {
        Cell::Text(t) if c == column => Some(t.as_str()),
        _ => None,
    })
}

/// A text cell of the effect of a `Change`.
fn effect_text<'a>(change: &'a Change, column: &str) -> Option<&'a str> {
    effect_cells(change).iter().find_map(|(c, v)| match v {
        Cell::Text(t) if c == column => Some(t.as_str()),
        _ => None,
    })
}

/// The attribute of an entry under a ticket that says whether its add is removed: `1` or `0`.
pub const REMOVED: &str = "removed";

/// One OR-set whose adds get an entry under their ticket (see the module doc, "Who owns a label or
/// a thread link").
pub(crate) struct Owned {
    pub(crate) adds: &'static str,
    pub(crate) removes: &'static str,
    /// The add's column that names the ticket the entry is filed under.
    pub(crate) owner: &'static str,
    /// Only adds whose cell `.0` is `.1` get an entry; `None`: every add.
    pub(crate) only: Option<(&'static str, &'static str)>,
    /// The entry's key from the ticket and the tag.
    pub(crate) key: fn(&str, &str) -> String,
    /// The add's cells the entry copies, beside the tag.
    pub(crate) copies: &'static [&'static str],
}

pub(crate) const LABELS: Owned = Owned {
    adds: "label_adds",
    removes: "label_removes",
    owner: "item_id",
    only: None,
    key: label_adjacency_key,
    copies: &["label"],
};

pub(crate) const LINKS: Owned = Owned {
    adds: "thread_link_adds",
    removes: "thread_link_removes",
    owner: "item_id",
    only: None,
    key: link_adjacency_key,
    copies: &["thread_id", "weight", "lamport", "site"],
};

/// A ticket's outgoing `contributes_to` edges (6j6v.15ed, index revision 2): filed under the edge's
/// `from_id`, what the facade's container filter reads beside `belongs_to`.
pub(crate) const CONTRIBUTES: Owned = Owned {
    adds: "edge_adds",
    removes: "edge_removes",
    owner: "from_id",
    only: Some(("kind", "contributes_to")),
    key: contributes_adjacency_key,
    copies: &["to_id"],
};

const OWNED: [Owned; 3] = [LABELS, LINKS, CONTRIBUTES];

impl Owned {
    /// Whether an add whose cells `cell` reads gets an entry.
    fn files(&self, cell: impl Fn(&str) -> Option<String>) -> bool {
        self.only
            .is_none_or(|(column, value)| cell(column).as_deref() == Some(value))
    }
}

/// The entry of add `tag` from the add row's cells and whether it is removed. `None` for an add
/// without a ticket, which no entry can be filed under, and for an add the OR-set does not file.
fn owned_write(owned: &Owned, add: &Row, tag: &str, removed: bool) -> Option<Write> {
    if !owned.files(|c| text(add, c).map(str::to_string)) {
        return None;
    }
    let item = text(add, owned.owner)?;
    let mut set = vec![("tag".to_string(), Cell::Text(tag.to_string()))];
    for column in owned.copies {
        match add.get(*column) {
            None | Some(Cell::Null) => {}
            Some(cell) => set.push((column.to_string(), cell.clone())),
        }
    }
    set.push((REMOVED.to_string(), Cell::Int(i64::from(removed))));
    Some(Write::put((owned.key)(item, tag), set))
}

/// Write the entry of add `tag` from the stored add and its remove. Nothing while the add has not
/// been folded.
async fn refresh_owned<T: Table>(table: &T, owned: &Owned, tag: &str) -> Result<(), T::Error> {
    let Some(add) = table.get(&tag_key(owned.adds, tag)).await? else {
        return Ok(());
    };
    refresh_owned_add(table, owned, &add, tag).await
}

async fn refresh_owned_add<T: Table>(
    table: &T,
    owned: &Owned,
    add: &Row,
    tag: &str,
) -> Result<(), T::Error> {
    // An add the OR-set does not file (a `dep` edge) costs no read of its remove.
    if !owned.files(|c| text(add, c).map(str::to_string)) {
        return Ok(());
    }
    let removed = table.get(&tag_key(owned.removes, tag)).await?.is_some();
    if let Some(write) = owned_write(owned, add, tag, removed) {
        table.write(&write).await?;
    }
    Ok(())
}

/// The key of the entry an add described by `change` files under its ticket, if it is an owned add.
fn owned_entry_key(change: &Change) -> Option<String> {
    let owned = OWNED.iter().find(|o| o.adds == change.table)?;
    if !owned.files(|c| effect_text(change, c).map(str::to_string)) {
        return None;
    }
    Some((owned.key)(
        effect_text(change, owned.owner)?,
        key_text(change, "tag")?,
    ))
}

pub(crate) fn item_key(id: &str) -> String {
    row_key("items", &[("id".into(), Cell::Text(id.into()))]).expect("a text key")
}

fn tag_key(table: &str, tag: &str) -> String {
    row_key(table, &[("tag".into(), Cell::Text(tag.into()))]).expect("a text key")
}

/// Whether a stored `items` row is an active ticket.
pub fn is_active(row: &Row) -> bool {
    nexus_flow_core::model::is_active(
        text(row, "deleted"),
        text(row, "archived"),
        text(row, "status"),
    )
}

/// The type a stored `items` row is listed under on the `typed` index (6j6v.xkr7): its type, when
/// it has one and is live — not deleted; archived or not. A type too long for an index partition
/// ([`MAX_PARTITION_BYTES`](crate::layout::MAX_PARTITION_BYTES)) is never folded: [`typed_partitions`] refuses its op first.
pub fn typed(row: &Row) -> Option<&str> {
    if text(row, "deleted") == Some("1") {
        return None;
    }
    text(row, "type")
}

/// The `typed` partitions the item rows these changes write would be listed under — what
/// [`Folder`](crate::fold::Folder) checks against
/// [`MAX_PARTITION_BYTES`](crate::layout::MAX_PARTITION_BYTES) before an op writes
/// anything.
pub(crate) fn typed_partitions(stream: &str, changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .filter(|c| c.table == "items")
        .filter_map(|c| effect_text(c, "type"))
        .map(|ty| typed_partition(stream, ty))
        .collect()
}

/// The `dated` lane of a stored `items` row, and the instant it sorts by.
pub fn lane(row: &Row) -> Option<(Dated, String)> {
    if text(row, "deleted") == Some("1") {
        return None;
    }
    let at = |v: Option<&str>| v.filter(|v| !v.is_empty()).unwrap_or("0").to_string();
    if let Some(archived) = text(row, "archived") {
        return Some((Dated::Archived, at(Some(archived))));
    }
    (text(row, "status") == Some("closed")).then(|| (Dated::Closed, at(text(row, "closed_at"))))
}

/// The `dated` sort key of ticket `id` entering its lane at `at`: the instant, then the id, so that
/// tickets with the same instant still have one order — by id, descending, as the index reads newest
/// first. The separator is U+001F, below every printable character, so an instant that is a prefix
/// of another still sorts before it, exactly as the instants alone compare. An instant too long for
/// an index key (`MAX_KEY_BYTES` with the id) sorts as `"0"`, with the tickets that have none.
pub fn dated_key(at: &str, id: &str) -> String {
    let key = format!("{at}\u{1f}{id}");
    if key.len() <= MAX_KEY_BYTES {
        key
    } else {
        format!("0\u{1f}{id}")
    }
}

/// The index attributes a stored `items` row of ticket `id` carries by the rules above — what
/// [`refresh_item`] leaves on it: `active`, `typed` and the `dated` pair, each only where it
/// applies.
pub(crate) fn index_attributes(stream: &str, id: &str, row: &Row) -> Vec<(String, Cell)> {
    let mut cells = Vec::new();
    if is_active(row) {
        cells.push((ACTIVE.to_string(), Cell::Text(stream.to_string())));
    }
    if let Some(ty) = typed(row) {
        cells.push((TYPED.to_string(), Cell::Text(typed_partition(stream, ty))));
    }
    if let Some((l, at)) = lane(row) {
        cells.push((DATED.to_string(), Cell::Text(l.partition(stream))));
        cells.push((DATED_AT.to_string(), Cell::Text(dated_key(&at, id))));
    }
    cells
}

/// What an `items` row weighs toward `MAX_ITEM_BYTES` once it carries the index attributes its
/// cells call for ([`index_attributes`]) instead of the ones it has — what
/// [`Folder::fold`](crate::fold::Folder::fold) checks before an op writes anything, so a row with
/// no room left for them is refused rather than left off an index (review of PR #55, Integrity
/// #4).
pub(crate) fn indexed_bytes(stream: &str, id: &str, row: &Row) -> usize {
    let own: usize = row
        .iter()
        .filter(|(n, _)| ![ACTIVE, TYPED, DATED, DATED_AT].contains(&n.as_str()))
        .map(|(n, c)| attribute_bytes(n, c))
        .sum();
    own + index_attributes(stream, id, row)
        .iter()
        .map(|(n, c)| attribute_bytes(n, c))
        .sum::<usize>()
}

/// Keep the indexes after one op's changes were carried out. Returns what could not be written
/// ([`Outcome::TooLarge`]), each said in words — an item row with no room for its index attributes
/// (none, when the fold's size check ran first), a reverse-reference row (unreachable, see
/// [`reconcile_refs`]) — for the fold to answer [`Folded::Refused`](crate::fold::Folded::Refused)
/// with, never a fold error that would wedge the stream.
pub(crate) async fn after<T: Table>(
    table: &T,
    changes: &[Change],
) -> Result<Vec<String>, T::Error> {
    let mut items = BTreeSet::new();
    let mut added = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut owned_tags: BTreeSet<(usize, &str)> = BTreeSet::new();
    // Every place a custom value lives that a change touched, and whether a change carried a value
    // the reference rule admits there (6j6v.9tx3).
    let mut places: BTreeMap<(&str, &str), bool> = BTreeMap::new();
    for change in changes {
        if let ("custom_fields" | "field_entries", Some(item), Some(field)) = (
            change.table,
            key_text(change, "item_id"),
            key_text(change, "field"),
        ) {
            let admitted =
                effect_text(change, "value").is_some_and(|v| ref_place_ok(item, field, v));
            *places.entry((item, field)).or_default() |= admitted;
        }
        if let Some(at) = OWNED
            .iter()
            .position(|o| o.adds == change.table || o.removes == change.table)
        {
            // An add the OR-set does not file (a `dep` edge add) is known from the change alone.
            let owned = &OWNED[at];
            let filed = change.table != owned.adds
                || owned.files(|c| effect_text(change, c).map(str::to_string));
            if filed {
                owned_tags.extend(key_text(change, "tag").map(|tag| (at, tag)));
            }
        }
        match change.table {
            "items" => items.extend(key_text(change, "id")),
            "edge_adds" => added.extend(key_text(change, "tag")),
            "edge_removes" => removed.extend(key_text(change, "tag")),
            _ => {}
        }
    }
    let mut unwritten = Vec::new();
    for id in items {
        if refresh_item(table, id).await? == Some(Outcome::TooLarge) {
            unwritten.push(format!("the index attributes of item {id}"));
        }
    }
    for tag in added {
        link_edge(table, tag).await?;
    }
    for tag in removed {
        link_edge(table, tag).await?;
    }
    for (at, tag) in owned_tags {
        refresh_owned(table, &OWNED[at], tag).await?;
    }
    for ((item, field), touched) in places {
        unwritten.extend(
            reconcile_refs(table, item, field, touched)
                .await?
                .into_iter()
                .map(|sk| format!("the reverse-reference row {sk}")),
        );
    }
    Ok(unwritten)
}

/// The cells of a reverse-reference hint and of its mirror: the same triple.
fn ref_cells(source: &str, field: &str, value: &str) -> Vec<(String, Cell)> {
    vec![
        ("target_id".to_string(), Cell::Text(value.to_string())),
        ("field".to_string(), Cell::Text(field.to_string())),
        ("source_id".to_string(), Cell::Text(source.to_string())),
    ]
}

/// The sort key of the `custom_fields` row of `item`'s `field`, and the prefix of the
/// `field_entries` rows of that place.
fn place_keys(item: &str, field: &str) -> (String, String) {
    let cells = [
        ("item_id".to_string(), Cell::Text(item.to_string())),
        ("field".to_string(), Cell::Text(field.to_string())),
    ];
    let register = row_key("custom_fields", &cells).expect("a text key");
    let mut entries = row_key("field_entries", &cells).expect("a text key");
    entries.push('#');
    (register, entries)
}

/// Whether a stored `field_entries` row is a present entry: an add with a value that no remove
/// observed — what `records::custom_lists` reads and [`model::present_entries`] projects, whose
/// values are exactly these rows' values.
///
/// [`model::present_entries`]: nexus_flow_core::model::present_entries
fn present_entry(row: &Row) -> Option<&str> {
    let removed = matches!(row.get(REMOVED), Some(Cell::Int(v)) if *v != 0);
    (!removed && text(row, "tag").is_some())
        .then(|| text(row, "value"))
        .flatten()
}

/// Put the hint of `(source, field, value)` and then its mirror. A row that does not fit
/// ([`Outcome::TooLarge`] — unreachable: the longest is about 1.2 KiB) is reported in `unwritten`
/// and its mirror is not put, so the place does not read as hinted.
async fn link_ref<T: Table>(
    table: &T,
    source: &str,
    field: &str,
    value: &str,
    unwritten: &mut Vec<String>,
) -> Result<(), T::Error> {
    let cells = ref_cells(source, field, value);
    let hint = ref_adjacency_key(value, field, source);
    if table
        .write(&Write::put(hint.clone(), cells.clone()))
        .await?
        == Outcome::TooLarge
    {
        unwritten.push(hint);
        return Ok(());
    }
    let mirror = ref_source_key(source, field, value);
    if table.write(&Write::put(mirror.clone(), cells)).await? == Outcome::TooLarge {
        unwritten.push(mirror);
    }
    Ok(())
}

/// Delete the hint of `(source, field, value)` and then its mirror.
async fn unlink_ref<T: Table>(
    table: &T,
    source: &str,
    field: &str,
    value: &str,
) -> Result<(), T::Error> {
    table
        .delete(&ref_adjacency_key(value, field, source))
        .await?;
    table.delete(&ref_source_key(source, field, value)).await
}

/// The reverse-reference hints of one place (6j6v.9tx3), made exactly what its rows warrant:
/// `.radj#<v>#<field>#<item>` and its mirror `.rsrc#<item>#<field>#<v>` for every value `v` the
/// register holds or a present entry holds, when `nexus_flow_core::model::ref_place_ok` admits it —
/// the same predicate a replica's `referenced_by_bulk` keeps rows by, so both answer `list --ref`
/// from the same rows. Knows no declarations and needs none.
///
/// Reads the values hinted now from the mirrors (one Query); when none is hinted and `touched` —
/// whether a change of this op carried a value the predicate admits here — is false, there is
/// nothing to hint and it stops: a plain enum or date `set` costs this one Query. Otherwise one
/// `GetItem` of the register and one Query of the place's `field_entries` range (its whole history,
/// tombstones included), then the delta: a missing value's hint put THEN its mirror, a value the
/// rows no longer hold its hint deleted THEN its mirror. Order-independent and idempotent: the rows
/// after the write decide, whatever order the ops folded in.
///
/// `.radj#` goes first on a put and on a delete, so a run that dies half-way leaves a hint without
/// its mirror (readers ignore it: the read re-checks the rows) or a mirror without its hint. A
/// put-crash is repaired by the replay of the same op, which the batch watermark guarantees, and
/// by [`reindex`] — not by a different later op, which reads the mirrors and cannot see the orphan;
/// a delete-crash also by any later op on this place.
///
/// Returns the keys of the rows that did not fit ([`Outcome::TooLarge`]): unreachable, since a row
/// the predicate admits is about 1.2 KiB at most (the `layout` unit test); reported all the same,
/// for the op to be refused with the reason rather than the batch failed.
pub(crate) async fn reconcile_refs<T: Table>(
    table: &T,
    item: &str,
    field: &str,
    touched: bool,
) -> Result<Vec<String>, T::Error> {
    let hinted: BTreeSet<String> = table
        .query_prefix(&ref_source_prefix(item, field))
        .await?
        .iter()
        .filter_map(|r| text(r, "target_id").map(str::to_string))
        .collect();
    if hinted.is_empty() && !touched {
        return Ok(Vec::new());
    }
    let (register, entries) = place_keys(item, field);
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    if let Some(row) = table.get(&register).await? {
        wanted.extend(text(&row, "value").map(str::to_string));
    }
    for row in table.query_prefix(&entries).await? {
        if text(&row, "item_id") == Some(item) && text(&row, "field") == Some(field) {
            wanted.extend(present_entry(&row).map(str::to_string));
        }
    }
    wanted.retain(|v| ref_place_ok(item, field, v));
    let mut unwritten = Vec::new();
    for v in wanted.difference(&hinted) {
        link_ref(table, item, field, v, &mut unwritten).await?;
    }
    for v in hinted.difference(&wanted) {
        unlink_ref(table, item, field, v).await?;
    }
    Ok(unwritten)
}

/// Make every reverse-reference hint and mirror what the rows warrant ([`reconcile_refs`]'s rule
/// over the whole stream): put the missing ones, rewrite any whose cells differ, delete every one
/// no row backs. Returns the keys of the rows that did not fit.
async fn reindex_refs<T: Table>(table: &T) -> Result<Vec<String>, T::Error> {
    let mut wanted: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut want = |item: Option<&str>, field: Option<&str>, value: Option<&str>| {
        if let (Some(item), Some(field), Some(value)) = (item, field, value) {
            if ref_place_ok(item, field, value) {
                wanted.insert((item.to_string(), field.to_string(), value.to_string()));
            }
        }
    };
    for row in table.query_prefix(&table_prefix("custom_fields")).await? {
        want(
            text(&row, "item_id"),
            text(&row, "field"),
            text(&row, "value"),
        );
    }
    for row in table.query_prefix(&table_prefix("field_entries")).await? {
        want(
            text(&row, "item_id"),
            text(&row, "field"),
            present_entry(&row),
        );
    }
    let stored = |rows: Vec<Row>| -> BTreeMap<String, Row> {
        rows.into_iter()
            .filter_map(|r| text(&r, SK).map(str::to_string).map(|sk| (sk, r)))
            .collect()
    };
    let hints = stored(table.query_prefix(".radj#").await?);
    let mirrors = stored(table.query_prefix(".rsrc#").await?);
    let holds = |row: Option<&Row>, cells: &[(String, Cell)]| {
        row.is_some_and(|r| cells.iter().all(|(n, c)| r.get(n) == Some(c)))
    };
    let mut unwritten = Vec::new();
    let mut backed: BTreeSet<String> = BTreeSet::new();
    for (item, field, value) in &wanted {
        let cells = ref_cells(item, field, value);
        let hint = ref_adjacency_key(value, field, item);
        let mirror = ref_source_key(item, field, value);
        if !holds(hints.get(&hint), &cells) || !holds(mirrors.get(&mirror), &cells) {
            link_ref(table, item, field, value, &mut unwritten).await?;
        }
        backed.insert(hint);
        backed.insert(mirror);
    }
    // The hints first, as a reconcile deletes them.
    for sk in hints.keys().chain(mirrors.keys()) {
        if !backed.contains(sk) {
            table.delete(sk).await?;
        }
    }
    Ok(unwritten)
}

/// Set ticket `id`'s index attributes from its row; when its activity flipped, refresh every edge
/// touching it.
///
/// The edges go FIRST, told the ticket's new activity, and the ticket's own flag after: the flag is
/// what says whether the activity flipped, so a run that dies in between still sees the flip when
/// it folds the op again, and redoes the fan-out (review of PR #24, Integrity #2).
///
/// Returns the outcome of the write of its index attributes, `None` when nothing changed — a
/// [`Outcome::TooLarge`] means the row is left with the attributes it had, which the caller must
/// report rather than drop (review of PR #55, Integrity #4).
pub(crate) async fn refresh_item<T: Table>(
    table: &T,
    id: &str,
) -> Result<Option<Outcome>, T::Error> {
    let sk = item_key(id);
    let Some(row) = table.get(&sk).await? else {
        return Ok(None);
    };
    let active = is_active(&row);
    let was_active = row.contains_key(ACTIVE);
    if active != was_active {
        for entry in table.query_prefix(&adjacency_prefix(id)).await? {
            if let Some(tag) = text(&entry, "tag") {
                if let Some(edge) = table.get(&tag_key("edge_adds", tag)).await? {
                    flag_edge(table, tag, edge, None, Some((id, active))).await?;
                }
            }
        }
    }
    let mut set = Vec::new();
    let mut remove = Vec::new();
    if active != was_active {
        if active {
            set.push((ACTIVE.to_string(), Cell::Text(table.stream().to_string())));
        } else {
            remove.push(ACTIVE.to_string());
        }
    }
    let typed_now = typed(&row).map(|ty| typed_partition(table.stream(), ty));
    let typed_was = text(&row, TYPED).map(str::to_string);
    if typed_now != typed_was {
        match typed_now {
            Some(p) => set.push((TYPED.to_string(), Cell::Text(p))),
            None => remove.push(TYPED.to_string()),
        }
    }
    let dated_now = lane(&row).map(|(l, at)| (l.partition(table.stream()), dated_key(&at, id)));
    let dated_was = match (row.get(DATED), row.get(DATED_AT)) {
        (Some(Cell::Text(p)), Some(Cell::Text(at))) => Some((p.clone(), at.clone())),
        _ => None,
    };
    if dated_now != dated_was {
        match dated_now {
            Some((p, at)) => {
                set.push((DATED.to_string(), Cell::Text(p)));
                set.push((DATED_AT.to_string(), Cell::Text(at)));
            }
            None => {
                remove.push(DATED.to_string());
                remove.push(DATED_AT.to_string());
            }
        }
    }
    if set.is_empty() && remove.is_empty() {
        return Ok(None);
    }
    let outcome = table
        .write(&Write {
            sk,
            set,
            remove,
            condition: Some(Cond::RowPresent),
        })
        .await?;
    Ok(Some(outcome))
}

/// The cells of edge add `edge` an adjacency entry copies beside its tag and [`REMOVED`] (index
/// revision 3, 6j6v.1c5s): its ends, its kind and its coordinate.
const EDGE_COPIES: [&str; 5] = ["from_id", "to_id", "kind", "lamport", "site"];

/// The adjacency entries of edge add `tag` — one under each end, the same entry twice for an edge
/// from a ticket to itself — from the add's row and whether it is removed.
async fn write_adjacency<T: Table>(
    table: &T,
    tag: &str,
    edge: &Row,
    removed: bool,
) -> Result<(), T::Error> {
    let mut set = vec![("tag".to_string(), Cell::Text(tag.to_string()))];
    for column in EDGE_COPIES {
        match edge.get(column) {
            None | Some(Cell::Null) => {}
            Some(cell) => set.push((column.to_string(), cell.clone())),
        }
    }
    set.push((REMOVED.to_string(), Cell::Int(i64::from(removed))));
    let ends: BTreeSet<&str> = [text(edge, "from_id"), text(edge, "to_id")]
        .into_iter()
        .flatten()
        .collect();
    for end in ends {
        table
            .write(&Write::put(adjacency_key(end, tag), set.clone()))
            .await?;
    }
    Ok(())
}

/// Record that edge `tag` touches its ends — the entry under each end says what the edge is and
/// whether it is removed — then set its flag. Run when the add is folded and again when its remove
/// is; nothing while the add has not been folded, since the add writes both.
pub(crate) async fn link_edge<T: Table>(table: &T, tag: &str) -> Result<(), T::Error> {
    let Some(edge) = table.get(&tag_key("edge_adds", tag)).await? else {
        return Ok(());
    };
    let removed = table.get(&tag_key("edge_removes", tag)).await?.is_some();
    write_adjacency(table, tag, &edge, removed).await?;
    flag_edge(table, tag, edge, Some(removed), None).await
}

/// `removed` is whether the edge's remove is folded, when the caller has read it already; `known`
/// a ticket whose activity the caller knows ahead of its stored flag.
async fn flag_edge<T: Table>(
    table: &T,
    tag: &str,
    edge: Row,
    removed: Option<bool>,
    known: Option<(&str, bool)>,
) -> Result<(), T::Error> {
    let indexed = matches!(
        text(&edge, "kind").and_then(EdgeKind::parse),
        Some(EdgeKind::Dep | EdgeKind::Parent)
    );
    let mut flag = indexed
        && match removed {
            Some(removed) => !removed,
            None => table.get(&tag_key("edge_removes", tag)).await?.is_none(),
        };
    if flag {
        let mut any_active = false;
        for end in [text(&edge, "from_id"), text(&edge, "to_id")]
            .into_iter()
            .flatten()
        {
            let end_active = match known {
                Some((id, active)) if id == end => active,
                _ => table
                    .get(&item_key(end))
                    .await?
                    .is_some_and(|r| r.contains_key(ACTIVE)),
            };
            if end_active {
                any_active = true;
                break;
            }
        }
        flag = any_active;
    }
    if flag == edge.contains_key(ACTIVE) {
        return Ok(());
    }
    let (set, remove) = if flag {
        (
            vec![(ACTIVE.to_string(), Cell::Text(table.stream().to_string()))],
            vec![],
        )
    } else {
        (vec![], vec![ACTIVE.to_string()])
    };
    table
        .write(&Write {
            sk: tag_key("edge_adds", tag),
            set,
            remove,
            condition: Some(Cond::RowPresent),
        })
        .await?;
    Ok(())
}

/// Recompute every index attribute and adjacency entry from the rows — after a snapshot was
/// loaded, or to repair a stream two folders wrote at once — and make the reverse-reference hints
/// exactly what the rows warrant, deleting every one no row backs ("Reverse references" above).
/// Returns the items whose row could not take its index attributes (past `MAX_ITEM_BYTES` with
/// them), which keep the ones they had, and the key of any reverse-reference row that did not fit
/// (none can: the `layout` unit test).
pub async fn reindex<T: Table>(table: &T) -> Result<Vec<String>, T::Error> {
    let mut unindexed = Vec::new();
    for row in table.query_prefix(&table_prefix("items")).await? {
        if let Some(id) = text(&row, "id") {
            if refresh_item(table, id).await? == Some(Outcome::TooLarge) {
                unindexed.push(id.to_string());
            }
        }
    }
    for row in table.query_prefix(&table_prefix("edge_adds")).await? {
        if let Some(tag) = text(&row, "tag") {
            link_edge(table, tag).await?;
        }
    }
    for owned in &OWNED {
        for row in table.query_prefix(&table_prefix(owned.adds)).await? {
            if let Some(tag) = text(&row, "tag") {
                refresh_owned_add(table, owned, &row, tag).await?;
            }
        }
    }
    unindexed.extend(reindex_refs(table).await?);
    Ok(unindexed)
}

/// The active tickets — their whole rows — and the board the library derives the lanes from.
///
/// `#[non_exhaustive]`: only [`select`] builds one, so a later field stays an additive change.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Selection {
    pub items: BTreeMap<String, Row>,
    pub board: Board,
    /// Each active ticket's current parent, by ticket id — its `belongs_to`, as the local
    /// `present_parent` view picks it: among the ticket's present `parent` edges, the one whose add
    /// has the highest `(lamport, site, to_id)`. Every such edge has an active end, the ticket
    /// itself, so the same Query already returned it (6j6v.t1ym).
    pub parents: BTreeMap<String, String>,
}

/// The selection for a hosted board: one Query on the `active` index (6j6v.vvw6 point 4).
///
/// Eventually consistent, as every index read: right after a fold it may still answer the state
/// before it (see the crate doc, "Freshness"). A ticket the index still lists but whose row is no
/// longer active is left out.
pub async fn select<T: Table>(table: &T) -> Result<Selection, T::Error> {
    let mut items = BTreeMap::new();
    let mut tickets = Vec::new();
    let mut edges = Vec::new();
    // child → the highest (lamport, site, to_id) of its parent edges; NULL coordinates sort as -1,
    // as `present_parent`'s COALESCE does.
    let mut parents: BTreeMap<String, (i64, i64, String)> = BTreeMap::new();
    for row in table.query_active().await? {
        let sk = text(&row, SK).unwrap_or_default();
        if sk.starts_with(&table_prefix("items")) {
            // The index may lag the rows; the rule is asked again rather than trusted.
            let (Some(id), true) = (text(&row, "id"), is_active(&row)) else {
                continue;
            };
            tickets.push(Ticket {
                id: id.to_string(),
                in_progress: text(&row, "status") == Some("in_progress"),
                defer_until: text(&row, "defer_until").map(str::to_string),
            });
            items.insert(id.to_string(), row);
        } else if sk.starts_with(&table_prefix("edge_adds")) {
            let (Some(from), Some(to), Some(kind)) = (
                text(&row, "from_id"),
                text(&row, "to_id"),
                text(&row, "kind").and_then(EdgeKind::parse),
            ) else {
                continue;
            };
            if kind == EdgeKind::Parent {
                let coordinate = (
                    int(&row, "lamport").unwrap_or(-1),
                    int(&row, "site").unwrap_or(-1),
                    to.to_string(),
                );
                let best = parents
                    .entry(from.to_string())
                    .or_insert(coordinate.clone());
                if coordinate > *best {
                    *best = coordinate;
                }
            }
            edges.push(Edge {
                from: from.to_string(),
                to: to.to_string(),
                kind,
            });
        }
    }
    let parents = parents
        .into_iter()
        .filter(|(child, _)| items.contains_key(child))
        .map(|(child, (_, _, parent))| (child, parent))
        .collect();
    Ok(Selection {
        items,
        board: Board::new(tickets, edges),
        parents,
    })
}

pub(crate) fn int(row: &Row, column: &str) -> Option<i64> {
    match row.get(column) {
        Some(Cell::Int(v)) => Some(*v),
        _ => None,
    }
}

/// One `dated` lane, newest first — the closed or the archived tickets' rows, at most `limit`.
/// Tickets with the same instant come in id order, descending ([`dated_key`]); the index holds
/// that order itself, so a `limit` cuts it at the same place every time.
pub async fn dated<T: Table>(
    table: &T,
    lane: Dated,
    limit: Option<usize>,
) -> Result<Vec<Row>, T::Error> {
    table.query_dated(lane, limit).await
}

/// Keys the board would write for these changes, beside the rows the changes name: one adjacency
/// entry per end of every edge add, and one entry under its ticket per label or thread link add. What [`Folder`](crate::fold::Folder) checks
/// against the key limit before an op writes anything. The reverse-reference hints are not among
/// them: a place `ref_place_ok` admits always fits a sort key, and one it does not admit gets no
/// hint ("Reverse references" above), so a hint never refuses an op.
pub(crate) fn derived_keys(changes: &[Change]) -> Vec<String> {
    let mut keys = Vec::new();
    for change in changes.iter().filter(|c| c.table == "edge_adds") {
        let tag = key_text(change, "tag").unwrap_or_default();
        let cell = |c: &str| {
            effect_cells(change).iter().find_map(|(n, v)| match v {
                Cell::Text(t) if n == c => Some(t.clone()),
                _ => None,
            })
        };
        for end in [cell("from_id"), cell("to_id")].into_iter().flatten() {
            keys.push(adjacency_key(&end, tag));
        }
    }
    keys.extend(changes.iter().filter_map(owned_entry_key));
    keys
}
