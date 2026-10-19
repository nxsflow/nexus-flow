//! A server answers a ranked `list` as a replica does (6j6v.xkr7, docs/specs/derived-ranking-keys.md
//! §6 and §9 "differential"), and ranks a `next` whose keys read a `[ranking.list]` rank.
//!
//! Generated boards of four types — goals, projects, actions, persons — written by two replicas
//! apart and then merged: every status, archived and deleted items, retypes, priorities, the goals'
//! `order` custom field, `waiting` labels, and `parent`, `contributes_to`, `mentions` and `dep`
//! edges with removes. Each board is folded into a [`MemTable`] in a shuffled order with
//! duplicates, as the relay hands it over.
//!
//! The plugin ranks with every derived key kind across both sections: persons by a `sum` of
//! mentions, the `list` rank of the goals and the `next` rank of the projects they contribute to;
//! goals by their contributors under `[ranking.list]`; projects under `[ranking.next]` by the
//! `list` rank of their parent goal; actions under `[ranking.next]` by the `list` rank of the
//! persons they mention — the cross-section reads that were refused on the server until the
//! `typed` index — and, in `list`, by the `[ranking.list]` default order with a derived key.
//!
//! Three paths answer each of a set of `list` queries (types, none, container, label, status,
//! sort, limit):
//!
//! 1. the SQL path: `read::list_page` over the store;
//! 2. the Board path: `read::list_board_page` over the store's own [`read::ListBoard`]
//!    (`read::list_board` for `read::list_reads`);
//! 3. the fold-ddb path: `read::list_board_page` over the folded tables' (`records::list_board`).
//!
//! All three must give the same rows, order and total, the two boards must be equal item for item
//! and edge for edge, and the JSON must be `Engine::list_query_value`'s. `next` is held to the
//! same three paths.
//!
//! The boards also carry ref fields (6j6v.9tx3) — a single `owner` and a list `members`, written
//! through the plain `set` / `add` / `remove` ops on the fields themselves: retargets, clears, a
//! non-id text on a ref field, a person's id in the text field `order`, refs to persons that end
//! up deleted or archived, and a 129-byte project holding one. Their `list --ref` queries are held
//! to the SQL and Board paths; the fold-ddb path answers them from its hints in cut 3 (6j6v.1w3m).

use nexus_flow_core::model::{EdgeKind, ItemRow, MergeStrategy};
use nexus_flow_core::store::Store;
use nexus_flow_facade::plugin::{self, PluginConfig, PluginRegistration};
use nexus_flow_facade::read::{self, ListQuery, NextFilter, SortKey};
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::{board, records};
use std::collections::BTreeMap;

mod common;
use common::{delivered, Lcg, NOW};

