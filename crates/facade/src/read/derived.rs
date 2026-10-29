//! Derived ranking keys on the store path (docs/specs/derived-ranking-keys.md, 6j6v.x0wf): the
//! values a ranking's `related` / `count` / `sum` keys compare by, computed once per query.
//!
//! **Bottom-up, once per query** (spec §4, §5). A query ranks a set of rows by the orders of their
//! types. The `of = "rank"` keys of those orders name other per-type orders (the type chain, kept
//! acyclic at load); those may name others in turn. [`compute`] walks the chain from the leaves:
//!
//! 1. one EDGE PASS ([`Store::present_edges_of_kinds`]) over every present edge of the kinds the
//!    keys name, for the ranked rows and every row in the chain's scopes at once. A `parent` edge
//!    is the `present_parent` projection — each child's one current parent, the one `belongs_to`,
//!    `--in` and `show` read — so a child re-parented concurrently counts under one parent only.
//!    The reverse mentions come from [`mentioners_of`], the one implementation `mention by` reads
//!    too (spec §11);
//! 2. one bulk read of the neighbours' rows, and of their labels / custom fields when a key needs
//!    them;
//! 3. per type in the chain, leaves first: its rows' values, ONE sort, and a dense rank. A
//!    neighbour's rank is then a lookup, never a recursion;
//! 4. the ranked rows' values, which the comparator reads per pair.
//!
//! So a query costs O(E + N log N) in the relevant edges and the items in scope, and a plugin
//! that declares no derived key pays nothing: [`compute`] returns `None` before any read.
//!
//! **Without a store** (6j6v.1c5s). The store-free `next` ([`super::next_active`]) ranks a board a
//! server read: [`compute_active`] takes steps 1 and 2 from the board's [`RankingGraph`] — the
//! edges, already projected and filtered to live ends, and the live neighbours' rows, labels and
//! custom values — and runs steps 3 and 4 on the same [`Context`], so the two paths cannot evaluate
//! a key apart. A neighbour counts when it is LIVE, closed and archived ones included, on both
//! paths (spec §6, owner 2026-10-11). What the graph must hold is [`reads_for_next`].
//!
//! The store-free `list` (6j6v.xkr7, [`super::list_board_page`]) ranks a [`ListBoard`] the same
//! way: [`compute_list`] takes the listed rows from the board, `next`'s candidates — when a key
//! reads a rank `next` gives — from its active board, and every other row from its graph. A
//! `[ranking.list]` rank a key reads is computed over the type's live items, which the graph holds
//! for every type in [`RankingReads::list_types`] (the `typed` index on a server), so a `next` key
//! may read one too ([`compute_active`]). What a `list` reads is [`reads_for_list`].

use super::{
    lanes, mentioners_of, resolve_items, ActiveBoard, ListBoard, ListQuery, ListReads, ListSelect,
    RankingGraph, RankingReads, SortKey, DEFAULT_SORT_LIST,
};
use crate::error::{NxfError, Result};
use crate::plugin::{Dir, Nulls, Pick, PluginConfig, RankKey, RankOf, RankSpec, Verb};
use crate::record::{raw_field, RANKABLE_FIELDS};
use nexus_flow_core::model::{EdgeKind, ItemRow};
use nexus_flow_core::store::Store;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One derived key's value for one item. Compares like a field value: numbers before text,
/// nulls by the key's `nulls`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DVal {
    /// No value: no neighbour (or none with a value, or none in scope).
    Null,
    /// A count, a rank, a precedence ordinal, or a field value that parses as an integer.
    Num(i64),
    /// Any other field value.
    Text(String),
}

impl DVal {
    /// A raw field value as the comparator reads it — numeric when it parses as an integer, as
    /// [`cmp_rank_key`](super::cmp_rank_key) does.
    fn of_raw(v: Option<String>) -> DVal {
        match v {
            None => DVal::Null,
            Some(s) => match s.parse::<i64>() {
                Ok(n) => DVal::Num(n),
                Err(_) => DVal::Text(s),
            },
        }
    }
}

/// The ascending order of two values, numbers before text (review of #44, Integrity #2). A null
/// sorts after both; [`cmp_dval`] places nulls itself, so that arm only keeps the order total.
fn base_cmp(a: &DVal, b: &DVal) -> Ordering {
    match (a, b) {
        (DVal::Num(x), DVal::Num(y)) => x.cmp(y),
        (DVal::Num(_), DVal::Text(_)) => Ordering::Less,
        (DVal::Text(_), DVal::Num(_)) => Ordering::Greater,
        (DVal::Text(x), DVal::Text(y)) => x.cmp(y),
        (DVal::Null, DVal::Null) => Ordering::Equal,
        (DVal::Null, _) => Ordering::Greater,
        (_, DVal::Null) => Ordering::Less,
    }
}

/// Compare two values of one derived key with its `dir` and `nulls` — the field key's rules.
pub(super) fn cmp_dval(a: &DVal, b: &DVal, dir: Dir, nulls: Nulls) -> Ordering {
    match (a, b) {
        (DVal::Null, DVal::Null) => Ordering::Equal,
        (DVal::Null, _) => match nulls {
            Nulls::First => Ordering::Less,
            Nulls::Last => Ordering::Greater,
        },
        (_, DVal::Null) => match nulls {
            Nulls::First => Ordering::Greater,
            Nulls::Last => Ordering::Less,
        },
        _ => match dir {
            Dir::Asc => base_cmp(a, b),
            Dir::Desc => base_cmp(a, b).reverse(),
        },
    }
}

