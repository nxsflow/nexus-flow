//! The item records of a hosted board's active tickets (6j6v.t1ym): what a server hands the facade's
//! store-free lanes — [`nexus_flow_facade::read::next_active_value`], `blocked_active_value` and
//! `deferred_active_value` — so it answers `next`, `blocked` and `deferred` exactly as a replica's
//! `Engine` does, without re-implementing the ranking or the record.
//!
//! # What is read, and what it costs
//!
//! The lanes need nothing but the rows and the board, which [`board::select`](crate::board::select)
//! reads with one Query on the `active` index. The records also carry joins that live in other
//! tables, and [`active_board`] reads them per active ticket — never a Scan, never a ticket outside
//! the selection, and a fixed number of requests per ticket, whatever its history:
//!
//! | join                       | read                                                        |
//! |----------------------------|-------------------------------------------------------------|
//! | (the stream's rules)       | one `GetItem` of `.meta#watermark`, per board               |
//! | `created_at`, `updated_at` | one `GetItem` of `item_timestamps#<ticket>`                 |
//! | `custom`                   | one Query on `custom_fields#<ticket>#`                      |
//! | `labels`                   | one Query on `.ladj#<ticket>#`                              |
//! | `conversations`            | one Query on `.tadj#<ticket>#`                              |
//! | `parent`                   | one `GetItem` per distinct parent outside the selection     |
//!
//! So a board of `n` active tickets costs `1 + n` `GetItem`s and `3n` Queries, plus one `GetItem`
//! per distinct parent that is not itself active — `4n + 1 + p` requests besides the selection.
//!
//! One join is read only when asked for: [`with_contributes_to`] fills each ticket's outgoing
//! `contributes_to` targets, which only the facade's container filter (`next --in`) reads. It is one
//! more Query per ticket, on `.cadj#<ticket>#` — `5n + 1 + p` with it — bounded the same way. A
//! Query returns more than one page only past 1 MB, which one ticket's custom values, labels or
//! links do not reach. The entries under a ticket carry their add's label or link and its presence
//! (see [`board`](crate::board), "Who owns a label or a thread link"), so a label toggled many
//! times costs nothing more to read.
//!
//! The requests are not sent one after the other: at most [`READ_CONCURRENCY`] tickets are read at
//! a time, so a board's latency is about `n / READ_CONCURRENCY` round trips, and no board sends more
//! than that many requests at once.
//!
//! # Two points in time
//!
//! The selection is an index read, eventually consistent; every join is a row read, strongly
//! consistent, made after it. Right after a fold a record can therefore pair the state the index
//! still shows — a ticket's status, its lane — with joins that already show the fold: a label
//! added a moment ago on a ticket the index still lists as open. Both halves are states the stream
//! really had, and the next read answers the newer one throughout. Re-reading every row strongly
//! would not close the gap: the lanes come from the index, and a row newer than its lane would
//! disagree with the lane instead.
//!
//! # A stream folded by older rules
//!
//! The entries under a ticket came with index revision 1, the `.cadj#` entries with revision 2
//! (6j6v.15ed). A stream whose watermark names an older
//! one — or none, folded before revisions were recorded for the index — has no entries, and its
//! records would come back without labels or conversations. [`active_board`] refuses such a stream
//! ([`RecordsError::NotCurrent`]); the fold run refolds it ([`Folder::is_current`]), after which it
//! reads normally.
//!
//! [`Folder::is_current`]: crate::fold::Folder::is_current

use crate::board::{int, item_key, Selection, REMOVED};
use crate::fold::{Folder, INDEX_DOMAIN, INDEX_REVISION};
use crate::layout::{
    contributes_adjacency_prefix, label_adjacency_prefix, link_adjacency_prefix, row_key,
};
use crate::table::{text, Row, Table};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use nexus_flow_core::model::{ItemRow, LinkWeight};
use nexus_flow_facade::read::{ActiveBoard, ActiveTicket, ParentRef};
use nxs_foundation::change::Cell;
use std::collections::{BTreeMap, BTreeSet};

/// How many tickets [`active_board`] reads at a time. Each ticket is four requests, so at most four
/// times this many requests are in flight for one board.
pub const READ_CONCURRENCY: usize = 16;

/// Why the records of a stream could not be read.
#[derive(Debug)]
pub enum RecordsError<E> {
    /// The table refused a request.
    Table(E),
    /// The stream was folded by older index rules than this build reads (`found`: the revision its
    /// watermark names, `None` when it names none). Its records would lack joins; the fold run
    /// refolds it.
    NotCurrent { found: Option<i64> },
}

impl<E: std::fmt::Display> std::fmt::Display for RecordsError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordsError::Table(e) => write!(f, "the views table refused a request: {e}"),
            RecordsError::NotCurrent { found } => write!(
                f,
                "the stream was folded by index revision {found:?}, this build reads revision \
                 {INDEX_REVISION}; it has to be folded again"
            ),
        }
    }
}

impl<E: std::error::Error> std::error::Error for RecordsError<E> {}

impl<E> From<E> for RecordsError<E> {
    fn from(e: E) -> Self {
        RecordsError::Table(e)
    }
}