const ORDERS: &str = r#"
name = "list-differential-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]
[types]
list = ["goal", "project", "action", "person"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
types = ["action", "project"]
order = [ { field = "priority", dir = "asc" } ]
[ranking.next.project]
order = [
  { related = "parent", type = "goal", of = "rank", pick = "max", nulls = "first" },
  { related = "contributes_to", type = "goal", of = { field = "order" }, pick = "min", nulls = "last" },
  { count = "children", type = "action", dir = "desc" },
  { field = "priority", dir = "asc" },
]
[ranking.next.action]
order = [
  { related = "mentions", type = "person", of = "rank", pick = "min", nulls = "last" },
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { related = "dep", of = { field = "status", precedence = ["closed", "in_progress", "open"] }, pick = "min" },
  { field = "priority", dir = "asc" },
]
[ranking.list]
types = ["person", "goal", "project"]
order = [
  { count = "mentioned_by", dir = "desc" },
  { field = "priority", dir = "asc" },
]
[ranking.list.goal]
order = [
  { count = "contributors", type = "project", dir = "desc" },
  { field = "priority", dir = "asc" },
]
[ranking.list.person]
order = [
  { sum = [ { count = "mentioned_by", type = "project" },
            { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"] } ], dir = "desc" },
  { related = "contributes_to", type = "goal", of = "rank", pick = "min", nulls = "last" },
  { related = "contributes_to", type = "project", of = "rank", pick = "min", nulls = "last" },
  { count = "dependents", status = ["closed"], dir = "desc" },
  { field = "priority", dir = "asc" },
]
[presentation.list]
columns = ["id"]
[fields.order]
type = "text"
on = ["goal"]
[fields.owner]
type = "ref"
targets = ["person"]
on = ["project"]
[fields.members]
type = "ref"
targets = ["person"]
list = true
on = ["project", "goal"]
"#;

inventory::submit!(PluginRegistration {
    name: "list-differential-fixture",
    toml: ORDERS,
    order: 141,
});

fn cfg() -> PluginConfig {
    plugin::load("list-differential-fixture").expect("the fixture is a valid plugin")
}

const TYPES: [&str; 4] = ["goal", "project", "action", "person"];

/// The items of a generated board: three goals, four projects, six actions, five persons.
fn items() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (ty, letter, n) in [
        ("goal", 'g', 3),
        ("project", 'p', 4),
        ("action", 'a', 6),
        ("person", 'u', 5),
    ] {
        for i in 0..n {
            out.push((format!("ab12.{letter}{i:03}"), ty));
        }
    }
    out
}

/// A person's id, for a ref value: one of the board's five, or one nobody holds.
fn person(r: &mut Lcg) -> String {
    format!("ab12.u{:03}", r.next(6))
}

/// The one project whose own id is 129 bytes: it holds refs (a single `owner` through a plain
/// `set`) that `ref_place_ok` keeps out of every reverse lookup (6j6v.9tx3).
fn long_project() -> String {
    format!("ab12.p{}", "x".repeat(123))
}

/// One replica's share of the writing.
fn write(s: &mut Store, r: &mut Lcg, added: &mut Vec<(String, String, EdgeKind)>, steps: usize) {
    let all = items();
    let pick = |r: &mut Lcg| all[r.next(all.len() as u64) as usize].0.clone();
    for step in 0..steps {
        s.set_wall_clock(&format!(
            "2026-10-0{}T{:02}:00:00Z",
            1 + r.next(7),
            step % 24
        ));
        let (x, y) = (pick(r), pick(r));
        match r.next(24) {
            0 => s.set_field(&x, "status", Some("in_progress".into()), "u"),
            1 | 2 => s.set_field(&x, "status", Some("closed".into()), "u"),
            3 => s.set_field(&x, "status", Some("open".into()), "u"),
            4 => s.set_field(
                &x,
                "archived",
                Some(format!("2026-10-0{}", 1 + r.next(7))),
                "u",
            ),
            5 => s.set_field(&x, "archived", None, "u"),
            6 if r.next(3) == 0 => s.delete_item(&x, "u"),
            7 => s.set_field(&x, "priority", Some(r.next(5).to_string()), "u"),
            8..=12 => {
                let kind = match r.next(10) {
                    0..=2 => EdgeKind::Parent,
                    3..=5 => EdgeKind::ContributesTo,
                    6..=8 => EdgeKind::Mentions,
                    _ => EdgeKind::Dep,
                };
                s.add_edge(&x, &y, kind, "u");
                added.push((x, y, kind));
            }
            13 if !added.is_empty() => {
                let (from, to, kind) = added.swap_remove(r.next(added.len() as u64) as usize);
                s.remove_edge(&from, &to, kind, "u");
            }
            14 => {
                if r.next(3) == 0 {
                    s.remove_label(&x, "waiting", "u");
                } else {
                    s.add_label(&x, "waiting", "u");
                }
            }
            15 if r.next(3) == 0 => {
                s.set_field(&x, "type", Some(TYPES[r.next(4) as usize].into()), "u")
            }
            // Refs through the plain ops on the ref fields themselves (6j6v.9tx3): a single
            // `owner` set, retargeted, cleared, or overwritten by a non-id text; `members` entries
            // added and removed; a person's id in the text field `order`.
            16 | 17 => {
                let value = match r.next(6) {
                    0 => None,
                    1 => Some("not an id".to_string()),
                    _ => Some(person(r)),
                };
                s.set_custom_field_merge(&x, "owner", value, "u", MergeStrategy::Lww);
            }
            18 | 19 => {
                let p = person(r);
                if r.next(3) == 0 {
                    s.remove_field_entry(&x, "members", &p, "u");
                } else {
                    s.add_field_entry(&x, "members", &p, None, "u");
                }
            }
            20 => s.set_custom_field_merge(&x, "order", Some(person(r)), "u", MergeStrategy::Lww),
            _ => s.set_custom_field_merge(
                &x,
                "order",
                Some(r.next(6).to_string()),
                "u",
                MergeStrategy::Lww,
            ),
        }
    }
}

/// A merged board: two replicas that wrote apart, then exchanged everything.
fn gen_board(seed: u64) -> Store {
    let mut r = Lcg(seed ^ 0x115d);
    let mut a = Store::open_in_memory(1);
    for (id, ty) in items() {
        a.create_item(&id, ty, "T", "u");
        a.set_field(&id, "priority", Some(r.next(5).to_string()), "u");
        if ty == "goal" && r.next(4) != 0 {
            a.set_custom_field_merge(
                &id,
                "order",
                Some(r.next(6).to_string()),
                "u",
                MergeStrategy::Lww,
            );
        }
    }
    let long = long_project();
    a.create_item(&long, "project", "Long", "u");
    a.set_custom_field_merge(
        &long,
        "owner",
        Some(person(&mut r)),
        "u",
        MergeStrategy::Lww,
    );
    let mut b = Store::open_in_memory(2);
    b.apply(&a.export());
    let steps = 30 + r.next(40) as usize;
    write(&mut a, &mut r, &mut Vec::new(), steps);
    write(&mut b, &mut r, &mut Vec::new(), steps);
    let from_b = b.export();
    a.apply(&from_b);
    a
}

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

/// The `list` queries each board is asked: a type, several, none; a container, a label, a status;
/// the default order; a cut.
fn queries(container: &str) -> Vec<ListQuery> {
    let ranked = || ListQuery::new().sort(SortKey::Rank).now(NOW);
    let types = |tys: &[&str]| tys.iter().fold(NextFilter::new(), |f, t| f.with_type(*t));
    vec![
        ranked().filter(types(&["person"])),
        ranked().filter(types(&["goal"])),
        ranked().filter(types(&["project", "person"])),
        ranked(),
        ranked().filter(types(&["action"]).with_container(container)),
        ranked().filter(NextFilter::new().with_label("waiting")),
        ranked().status("closed").filter(types(&["person"])),
        ListQuery::new().filter(types(&["person"])),
        ranked().filter(types(&["person"])).limit(3),
        // Not ranked: kept and cut before the joins (review of PR #55, Integrity #1).
        ListQuery::new().status("open").limit(4),
        ListQuery::new()
            .sort(SortKey::Closed)
            .filter(NextFilter::new().with_label("waiting"))
            .limit(2),
        ListQuery::new()
            .filter(types(&["action", "project"]).with_container(container))
            .limit(1),
    ]
}

/// The `list --ref` queries (6j6v.9tx3) for the person `p`: one ref, and two as alternatives,
/// each alone and with a type, a status, a cut and the rank.
fn ref_queries(p: &str) -> Vec<ListQuery> {
    let one = || ListQuery::new().referencing("owner", p);
    let two = || {
        ListQuery::new()
            .referencing("members", p)
            .referencing("owner", p)
    };
    let mut out = Vec::new();
    for base in [one(), two()] {
        out.push(base.clone());
        out.push(base.clone().filter(NextFilter::new().with_type("project")));
        out.push(base.clone().status("open"));
        out.push(base.clone().limit(2));
        out.push(base.clone().sort(SortKey::Rank).now(NOW));
        out.push(
            base.filter(NextFilter::new().with_type("project"))
                .sort(SortKey::Rank)
                .now(NOW),
        );
    }
    out
}

fn ref_queries_of(people: &[String]) -> Vec<ListQuery> {
    people
        .iter()
        .map(String::as_str)
        .flat_map(ref_queries)
        .collect()
}

/// What the generated boards held, so the comparison is known to have compared something worth
/// comparing.
#[derive(Default)]
struct Seen(BTreeMap<&'static str, usize>);

impl Seen {
    fn add(&mut self, what: &'static str, n: usize) {
        *self.0.entry(what).or_default() += n;
    }
}

#[test]
fn the_server_answers_a_ranked_list_as_the_replica_does() {
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let mut seen = Seen::default();
    for seed in 0..40 {
        let s = gen_board(seed);
        let ctx = format!("seed {seed}");
        let t = MemTable::new("stream-1");
        ready(folder.fold_batch(&t, &delivered(&s, seed))).unwrap();

        // Three persons per board: two the generator draws from, and one nobody references.
        let people = [
            format!("ab12.u{:03}", seed % 5),
            format!("ab12.u{:03}", (seed + 2) % 5),
            "ab12.unone".to_string(),
        ];
        for (n, q) in ref_queries_of(&people).iter().enumerate() {
            let ctx = format!("{ctx}, ref query {n}");
            let reads = read::list_reads(&cfg, q);
            assert_eq!(reads.refs, q.refs, "the reads carry the refs, {ctx}");
            let local = read::list_board(&s, &reads).unwrap();
            let want = read::list_page(&cfg, &s, q).unwrap();
            let board_path = read::list_board_page(&cfg, &local, q).unwrap();
            assert_eq!(
                ids(&board_path.items),
                ids(&want.items),
                "SQL vs Board, {ctx}"
            );
            assert_eq!(board_path.total, want.total, "total, {ctx}");
            let records = read::list_to_value_with_custom(&cfg, &s, &want.items).unwrap();
            assert_eq!(
                read::list_board_value(&cfg, &local, q).unwrap(),
                read::next_page_envelope(records, &want, false),
                "list JSON, {ctx}"
            );
            assert!(
                !ids(&want.items).contains(&long_project()),
                "the 129-byte source is in no answer, {ctx}"
            );
            // The store board: only the candidates for an unranked query, the whole scope of the
            // listed types for a ranked one (its rank is computed over all of it).
            let scope: Vec<String> = s
                .list_items()
                .unwrap()
                .into_iter()
                .filter(|i| i.deleted.as_deref() != Some("1") && i.archived.is_none())
                .filter(|i| {
                    reads
                        .types
                        .as_ref()
                        .is_none_or(|t| i.item_type.as_deref().is_some_and(|ty| t.contains(ty)))
                })
                .map(|i| i.id)
                .collect();
            if reads.select.is_some() {
                let mut candidates = std::collections::BTreeSet::new();
                for r in &q.refs {
                    for rows in read::referrers_of(&s, &r.field, &[r.id.as_str()])
                        .unwrap()
                        .into_values()
                    {
                        candidates.extend(rows.into_iter().map(|i| i.id));
                    }
                }
                assert!(
                    local.items.keys().all(|id| candidates.contains(id)),
                    "an unranked ref board holds candidates only, {ctx}"
                );
                seen.add("unranked ref boards", 1);
            } else {
                let mut held: Vec<String> = local.items.keys().cloned().collect();
                let mut scope = scope.clone();
                held.sort();
                scope.sort();
                assert_eq!(held, scope, "a ranked ref board is the whole scope, {ctx}");
                seen.add(
                    "ranked ref boards holding non-referrers",
                    usize::from(local.items.len() > want.total),
                );
            }
            // cut 3 (6j6v.1w3m): the fold-ddb path answers ref queries from its hints; until
            // then it is not compared here.
            seen.add("ref rows", want.items.len());
            let target = s.get_item(&q.refs[0].id).unwrap();
            seen.add(
                "ref rows of a deleted or archived target",
                usize::from(
                    !want.items.is_empty()
                        && target.is_some_and(|t| {
                            t.deleted.as_deref() == Some("1") || t.archived.is_some()
                        }),
                ),
            );
            seen.add("empty ref answers", usize::from(want.items.is_empty()));
            seen.add(
                "ref rows held as a list entry",
                want.items
                    .iter()
                    .filter(|i| {
                        s.field_entries_of_bulk(&[i.id.as_str()])
                            .unwrap()
                            .get(&i.id)
                            .and_then(|f| f.get("members"))
                            .is_some_and(|es| {
                                es.iter().any(|e| q.refs.iter().any(|r| r.id == e.value))
                            })
                    })
                    .count(),
            );
            seen.add(
                "closed ref rows",
                want.items
                    .iter()
                    .filter(|i| i.status.as_deref() == Some("closed"))
                    .count(),
            );
        }
        seen.add(
            "persons' ids in a text field",
            s.list_items()
                .unwrap()
                .iter()
                .filter(|i| {
                    s.custom_fields_of(&i.id)
                        .unwrap()
                        .get("order")
                        .is_some_and(|v| v.starts_with("ab12.u"))
                })
                .count(),
        );
        seen.add(
            "non-id values on a ref field",
            s.list_items()
                .unwrap()
                .iter()
                .filter(|i| {
                    s.custom_fields_of(&i.id)
                        .unwrap()
                        .get("owner")
                        .is_some_and(|v| v == "not an id")
                })
                .count(),
        );

        for (n, q) in queries(&format!("ab12.p{:03}", seed % 4))
            .iter()
            .enumerate()
        {
            let ctx = format!("{ctx}, query {n}");
            let reads = read::list_reads(&cfg, q);
            let local = read::list_board(&s, &reads).unwrap();
            let server = ready(records::list_board(&t, &reads)).unwrap();
            assert_eq!(server.items, local.items, "listed items, {ctx}");
            assert_eq!(server.ranking, local.ranking, "the ranking graph, {ctx}");
            assert_eq!(
                server.active.as_ref().map(|a| &a.tickets),
                local.active.as_ref().map(|a| &a.tickets),
                "next's board, {ctx}"
            );

            // 1 = 2 = 3: the SQL path, the Board path, the fold-ddb path.
            let want = read::list_page(&cfg, &s, q).unwrap();
            let board_path = read::list_board_page(&cfg, &local, q).unwrap();
            let served = read::list_board_page(&cfg, &server, q).unwrap();
            assert_eq!(
                ids(&board_path.items),
                ids(&want.items),
                "SQL vs Board, {ctx}"
            );
            assert_eq!(
                ids(&served.items),
                ids(&want.items),
                "SQL vs fold-ddb, {ctx}"
            );
            assert_eq!(served.total, want.total, "total, {ctx}");
            let records = read::list_to_value_with_custom(&cfg, &s, &want.items).unwrap();
            assert_eq!(
                read::list_board_value(&cfg, &server, q).unwrap(),
                read::next_page_envelope(records, &want, false),
                "list JSON, {ctx}"
            );

            seen.add("listed rows", want.items.len());
            if let Some(graph) = &server.ranking {
                seen.add("graph listed items", graph.listed.len());
                seen.add("graph edges", graph.edges.len());
                // What a list graph must hold and the lanes would not (review of PR #55, Test #5):
                // closed and archived neighbours, inbound `contributes_to` into a listed item, and
                // archived items of a graph list type, which its scope leaves out.
                for n in graph.neighbours.values() {
                    let archived = n.item.archived.is_some();
                    seen.add(
                        "closed neighbours in a list graph",
                        usize::from(!archived && n.item.status.as_deref() == Some("closed")),
                    );
                    seen.add("archived neighbours in a list graph", usize::from(archived));
                }
                seen.add(
                    "inbound contributes_to into a listed item",
                    graph
                        .edges
                        .iter()
                        .filter(|e| {
                            e.kind == EdgeKind::ContributesTo && server.items.contains_key(&e.to)
                        })
                        .count(),
                );
                let archived_in_scope: Vec<String> = s
                    .list_items()
                    .unwrap()
                    .into_iter()
                    .filter(|i| i.archived.is_some() && i.deleted.as_deref() != Some("1"))
                    .filter(|i| {
                        i.item_type
                            .as_deref()
                            .is_some_and(|t| graph.reads.list_types.contains(t))
                    })
                    .map(|i| i.id)
                    .collect();
                assert!(
                    archived_in_scope
                        .iter()
                        .all(|id| !graph.listed.contains(id)),
                    "an archived item is out of the list scope, {ctx}"
                );
                seen.add(
                    "archived items of a graph list type",
                    archived_in_scope.len(),
                );
            }
            seen.add(
                "boards with next's board",
                usize::from(server.active.is_some()),
            );
            seen.add(
                "boards kept and cut before their joins",
                usize::from(server.cut.is_some()),
            );
            seen.add(
                "listed items with a closed-parent reason",
                server
                    .items
                    .values()
                    .filter(|i| !i.parent_closed.is_empty())
                    .count(),
            );
            seen.add(
                "closed persons listed",
                want.items
                    .iter()
                    .filter(|i| {
                        i.item_type.as_deref() == Some("person")
                            && i.status.as_deref() == Some("closed")
                    })
                    .count(),
            );
        }

        // `next` whose keys read the `list` rank of persons and goals (spec §12 point 1).
        let reads = read::next_ranking_reads(&cfg)
            .unwrap()
            .expect("derived next keys");
        let selection = ready(board::select(&t)).unwrap();
        let plain = ready(records::active_board(&t, selection)).unwrap();
        let plain = ready(records::with_contributes_to(&t, plain)).unwrap();
        let server = ready(records::with_ranking(&t, plain, &reads)).unwrap();
        let local = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
        assert_eq!(server.ranking, local.ranking, "next's graph, {ctx}");
        let want = read::next(&cfg, &s, NOW, None).unwrap();
        assert_eq!(
            ids(&read::next_active(&cfg, &local, NOW).unwrap()),
            ids(&want),
            "next: SQL vs Board, {ctx}"
        );
        assert_eq!(
            ids(&read::next_active(&cfg, &server, NOW).unwrap()),
            ids(&want),
            "next: SQL vs fold-ddb, {ctx}"
        );
        seen.add("next rows", want.len());
        seen.add(
            "archived items",
            s.list_items()
                .unwrap()
                .iter()
                .filter(|i| i.archived.is_some() && i.deleted.as_deref() != Some("1"))
                .count(),
        );
    }
    for what in [
        "listed rows",
        "graph listed items",
        "graph edges",
        "boards with next's board",
        "listed items with a closed-parent reason",
        "closed persons listed",
        "next rows",
        "archived items",
        "closed neighbours in a list graph",
        "archived neighbours in a list graph",
        "inbound contributes_to into a listed item",
        "archived items of a graph list type",
        "boards kept and cut before their joins",
        "unranked ref boards",
        "ranked ref boards holding non-referrers",
        "ref rows",
        "ref rows of a deleted or archived target",
        "empty ref answers",
        "ref rows held as a list entry",
        "closed ref rows",
        "persons' ids in a text field",
        "non-id values on a ref field",
    ] {
        assert!(
            seen.0.get(what).copied().unwrap_or(0) > 0,
            "the generated boards have {what}: {:?}",
            seen.0
        );
    }
}

#[test]
fn a_stream_not_folded_by_this_index_revision_is_refused_for_a_list() {
    // The `typed` index and the self-describing adjacency entries come with index revision 3: a
    // stream folded before has neither, and its list would come back empty — refused instead.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let s = gen_board(5);
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &delivered(&s, 5))).unwrap();
    use nxs_fold_ddb::fold::INDEX_DOMAIN;
    use nxs_fold_ddb::layout::WATERMARK_KEY;
    use nxs_fold_ddb::table::Table;
    use nxs_fold_ddb::write::{Cond, Write};
    use nxs_foundation::change::Cell;
    ready(t.write(&Write {
        sk: WATERMARK_KEY.into(),
        set: vec![(format!("revision_{INDEX_DOMAIN}"), Cell::Int(2))],
        remove: Vec::new(),
        condition: Some(Cond::RowPresent),
    }))
    .unwrap();
    let q = &queries("ab12.p000")[0];
    match ready(records::list_board(&t, &read::list_reads(&cfg, q))) {
        Err(records::RecordsError::NotCurrent {
            domain: "index",
            found: Some(2),
        }) => {}
        other => panic!("a revision-2 stream must be refused, got {other:?}"),
    }
}

#[test]
fn reading_a_typed_list_costs_one_index_query_four_per_item_and_one_batch() {
    // §9 "bounded cost": the persons of a board are one Query on the `typed` index, and each costs
    // its adjacency, labels, custom values and list entries (6j6v.4g6r), the timestamps of all of them one `BatchGetItem` —
    // whatever else the board holds, which this list reads nothing of. A type filter reads only its
    // own type.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let n = 8u64;
    for i in 0..n {
        s.create_item(&format!("ab12.u{i:03}"), "person", "P", "u");
    }
    for i in 0..30 {
        s.create_item(&format!("ab12.a{i:03}"), "action", "A", "u");
    }
    s.create_item("ab12.uzzz", "person", "Archived", "u");
    s.set_field("ab12.uzzz", "archived", Some("2026-10-01".into()), "u");
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &delivered(&s, 1))).unwrap();
    let q = ListQuery::new().filter(NextFilter::new().with_type("person"));
    let reads = read::list_reads(&cfg, &q);
    assert_eq!(reads.ranking, None);
    assert!(!reads.next);
    let before = t.requests();
    let server = ready(records::list_board(&t, &reads)).unwrap();
    let after = t.requests();
    assert_eq!(after.index_queries - before.index_queries, 1);
    assert_eq!(after.queries - before.queries, 4 * n);
    // The watermark; the persons' timestamps in one batch; no parents.
    assert_eq!(after.gets - before.gets, 1);
    assert_eq!(after.batch_gets - before.batch_gets, 1);
    assert_eq!(after.writes, before.writes);
    assert_eq!(
        server.items.len(),
        n as usize,
        "the archived person is not listed"
    );
    let page = read::list_board_page(&cfg, &server, &q).unwrap();
    assert_eq!(
        ids(&page.items),
        ids(&read::list_page(&cfg, &s, &q).unwrap().items)
    );
}

