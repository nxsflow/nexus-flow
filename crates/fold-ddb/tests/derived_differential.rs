//! A server ranks `next` by derived ranking keys as a replica does (6j6v.1c5s,
//! docs/specs/derived-ranking-keys.md §6 and §9 "differential").
//!
//! Generated boards of three types — goals, projects, actions — written by two replicas apart and
//! then merged: every status, archived and deleted items, priorities, the goals' `order` custom
//! field, `waiting` labels, and `parent`, `contributes_to`, `mentions` and `dep` edges with removes.
//! Two replicas parent the same child apart, so a child can have two present parent edges and only
//! `present_parent`'s pick may count. Each board is folded into a [`MemTable`] in a shuffled order
//! with duplicates, as the relay hands it over.
//!
//! Then three paths rank `next` under a plugin whose orders hold every derived key kind, each
//! reading closed, archived and deleted neighbours, down a three-type chain (action → project →
//! goal):
//!
//! 1. the SQL path: `read::next` over the store;
//! 2. the Board path: `read::next_active` over the store's own [`read::ActiveBoard`] and
//!    [`read::RankingGraph`] (`read::active_board` + `read::with_ranking`);
//! 3. the fold-ddb path: `read::next_active` over the folded tables' (`board::select` +
//!    `records::active_board` + `records::with_ranking`).
//!
//! All three must give the same rows in the same order and the same JSON, and the two inputs must
//! be equal, edge for edge and neighbour for neighbour. Folding the same log in another order must
//! read the same graph (determinism).

use nexus_flow_core::model::{EdgeKind, ItemRow, MergeStrategy};
use nexus_flow_core::store::Store;
use nexus_flow_facade::plugin::{self, PluginConfig, PluginRegistration};
use nexus_flow_facade::read;
use nxs_fold_ddb::fold::{Folder, INDEX_DOMAIN, INDEX_REVISION};
use nxs_fold_ddb::layout::WATERMARK_KEY;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::table::Table;
use nxs_fold_ddb::write::{Cond, Write};
use nxs_fold_ddb::{board, records};
use nxs_foundation::change::Cell;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{delivered, head, Lcg, NOW};

