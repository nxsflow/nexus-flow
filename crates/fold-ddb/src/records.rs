//! The item records of a hosted board's active tickets (6j6v.t1ym): what a server hands the facade's
//! store-free lanes — [`nexus_flow_facade::read::next_active_value`], `blocked_active_value` and
//! `deferred_active_value` — so it answers `next`, `blocked` and `deferred` exactly as a replica's
//! `Engine` does, without re-implementing the ranking or the record.
//!
//! # What is read, and what it costs
//!
//! The rows and the board come from [`board::select`](crate::board::select), one Query on the
//! `active` index. Each record then carries joins that live in other tables, and [`active_board`]
//! reads them per active ticket — never a Scan, never a ticket outside the selection:
//!
//! | join            | read                                                                  |
//! |-----------------|-----------------------------------------------------------------------|
//! | `created_at`, `updated_at` | one `GetItem` of `item_timestamps#<ticket>`               |
//! | `custom`        | one Query on `custom_fields#<ticket>#`                                |
//! | `labels`        | one Query on `.ladj#<ticket>#`, then per label add one `GetItem` of the add and one of its remove |
//! | `conversations` | one Query on `.tadj#<ticket>#`, then per thread link add one `GetItem` of the add and one of its remove |
//! | `parent`        | one `GetItem` of the parent's row, once per parent outside the selection |
//!
//! So a board of `n` active tickets costs `4n` requests plus two per label or thread link add its
//! tickets ever had, plus one per distinct parent that is not itself active. Every row read is
//! strongly consistent; only the selection is an index read (see the crate doc, "Freshness").

use crate::board::{item_key, Selection};
use crate::layout::{label_adjacency_prefix, link_adjacency_prefix, row_key};
use crate::table::{text, Row, Table};
use nexus_flow_core::model::{ItemRow, LinkWeight};
use nexus_flow_facade::read::{ActiveBoard, ActiveTicket, ParentRef};
use nxs_foundation::change::Cell;
use std::collections::{BTreeMap, BTreeSet};

/// The selection's tickets with every join their records carry, ready for the facade's store-free
/// lanes. See the module doc for what is read.
pub async fn active_board<T: Table>(
    table: &T,
    selection: Selection,
) -> Result<ActiveBoard, T::Error> {
    let Selection {
        items,
        board,
        parents,
    } = selection;
    // A parent outside the selection is read once, however many children it has.
    let mut outside: BTreeMap<String, Option<Row>> = BTreeMap::new();
    let mut tickets = Vec::with_capacity(items.len());
    for (id, row) in &items {
        let belongs_to = parents.get(id).cloned();
        let parent = match belongs_to.as_deref() {
            None => None,
            Some(pid) => {
                let prow = match items.get(pid) {
                    Some(r) => Some(r.clone()),
                    None => match outside.get(pid) {
                        Some(r) => r.clone(),
                        None => {
                            let r = table.get(&item_key(pid)).await?;
                            outside.insert(pid.to_string(), r.clone());
                            r
                        }
                    },
                };
                prow.filter(|r| text(r, "deleted") != Some("1"))
                    .map(|r| ParentRef {
                        id: text(&r, "id").unwrap_or(pid).to_string(),
                        title: text(&r, "title").map(str::to_string),
                        item_type: text(&r, "type").map(str::to_string),
                    })
            }
        };
        let (created_at, updated_at) = timestamps(table, id).await?;
        let mut t = ActiveTicket::new(item_row(row, belongs_to));
        t.labels = labels(table, id).await?;
        t.custom = custom(table, id).await?;
        t.created_at = created_at;
        t.updated_at = updated_at;
        t.conversations = conversations(table, id).await?;
        t.parent = parent;
        tickets.push(t);
    }
    Ok(ActiveBoard::new(board, tickets))
}

/// An `items` row as the store reads it, with the ticket's `belongs_to` (the `present_parent` join).
pub fn item_row(row: &Row, belongs_to: Option<String>) -> ItemRow {
    let cell = |c: &str| text(row, c).map(str::to_string);
    ItemRow {
        id: cell("id").unwrap_or_default(),
        item_type: cell("type"),
        title: cell("title"),
        completion_criterion: cell("completion_criterion"),
        description: cell("description"),
        design: cell("design"),
        status: cell("status"),
        priority: cell("priority"),
        due: cell("due"),
        defer_until: cell("defer_until"),
        assignee: cell("assignee"),
        belongs_to,
        closing_comment: cell("closing_comment"),
        deleted: cell("deleted"),
        archived: cell("archived"),
        closed_at: cell("closed_at"),
    }
}