/// The computed values of one query: for each ranked row, one value per key of the order it ranks
/// by, aligned with that order — a derived key's, a custom-field key's (6j6v.z19s); a canonical
/// field key's position holds [`DVal::Null`] and is never read.
#[derive(Debug, Default)]
pub(super) struct Derived {
    values: HashMap<String, Vec<DVal>>,
}

impl Derived {
    /// The values of row `id`, aligned with its order.
    pub(super) fn of(&self, id: &str) -> Option<&[DVal]> {
        self.values.get(id).map(Vec::as_slice)
    }
}

/// What the caller already holds of the scopes "rank" is computed over (spec §4), so the pass
/// reads them only when the caller does not have them.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Scopes<'a> {
    /// `next`'s actionable candidates over the whole board, before any filter.
    pub(super) next: Option<&'a [ItemRow]>,
    /// The live items `list` returns without filters or paging (not deleted, not archived).
    pub(super) list: Option<&'a [ItemRow]>,
    /// The reference time, to compute `next`'s candidates when a `list` key reads the rank of a
    /// type `next` ranks.
    pub(super) now: Option<&'a str>,
}

/// A per-type order in the type chain: the verb whose policy declares it, and the type.
type Node = (Verb, String);

/// `(edge kind, endpoint) → the other endpoints`, one direction of the edge pass.
type Adjacency = HashMap<(EdgeKind, String), Vec<String>>;

/// The live items `list` returns without filters: not deleted, not archived.
pub(super) fn list_scope(store: &Store) -> Result<Vec<ItemRow>> {
    Ok(store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .filter(|i| i.archived.is_none())
        .collect())
}

/// A live row and its joins, wherever a store-free board holds it: an active ticket, a listed item,
/// or a neighbour in the graph.
#[derive(Clone, Copy)]
pub(super) struct Joined<'b> {
    item: &'b ItemRow,
    labels: &'b [String],
    custom: &'b BTreeMap<String, String>,
}

/// The row `id` of an [`ActiveBoard`] with its `graph`: an active ticket's, else a neighbour's.
fn active_joined<'b>(
    board: &'b ActiveBoard,
    graph: &'b RankingGraph,
    id: &str,
) -> Option<Joined<'b>> {
    match board.tickets.get(id) {
        Some(t) => Some(Joined {
            item: &t.item,
            labels: &t.labels,
            custom: &t.custom,
        }),
        None => graph.neighbours.get(id).map(|n| Joined {
            item: &n.item,
            labels: &n.labels,
            custom: &n.custom,
        }),
    }
}

/// The row `id` of a [`ListBoard`] with its `graph`: a listed item's, else an active ticket's,
/// else a neighbour's.
fn list_joined<'b>(board: &'b ListBoard, graph: &'b RankingGraph, id: &str) -> Option<Joined<'b>> {
    if let Some(i) = board.items.get(id) {
        return Some(Joined {
            item: &i.item,
            labels: &i.labels,
            custom: &i.custom,
        });
    }
    match &board.active {
        Some(active) => active_joined(active, graph, id),
        None => graph.neighbours.get(id).map(|n| Joined {
            item: &n.item,
            labels: &n.labels,
            custom: &n.custom,
        }),
    }
}

/// The derived side of one query, decided before any read: the type chain the ranked rows' orders
/// read the rank of (leaves first), and what the derived keys in play read.
struct Plan<'c> {
    cfg: &'c PluginConfig,
    verb: Verb,
    spec: &'c RankSpec,
    chain: Vec<Node>,
    needs: Needs,
}

/// The [`Plan`] for ranking `ranked` under `verb` — `None` when the policy has no derived key and
/// no custom-field key (6j6v.z19s).
fn plan<'c>(cfg: &'c PluginConfig, verb: Verb, ranked: &[ItemRow]) -> Option<Plan<'c>> {
    let ranking = &cfg.ranking;
    let spec = ranking.spec(verb);
    if !spec.has_computed() {
        return None;
    }

    // The chain, leaves first: the orders the ranked rows' orders read the rank of, transitively.
    let ranked_types: BTreeSet<Option<&str>> =
        ranked.iter().map(|i| i.item_type.as_deref()).collect();
    let mut chain: Vec<Node> = Vec::new();
    let mut seen: BTreeSet<Node> = BTreeSet::new();
    for ty in &ranked_types {
        for dep in ranking.rank_deps(spec.order_for(*ty), verb) {
            post_order(cfg, dep, &mut seen, &mut chain);
        }
    }

    // Every computed key in play.
    let mut keys: Vec<&RankKey> = Vec::new();
    for ty in &ranked_types {
        keys.extend(spec.order_for(*ty).iter().filter(|k| k.is_computed()));
    }
    keys.extend(chain_keys(cfg, &chain));
    let needs = Needs::of(&keys);
    Some(Plan {
        cfg,
        verb,
        spec,
        chain,
        needs,
    })
}

