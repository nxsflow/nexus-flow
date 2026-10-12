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
//! | `custom_lists`             | one Query on `field_entries#<ticket>#` (6j6v.4g6r)           |
//! | `labels`                   | one Query on `.ladj#<ticket>#`                              |
//! | `conversations`            | one Query on `.tadj#<ticket>#`                              |
//! | `parent`                   | one `GetItem` per distinct parent outside the selection     |
//!
//! So a board of `n` active tickets costs `1 + n` `GetItem`s and `4n` Queries, plus one `GetItem`
//! per distinct parent that is not itself active — `5n + 1 + p` requests besides the selection.
//!
//! One join is read only when asked for: [`with_contributes_to`] fills each ticket's outgoing
//! `contributes_to` targets, which only the facade's container filter (`next --in`) reads. It is one
//! more Query per ticket, on `.cadj#<ticket>#` — `6n + 1 + p` with it — bounded the same way. A
//! Query returns more than one page only past 1 MB, which one ticket's custom values, labels or
//! links do not reach. The entries under a ticket carry their add's label or link and its presence
//! (see [`board`](crate::board), "Who owns a label or a thread link"), so a label toggled many
//! times costs nothing more to read.
//!
//! List-field entries are the exception (6j6v.4g6r): `field_entries#<ticket>#` holds one row per
//! add ever folded, and a remove keeps the row, value and all, marked `removed` — the observed-
//! remove tombstone a later add must still be checked against. So the range grows with the field's
//! history, not with its present entries: every relabel is a row, every removed entry stays one,
//! and a peer can tombstone tags no add ever carried (up to 1,024 per remove op), each a row with
//! no value. At 16 KiB a row, about 64 rows pass 1 MB, and the Query then takes more than one
//! page; [`Table::query_prefix`] follows every page, so the answer stays whole and only the cost
//! grows. Nothing collects the tombstones yet (spec plugin-custom-fields §5.2; compaction is filed
//! as 6j6v.2sbr).
//!
//! The requests are not sent one after the other: at most [`READ_CONCURRENCY`] tickets are read at
//! a time, so a board's latency is about `n / READ_CONCURRENCY` round trips, and no board sends more
//! than that many requests at once.
//!
//! One more is read only for a plugin that ranks `next` by derived ranking keys:
//! [`with_ranking`] fills the board's [`RankingGraph`] — the edges around the active tickets and
//! their live neighbours — from the adjacency entries (index revision 3, 6j6v.1c5s), and the live
//! items of a type whose `[ranking.list]` rank a key reads from the `typed` index (6j6v.xkr7). Its
//! cost is in its own doc. Where it reads many rows by key it sends `BatchGetItem`s
//! ([`Table::get_many`], 100 keys a request) instead of one `GetItem` each.
//!
//! # A hosted `list`
//!
//! [`list_board`] reads what the facade's store-free `list` ranks and projects (6j6v.xkr7): the
//! live, unarchived items of the listed types off the `typed` index, with the joins a `list`
//! record and its filters carry, `next`'s board when a key reads a rank `next` gives, and the
//! graph. The server asks [`read::list_reads`](nexus_flow_facade::read::list_reads) what to read
//! for its query; its cost is in its own doc.
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
//! (6j6v.15ed), the self-describing `.adj#` entries and the `typed` index attribute with revision 3
//! (6j6v.1c5s, 6j6v.xkr7). A stream whose watermark names an older one — or none, folded before
//! revisions were recorded for the index — lacks them, and its records would come back without
//! labels or conversations, its list empty. The rows themselves are the task reducer's, so its
//! revision counts too: a stream folded by task revision 2 holds list-field entry ops an older
//! server stored without folding them (revision 3, 6j6v.4g6r), and would serve their lists empty.
//! [`active_board`], [`with_ranking`] and [`list_board`] refuse a stream whose watermark names
//! another index or task revision than this build's ([`RecordsError::NotCurrent`], naming the
//! domain); the fold run refolds it ([`Folder::is_current`]), after which it reads normally.
//!
//! A refold is refused for its whole length, not only until it starts: [`Folder::clear`] leaves a
//! sentinel watermark that names no revision (`NotCurrent`), and once the refold has written this
//! build's revisions the readers answer [`RecordsError::Refolding`] until it reaches the relay
//! position the clear named. A stream with no watermark at all was never folded, and reads as the
//! empty board it is.
//!
//! [`Folder::is_current`]: crate::fold::Folder::is_current
//! [`Folder::clear`]: crate::fold::Folder::clear