/// Every derived key kind, each able to read a neighbour that is not active: a goal's order and
/// rank from a closed or archived goal, counts without a status filter, a `dep` blocker's status
/// (only a closed blocker leaves its dependent in `next`), the `belongs_to` of a mentioned item.
const ORDERS: &str = r#"
name = "derived-differential-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]
[types]
list = ["goal", "project", "action"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
types = ["action", "project", "goal"]
order = [ { field = "priority", dir = "asc" } ]
[ranking.next.goal]
order = [
  { count = "contributors", type = "project", dir = "desc" },
  { field = "priority", dir = "asc" },
]
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order" }, pick = "min", nulls = "last" },
  { related = "parent", type = "goal", of = "rank", pick = "max", nulls = "first" },
  { sum = [ { count = "mentioned_by" },
            { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"] } ], dir = "desc" },
  { count = "children", type = "action", dir = "desc" },
  { field = "priority", dir = "asc" },
]
[ranking.next.action]
order = [
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { related = "dep", of = { field = "status", precedence = ["closed", "in_progress", "open"] }, pick = "min" },
  { count = "dependents", dir = "desc" },
  { related = "mentions", of = { field = "belongs_to" }, pick = "max", nulls = "last" },
  { count = "contributors", status = ["closed"], dir = "desc" },
  { field = "priority", dir = "asc" },
]
[presentation.list]
columns = ["id"]
[fields.order]
type = "text"
on = ["goal"]
"#;

inventory::submit!(PluginRegistration {
    name: "derived-differential-fixture",
    toml: ORDERS,
    order: 140,
});

fn cfg() -> PluginConfig {
    plugin::load("derived-differential-fixture").expect("the fixture is a valid plugin")
}

/// The items of a generated board: three goals, five projects, eight actions.
fn items() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (ty, letter, n) in [("goal", 'g', 3), ("project", 'p', 5), ("action", 'a', 8)] {
        for i in 0..n {
            out.push((format!("ab12.{letter}{i:03}"), ty));
        }
    }
    out
}

/// One replica's share of the writing. `added` holds the edges this replica added, so a remove
/// usually takes back a real one.
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
        match r.next(16) {
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
                // Dependencies are rare: an active blocker takes its dependent out of `next`.
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
    let mut r = Lcg(seed ^ 0xd1ff);
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

/// `ops` folded into a fresh table.
fn fold(folder: &Folder, ops: &[(i64, nxs_foundation::model::Op)]) -> MemTable {
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, ops)).unwrap();
    t
}

/// The served board with its ranking graph, as a server reads it for `reads` — and with the
/// contributes-to targets, so its tickets compare whole against `read::active_board`'s.
fn served(t: &MemTable, reads: &read::RankingReads) -> read::ActiveBoard {
    let selection = ready(board::select(t)).unwrap();
    let plain = ready(records::active_board(t, selection)).unwrap();
    let plain = ready(records::with_contributes_to(t, plain)).unwrap();
    ready(records::with_ranking(t, plain, reads)).unwrap()
}

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

/// An active ticket, as the SQL `items` row `a` says: live, unarchived, open or in progress.
const ACTIVE_SQL: &str = "COALESCE(a.deleted, '') != '1' AND a.archived IS NULL
     AND a.status IN ('open', 'in_progress')";

fn count(s: &Store, sql: &str) -> usize {
    s.connection()
        .query_row(sql, [], |r| r.get::<_, i64>(0))
        .unwrap() as usize
}

/// What the generated boards held, so the comparison is known to have compared something worth
/// comparing.
#[derive(Default)]
struct Seen(BTreeMap<&'static str, usize>);

impl Seen {
    fn add(&mut self, what: &'static str, n: usize) {
        *self.0.entry(what).or_default() += n;
    }

    fn assert_all(&self) {
        for (what, n) in &self.0 {
            assert!(*n > 0, "the generated boards have {what}: {:?}", self.0);
        }
    }
}

#[test]
fn the_server_ranks_next_by_derived_keys_as_the_replica_does() {
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let reads = read::next_ranking_reads(&cfg)
        .unwrap()
        .expect("derived next keys");
    assert_eq!(reads.kinds.len(), 4, "every edge kind: {reads:?}");
    assert!(reads.labels && reads.custom, "{reads:?}");
    let mut seen = Seen::default();
    for seed in 0..60 {
        let s = gen_board(seed);
        let ctx = format!("seed {seed}");
        let t = fold(&folder, &delivered(&s, seed));

        let local = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
        let server = served(&t, &reads);
        assert_eq!(server.tickets, local.tickets, "active tickets, {ctx}");
        assert_eq!(server.ranking, local.ranking, "the ranking graph, {ctx}");

        // 1 = 2 = 3: the SQL path, the Board path, the fold-ddb path.
        let want = read::next(&cfg, &s, NOW, None).unwrap();
        assert_eq!(
            ids(&read::next_active(&cfg, &local, NOW).unwrap()),
            ids(&want),
            "SQL path vs Board path, {ctx}"
        );
        assert_eq!(
            ids(&read::next_active(&cfg, &server, NOW).unwrap()),
            ids(&want),
            "SQL path vs fold-ddb path, {ctx}"
        );
        assert_eq!(
            read::next_active_value(&cfg, &server, NOW).unwrap(),
            read::next_to_value_with_custom(&cfg, &s, &want).unwrap(),
            "next JSON, {ctx}"
        );

        // Determinism: the same log folded in another order reads the same graph (every fourth
        // board — a fold is most of a seed's time).
        if seed % 4 == 0 {
            let again = served(&fold(&folder, &delivered(&s, seed ^ 0xffff)), &reads);
            assert_eq!(again.ranking, server.ranking, "another fold order, {ctx}");
        }

        // What the boards held.
        let graph = server.ranking.as_ref().unwrap();
        let candidates: BTreeSet<&str> = want.iter().map(|i| i.id.as_str()).collect();
        seen.add("next rows", want.len());
        for n in graph.neighbours.values() {
            let archived = n.item.archived.is_some();
            seen.add(
                "closed neighbours",
                usize::from(!archived && n.item.status.as_deref() == Some("closed")),
            );
            seen.add("archived neighbours", usize::from(archived));
            seen.add("labelled neighbours", usize::from(!n.labels.is_empty()));
            seen.add(
                "goal neighbours with an order",
                usize::from(n.custom.contains_key("order")),
            );
            seen.add(
                "neighbours with a parent",
                usize::from(n.item.belongs_to.is_some()),
            );
        }
        for e in &graph.edges {
            let into_candidate = candidates.contains(e.to.as_str());
            let from_neighbour = graph.neighbours.contains_key(&e.from);
            seen.add(
                "inbound contributes_to to a candidate",
                usize::from(into_candidate && e.kind == EdgeKind::ContributesTo),
            );
            seen.add(
                "mentions of a candidate from an inactive item",
                usize::from(into_candidate && from_neighbour && e.kind == EdgeKind::Mentions),
            );
            seen.add(
                "inactive children of a candidate",
                usize::from(into_candidate && from_neighbour && e.kind == EdgeKind::Parent),
            );
            seen.add(
                "dep edges from a candidate to an inactive blocker",
                usize::from(
                    candidates.contains(e.from.as_str())
                        && graph.neighbours.contains_key(&e.to)
                        && e.kind == EdgeKind::Dep,
                ),
            );
        }
        // Counted where they matter (review of PR #55, Test #4): at an active ticket, whose edges
        // the graph reads — not anywhere on the board.
        seen.add(
            "edges dropped for a deleted end",
            count(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM present_edges e
                     JOIN items d ON d.id IN (e.from_id, e.to_id) AND d.deleted = '1'
                     JOIN items a ON a.id IN (e.from_id, e.to_id) AND a.id != d.id
                     WHERE {ACTIVE_SQL}"
                ),
            ),
        );
        // A child with two present parent edges: only `present_parent`'s pick may count.
        seen.add(
            "children parented twice",
            count(
                &s,
                "SELECT COUNT(*) FROM (SELECT from_id FROM present_edges WHERE kind = 'parent'
                 GROUP BY from_id HAVING COUNT(*) > 1)",
            ),
        );
        seen.add(
            "removed edges at an active ticket",
            count(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM edge_removes r JOIN edge_adds e ON e.tag = r.tag
                     JOIN items a ON a.id IN (e.from_id, e.to_id) WHERE {ACTIVE_SQL}"
                ),
            ),
        );
        // A derived key decides: a project ranked ahead of one with a better priority.
        let projects: Vec<&ItemRow> = want
            .iter()
            .filter(|i| i.item_type.as_deref() == Some("project"))
            .collect();
        seen.add(
            "projects ordered against their priority",
            projects
                .windows(2)
                .filter(|w| w[0].priority > w[1].priority)
                .count(),
        );
    }
    seen.assert_all();
    for what in [
        "closed neighbours",
        "archived neighbours",
        "inbound contributes_to to a candidate",
        "mentions of a candidate from an inactive item",
        "inactive children of a candidate",
        "dep edges from a candidate to an inactive blocker",
        "edges dropped for a deleted end",
        "removed edges at an active ticket",
        "children parented twice",
        "projects ordered against their priority",
    ] {
        assert!(seen.0.contains_key(what), "{what} is counted");
    }
}