impl Plan<'_> {
    /// The rows each order of the chain ranks: its type's rows in the scope of its verb.
    fn node_rows<'r>(&self, next: &'r [ItemRow], list: &'r [ItemRow]) -> Vec<Vec<&'r ItemRow>> {
        self.chain
            .iter()
            .map(|(v, ty)| {
                let scope = match v {
                    Verb::Next => next,
                    Verb::List => list,
                };
                scope
                    .iter()
                    .filter(|i| i.item_type.as_deref() == Some(ty.as_str()))
                    .collect()
            })
            .collect()
    }

    /// Whether some order of the chain is a `[ranking.list]` order.
    fn reads_list(&self) -> bool {
        self.chain.iter().any(|(v, _)| *v == Verb::List)
    }

    /// Whether some order of the chain is a `[ranking.next]` order.
    fn reads_next(&self) -> bool {
        self.chain.iter().any(|(v, _)| *v == Verb::Next)
    }

    /// What a [`RankingGraph`] must hold for this plan: its keys' reads, and the live items of
    /// every `[ranking.list]` type of the chain the board does not list itself (`held`).
    fn reads(&self, held: impl Fn(&str) -> bool) -> RankingReads {
        let mut reads = self.needs.reads();
        reads.custom |= graph_custom(self.cfg, &self.chain, self.verb, &held);
        reads.list_types = self
            .chain
            .iter()
            .filter(|(v, ty)| *v == Verb::List && !held(ty))
            .map(|(_, ty)| ty.clone())
            .collect();
        reads
    }
}

/// The rows of a graph's listed items (the `list` scope of [`RankingReads::list_types`]), looked
/// up through `rows` — every one of them is a live row the board holds.
fn listed_rows<'b>(
    graph: &RankingGraph,
    rows: impl Fn(&str) -> Option<Joined<'b>>,
) -> Vec<ItemRow> {
    graph
        .listed
        .iter()
        .filter_map(|id| rows(id).map(|j| j.item.clone()))
        .collect()
}

/// The graph a store-free query ranks with: the board's, when it was read for `needed`; an empty
/// one when the keys in play read nothing beyond the rows. `validation` otherwise — ranking without
/// those edges would order apart from the store path in silence.
fn graph_for<'g>(
    graph: Option<&'g RankingGraph>,
    needed: &RankingReads,
    empty: &'g RankingGraph,
    what: &str,
) -> Result<&'g RankingGraph> {
    if *needed == RankingReads::default() {
        return Ok(graph.unwrap_or(empty));
    }
    graph.filter(|g| g.reads.covers(needed)).ok_or_else(|| {
        NxfError::validation(format!(
            "{what} ranks by derived ranking keys, and the board was read without the graph they \
             read — fill its `ranking` for the reads its plugin names"
        ))
    })
}

/// Every row a key is evaluated on: the ranked rows and the rows of the chain's scopes.
fn subjects<'a>(
    ranked: &'a [ItemRow],
    node_rows: &[Vec<&'a ItemRow>],
) -> HashMap<&'a str, &'a ItemRow> {
    let mut subjects: HashMap<&str, &ItemRow> = HashMap::new();
    for row in ranked.iter().chain(node_rows.iter().flatten().copied()) {
        subjects.insert(row.id.as_str(), row);
    }
    subjects
}

/// The values the derived keys of `verb`'s policy rank `ranked` by — `None`, before any read,
/// when the policy has no derived key. `validation` when a `list` key reads the rank of a type
/// `next` ranks and the caller passed neither `next`'s candidates nor a reference time.
pub(super) fn compute(
    cfg: &PluginConfig,
    store: &Store,
    verb: Verb,
    ranked: &[ItemRow],
    scopes: Scopes<'_>,
) -> Result<Option<Derived>> {
    let Some(plan) = plan(cfg, verb, ranked) else {
        return Ok(None);
    };

    // The scopes the chain ranks over, read only when the caller does not hold them.
    let next_owned;
    let next_rows: &[ItemRow] = match scopes.next {
        _ if !plan.chain.iter().any(|(v, _)| *v == Verb::Next) => &[],
        Some(rows) => rows,
        None => {
            let now = scopes.now.ok_or_else(|| {
                NxfError::validation(
                    "the plugin's [ranking.list] reads the rank of a type that `next` ranks, \
                     which is computed over `next`'s candidates at a reference time — pass `now`",
                )
            })?;
            let candidates = lanes(store, now)?.candidates;
            let ids: Vec<String> = candidates.into_iter().map(|c| c.id).collect();
            next_owned = resolve_items(store, &ids)?;
            &next_owned
        }
    };
    let list_owned;
    let list_rows: &[ItemRow] = match scopes.list {
        _ if !plan.reads_list() => &[],
        Some(rows) => rows,
        None => {
            list_owned = list_scope(store)?;
            &list_owned
        }
    };
    let node_rows = plan.node_rows(next_rows, list_rows);
    let mut ctx = Context::new(cfg, subjects(ranked, &node_rows));
    ctx.read_edges(store, &plan.needs)?;
    Ok(Some(ctx.finish(&plan, verb, ranked, &node_rows)))
}

