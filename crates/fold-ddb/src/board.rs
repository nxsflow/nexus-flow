//! The board's two indexes, kept beside the folded rows, and the reads that use them
//! (6j6v.vvw6 points 4 and 5).
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
//! # Who owns a label or a thread link
//!
//! `label_adds` and `thread_link_adds` key on the add's tag alone, so nothing in them is found by
//! ticket. Beside each add the fold writes one entry under the ticket — `.ladj#<ticket>#<tag>` for a
//! label, `.tadj#<ticket>#<tag>` for a thread link — and a reader of the item records
//! ([`crate::records`]) finds one ticket's adds with one Query on that prefix. The entry is written
//! when the add is folded and never changes: an add's ticket is fixed. Whether the add is still
//! present is asked of its remove when it is read, so the order the ops arrive in does not decide.
//!
//! # One writer per stream
//!
//! The rows a reducer describes converge whatever the order and however many folders write them.
//! The index flags are DERIVED — read a row, then write a flag — so two folders of the same
//! stream running at once could leave a flag that answers an older state. A stream is folded by
//! one run at a time (the fold run's queue is per stream); [`reindex`] recomputes every flag
//! from the rows if that was ever broken.

use crate::layout::{
    adjacency_key, adjacency_prefix, label_adjacency_key, link_adjacency_key, row_key,
    table_prefix, ACTIVE, DATED, DATED_AT, MAX_KEY_BYTES, SK,
};
use crate::table::{text, Dated, Row, Table};
use crate::write::{Cond, Write};
use nexus_flow_core::graph::{Board, Edge, Ticket};
use nexus_flow_core::model::EdgeKind;
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

/// The tables whose adds get an entry under their ticket, and that entry's key (see the module doc,
/// "Who owns a label or a thread link").
const OWNED: [(&str, fn(&str, &str) -> String); 2] = [
    ("label_adds", label_adjacency_key),
    ("thread_link_adds", link_adjacency_key),
];

/// The entry under its ticket for an add of `table` described by `change`, if `table` is owned.
fn owned_entry(change: &Change) -> Option<Write> {
    let (_, key) = OWNED.iter().find(|(t, _)| *t == change.table)?;
    let tag = key_text(change, "tag")?;
    let item = effect_text(change, "item_id")?;
    Some(owned_write(*key, item, tag))
}

fn owned_write(key: fn(&str, &str) -> String, item: &str, tag: &str) -> Write {
    Write::put(
        key(item, tag),
        vec![("tag".to_string(), Cell::Text(tag.to_string()))],
    )
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

/// Keep the indexes after one op's changes were carried out.
pub(crate) async fn after<T: Table>(table: &T, changes: &[Change]) -> Result<(), T::Error> {
    let mut items = BTreeSet::new();
    let mut added = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for change in changes {
        if let Some(entry) = owned_entry(change) {
            table.write(&entry).await?;
        }
        match change.table {
            "items" => items.extend(key_text(change, "id")),
            "edge_adds" => added.extend(key_text(change, "tag")),
            "edge_removes" => removed.extend(key_text(change, "tag")),
            _ => {}
        }
    }
    for id in items {
        refresh_item(table, id).await?;
    }
    for tag in added {
        link_edge(table, tag).await?;
    }
    for tag in removed {
        refresh_edge(table, tag).await?;
    }
    Ok(())
}

/// Set ticket `id`'s index attributes from its row; when its activity flipped, refresh every edge
/// touching it.
///
/// The edges go FIRST, told the ticket's new activity, and the ticket's own flag after: the flag is
/// what says whether the activity flipped, so a run that dies in between still sees the flip when
/// it folds the op again, and redoes the fan-out (review of PR #24, Integrity #2).
pub(crate) async fn refresh_item<T: Table>(table: &T, id: &str) -> Result<(), T::Error> {
    let sk = item_key(id);
    let Some(row) = table.get(&sk).await? else {
        return Ok(());
    };
    let active = is_active(&row);
    let was_active = row.contains_key(ACTIVE);
    if active != was_active {
        for entry in table.query_prefix(&adjacency_prefix(id)).await? {
            if let Some(tag) = text(&entry, "tag") {
                if let Some(edge) = table.get(&tag_key("edge_adds", tag)).await? {
                    flag_edge(table, tag, edge, Some((id, active))).await?;
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
        return Ok(());
    }
    table
        .write(&Write {
            sk,
            set,
            remove,
            condition: Some(Cond::RowPresent),
        })
        .await?;
    Ok(())
}

/// Record that edge `tag` touches its ends, then set its flag.
pub(crate) async fn link_edge<T: Table>(table: &T, tag: &str) -> Result<(), T::Error> {
    let Some(edge) = table.get(&tag_key("edge_adds", tag)).await? else {
        return Ok(());
    };
    let ends: BTreeSet<&str> = [text(&edge, "from_id"), text(&edge, "to_id")]
        .into_iter()
        .flatten()
        .collect();
    for end in ends {
        table
            .write(&Write::put(
                adjacency_key(end, tag),
                vec![("tag".to_string(), Cell::Text(tag.to_string()))],
            ))
            .await?;
    }
    flag_edge(table, tag, edge, None).await
}

/// Set edge `tag`'s flag from its row, its remove and its ends.
pub(crate) async fn refresh_edge<T: Table>(table: &T, tag: &str) -> Result<(), T::Error> {
    match table.get(&tag_key("edge_adds", tag)).await? {
        Some(edge) => flag_edge(table, tag, edge, None).await,
        None => Ok(()),
    }
}

/// `known` is a ticket whose activity the caller knows ahead of its stored flag.
async fn flag_edge<T: Table>(
    table: &T,
    tag: &str,
    edge: Row,
    known: Option<(&str, bool)>,
) -> Result<(), T::Error> {
    let indexed = matches!(
        text(&edge, "kind").and_then(EdgeKind::parse),
        Some(EdgeKind::Dep | EdgeKind::Parent)
    );
    let mut flag = indexed && table.get(&tag_key("edge_removes", tag)).await?.is_none();
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
/// loaded, or to repair a stream two folders wrote at once.
pub async fn reindex<T: Table>(table: &T) -> Result<(), T::Error> {
    for row in table.query_prefix(&table_prefix("items")).await? {
        if let Some(id) = text(&row, "id") {
            refresh_item(table, id).await?;
        }
    }
    for row in table.query_prefix(&table_prefix("edge_adds")).await? {
        if let Some(tag) = text(&row, "tag") {
            link_edge(table, tag).await?;
        }
    }
    for (owned, key) in OWNED {
        for row in table.query_prefix(&table_prefix(owned)).await? {
            if let (Some(tag), Some(item)) = (text(&row, "tag"), text(&row, "item_id")) {
                table.write(&owned_write(key, item, tag)).await?;
            }
        }
    }
    Ok(())
}

/// The active tickets — their whole rows — and the board the library derives the lanes from.
#[derive(Debug, Clone, Default)]
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

fn int(row: &Row, column: &str) -> Option<i64> {
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
/// against the key limit before an op writes anything.
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
    keys.extend(changes.iter().filter_map(owned_entry).map(|w| w.sk));
    keys
}