use crate::board::{int, is_active, item_key, select, Selection, REMOVED};
use crate::fold::{Folder, INDEX_DOMAIN, INDEX_REVISION};
use crate::layout::{
    adjacency_prefix, contributes_adjacency_prefix, label_adjacency_prefix, link_adjacency_prefix,
    row_key, table_prefix,
};
use crate::table::{text, Row, Table};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use nexus_flow_core::graph::Edge;
use nexus_flow_core::model::{
    self, EdgeKind, EntryAdd, FieldEntry, ItemRow, LinkWeight, DOMAIN_TASK,
};
use nexus_flow_core::task_reducer::TaskReducer;
use nexus_flow_facade::read::{
    self, ActiveBoard, ActiveTicket, ListBoard, ListItem, ListReads, ListSelect, Neighbour,
    ParentRef, RankingGraph, RankingReads,
};
use nxs_foundation::change::Cell;
use nxs_foundation::reducer::Reducer;
use std::collections::{BTreeMap, BTreeSet};

/// How many tickets [`active_board`] reads at a time. Each ticket is five requests, so at most five
/// times this many requests are in flight for one board.
pub const READ_CONCURRENCY: usize = 16;

/// Why the records of a stream could not be read.
#[derive(Debug)]
pub enum RecordsError<E> {
    /// The table refused a request.
    Table(E),
    /// The stream was folded by other rules than this build reads: `domain` is the watermark
    /// revision that differs — [`INDEX_DOMAIN`] for this crate's index rules, or the task
    /// reducer's domain (`task`) for the rows the records read — and `found` the revision the
    /// watermark names for it (`None` when it names none). Its records would lack joins or rows;
    /// the fold run refolds it.
    NotCurrent {
        domain: &'static str,
        found: Option<i64>,
    },
    /// The stream is being folded again from position 0 ([`Folder::clear`]) and has not yet
    /// reached `until`, the relay position the refold has to reach before it is served: its
    /// tables hold part of its history only.
    Refolding { folded_through: i64, until: i64 },
}

