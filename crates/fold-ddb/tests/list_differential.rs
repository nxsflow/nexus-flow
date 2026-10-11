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
        match r.next(18) {
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
    ]
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
            }
            seen.add(
                "boards with next's board",
                usize::from(server.active.is_some()),
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
        Err(records::RecordsError::NotCurrent { found: Some(2) }) => {}
        other => panic!("a revision-2 stream must be refused, got {other:?}"),
    }
}

#[test]
fn reading_a_typed_list_costs_one_index_query_and_four_requests_per_item() {
    // §9 "bounded cost": the persons of a board are one Query on the `typed` index, and each costs
    // its adjacency, labels, custom values and timestamps — whatever else the board holds, which
    // this list reads nothing of. A type filter reads only its own type.
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
    assert_eq!(after.queries - before.queries, 3 * n);
    // The watermark, and each person's timestamps; no parents.
    assert_eq!(after.gets - before.gets, 1 + n);
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