/// [`compute`] for the store-free `next` (6j6v.1c5s): the values the derived keys of
/// `[ranking.next]` rank `candidates` — every candidate of the board — by, read from the
/// [`RankingGraph`] a reader filled instead of from a store. The candidates are `next`'s scope for
/// "rank", as on the store path; the `list` scope of a `[ranking.list]` type the chain reaches is
/// the graph's listed items (6j6v.xkr7). `None` when the policy has no derived key.
///
/// The same [`Context`] evaluates the keys as on the store path; only where its edges, rows,
/// labels and custom values come from differs ([`Context::read_graph`]). The caller,
/// [`next_active`](super::next_active), has refused a graph read for less than
/// [`next_ranking_reads`](super::next_ranking_reads) names, which covers every plan of the section.
pub(super) fn compute_active(
    cfg: &PluginConfig,
    board: &ActiveBoard,
    graph: &RankingGraph,
    candidates: &[ItemRow],
) -> Result<Option<Derived>> {
    let Some(plan) = plan(cfg, Verb::Next, candidates) else {
        return Ok(None);
    };
    // `next_active` admitted the graph only when it covers `next_ranking_reads`, which gathers the
    // reads of every order of the section and of its chain — so of this plan's too.
    debug_assert!(graph.reads.covers(&plan.reads(|_| false)));
    let rows = |id: &str| active_joined(board, graph, id);
    let listed = listed_rows(graph, rows);
    let node_rows = plan.node_rows(candidates, &listed);
    let mut ctx = Context::new(cfg, subjects(candidates, &node_rows));
    ctx.read_graph(rows, graph, &plan.needs);
    Ok(Some(ctx.finish(&plan, Verb::Next, candidates, &node_rows)))
}

/// [`compute`] for the store-free `list` (6j6v.xkr7): the values the derived keys of
/// `[ranking.list]` rank `ranked` by, from a [`ListBoard`]. The `list` scope of a type is its live,
/// unarchived items — the board's listed items for the types it lists, the graph's listed items
/// for the others; `next`'s scope, when the chain reaches a `[ranking.next]` order, is the
/// candidates of the board's active board at `now`. `None` when the policy has no derived key.
///
/// `validation` as on the store path when the chain reads a `next` rank and `now` is missing, and
/// when the board was read for less than the plan needs: no active board for such a chain, a
/// graph read for fewer reads, a chain type neither listed nor in the graph.
pub(super) fn compute_list(
    cfg: &PluginConfig,
    board: &ListBoard,
    ranked: &[ItemRow],
    now: Option<&str>,
) -> Result<Option<Derived>> {
    let Some(plan) = plan(cfg, Verb::List, ranked) else {
        return Ok(None);
    };
    let held = |ty: &str| board.types.as_ref().is_none_or(|t| t.contains(ty));
    let needed = plan.reads(held);
    let empty = RankingGraph::default();
    let graph = graph_for(
        board.ranking.as_ref(),
        &needed,
        &empty,
        &format!("plugin '{}' (`list`)", cfg.name),
    )?;
    let next_owned;
    let next_rows: &[ItemRow] = if plan.reads_next() {
        let now = now.ok_or_else(|| {
            NxfError::validation(
                "the plugin's [ranking.list] reads the rank of a type that `next` ranks, which is \
                 computed over `next`'s candidates at a reference time — pass `now`",
            )
        })?;
        let active = board.active.as_ref().ok_or_else(|| {
            NxfError::validation(format!(
                "plugin '{}' ranks `list` by the rank of a type that `next` ranks, and the board \
                 was read without the active board that rank is computed over — read it with \
                 `ListReads::next`",
                cfg.name
            ))
        })?;
        let candidates = active.board.lanes(now).candidates;
        next_owned = active.items(candidates.iter().map(|c| &c.id));
        &next_owned
    } else {
        &[]
    };
    let rows = |id: &str| list_joined(board, graph, id);
    let list_owned: Vec<ItemRow> = if plan.reads_list() {
        let mut scope: Vec<ItemRow> = board.items.values().map(|i| i.item.clone()).collect();
        scope.extend(
            listed_rows(graph, rows)
                .into_iter()
                .filter(|i| !board.items.contains_key(&i.id)),
        );
        scope
    } else {
        Vec::new()
    };
    let node_rows = plan.node_rows(next_rows, &list_owned);
    let mut ctx = Context::new(cfg, subjects(ranked, &node_rows));
    ctx.read_graph(rows, graph, &plan.needs);
    Ok(Some(ctx.finish(&plan, Verb::List, ranked, &node_rows)))
}

/// What the store-free `next` of `cfg` reads beyond the active tickets' records (6j6v.1c5s):
/// `None` when `[ranking.next]` declares no derived key. Gathered over every order of the section
/// and of its type chain, so it does not depend on which types a board holds. Since 6j6v.xkr7 an
/// order that reads the rank of a type only `[ranking.list]` orders is no longer refused: the
/// type lands in [`RankingReads::list_types`], whose live items a reader puts in the graph.
pub(super) fn reads_for_next(cfg: &PluginConfig) -> Result<Option<RankingReads>> {
    let spec = cfg.ranking.spec(Verb::Next);
    if !spec.has_computed() {
        return Ok(None);
    }
    let orders: Vec<&[RankKey]> = std::iter::once(spec.order.as_slice())
        .chain(spec.by_type.values().map(|o| o.order.as_slice()))
        .collect();
    // `next`'s own per-type orders start the chain: an order there may read another's rank.
    let starts = spec.by_type.keys().map(|ty| (Verb::Next, ty.clone()));
    let reads = chain_reads(cfg, Verb::Next, &orders, starts, |_| false).0;
    // Custom-field keys alone read the tickets' own custom values, which every board's records
    // carry (6j6v.z19s): no graph to read.
    Ok((reads != RankingReads::default()).then_some(reads))
}