impl<E: std::fmt::Display> std::fmt::Display for RecordsError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordsError::Table(e) => write!(f, "the views table refused a request: {e}"),
            RecordsError::NotCurrent { domain, found } => write!(
                f,
                "the stream was folded by {domain} revision {found:?}, this build reads revision \
                 {}; it has to be folded again",
                served_revision(domain)
            ),
            RecordsError::Refolding {
                folded_through,
                until,
            } => write!(
                f,
                "the stream is being folded again, through relay position {folded_through} of \
                 {until}; it is served once the refold reaches {until}"
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
    check_current(table).await?;
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
                t.custom_lists = custom_lists(table, id).await?;
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

/// One present edge as its adjacency entry tells it (index revision 3): the add's ends, kind and
/// coordinate (`-1` where the add has none, as `present_parent` reads it).
#[derive(Debug, Clone)]
struct EdgeEntry {
    from: String,
    to: String,
    kind: EdgeKind,
    lamport: i64,
    site: i64,
}

/// The present edges touching ticket `id`, by tag: one Query on its `.adj#` prefix. Only entries
/// that name `id` as an end count.
async fn present_edges<T: Table>(
    table: &T,
    id: &str,
) -> Result<BTreeMap<String, EdgeEntry>, T::Error> {
    Ok(present_entries(table, &adjacency_prefix(id))
        .await?
        .iter()
        .filter_map(|e| {
            let entry = EdgeEntry {
                from: text(e, "from_id")?.to_string(),
                to: text(e, "to_id")?.to_string(),
                kind: text(e, "kind").and_then(EdgeKind::parse)?,
                lamport: int(e, "lamport").unwrap_or(-1),
                site: int(e, "site").unwrap_or(-1),
            };
            Some((text(e, "tag")?.to_string(), entry))
        })
        // An entry filed under `id` names it as an end. One that does not — an add whose tag was
        // reused with other ends, leaving the entry under its old end behind — is not an edge of
        // `id`, whatever its copy says (review of PR #55, Integrity #5).
        .filter(|(_, e)| e.from == id || e.to == id)
        .collect())
}

/// The current parent of `id` among the present edges touching it — `present_parent`'s pick: of its
/// present outgoing `parent` edges, the one with the highest `(lamport, site, to_id)`.
fn current_parent(id: &str, edges: &BTreeMap<String, EdgeEntry>) -> Option<String> {
    edges
        .values()
        .filter(|e| e.kind == EdgeKind::Parent && e.from == id)
        .max_by(|a, b| (a.lamport, a.site, &a.to).cmp(&(b.lamport, b.site, &b.to)))
        .map(|e| e.to.clone())
}

/// `board` with its [`RankingGraph`] for `reads` (6j6v.1c5s): the graph the derived ranking keys of
/// `[ranking.next]` read, from the folded tables — what [`read::with_ranking`] reads from a store,
/// edge for edge and neighbour for neighbour. A server calls it when
/// [`read::next_ranking_reads`] names reads for its plugin, so a plugin without derived `next` keys
/// pays nothing for it. The stream must be current, as for [`active_board`] — the entries read here
/// came with index revision 3 — and it is asked again here ([`RecordsError::NotCurrent`],
/// [`RecordsError::Refolding`]): a revision-2 entry says nothing of its edge, and would read as no
/// edge at all.
///
/// | read                                          | per                                       |
/// |-----------------------------------------------|-------------------------------------------|
/// | one `GetItem` of `.meta#watermark`            | board                                     |
/// | one Query on `.adj#<ticket>#`                 | active ticket                             |
/// | one Query on the `typed` index                | type in `reads.list_types` (6j6v.xkr7)    |
/// | one Query on `.adj#<item>#`                   | live, unarchived item of such a type that is not active |
/// | its row, by `BatchGetItem`                    | other end of such an edge that is not a subject — `⌈p / 100⌉` requests |
/// | one Query on `.adj#<item>#` (its parent)      | such end that is live                     |
/// | one Query on `.ladj#<item>#`, `custom_fields#<item>#` | such end and listed item, when `reads` asks for labels, custom values |
///
/// So `n` active tickets with `p` distinct inactive neighbours, `q` of them live, cost `n + q`
/// Queries, one `GetItem` and `⌈p / 100⌉` `BatchGetItem`s — `n + 3q` Queries with labels and
/// custom values — at most
/// [`READ_CONCURRENCY`] items at a time, and nothing grows with an item's history: an entry says
/// whether its add is removed. Every edge touching a subject is read once, from one end's entries,
/// whatever the plugin's keys — the edge pass the store path runs once per query. A `next` key
/// that reads the rank of a type only `[ranking.list]` orders adds, per such type, one index Query
/// and its live items as subjects ([`RankingReads::list_types`]) — one Query on `.adj#`, and one each
/// on `.ladj#` / `custom_fields#` when the keys read them, per live, unarchived item of the type,
/// however few the answer ranks: the rank is computed over the type's whole `list` scope
/// (docs/specs/derived-ranking-keys.md §12, decision 2).
///
/// [`read::with_ranking`]: nexus_flow_facade::read::with_ranking
/// [`read::next_ranking_reads`]: nexus_flow_facade::read::next_ranking_reads
pub async fn with_ranking<T: Table>(
    table: &T,
    mut board: ActiveBoard,
    reads: &RankingReads,
) -> Result<ActiveBoard, RecordsError<T::Error>> {
    // The board may have been read before a refold began, and an older stream's entries say
    // nothing of their edge: asked here again, one `GetItem` (review of PR #55, Integrity #3).
    check_current(table).await?;
    let ids: Vec<String> = board.tickets.keys().cloned().collect();
    let held: BTreeMap<String, Held> = stream::iter(ids)
        .map(|id| async move {
            let edges = present_edges(table, &id).await?;
            Ok::<_, T::Error>((id, edges))
        })
        .buffered(READ_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|(id, edges)| {
            let parent = board
                .tickets
                .get(&id)
                .and_then(|t| t.item.belongs_to.clone());
            (id, Held { parent, edges })
        })
        .collect();
    board.ranking = Some(ranking_graph(table, reads, held).await?);
    Ok(board)
}

/// A row the board holds itself — an active ticket, a listed item — as the graph reads it: its
/// current parent and the present edges touching it.
struct Held {
    parent: Option<String>,
    edges: BTreeMap<String, EdgeEntry>,
}

/// The live, unarchived items of `item_type` on the `typed` index — the type's `list` scope — in
/// id order.
///
/// What the index hands back is its copy of each row, and the copy's own cells are asked again:
/// the fold writes the `typed` attribute AFTER the row ([`board`](crate::board), "`typed`"), so a
/// copy taken between the two writes — or left by a run that died between them — can be retyped or
/// deleted while its attribute still names `item_type`; it is dropped. The base row itself is not
/// read again: the index's lag behind it is the one the module doc's "Two points in time"
/// describes, a state the stream really had, and is not hidden here.
async fn typed_items<T: Table>(table: &T, item_type: &str) -> Result<Vec<Row>, T::Error> {
    Ok(table
        .query_typed(item_type)
        .await?
        .into_iter()
        .filter(|r| {
            crate::board::typed(r) == Some(item_type)
                && text(r, "archived").is_none()
                && text(r, "id").is_some()
        })
        .collect())
}

/// One live row the graph holds, read in full: its row with its current parent, its labels and
/// custom values when `reads` asks, and its present edges.
async fn neighbour<T: Table>(
    table: &T,
    id: &str,
    row: &Row,
    reads: &RankingReads,
) -> Result<(Neighbour, BTreeMap<String, EdgeEntry>), T::Error> {
    let edges = present_edges(table, id).await?;
    let mut n = Neighbour::new(item_row(row, current_parent(id, &edges)));
    if reads.labels {
        n.labels = labels(table, id).await?;
    }
    if reads.custom {
        n.custom = custom(table, id).await?;
    }
    Ok((n, edges))
}

/// The [`RankingGraph`] for `reads` around the rows a board holds itself (`held`, each a subject,
/// with its edges already read) — what [`with_ranking`] and [`list_board`] put on their boards.
async fn ranking_graph<T: Table>(
    table: &T,
    reads: &RankingReads,
    held: BTreeMap<String, Held>,
) -> Result<RankingGraph, T::Error> {
    // The listed items of `reads.list_types` the board does not hold: subjects too.
    let mut listed: BTreeSet<String> = BTreeSet::new();
    let mut scope_rows: Vec<(String, Row)> = Vec::new();
    for ty in &reads.list_types {
        for row in typed_items(table, ty).await? {
            let id = text(&row, "id").unwrap_or_default().to_string();
            listed.insert(id.clone());
            if !held.contains_key(&id) {
                scope_rows.push((id, row));
            }
        }
    }
    let scope: BTreeMap<String, (Neighbour, BTreeMap<String, EdgeEntry>)> =
        stream::iter(scope_rows)
            .map(|(id, row)| async move {
                let read = neighbour(table, &id, &row, reads).await?;
                Ok::<_, T::Error>((id, read))
            })
            .buffered(READ_CONCURRENCY)
            .try_collect()
            .await?;
    let subject = |id: &str| held.contains_key(id) || scope.contains_key(id);

    // Every present edge of a kind read that touches a subject, once per tag.
    let mut touching: BTreeMap<String, EdgeEntry> = BTreeMap::new();
    for edges in held
        .values()
        .map(|h| &h.edges)
        .chain(scope.values().map(|(_, e)| e))
    {
        touching.extend(
            edges
                .iter()
                .filter(|(_, e)| reads.kinds.contains(&e.kind))
                .map(|(tag, e)| (tag.clone(), e.clone())),
        );
    }

    // The other ends: their rows, and for the live ones their current parent and the joins asked
    // for.
    let others: BTreeSet<String> = touching
        .values()
        .flat_map(|e| [&e.from, &e.to])
        .filter(|id| !subject(id))
        .cloned()
        .collect();
    let keys: Vec<String> = others.iter().map(|id| item_key(id)).collect();
    let mut rows = table.get_many(&keys).await?;
    let live: Vec<(String, Row)> = others
        .into_iter()
        .filter_map(|id| rows.remove(&item_key(&id)).map(|r| (id, r)))
        .filter(|(_, r)| text(r, "deleted") != Some("1"))
        .collect();
    let ends: BTreeMap<String, Neighbour> = stream::iter(live)
        .map(|(id, row)| async move {
            let (n, _) = neighbour(table, &id, &row, reads).await?;
            Ok::<_, T::Error>((id, n))
        })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;

    // The edges between live ends, a `parent` edge only from a child to its current parent.
    let parent_of = |id: &str| -> Option<&str> {
        match held.get(id) {
            Some(h) => h.parent.as_deref(),
            None => scope
                .get(id)
                .map(|(n, _)| n)
                .or_else(|| ends.get(id))
                .and_then(|n| n.item.belongs_to.as_deref()),
        }
    };
    let live_end = |id: &str| subject(id) || ends.contains_key(id);
    let edges: BTreeSet<Edge> = touching
        .into_values()
        .filter(|e| live_end(&e.from) && live_end(&e.to))
        .filter(|e| e.kind != EdgeKind::Parent || parent_of(&e.from) == Some(e.to.as_str()))
        .map(|e| Edge {
            from: e.from,
            to: e.to,
            kind: e.kind,
        })
        .collect();
    // A child whose current parent is elsewhere was read for its parent edge only, which is not in
    // the graph: it is no neighbour. A listed item always is one.
    let edge_ends: BTreeSet<&str> = edges
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    let mut neighbours: BTreeMap<String, Neighbour> = ends
        .into_iter()
        .filter(|(id, _)| edge_ends.contains(id.as_str()))
        .collect();
    neighbours.extend(scope.into_iter().map(|(id, (n, _))| (id, n)));
    let mut graph = RankingGraph::new(reads.clone());
    graph.edges = edges;
    graph.neighbours = neighbours;
    graph.listed = listed;
    Ok(graph)
}

/// The [`ListBoard`] for `reads` from the folded tables (6j6v.xkr7) — what [`read::list_board`]
/// reads from a store, item for item — for the facade's store-free `list`
/// ([`read::list_board_page`], [`read::list_board_value`]). A server takes `reads` from
/// [`read::list_reads`] for its query. The stream must be current, as for [`active_board`]: the
/// `typed` index and the self-describing `.adj#` entries came with index revision 3.
///
/// # What a list costs
///
/// The listed types' rows come first: one Query on the `typed` index per listed type, or — for a
/// query without a type filter — one Query on the partition's `items#` rows (every item row of the
/// stream, deleted ones included, strongly consistent; DynamoDB pages it at 1 MB, so it is one
/// request per MB of item rows). Archived and deleted items are dropped there: `list` returns
/// neither. One `GetItem` of `.meta#watermark` per board. What follows depends on the order.
///
/// **Any order but `rank`** ([`ListReads::select`]) reads the item rows alone, so the rows are
/// kept and cut BEFORE the joins a record carries are read ([`read::list_select`]): the status and
/// the type are asked of the row; a container filter reads one Query on `.adj#`, a label filter one
/// on `.ladj#`, per item the row kept; then only the `k` items of the page (at most `limit`) read
/// the rest:
///
/// | read                                                         | per                       |
/// |--------------------------------------------------------------|---------------------------|
/// | one Query each on `.adj#`, `.ladj#`, `custom_fields#`, `field_entries#` (less what the filter read) | page item |
/// | `item_timestamps` rows, by `BatchGetItem`                    | page — `⌈k / 100⌉` requests |
/// | parents of open page items outside the page, by `BatchGetItem` | page — `⌈p / 100⌉` requests |
///
/// So `list --limit 10` costs `1 + 40 + 2` requests beside the rows' Query, at most, however many
/// items the stream holds and however many the status refuses — plus one Query per item the row
/// kept when the query filters by container or label.
///
/// **`sort = rank`** reads every live, unarchived item of the listed types in full — one Query
/// each on `.adj#`, `.ladj#`, `custom_fields#`, `field_entries#` per item, and the timestamps and
/// outside parents by
/// `BatchGetItem` — whatever its status, filter or limit: a rank is computed over the type's whole
/// `list` scope (docs/specs/derived-ranking-keys.md §12, decision 2), and a derived key reads the
/// labels and custom values of every item in it. When a key reads a rank `next` gives, one Query on
/// the `active` index and per active ticket one Query each on `.ladj#` / `custom_fields#` when the
/// graph reads them; with derived keys, [`with_ranking`]'s reads around the listed items and the
/// active tickets. The `.adj#` Query gives each item's parent, its `contributes_to` targets and its
/// parents for `parent_closed_reason` at once, and the graph reuses it.
///
/// Nothing here caps the requests of one call: a ranked list of a type with `n` live items costs
/// about `4n` Queries, the number its rank is computed over. A server that must bound one request
/// sets its own budget around the table it hands in (`6j6v.wq9e`, E4 spec §5.1).
///
/// [`read::list_board`]: nexus_flow_facade::read::list_board
/// [`read::list_board_page`]: nexus_flow_facade::read::list_board_page
/// [`read::list_board_value`]: nexus_flow_facade::read::list_board_value
/// [`read::list_reads`]: nexus_flow_facade::read::list_reads
/// [`read::list_select`]: nexus_flow_facade::read::list_select
pub async fn list_board<T: Table>(
    table: &T,
    reads: &ListReads,
) -> Result<ListBoard, RecordsError<T::Error>> {
    check_current(table).await?;
    let mut rows: Vec<Row> = match &reads.types {
        Some(types) => {
            let mut rows = Vec::new();
            for ty in types {
                rows.extend(typed_items(table, ty).await?);
            }
            rows
        }
        None => table
            .query_prefix(&table_prefix("items"))
            .await?
            .into_iter()
            .filter(|r| {
                text(r, "deleted") != Some("1")
                    && text(r, "archived").is_none()
                    && text(r, "id").is_some()
            })
            .collect(),
    };
    let keep = reads.select.as_ref();
    // A list that is not ranked keeps what its row decides — status, type — before any join.
    if let Some(select) = keep {
        rows.retain(|r| select.keeps_row(&item_row(r, None)));
    }

    // What keeping an item reads: for a ranked list every join below, for any other only what its
    // filter reads — a container's edges, a label's labels.
    let edges_first = keep.is_none_or(ListSelect::reads_edges);
    let labels_first = keep.is_none_or(ListSelect::reads_labels);
    type Read = (Row, Option<BTreeMap<String, EdgeEntry>>, ListItem);
    let first: Vec<Read> = stream::iter(rows)
        .map(|row| async move {
            let id = text(&row, "id").unwrap_or_default().to_string();
            let edges = if edges_first {
                Some(present_edges(table, &id).await?)
            } else {
                None
            };
            let mut i = list_item(&row, &id, edges.as_ref());
            if labels_first {
                i.labels = labels(table, &id).await?;
            }
            Ok::<_, T::Error>((row, edges, i))
        })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;

    // Kept and cut as `list_board_page` keeps and cuts, so the joins below are read for the page.
    let (first, cut) = match keep {
        None => (first, None),
        Some(select) => {
            let mut held: BTreeMap<String, (Row, Option<BTreeMap<String, EdgeEntry>>)> =
                BTreeMap::new();
            let items: Vec<ListItem> = first
                .into_iter()
                .map(|(row, edges, i)| {
                    held.insert(i.item.id.clone(), (row, edges));
                    i
                })
                .collect();
            let (kept, cut) = read::list_select(select, items);
            let kept = kept
                .into_iter()
                .filter_map(|i| held.remove(&i.item.id).map(|(row, edges)| (row, edges, i)))
                .collect();
            (kept, Some(cut))
        }
    };

    // The rest of each kept item's joins.
    let mut read: Vec<(Row, BTreeMap<String, EdgeEntry>, ListItem)> = stream::iter(first)
        .map(|(row, edges, mut i)| async move {
            let id = i.item.id.clone();
            let edges = match edges {
                Some(edges) => edges,
                None => {
                    let edges = present_edges(table, &id).await?;
                    let full = list_item(&row, &id, Some(&edges));
                    i.item = full.item;
                    i.contributes_to = full.contributes_to;
                    edges
                }
            };
            if !labels_first {
                i.labels = labels(table, &id).await?;
            }
            i.custom = custom(table, &id).await?;
            i.custom_lists = custom_lists(table, &id).await?;
            Ok::<_, T::Error>((row, edges, i))
        })
        .buffered(READ_CONCURRENCY)
        .try_collect()
        .await?;
    let keys: Vec<String> = read
        .iter()
        .map(|(_, _, i)| timestamps_key(&i.item.id))
        .collect();
    let stamped = table.get_many(&keys).await?;
    for (_, _, i) in &mut read {
        (i.created_at, i.updated_at) = stamps(stamped.get(&timestamps_key(&i.item.id)));
    }

    // `parent_closed_reason`: an OPEN item with parents, none of them active, lists them all. A
    // parent outside the list is read once, however many children it has.
    let parents_of = |id: &str, edges: &BTreeMap<String, EdgeEntry>| -> BTreeSet<String> {
        edges
            .values()
            .filter(|e| e.kind == EdgeKind::Parent && e.from == id)
            .map(|e| e.to.clone())
            .collect()
    };
    let listed_rows: BTreeMap<&str, &Row> = read
        .iter()
        .map(|(row, _, i)| (i.item.id.as_str(), row))
        .collect();
    let outside: Vec<String> = read
        .iter()
        .filter(|(_, _, i)| i.item.status.as_deref() == Some("open"))
        .flat_map(|(_, edges, i)| parents_of(&i.item.id, edges))
        .filter(|p| !listed_rows.contains_key(p.as_str()))
        .map(|p| item_key(&p))
        .collect();
    let outside = table.get_many(&outside).await?;
    let parent_row = |pid: &str| -> Option<&Row> {
        listed_rows
            .get(pid)
            .copied()
            .or_else(|| outside.get(&item_key(pid)))
    };
    let parent_closed: Vec<Vec<(String, Option<String>)>> = read
        .iter()
        .map(|(_, edges, i)| {
            if i.item.status.as_deref() != Some("open") {
                return Vec::new();
            }
            let mut closed = Vec::new();
            for pid in parents_of(&i.item.id, edges) {
                match parent_row(&pid) {
                    Some(r) if is_active(r) => return Vec::new(),
                    Some(r) => closed.push((
                        text(r, "id").unwrap_or(&pid).to_string(),
                        text(r, "closing_comment").map(str::to_string),
                    )),
                    None => closed.push((pid, None)),
                }
            }
            closed.sort_by(|a, b| a.0.cmp(&b.0));
            closed
        })
        .collect();
    let mut items = Vec::with_capacity(read.len());
    let mut held: BTreeMap<String, Held> = BTreeMap::new();
    for ((_, edges, mut i), closed) in read.into_iter().zip(parent_closed) {
        i.parent_closed = closed;
        held.insert(
            i.item.id.clone(),
            Held {
                parent: i.item.belongs_to.clone(),
                edges,
            },
        );
        items.push(i);
    }
    let mut board = ListBoard::new(reads.types.clone(), items);
    board.cut = cut;

    let graph_reads = reads.ranking.clone().unwrap_or_default();
    if reads.next {
        let Selection {
            items: rows,
            board: lanes,
            parents,
        } = select(table).await?;
        let tickets: Vec<ActiveTicket> = stream::iter(rows)
            .map(|(id, row)| {
                let parent = parents.get(&id).cloned();
                let graph_reads = &graph_reads;
                async move {
                    let mut t = ActiveTicket::new(item_row(&row, parent));
                    if graph_reads.labels {
                        t.labels = labels(table, &id).await?;
                    }
                    if graph_reads.custom {
                        t.custom = custom(table, &id).await?;
                    }
                    Ok::<_, T::Error>(t)
                }
            })
            .buffered(READ_CONCURRENCY)
            .try_collect()
            .await?;
        board.active = Some(ActiveBoard::new(lanes, tickets));
    }
    if let Some(r) = &reads.ranking {
        if let Some(active) = &board.active {
            let missing: Vec<(String, Option<String>)> = active
                .tickets
                .iter()
                .filter(|(id, _)| !held.contains_key(*id))
                .map(|(id, t)| (id.clone(), t.item.belongs_to.clone()))
                .collect();
            let more: Vec<(String, Held)> = stream::iter(missing)
                .map(|(id, parent)| async move {
                    let edges = present_edges(table, &id).await?;
                    Ok::<_, T::Error>((id, Held { parent, edges }))
                })
                .buffered(READ_CONCURRENCY)
                .try_collect()
                .await?;
            held.extend(more);
        }
        board.ranking = Some(ranking_graph(table, r, held).await?);
    }
    Ok(board)
}

/// A listed item from its row and, when they were read, its present edges: its `belongs_to` and its
/// `contributes_to` targets come from them, and stay empty without them.
fn list_item(row: &Row, id: &str, edges: Option<&BTreeMap<String, EdgeEntry>>) -> ListItem {
    let mut i = ListItem::new(item_row(row, edges.and_then(|e| current_parent(id, e))));
    if let Some(edges) = edges {
        let targets: BTreeSet<String> = edges
            .values()
            .filter(|e| e.kind == EdgeKind::ContributesTo && e.from == id)
            .map(|e| e.to.clone())
            .collect();
        i.contributes_to = targets.into_iter().collect();
    }
    i
}

/// The revisions a stream must have been folded by for [`check_current`] to serve it: this crate's
/// index rules, and the task reducer's — every row the records read is a task row, so a stream
/// folded by an older task reducer lacks what that reducer folds since (revision 3, 6j6v.4g6r: a
/// stream that held entry ops an older server stored unfolded would serve empty lists; review of
/// PR #63, Code Quality #2). Not the chat's or the memory's: a board read reads none of their rows,
/// and the fold run refolds a stream for them on its own ([`Folder::is_current`]).
fn served_revisions() -> [(&'static str, i64); 2] {
    [
        (INDEX_DOMAIN, INDEX_REVISION),
        (DOMAIN_TASK, TaskReducer.fold_revision()),
    ]
}

/// This build's revision of `domain`, one of [`served_revisions`].
fn served_revision(domain: &str) -> i64 {
    served_revisions()
        .into_iter()
        .find(|(d, _)| *d == domain)
        .map_or(0, |(_, rev)| rev)
}

/// Refuse a stream folded by other index or task-reducer rules than this build reads
/// ([`served_revisions`]), and one being folded again that has not caught up yet. A cleared
/// stream's watermark names no revision ([`Folder::clear`]), so it is refused from the clear on;
/// once the refold has written this build's revisions it is still refused until it reaches the
/// position the clear named. A stream with no watermark at all has never been folded: it reads as
/// the empty board it is.
async fn check_current<T: Table>(table: &T) -> Result<(), RecordsError<T::Error>> {
    if let Some(mark) = Folder::watermark(table).await? {
        for (domain, revision) in served_revisions() {
            let found = mark.revisions.get(domain).copied();
            if found != Some(revision) {
                return Err(RecordsError::NotCurrent { domain, found });
            }
        }
        if let Some(until) = mark.refold_until.filter(|u| mark.folded_through < *u) {
            return Err(RecordsError::Refolding {
                folded_through: mark.folded_through,
                until,
            });
        }
    }
    Ok(())
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
    let row = table.get(&timestamps_key(id)).await?;
    Ok(stamps(row.as_ref()))
}

/// The sort key of the ticket's `item_timestamps` row.
fn timestamps_key(id: &str) -> String {
    key("item_timestamps", &[("item_id", id)])
}

/// The creation and last change an `item_timestamps` row holds, `''` read as none.
fn stamps(row: Option<&Row>) -> (Option<String>, Option<String>) {
    let stamp = |c: &str| {
        row.and_then(|r| text(r, c))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    (stamp("created_at"), stamp("updated_at"))
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

/// Every list field's present entries of the ticket, by field (6j6v.4g6r) — what
/// `Store::field_entries_of` answers on a replica, through the same projection
/// ([`model::present_entries`]). One Query on `field_entries#<ticket>#`: a row is one add, and a remove
/// marks the row it observed `removed`, so the range holds the tombstones too. A row with no value
/// was written by a remove whose add has not been folded, and is no entry.
async fn custom_lists<T: Table>(
    table: &T,
    id: &str,
) -> Result<BTreeMap<String, Vec<FieldEntry>>, T::Error> {
    let mut prefix = key("field_entries", &[("item_id", id)]);
    prefix.push('#');
    let mut adds: BTreeMap<String, Vec<EntryAdd>> = BTreeMap::new();
    for r in table.query_prefix(&prefix).await? {
        if text(&r, "item_id") != Some(id) || int(&r, "removed").unwrap_or(0) != 0 {
            continue;
        }
        let (Some(field), Some(value), Some(tag)) =
            (text(&r, "field"), text(&r, "value"), text(&r, "tag"))
        else {
            continue;
        };
        adds.entry(field.to_string()).or_default().push(EntryAdd {
            value: value.to_string(),
            label: text(&r, "label").map(str::to_string),
            lamport: int(&r, "lamport").unwrap_or(-1),
            site: int(&r, "site").unwrap_or(-1),
            tag: tag.to_string(),
        });
    }
    Ok(adds
        .into_iter()
        .map(|(field, adds)| (field, model::present_entries(adds)))
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