fn key(table: &str, cells: &[(&str, &str)]) -> String {
    let cells: Vec<(String, Cell)> = cells
        .iter()
        .map(|(c, v)| (c.to_string(), Cell::Text(v.to_string())))
        .collect();
    // Text key cells always form a row key.
    row_key(table, &cells).expect("a text key")
}

fn int(row: &Row, column: &str) -> Option<i64> {
    match row.get(column) {
        Some(Cell::Int(v)) => Some(*v),
        _ => None,
    }
}

/// The ticket's creation and last change; an unstamped `''` reads as none, as the store reads it.
async fn timestamps<T: Table>(
    table: &T,
    id: &str,
) -> Result<(Option<String>, Option<String>), T::Error> {
    let row = table
        .get(&key("item_timestamps", &[("item_id", id)]))
        .await?;
    let stamp = |c: &str| {
        row.as_ref()
            .and_then(|r| text(r, c))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Ok((stamp("created_at"), stamp("updated_at")))
}

/// Every non-empty custom value of the ticket, by field.
async fn custom<T: Table>(table: &T, id: &str) -> Result<BTreeMap<String, String>, T::Error> {
    let mut prefix = key("custom_fields", &[("item_id", id)]);
    prefix.push('#');
    Ok(table
        .query_prefix(&prefix)
        .await?
        .iter()
        .filter(|r| text(r, "item_id") == Some(id))
        .filter_map(|r| match (text(r, "field"), text(r, "value")) {
            (Some(f), Some(v)) if !v.is_empty() => Some((f.to_string(), v.to_string())),
            _ => None,
        })
        .collect())
}

/// The present adds of an owned table under the ticket: each entry's add row, unless its tag has a
/// remove.
async fn present_adds<T: Table>(
    table: &T,
    prefix: &str,
    adds: &str,
    removes: &str,
) -> Result<Vec<Row>, T::Error> {
    let mut out = Vec::new();
    for entry in table.query_prefix(prefix).await? {
        let Some(tag) = text(&entry, "tag") else {
            continue;
        };
        let Some(add) = table.get(&key(adds, &[("tag", tag)])).await? else {
            continue;
        };
        if table.get(&key(removes, &[("tag", tag)])).await?.is_none() {
            out.push(add);
        }
    }
    Ok(out)
}

/// The ticket's present labels, sorted and distinct — `present_labels` for one ticket.
async fn labels<T: Table>(table: &T, id: &str) -> Result<Vec<String>, T::Error> {
    let adds = present_adds(
        table,
        &label_adjacency_prefix(id),
        "label_adds",
        "label_removes",
    )
    .await?;
    let labels: BTreeSet<String> = adds
        .iter()
        .filter(|r| text(r, "item_id") == Some(id))
        .filter_map(|r| text(r, "label").map(str::to_string))
        .collect();
    Ok(labels.into_iter().collect())
}

/// How many threads bear on the ticket — `present_thread_links` for one ticket, counted where the
/// link is bearing. Of a thread's present adds the one with the highest `(lamport, site, tag)` is
/// the link, as the view picks it.
async fn conversations<T: Table>(table: &T, id: &str) -> Result<usize, T::Error> {
    let adds = present_adds(
        table,
        &link_adjacency_prefix(id),
        "thread_link_adds",
        "thread_link_removes",
    )
    .await?;
    let mut links: BTreeMap<String, ((i64, i64, String), bool)> = BTreeMap::new();
    for r in adds.iter().filter(|r| text(r, "item_id") == Some(id)) {
        let (Some(thread), Some(tag)) = (text(r, "thread_id"), text(r, "tag")) else {
            continue;
        };
        let coordinate = (
            int(r, "lamport").unwrap_or(-1),
            int(r, "site").unwrap_or(-1),
            tag.to_string(),
        );
        let bearing = text(r, "weight") == Some(LinkWeight::Bearing.as_str());
        match links.get(thread) {
            Some((best, _)) if *best >= coordinate => {}
            _ => {
                links.insert(thread.to_string(), (coordinate, bearing));
            }
        }
    }
    Ok(links.values().filter(|(_, bearing)| *bearing).count())
}