/// What the store-free `list` `q` asks for reads (6j6v.xkr7): its listed types (`None`: every
/// type), whether `next`'s board is needed, and the graph. Only `sort = rank` ranks by the
/// derived keys; any other order reads the listed items alone.
pub(super) fn reads_for_list(cfg: &PluginConfig, q: &ListQuery) -> ListReads {
    let types: Option<BTreeSet<String>> =
        (!q.filter.types.is_empty()).then(|| q.filter.types.iter().cloned().collect());
    let mut reads = ListReads {
        types: types.clone(),
        next: false,
        ranking: None,
        select: None,
        refs: q.refs.clone(),
    };
    // Any order but `rank` reads the item rows alone: the reader may keep and cut before it reads
    // an item's joins (review of PR #55, Integrity #1).
    let sort = q.sort.unwrap_or(DEFAULT_SORT_LIST);
    if sort != SortKey::Rank {
        reads.select = Some(ListSelect {
            status: q.status.clone(),
            filter: q.filter.clone(),
            sort,
            limit: q.limit,
            refs: q.refs.clone(),
        });
        return reads;
    }
    let spec = cfg.ranking.spec(Verb::List);
    if !spec.has_computed() {
        return reads;
    }
    let orders: Vec<&[RankKey]> = match &types {
        Some(types) => types
            .iter()
            .map(|ty| spec.order_for(Some(ty.as_str())))
            .collect(),
        None => std::iter::once(spec.order.as_slice())
            .chain(spec.by_type.values().map(|o| o.order.as_slice()))
            .collect(),
    };
    let starts: Vec<Node> = orders
        .iter()
        .flat_map(|keys| cfg.ranking.rank_deps(keys, Verb::List))
        .collect();
    let held = |ty: &str| types.as_ref().is_none_or(|t| t.contains(ty));
    let (graph, reads_next) = chain_reads(cfg, Verb::List, &orders, starts, held);
    // Without `now` a chain that reads a `next` rank answers `validation` where a ranked row
    // needs it, as on the store path — reading `next`'s board for it would be wasted.
    reads.next = reads_next && q.now.is_some();
    if graph != RankingReads::default() {
        reads.ranking = Some(graph);
    }
    reads
}

/// The reads of `orders` — the orders a store-free `verb` may rank by — and of every order down the
/// type chain from `starts`, the chain's `[ranking.list]` types not `held` among them, and whether
/// that chain holds a `[ranking.next]` order.
fn chain_reads(
    cfg: &PluginConfig,
    verb: Verb,
    orders: &[&[RankKey]],
    starts: impl IntoIterator<Item = Node>,
    held: impl Fn(&str) -> bool,
) -> (RankingReads, bool) {
    let mut chain: Vec<Node> = Vec::new();
    let mut seen: BTreeSet<Node> = BTreeSet::new();
    for node in starts {
        post_order(cfg, node, &mut seen, &mut chain);
    }
    let mut keys: Vec<&RankKey> = orders
        .iter()
        .flat_map(|o| o.iter())
        .filter(|k| k.is_computed())
        .collect();
    keys.extend(chain_keys(cfg, &chain));
    let mut reads = Needs::of(&keys).reads();
    reads.custom |= graph_custom(cfg, &chain, verb, &held);
    reads.list_types = chain
        .iter()
        .filter(|(v, ty)| *v == Verb::List && !held(ty))
        .map(|(_, ty)| ty.clone())
        .collect();
    (reads, chain.iter().any(|(v, _)| *v == Verb::Next))
}

/// The computed keys of the orders of `chain`.
fn chain_keys<'c>(
    cfg: &'c PluginConfig,
    chain: &'c [Node],
) -> impl Iterator<Item = &'c RankKey> + 'c {
    chain
        .iter()
        .flat_map(|(v, ty)| cfg.ranking.spec(*v).by_type[ty].order.iter())
        .filter(|k| k.is_computed())
}

/// Whether a custom-field key (6j6v.z19s) of an order of `chain` ranks rows a store-free board of
/// `verb` holds only in its graph, so their custom values are read from it: under `next`, the
/// listed items of a `[ranking.list]` type; under `list`, `next`'s tickets, and the listed items of
/// a type the board does not list itself (not `held`). The rows a board holds itself — `next`'s
/// candidates, `list`'s listed items — carry their custom values in their records.
fn graph_custom(
    cfg: &PluginConfig,
    chain: &[Node],
    verb: Verb,
    held: impl Fn(&str) -> bool,
) -> bool {
    chain.iter().any(|(v, ty)| {
        let in_graph = *v != verb || (*v == Verb::List && !held(ty));
        in_graph
            && cfg.ranking.spec(*v).by_type[ty]
                .order
                .iter()
                .any(RankKey::is_custom)
    })
}

/// Depth-first post-order over the type chain: every order a node reads the rank of lands before
/// it. The chain is acyclic (refused at load otherwise); `seen` is marked on entry, so even a
/// cycle could not recurse forever — its back edge would simply read no rank.
fn post_order(cfg: &PluginConfig, node: Node, seen: &mut BTreeSet<Node>, out: &mut Vec<Node>) {
    if !seen.insert(node.clone()) {
        return;
    }
    let keys = &cfg.ranking.spec(node.0).by_type[&node.1].order;
    for dep in cfg.ranking.rank_deps(keys, node.0) {
        post_order(cfg, dep, seen, out);
    }
    out.push(node);
}