#[test]
fn an_unranked_list_costs_its_page_not_the_stream() {
    // Review of PR #55, Integrity #1: `list --status open --limit 5` over a stream of many closed
    // items reads the joins of its five rows, not of every item. The rows' own Query is the one
    // read that grows with the stream; every other request is the same with 0 closed items or 300.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let cost = |closed: usize| {
        let mut s = Store::open_in_memory(1);
        for i in 0..12 {
            let id = format!("ab12.a{i:03}");
            s.create_item(&id, "action", "Open", "u");
            if i % 3 == 0 {
                s.add_label(&id, "waiting", "u");
            }
        }
        for i in 0..closed {
            let id = format!("ab12.c{i:03}");
            s.create_item(&id, "action", "Closed", "u");
            s.set_field(&id, "status", Some("closed".into()), "u");
        }
        let t = MemTable::new("stream-1");
        ready(folder.fold_batch(&t, &delivered(&s, 7))).unwrap();
        let mut costs = Vec::new();
        for q in [
            ListQuery::new().status("open").limit(5),
            ListQuery::new()
                .status("open")
                .filter(NextFilter::new().with_label("waiting"))
                .limit(2),
        ] {
            let reads = read::list_reads(&cfg, &q);
            assert!(reads.select.is_some() && reads.ranking.is_none() && !reads.next);
            let before = t.requests();
            let server = ready(records::list_board(&t, &reads)).unwrap();
            let after = t.requests();
            let page = read::list_board_page(&cfg, &server, &q).unwrap();
            let want = read::list_page(&cfg, &s, &q).unwrap();
            assert_eq!(ids(&page.items), ids(&want.items));
            assert_eq!(page.total, want.total, "the total counts every kept item");
            assert_eq!(
                server.items.len(),
                page.items.len(),
                "only the page is held"
            );
            costs.push((
                after.gets - before.gets,
                after.batch_gets - before.batch_gets,
                after.queries - before.queries,
                after.index_queries - before.index_queries,
            ));
        }
        costs
    };
    let small = cost(0);
    // The watermark; one `items#` Query; per page row `.adj#`, `.ladj#`, `custom_fields#`,
    // `field_entries#` (6j6v.4g6r); its timestamps in one batch. With a label filter, each open
    // row's labels first (12), then the page's adjacency, custom values and list entries.
    assert_eq!(small, [(1, 1, 1 + 4 * 5, 0), (1, 1, 1 + 12 + 3 * 2, 0)]);
    assert_eq!(cost(300), small, "300 closed items cost nothing more");
}

