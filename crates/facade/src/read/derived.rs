//! Derived ranking keys on the store path (docs/specs/derived-ranking-keys.md, 6j6v.x0wf): the
//! values a ranking's `related` / `count` / `sum` keys compare by, computed once per query.
//!
//! **Bottom-up, once per query** (spec §4, §5). A query ranks a set of rows by the orders of their
//! types. The `of = "rank"` keys of those orders name other per-type orders (the type chain, kept
//! acyclic at load); those may name others in turn. [`compute`] walks the chain from the leaves:
//!
//! 1. one EDGE PASS ([`Store::present_edges_of_kinds`]) over every present edge of the kinds the
//!    keys name, for the ranked rows and every row in the chain's scopes at once; the reverse
//!    mentions come from [`mentioners_of`], the one implementation `mention by` reads too
//!    (spec §11);
//! 2. one bulk read of the neighbours' rows, and of their labels / custom fields when a key needs
//!    them;
//! 3. per type in the chain, leaves first: its rows' values, ONE sort, and a dense rank. A
//!    neighbour's rank is then a lookup, never a recursion;
//! 4. the ranked rows' values, which the comparator reads per pair.
//!
//! So a query costs O(E + N log N) in the relevant edges and the items in scope, and a plugin
//! that declares no derived key pays nothing: [`compute`] returns `None` before any read.

use super::{lanes, mentioners_of, resolve_items};
use crate::error::{NxfError, Result};
use crate::plugin::{Dir, Nulls, Pick, PluginConfig, RankKey, RankOf, Verb};
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

/// The derived values of one query: for each ranked row, one value per key of the order it ranks
/// by, aligned with that order (a field key's position holds [`DVal::Null`] and is never read).
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
    let ranking = &cfg.ranking;
    let spec = ranking.spec(verb);
    if !spec.has_derived() {
        return Ok(None);
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

    // The scopes the chain ranks over, read only when the caller does not hold them.
    let next_owned;
    let next_rows: &[ItemRow] = match scopes.next {
        _ if !chain.iter().any(|(v, _)| *v == Verb::Next) => &[],
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
        _ if !chain.iter().any(|(v, _)| *v == Verb::List) => &[],
        Some(rows) => rows,
        None => {
            list_owned = list_scope(store)?;
            &list_owned
        }
    };
    let node_rows: Vec<Vec<&ItemRow>> = chain
        .iter()
        .map(|(v, ty)| {
            let scope = match v {
                Verb::Next => next_rows,
                Verb::List => list_rows,
            };
            scope
                .iter()
                .filter(|i| i.item_type.as_deref() == Some(ty.as_str()))
                .collect()
        })
        .collect();

    // Every row a key is evaluated on, and every derived key in play.
    let mut subjects: HashMap<&str, &ItemRow> = HashMap::new();
    for row in ranked.iter().chain(node_rows.iter().flatten().copied()) {
        subjects.insert(row.id.as_str(), row);
    }
    let mut keys: Vec<&RankKey> = Vec::new();
    for ty in &ranked_types {
        keys.extend(spec.order_for(*ty).iter().filter(|k| k.is_derived()));
    }
    for (v, ty) in &chain {
        keys.extend(
            ranking.spec(*v).by_type[ty]
                .order
                .iter()
                .filter(|k| k.is_derived()),
        );
    }

    let needs = Needs::of(&keys);
    let mut ctx = Context {
        cfg,
        subjects,
        out: Adjacency::new(),
        inb: Adjacency::new(),
        mentioners: BTreeMap::new(),
        neighbours: HashMap::new(),
        labels: BTreeMap::new(),
        custom: BTreeMap::new(),
        ranks: HashMap::new(),
    };
    ctx.read_edges(store, &needs)?;

    // Bottom-up: each order of the chain ranks its scope once; its dependents look the rank up.
    for (node, rows) in chain.iter().zip(&node_rows) {
        let keys = &ranking.spec(node.0).by_type[&node.1].order;
        let values: Vec<Vec<DVal>> = rows.iter().map(|r| ctx.values(keys, node.0, r)).collect();
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let cmp = |a: usize, b: usize| {
            super::keys_cmp_values(rows[a], rows[b], keys, Some(&values[a]), Some(&values[b]))
        };
        order.sort_by(|a, b| cmp(*a, *b).then_with(|| rows[*a].id.cmp(&rows[*b].id)));
        // A DENSE rank over the declared keys (spec §4): rows equal on every key share a rank, and
        // the next distinct row takes the next integer. The id orders the sort, never the rank.
        let mut ranks: HashMap<String, i64> = HashMap::with_capacity(rows.len());
        let mut rank = 0_i64;
        for (n, &i) in order.iter().enumerate() {
            if n > 0 && cmp(order[n - 1], i) != Ordering::Equal {
                rank += 1;
            }
            ranks.insert(rows[i].id.clone(), rank);
        }
        ctx.ranks.insert(node.clone(), ranks);
    }

    let values = ranked
        .iter()
        .map(|r| {
            let keys = spec.order_for(r.item_type.as_deref());
            (r.id.clone(), ctx.values(keys, verb, r))
        })
        .collect();
    Ok(Some(Derived { values }))
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
    /// The custom fields of the neighbours, when an `of` reads one.
    custom: BTreeMap<String, BTreeMap<String, String>>,
    /// The dense rank of each order of the chain computed so far: `node → id → rank`.
    ranks: HashMap<Node, HashMap<String, i64>>,
}

impl Context<'_> {
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
        if needs.labels || needs.custom {
            let mut ids: BTreeSet<&str> = self.neighbours.keys().map(String::as_str).collect();
            ids.extend(self.subjects.keys().copied());
            ids.extend(self.mentioners.values().flatten().map(|i| i.id.as_str()));
            let ids: Vec<&str> = ids.into_iter().collect();
            if needs.labels {
                self.labels = store.labels_of_bulk(&ids)?;
            }
            if needs.custom {
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
                } else if key.is_derived() {
                    let n: usize = counts_of(key).iter().map(|c| self.count(c, item)).sum();
                    DVal::Num(i64::try_from(n).unwrap_or(i64::MAX))
                } else {
                    DVal::Null
                }
            })
            .collect()
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