/// What the keys in play need read, gathered before the reads.
#[derive(Debug, Default)]
struct Needs {
    /// The edge kinds of the one pass.
    kinds: Vec<EdgeKind>,
    /// A `count = "mentioned_by"` is in play: the reverse mentions through [`mentioners_of`].
    mentioned_by: bool,
    /// A count filters on a label.
    labels: bool,
    /// An `of` reads a custom field.
    custom: bool,
    /// A custom-field key ranks (6j6v.z19s): the subjects' own custom values.
    own_custom: bool,
}

impl Needs {
    fn of(keys: &[&RankKey]) -> Needs {
        let mut needs = Needs::default();
        let add = |needs: &mut Needs, kind: Option<EdgeKind>| {
            if let Some(k) = kind {
                if !needs.kinds.contains(&k) {
                    needs.kinds.push(k);
                }
            }
        };
        for key in keys {
            needs.own_custom |= key.is_custom();
            if let Some(edge) = &key.related {
                add(&mut needs, EdgeKind::parse(edge));
                if let Some(RankOf::Field(of)) = &key.of {
                    needs.custom |= !RANKABLE_FIELDS.contains(&of.field.as_str());
                }
            }
            for c in counts_of(key) {
                match c.kind {
                    "mentioned_by" => needs.mentioned_by = true,
                    kind => add(&mut needs, count_edge(kind).map(|(k, _)| k)),
                }
                needs.labels |= c.label.is_some();
            }
        }
        needs
    }
}

impl Needs {
    /// These needs as the [`RankingReads`] of a graph: `mentions` among the kinds when a count reads
    /// `mentioned_by`. No list types; the caller adds them.
    fn reads(&self) -> RankingReads {
        let mut reads = RankingReads::default();
        reads.kinds.extend(self.kinds.iter().copied());
        if self.mentioned_by {
            reads.kinds.insert(EdgeKind::Mentions);
        }
        reads.labels = self.labels;
        reads.custom = self.custom;
        reads
    }
}

/// One count, from a `count` key or a `sum` summand.
struct Count<'a> {
    kind: &'a str,
    ty: Option<&'a str>,
    status: Option<&'a [String]>,
    label: Option<&'a str>,
}

/// The counts of a key: its own for a `count`, its summands for a `sum`, none else.
fn counts_of(key: &RankKey) -> Vec<Count<'_>> {
    if let Some(kind) = &key.count {
        return vec![Count {
            kind,
            ty: key.item_type.as_deref(),
            status: key.status.as_deref(),
            label: key.label.as_deref(),
        }];
    }
    key.sum
        .iter()
        .flatten()
        .map(|p| Count {
            kind: &p.count,
            ty: p.item_type.as_deref(),
            status: p.status.as_deref(),
            label: p.label.as_deref(),
        })
        .collect()
}

/// A count kind as an edge kind and a direction (`true`: inbound). `mentioned_by` is not here: it
/// reads [`mentioners_of`].
fn count_edge(kind: &str) -> Option<(EdgeKind, bool)> {
    match kind {
        "children" => Some((EdgeKind::Parent, true)),
        "contributors" => Some((EdgeKind::ContributesTo, true)),
        "dependents" => Some((EdgeKind::Dep, true)),
        out => EdgeKind::parse(out).map(|k| (k, false)),
    }
}

/// Everything one query's derived values are computed from.
struct Context<'a> {
    cfg: &'a PluginConfig,
    /// The ranked rows and every row of the chain's scopes — live by construction.
    subjects: HashMap<&'a str, &'a ItemRow>,
    /// Outbound edges from a subject: `(kind, from) → [to]`.
    out: Adjacency,
    /// Inbound edges to a subject: `(kind, to) → [from]`.
    inb: Adjacency,
    /// The live items mentioning each subject ([`mentioners_of`]).
    mentioners: BTreeMap<String, Vec<ItemRow>>,
    /// The live neighbours that are not subjects.
    neighbours: HashMap<String, ItemRow>,
    /// The labels of the neighbours, when a count filters on one.
    labels: BTreeMap<String, Vec<String>>,
    /// The custom fields of the neighbours, when an `of` reads one, and of the subjects, when a
    /// custom-field key ranks them.
    custom: BTreeMap<String, BTreeMap<String, String>>,
    /// The dense rank of each order of the chain computed so far: `node → id → rank`.
    ranks: HashMap<Node, HashMap<String, i64>>,
}