#[test]
fn a_ranked_list_with_next_and_a_graph_costs_what_its_scope_holds() {
    // Review of PR #55, Test #7: a ranked list of persons whose keys read the `next` rank of
    // projects and the `list` rank of goals reads next's board, the goals' scope and the graph.
    // Its cost follows what those hold — items of no listed type, not active, on no edge, cost
    // nothing.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let q = ListQuery::new()
        .sort(SortKey::Rank)
        .now(NOW)
        .filter(NextFilter::new().with_type("person"));
    let reads = read::list_reads(&cfg, &q);
    assert!(reads.next, "{reads:?}");
    let graph_reads = reads.ranking.clone().expect("a graph");
    assert!(!graph_reads.list_types.is_empty(), "{reads:?}");
    let cost = |unrelated: usize| {
        let mut s = Store::open_in_memory(1);
        for i in 0..4 {
            s.create_item(&format!("ab12.u{i:03}"), "person", "P", "u");
            s.create_item(&format!("ab12.a{i:03}"), "action", "A", "u");
            s.add_edge(
                &format!("ab12.a{i:03}"),
                &format!("ab12.u{i:03}"),
                EdgeKind::Mentions,
                "u",
            );
        }
        s.create_item("ab12.g000", "goal", "G", "u");
        s.create_item("ab12.p000", "project", "P", "u");
        s.add_edge("ab12.p000", "ab12.g000", EdgeKind::ContributesTo, "u");
        s.add_edge("ab12.u000", "ab12.g000", EdgeKind::ContributesTo, "u");
        s.add_edge("ab12.u001", "ab12.p000", EdgeKind::ContributesTo, "u");
        s.add_label("ab12.a000", "waiting", "u");
        for i in 0..unrelated {
            let id = format!("ab12.x{i:03}");
            s.create_item(&id, "action", "Done", "u");
            s.set_field(&id, "status", Some("closed".into()), "u");
        }
        let t = MemTable::new("stream-1");
        ready(folder.fold_batch(&t, &delivered(&s, 3))).unwrap();
        let before = t.requests();
        let server = ready(records::list_board(&t, &reads)).unwrap();
        let after = t.requests();
        let page = read::list_board_page(&cfg, &server, &q).unwrap();
        assert_eq!(
            ids(&page.items),
            ids(&read::list_page(&cfg, &s, &q).unwrap().items)
        );
        let graph = server.ranking.as_ref().unwrap();
        assert!(!graph.edges.is_empty() && !graph.listed.is_empty());
        let spent = (
            after.gets - before.gets,
            after.batch_gets - before.batch_gets,
            after.queries - before.queries,
            after.index_queries - before.index_queries,
        );
        // Every point read is the watermark or a batch; one index Query per listed type, the
        // `active` index and one per graph list type.
        assert_eq!(spent.0, 1);
        assert!(spent.1 <= 3, "{spent:?}");
        assert_eq!(spent.3, 1 + 1 + graph_reads.list_types.len() as u64);
        spent
    };
    let small = cost(0);
    assert_eq!(
        cost(200),
        small,
        "200 unrelated closed actions cost nothing more"
    );
}

