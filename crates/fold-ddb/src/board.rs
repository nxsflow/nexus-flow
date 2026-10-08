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
//! `<stream>#archived` under its `archived` instant. A ticket without the instant sits under `"0"`,
//! which sorts after every date — last, as the local lanes put it. [`dated`] reads one lane newest
//! first.
//!
//! # One writer per stream
//!
//! The rows a reducer describes converge whatever the order and however many folders write them.
//! The index flags are DERIVED — read a row, then write a flag — so two folders of the same
//! stream running at once could leave a flag that answers an older state. A stream is folded by
//! one run at a time (the fold run's queue is per stream); [`reindex`] recomputes every flag
//! from the rows if that was ever broken.

use crate::layout::{
    adjacency_key, adjacency_prefix, row_key, table_prefix, ACTIVE, DATED, DATED_AT, SK,
};
use crate::table::{text, Dated, Row, Table};
use crate::write::{Cond, Write};
use nexus_flow_core::graph::{Board, Edge, Ticket};
use nexus_flow_core::model::EdgeKind;
use nxs_foundation::change::{Cell, Change};
use std::collections::{BTreeMap, BTreeSet};

/// The key cell of a `Change` on `items` / `edge_adds` / `edge_removes`.
fn key_text<'a>(change: &'a Change, column: &str) -> Option<&'a str> {
    change.key.iter().find_map(|(c, v)| match v {
        Cell::Text(t) if c == column => Some(t.as_str()),
        _ => None,
    })
}

fn item_key(id: &str) -> String {
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

/// Keep the indexes after one op's changes were carried out.
pub(crate) async fn after<T: Table>(table: &T, changes: &[Change]) -> Result<(), T::Error> {
    let mut items = BTreeSet::new();
    let mut added = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for change in changes {
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
pub(crate) async fn refresh_item<T: Table>(table: &T, id: &str) -> Result<(), T::Error> {
    let sk = item_key(id);
    let Some(row) = table.get(&sk).await? else {
        return Ok(());
    };
    let active = is_active(&row);
    let lane = lane(&row);
    let was_active = row.contains_key(ACTIVE);
    let mut set = Vec::new();
    let mut remove = Vec::new();
    if active != was_active {
        if active {
            set.push((ACTIVE.to_string(), Cell::Text(table.stream().to_string())));
        } else {
            remove.push(ACTIVE.to_string());
        }
    }
    let dated_now = lane
        .as_ref()
        .map(|(l, at)| (l.partition(table.stream()), at.clone()));
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
    if active != was_active {
        for entry in table.query_prefix(&adjacency_prefix(id)).await? {
            if let Some(tag) = text(&entry, "tag") {
                refresh_edge(table, tag).await?;
            }
        }
    }
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
    flag_edge(table, tag, edge).await
}

/// Set edge `tag`'s flag from its row, its remove and its ends.
pub(crate) async fn refresh_edge<T: Table>(table: &T, tag: &str) -> Result<(), T::Error> {
    match table.get(&tag_key("edge_adds", tag)).await? {
        Some(edge) => flag_edge(table, tag, edge).await,
        None => Ok(()),
    }
}

async fn flag_edge<T: Table>(table: &T, tag: &str, edge: Row) -> Result<(), T::Error> {
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
            if table
                .get(&item_key(end))
                .await?
                .is_some_and(|r| r.contains_key(ACTIVE))
            {
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
    Ok(())
}

/// The active tickets — their whole rows — and the board the library derives the lanes from.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub items: BTreeMap<String, Row>,
    pub board: Board,
}

/// The selection for a hosted board: one Query on the `active` index (6j6v.vvw6 point 4).
pub async fn select<T: Table>(table: &T) -> Result<Selection, T::Error> {
    let mut items = BTreeMap::new();
    let mut tickets = Vec::new();
    let mut edges = Vec::new();
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
            edges.push(Edge {
                from: from.to_string(),
                to: to.to_string(),
                kind,
            });
        }
    }
    Ok(Selection {
        items,
        board: Board::new(tickets, edges),
    })
}

/// One `dated` lane, newest first — the closed or the archived tickets' rows, at most `limit`.
/// Tickets with the same instant come in id order.
pub async fn dated<T: Table>(
    table: &T,
    lane: Dated,
    limit: Option<usize>,
) -> Result<Vec<Row>, T::Error> {
    let mut rows = table.query_dated(lane, limit).await?;
    rows.sort_by(|a, b| {
        text(b, DATED_AT)
            .cmp(&text(a, DATED_AT))
            .then_with(|| text(a, "id").cmp(&text(b, "id")))
    });
    Ok(rows)
}