impl<'a> Context<'a> {
    /// A context over `subjects`, nothing read yet.
    fn new(cfg: &'a PluginConfig, subjects: HashMap<&'a str, &'a ItemRow>) -> Context<'a> {
        Context {
            cfg,
            subjects,
            out: Adjacency::new(),
            inb: Adjacency::new(),
            mentioners: BTreeMap::new(),
            neighbours: HashMap::new(),
            labels: BTreeMap::new(),
            custom: BTreeMap::new(),
            ranks: HashMap::new(),
        }
    }

    /// Rank the chain bottom-up — each order its scope once, its dependents look the rank up — and
    /// then the ranked rows' values.
    fn finish(
        mut self,
        plan: &Plan<'_>,
        verb: Verb,
        ranked: &[ItemRow],
        node_rows: &[Vec<&ItemRow>],
    ) -> Derived {
        let ranking = &self.cfg.ranking;
        for (node, rows) in plan.chain.iter().zip(node_rows) {
            let keys = &ranking.spec(node.0).by_type[&node.1].order;
            let values: Vec<Vec<DVal>> =
                rows.iter().map(|r| self.values(keys, node.0, r)).collect();
            let mut order: Vec<usize> = (0..rows.len()).collect();
            let cmp = |a: usize, b: usize| {
                super::keys_cmp_values(rows[a], rows[b], keys, Some(&values[a]), Some(&values[b]))
            };
            order.sort_by(|a, b| cmp(*a, *b).then_with(|| rows[*a].id.cmp(&rows[*b].id)));
            // A DENSE rank over the declared keys (spec §4): rows equal on every key share a rank,
            // and the next distinct row takes the next integer. The id orders the sort, never the
            // rank.
            let mut ranks: HashMap<String, i64> = HashMap::with_capacity(rows.len());
            let mut rank = 0_i64;
            for (n, &i) in order.iter().enumerate() {
                if n > 0 && cmp(order[n - 1], i) != Ordering::Equal {
                    rank += 1;
                }
                ranks.insert(rows[i].id.clone(), rank);
            }
            self.ranks.insert(node.clone(), ranks);
        }
        let values = ranked
            .iter()
            .map(|r| {
                let keys = plan.spec.order_for(r.item_type.as_deref());
                (r.id.clone(), self.values(keys, verb, r))
            })
            .collect();
        Derived { values }
    }

    /// The reads from a [`RankingGraph`] instead of a store (6j6v.1c5s): the same adjacency, reverse
    /// mentions, neighbour rows, labels and custom values [`read_edges`](Self::read_edges) reads,
    /// taken from the graph's edges and, through `rows`, from the board's rows — its active
    /// tickets, its listed items — and the graph's neighbours. The graph holds every edge of the
    /// kinds read that touches a subject with both ends live, `parent` as the `present_parent`
    /// projection — the edges the store's pass yields for the same subjects.
    fn read_graph<'b>(
        &mut self,
        rows: impl Fn(&str) -> Option<Joined<'b>>,
        graph: &RankingGraph,
        needs: &Needs,
    ) {
        let mut mention_sources: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for e in &graph.edges {
            if needs.kinds.contains(&e.kind) {
                if self.subjects.contains_key(e.to.as_str()) {
                    self.inb
                        .entry((e.kind, e.to.clone()))
                        .or_default()
                        .push(e.from.clone());
                }
                if self.subjects.contains_key(e.from.as_str()) {
                    self.out
                        .entry((e.kind, e.from.clone()))
                        .or_default()
                        .push(e.to.clone());
                }
            }
            if needs.mentioned_by
                && e.kind == EdgeKind::Mentions
                && self.subjects.contains_key(e.to.as_str())
            {
                mention_sources
                    .entry(e.to.clone())
                    .or_default()
                    .insert(e.from.as_str());
            }
        }
        for (target, froms) in mention_sources {
            let found: Vec<ItemRow> = froms
                .into_iter()
                .filter_map(|f| rows(f).map(|j| j.item.clone()))
                .collect();
            if !found.is_empty() {
                self.mentioners.insert(target, found);
            }
        }
        let missing: BTreeSet<&str> = self
            .out
            .values()
            .chain(self.inb.values())
            .flatten()
            .map(String::as_str)
            .filter(|id| !self.subjects.contains_key(id))
            .collect();
        self.neighbours = missing
            .into_iter()
            .filter_map(|id| rows(id).map(|j| (id.to_string(), j.item.clone())))
            .collect();
        let custom = needs.custom || needs.own_custom;
        if needs.labels || custom {
            let mut ids: BTreeSet<&str> = self.neighbours.keys().map(String::as_str).collect();
            ids.extend(self.subjects.keys().copied());
            ids.extend(self.mentioners.values().flatten().map(|i| i.id.as_str()));
            for id in ids {
                let Some(j) = rows(id) else { continue };
                if needs.labels && !j.labels.is_empty() {
                    self.labels.insert(id.to_string(), j.labels.to_vec());
                }
                if custom && !j.custom.is_empty() {
                    self.custom.insert(id.to_string(), j.custom.clone());
                }
            }
        }
    }

    /// The reads: the edge pass, the reverse mentions, the neighbours' rows, labels and custom
    /// fields — each ONE bulk read (per 500 ids where the store chunks), whatever the row count.
    fn read_edges(&mut self, store: &Store, needs: &Needs) -> Result<()> {
        for (from, to, kind) in store.present_edges_of_kinds(&needs.kinds)? {
            if self.subjects.contains_key(to.as_str()) {
                self.inb
                    .entry((kind, to.clone()))
                    .or_default()
                    .push(from.clone());
            }
            if self.subjects.contains_key(from.as_str()) {
                self.out.entry((kind, from)).or_default().push(to);
            }
        }
        if needs.mentioned_by {
            let mut ids: Vec<&str> = self.subjects.keys().copied().collect();
            ids.sort_unstable();
            self.mentioners = mentioners_of(store, &ids)?;
        }
        let missing: BTreeSet<&str> = self
            .out
            .values()
            .chain(self.inb.values())
            .flatten()
            .map(String::as_str)
            .filter(|id| !self.subjects.contains_key(id))
            .collect();
        let missing: Vec<&str> = missing.into_iter().collect();
        self.neighbours = store
            .get_items(&missing)?
            .into_iter()
            .filter(|i| i.deleted.as_deref() != Some("1"))
            .map(|i| (i.id.clone(), i))
            .collect();
        let custom = needs.custom || needs.own_custom;
        if needs.labels || custom {
            let mut ids: BTreeSet<&str> = self.neighbours.keys().map(String::as_str).collect();
            ids.extend(self.subjects.keys().copied());
            ids.extend(self.mentioners.values().flatten().map(|i| i.id.as_str()));
            let ids: Vec<&str> = ids.into_iter().collect();
            if needs.labels {
                self.labels = store.labels_of_bulk(&ids)?;
            }
            if custom {
                self.custom = store.custom_fields_of_bulk(&ids)?;
            }
        }
        Ok(())
    }