#[test]
fn an_item_whose_typed_attribute_lags_its_row_is_not_listed_under_its_old_type() {
    // Review of PR #55, Code #2 / Test #3: the fold writes `nxf_typed` after the row, so a run that
    // died between the two leaves a row retyped or deleted whose attribute still names its old
    // type. The list asks the index's copy of the row, not the attribute alone.
    use nxs_fold_ddb::table::{text, Table};
    use nxs_fold_ddb::write::{Cond, Write};
    use nxs_foundation::change::Cell;
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    for i in 0..3 {
        s.create_item(&format!("ab12.u{i:03}"), "person", "P", "u");
    }
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &delivered(&s, 1))).unwrap();
    for (id, column, value) in [("ab12.u001", "type", "goal"), ("ab12.u002", "deleted", "1")] {
        ready(t.write(&Write {
            sk: format!("items#{id}"),
            set: vec![(column.into(), Cell::Text(value.into()))],
            remove: Vec::new(),
            condition: Some(Cond::RowPresent),
        }))
        .unwrap();
    }
    let on_index = |ty: &str| -> Vec<String> {
        ready(t.query_typed(ty))
            .unwrap()
            .iter()
            .map(|r| text(r, "id").unwrap().to_string())
            .collect()
    };
    assert_eq!(
        on_index("person").len(),
        3,
        "the attributes still name `person`"
    );
    let listed = |ty: &str| -> Vec<String> {
        let q = ListQuery::new()
            .sort(SortKey::Rank)
            .filter(NextFilter::new().with_type(ty));
        let board = ready(records::list_board(&t, &read::list_reads(&cfg, &q))).unwrap();
        board.items.into_keys().collect()
    };
    assert_eq!(listed("person"), ["ab12.u000"]);
    assert!(
        listed("goal").is_empty(),
        "nor under its new type, until the attribute moves"
    );
}