/// The selection's tickets with every join their records carry, ready for the facade's store-free
/// lanes. See the module doc for what is read, in what order, and what a stream folded by older
/// rules gets.
pub async fn active_board<T: Table>(
    table: &T,
    selection: Selection,
) -> Result<ActiveBoard, RecordsError<T::Error>> {
    if let Some(mark) = Folder::watermark(table).await? {
        let found = mark.revisions.get(INDEX_DOMAIN).copied();
        if found != Some(INDEX_REVISION) {
            return Err(RecordsError::NotCurrent { found });
        }
    }
    let Selection {
        items,
        board,
        parents,
    } = selection;

    // A parent outside the selection is read once, however many children it has.
    let outside: BTreeSet<&str> = parents
        .values()
        .map(String::as_str)
        .filter(|p| !items.contains_key(*p))
        .collect();
    let outside: BTreeMap<&str, Option<Row>> = stream::iter(outside)
        .map(|pid| async move { Ok::<_, T::Error>((pid, table.get(&item_key(pid)).await?)) })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;

    let tickets: Vec<ActiveTicket> = stream::iter(&items)
        .map(|(id, row)| {
            let belongs_to: Option<&str> = parents.get(id).map(String::as_str);
            let parent_row = belongs_to.and_then(|pid| {
                items
                    .get(pid)
                    .or_else(|| outside.get(pid).and_then(Option::as_ref))
                    .map(|r| (pid, r))
            });
            async move {
                let (created_at, updated_at) = timestamps(table, id).await?;
                let mut t = ActiveTicket::new(item_row(row, belongs_to.map(str::to_string)));
                t.custom = custom(table, id).await?;
                t.labels = labels(table, id).await?;
                t.conversations = conversations(table, id).await?;
                t.created_at = created_at;
                t.updated_at = updated_at;
                // As the store's `parent_of`: a deleted or missing parent is no parent.
                t.parent = parent_row
                    .filter(|(_, r)| text(r, "deleted") != Some("1"))
                    .map(|(pid, r)| ParentRef {
                        id: text(r, "id").unwrap_or(pid).to_string(),
                        title: text(r, "title").map(str::to_string),
                        item_type: text(r, "type").map(str::to_string),
                    });
                Ok::<_, T::Error>(t)
            }
        })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(ActiveBoard::new(board, tickets))
}

/// `board` with every ticket's outgoing `contributes_to` targets filled in, sorted and distinct —
/// what `read::active_board` fills from a store, and what the facade's container filter
/// (`read::next_active_filtered` with `NextFilter::with_container`) reads beside `belongs_to`
/// (6j6v.15ed). One Query per ticket on its `.cadj#` entries, at most [`READ_CONCURRENCY`] tickets at
/// a time; a server calls it only when a request filters by container, so `next`, `blocked` and
/// `deferred` without one pay nothing for it. The stream must be current, as for [`active_board`].
pub async fn with_contributes_to<T: Table>(
    table: &T,
    mut board: ActiveBoard,
) -> Result<ActiveBoard, RecordsError<T::Error>> {
    let ids: Vec<String> = board.tickets.keys().cloned().collect();
    let targets: Vec<(String, Vec<String>)> = stream::iter(ids)
        .map(|id| async move {
            let targets = contributes_to(table, &id).await?;
            Ok::<_, T::Error>((id, targets))
        })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;
    for (id, targets) in targets {
        if let Some(t) = board.tickets.get_mut(&id) {
            t.contributes_to = targets;
        }
    }
    Ok(board)
}

/// The ticket's present outgoing `contributes_to` targets, sorted and distinct —
/// `Store::contributes_to_of` for one ticket.
async fn contributes_to<T: Table>(table: &T, id: &str) -> Result<Vec<String>, T::Error> {
    let targets: BTreeSet<String> = present_entries(table, &contributes_adjacency_prefix(id))
        .await?
        .iter()
        .filter_map(|e| text(e, "to_id").map(str::to_string))
        .collect();
    Ok(targets.into_iter().collect())
}

/// An `items` row as the store reads it, with the ticket's `belongs_to` (the `present_parent` join).
pub(crate) fn item_row(row: &Row, belongs_to: Option<String>) -> ItemRow {
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

/// The entries under the ticket at `prefix` whose add is present.
async fn present_entries<T: Table>(table: &T, prefix: &str) -> Result<Vec<Row>, T::Error> {
    Ok(table
        .query_prefix(prefix)
        .await?
        .into_iter()
        .filter(|e| int(e, REMOVED) == Some(0))
        .collect())
}

/// The ticket's present labels, sorted and distinct — `present_labels` for one ticket.
async fn labels<T: Table>(table: &T, id: &str) -> Result<Vec<String>, T::Error> {
    let labels: BTreeSet<String> = present_entries(table, &label_adjacency_prefix(id))
        .await?
        .iter()
        .filter_map(|e| text(e, "label").map(str::to_string))
        .collect();
    Ok(labels.into_iter().collect())
}

/// How many threads bear on the ticket — `present_thread_links` for one ticket, counted where the
/// link is bearing. Of a thread's present adds the one with the highest `(lamport, site, tag)` is
/// the link, as the view picks it.
async fn conversations<T: Table>(table: &T, id: &str) -> Result<usize, T::Error> {
    let mut links: BTreeMap<String, ((i64, i64, String), bool)> = BTreeMap::new();
    for e in present_entries(table, &link_adjacency_prefix(id)).await? {
        let (Some(thread), Some(tag)) = (text(&e, "thread_id"), text(&e, "tag")) else {
            continue;
        };
        let coordinate = (
            int(&e, "lamport").unwrap_or(-1),
            int(&e, "site").unwrap_or(-1),
            tag.to_string(),
        );
        let bearing = text(&e, "weight") == Some(LinkWeight::Bearing.as_str());
        match links.get(thread) {
            Some((best, _)) if *best >= coordinate => {}
            _ => {
                links.insert(thread.to_string(), (coordinate, bearing));
            }
        }
    }
    Ok(links.values().filter(|(_, bearing)| *bearing).count())
}