    /// The live row `id`, a subject or a neighbour.
    fn row(&self, id: &str) -> Option<&ItemRow> {
        self.subjects
            .get(id)
            .copied()
            .or_else(|| self.neighbours.get(id))
    }

    /// The values of `keys` (an order under `verb`) for `item`, aligned with the keys.
    fn values(&self, keys: &[RankKey], verb: Verb, item: &ItemRow) -> Vec<DVal> {
        keys.iter()
            .map(|key| {
                if key.related.is_some() {
                    self.related(key, verb, item)
                } else if key.is_custom() {
                    self.own_custom(key, item)
                } else if key.is_derived() {
                    let n: usize = counts_of(key).iter().map(|c| self.count(c, item)).sum();
                    DVal::Num(i64::try_from(n).unwrap_or(i64::MAX))
                } else {
                    DVal::Null
                }
            })
            .collect()
    }

    /// A custom-field key's value for `item` (6j6v.z19s): its own custom value, compared as a field
    /// key compares it — numeric when it parses as an integer — or, with a `precedence` list, the
    /// value's position in it.
    fn own_custom(&self, key: &RankKey, item: &ItemRow) -> DVal {
        let raw = self
            .custom
            .get(&item.id)
            .and_then(|c| c.get(&key.field))
            .map(String::as_str);
        if key.precedence.is_empty() {
            DVal::of_raw(raw.map(str::to_string))
        } else {
            DVal::Num(i64::try_from(key.precedence_ordinal(raw)).unwrap_or(i64::MAX))
        }
    }

    /// A `related` key's value for `item`: `of` read on each live neighbour over the edge (of the
    /// key's `type`, if it names one), then the smallest or largest non-null value by `pick`.
    fn related(&self, key: &RankKey, verb: Verb, item: &ItemRow) -> DVal {
        let Some(kind) = key.related.as_deref().and_then(EdgeKind::parse) else {
            return DVal::Null;
        };
        let Some(targets) = self.out.get(&(kind, item.id.clone())) else {
            return DVal::Null;
        };
        let values = targets
            .iter()
            .filter_map(|id| self.row(id))
            .filter(|n| key.item_type.is_none() || n.item_type == key.item_type)
            .map(|n| self.read_of(key, verb, n))
            .filter(|v| *v != DVal::Null);
        let pick = key.pick.unwrap_or_default();
        let best = match pick {
            Pick::Min => values.min_by(base_cmp),
            Pick::Max => values.max_by(base_cmp),
        };
        best.unwrap_or(DVal::Null)
    }

    /// What `of` reads on the neighbour `n`.
    fn read_of(&self, key: &RankKey, verb: Verb, n: &ItemRow) -> DVal {
        match &key.of {
            Some(RankOf::Rank) => n
                .item_type
                .as_deref()
                .and_then(|ty| {
                    let src = self.cfg.ranking.rank_source(verb, ty)?;
                    self.ranks.get(&(src, ty.to_string()))?.get(&n.id).copied()
                })
                .map_or(DVal::Null, DVal::Num),
            Some(RankOf::Field(of)) => {
                let raw = if RANKABLE_FIELDS.contains(&of.field.as_str()) {
                    raw_field(n, &of.field)
                } else {
                    self.custom
                        .get(&n.id)
                        .and_then(|c| c.get(&of.field))
                        .cloned()
                };
                if of.precedence.is_empty() {
                    DVal::of_raw(raw)
                } else {
                    let pos = raw
                        .and_then(|v| of.precedence.iter().position(|p| *p == v))
                        .unwrap_or(of.precedence.len());
                    DVal::Num(i64::try_from(pos).unwrap_or(i64::MAX))
                }
            }
            // `of` is required on a `related` key (checked at load).
            _ => DVal::Null,
        }
    }

    /// How many live neighbours of `item` over the count's kind pass its filters.
    fn count(&self, c: &Count<'_>, item: &ItemRow) -> usize {
        let keeps = |n: &ItemRow| {
            c.ty.is_none_or(|t| n.item_type.as_deref() == Some(t))
                && c.status.is_none_or(|s| {
                    n.status
                        .as_deref()
                        .is_some_and(|x| s.iter().any(|y| y == x))
                })
                && c.label.is_none_or(|l| {
                    self.labels
                        .get(&n.id)
                        .is_some_and(|ls| ls.iter().any(|x| x == l))
                })
        };
        if c.kind == "mentioned_by" {
            return self
                .mentioners
                .get(&item.id)
                .map_or(0, |rows| rows.iter().filter(|n| keeps(n)).count());
        }
        let Some((kind, inbound)) = count_edge(c.kind) else {
            return 0;
        };
        let side = if inbound { &self.inb } else { &self.out };
        side.get(&(kind, item.id.clone())).map_or(0, |ids| {
            ids.iter()
                .filter_map(|id| self.row(id))
                .filter(|n| keeps(n))
                .count()
        })
    }
}