#[test]
fn reading_the_ranking_graph_costs_one_query_per_ticket_and_per_live_neighbour() {
    // §9 "bounded cost": every edge touching an active ticket is read once, from its entries;
    // each inactive neighbour costs one GetItem and, when live, one Query for its parent plus one
    // per join asked for — and nothing grows with history: an edge added and removed many times
    // is still one entry per add, read in the same Query.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let reads = read::next_ranking_reads(&cfg).unwrap().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.g000", "goal", "Closed goal", "u");
    s.set_field("ab12.g000", "status", Some("closed".into()), "u");
    s.create_item("ab12.g001", "goal", "Deleted goal", "u");
    let n = 10u64;
    for i in 0..n {
        let id = format!("ab12.p{i:03}");
        s.create_item(&id, "project", "P", "u");
        for round in 0..5 {
            s.add_edge(&id, "ab12.g000", EdgeKind::ContributesTo, "u");
            if round < 4 {
                s.remove_edge(&id, "ab12.g000", EdgeKind::ContributesTo, "u");
            }
        }
        s.add_edge(&id, "ab12.g001", EdgeKind::Mentions, "u");
    }
    s.delete_item("ab12.g001", "u");
    let t = fold(&folder, &delivered(&s, 1));
    let plain = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
    let before = t.requests();
    let server = ready(records::with_ranking(&t, plain, &reads)).unwrap();
    let after = t.requests();
    // One adjacency Query per active ticket; the live neighbour's parent, labels and custom values.
    assert_eq!(after.queries - before.queries, n + 3);
    // The watermark, asked again (review of PR #55, Integrity #3); the rows of the inactive
    // neighbours — the closed goal and the deleted one — in one `BatchGetItem`.
    assert_eq!(after.gets - before.gets, 1);
    assert_eq!(after.batch_gets - before.batch_gets, 1);
    assert_eq!(after.index_queries, before.index_queries);
    assert_eq!(after.writes, before.writes);
    let local = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
    assert_eq!(server.ranking, local.ranking);
    let graph = server.ranking.unwrap();
    assert_eq!(
        graph.edges.len(),
        n as usize,
        "one contributes_to per project"
    );
    assert_eq!(
        graph.neighbours.keys().collect::<Vec<_>>(),
        ["ab12.g000"],
        "the closed goal is a neighbour, the deleted one is not"
    );
}

#[test]
fn a_stream_folded_by_index_revision_2_is_refused_then_refolded_and_ranks() {
    // A stream folded before the adjacency entries said what their edge is (index revision 2): its
    // entries hold only the tag, which would read no edge at all — so it is refused, and the fold
    // run's rule refolds it.
    let folder = Folder::platform().unwrap();
    let cfg = cfg();
    let reads = read::next_ranking_reads(&cfg).unwrap().unwrap();
    for seed in [3, 11, 27] {
        let s = gen_board(seed);
        let ops = delivered(&s, seed);
        let t = fold(&folder, &ops);
        // A board read while the stream was current — a server that holds one across the refold.
        let plain = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
        for (sk, row) in t.rows() {
            if let Some(Cell::Text(tag)) = row.get("tag").filter(|_| sk.starts_with(".adj#")) {
                ready(
                    t.write(&Write {
                        sk: sk.clone(),
                        set: vec![("tag".into(), Cell::Text(tag.clone()))],
                        remove: ["from_id", "to_id", "kind", "lamport", "site", "removed"]
                            .map(String::from)
                            .to_vec(),
                        condition: Some(Cond::RowPresent),
                    }),
                )
                .unwrap();
            }
        }
        ready(t.write(&Write {
            sk: WATERMARK_KEY.into(),
            set: vec![(format!("revision_{INDEX_DOMAIN}"), Cell::Int(2))],
            remove: Vec::new(),
            condition: Some(Cond::RowPresent),
        }))
        .unwrap();

        let selection = ready(board::select(&t)).unwrap();
        match ready(records::active_board(&t, selection)) {
            Err(records::RecordsError::NotCurrent { found: Some(2) }) => {}
            other => panic!("seed {seed}: a revision-2 stream must be refused, got {other:?}"),
        }
        // The graph read refuses it itself (review of PR #55, Integrity #3)…
        match ready(records::with_ranking(&t, plain.clone(), &reads)) {
            Err(records::RecordsError::NotCurrent { found: Some(2) }) => {}
            other => panic!(
                "seed {seed}: the graph of a revision-2 stream must be refused, got {other:?}"
            ),
        }
        // …because the stripped entries would read as no edge at all: under a watermark that
        // claims revision 3, the same read answers an empty graph where the store has edges.
        let local = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
        let claim = |revision: i64| {
            ready(t.write(&Write {
                sk: WATERMARK_KEY.into(),
                set: vec![(format!("revision_{INDEX_DOMAIN}"), Cell::Int(revision))],
                remove: Vec::new(),
                condition: Some(Cond::RowPresent),
            }))
            .unwrap();
        };
        claim(INDEX_REVISION);
        let stripped = ready(records::with_ranking(&t, plain.clone(), &reads)).unwrap();
        assert!(stripped.ranking.unwrap().edges.is_empty(), "seed {seed}");
        assert!(!local.ranking.unwrap().edges.is_empty(), "seed {seed}");
        claim(2);
        let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
        assert!(!folder.is_current(&mark), "seed {seed}");
        ready(Folder::clear(&t, head(&ops))).unwrap();
        ready(folder.fold_batch(&t, &ops)).unwrap();

        let server = served(&t, &reads);
        assert_eq!(
            ids(&read::next_active(&cfg, &server, NOW).unwrap()),
            ids(&read::next(&cfg, &s, NOW, None).unwrap()),
            "seed {seed}"
        );
    }
}
