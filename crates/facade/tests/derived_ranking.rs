//! Derived ranking keys on the local SQL path (6j6v.x0wf, docs/specs/derived-ranking-keys.md):
//! `related`, `count` and `sum` keys in `[ranking.next.<type>]` and `[ranking.list.<type>]`
//! orders, the type chain ranked bottom-up once per query, and its cost.
//!
//! - The spec's own orders (§8) at the seam the products take — `Engine::next_query` and
//!   `Engine::list_query` — over a registered fixture plugin: actions by their project's rank,
//!   projects by their goals' declared order (a custom field, §10.2), persons by a `sum` of
//!   mentions and then by the rank of the projects they contribute to (§9 "chains").
//! - Each kind over hand-built boards through `read::list_at`, the function `list_query` answers
//!   from (§9 "per kind").
//! - Determinism (§9): another edge insertion order ranks alike, and ties fall to the id.
//! - Bounded cost (§9, a query-shape guard in the style of xn8s): the edge pass runs once per query
//!   however many candidates there are.
//!
//! Its own test binary, so the fixture plugin does not pollute the facade lib tests' "exactly two
//! OSS plugins" assertion.

use nexus_flow_core::model::{EdgeKind, ItemRow, MergeStrategy};
use nexus_flow_core::store::{EdgeReadStats, ItemReadStats, Store};
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginConfig, PluginRegistration};
use nexus_flow_facade::read::{self, ListQuery, NextFilter, NextQuery, SortKey};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";

/// The spec's §8 orders, with `types` under `[ranking.list]` and the goals' declared sequence as
/// the custom field `order`.
const SPEC_ORDERS: &str = r#"
name = "derived-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["goal", "project", "action", "person"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
types = ["project", "action"]
order = [
    { field = "status", precedence = ["in_progress", "open"] },
    { field = "priority", dir = "asc" },
]
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order", dir = "asc" }, pick = "min", nulls = "last" },
  { field = "priority", dir = "asc" },
  { count = "children", type = "action", status = ["open", "in_progress"], dir = "desc" },
]
[ranking.next.action]
order = [
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { field = "priority", dir = "asc" },
]
[ranking.list]
types = ["person"]
[ranking.list.person]
order = [
  { sum = [ { count = "mentioned_by", type = "project" },
            { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"] } ], dir = "desc" },
  { related = "contributes_to", type = "project", of = "rank", pick = "min", nulls = "last" },
]
[presentation.list]
columns = ["id"]
[fields.order]
type = "text"
on = ["goal"]
"#;

inventory::submit!(PluginRegistration {
    name: "derived-rank-fixture",
    toml: SPEC_ORDERS,
    order: 130,
});

fn workspace(plugin: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), plugin).unwrap();
    tmp
}

fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

fn engine(tmp: &TempDir) -> Engine {
    Engine::open(None, tmp.path()).unwrap()
}

fn item(s: &mut Store, id: &str, ty: &str, priority: &str) {
    s.create_item(id, ty, id, "t");
    s.set_field(id, "priority", Some(priority.into()), "t");
}

fn custom(s: &mut Store, id: &str, field: &str, value: &str) {
    s.set_custom_field_merge(id, field, Some(value.into()), "t", MergeStrategy::Lww);
}

fn edge(s: &mut Store, from: &str, to: &str, kind: EdgeKind) {
    s.add_edge(from, to, kind, "t");
}

fn close(s: &mut Store, id: &str) {
    s.set_field(id, "status", Some("closed".into()), "t");
}

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

/// The rows of `ty`, in the order the list holds them.
fn of_type(items: &[ItemRow], ty: &str) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.item_type.as_deref() == Some(ty))
        .map(|i| i.id.clone())
        .collect()
}

/// The spec's three cases on one board (every ticket open, so all of `next` is one tier):
///
/// - goals: `g1` order 2, `g2` order 1, `g3` no order;
/// - projects, by their goals' smallest order, then priority, then open child actions (desc):
///   `p3` (g1+g2 → 1, P1), `p2` (g2 → 1, P3), `p5` (g1 → 2, P0, two open children),
///   `p1` (g1 → 2, P0, one open child), `p4` (no goal → null last). Ranks 0..4 in that order;
///   `pc` is closed, so it is not a `next` candidate and has no rank;
/// - actions, by their parent project's rank, then priority: `a3` (p3, P1), `a2` (p3, P3),
///   `a5`, `a6` (p5, P2 each — the id decides), `a1` (p1), `a4` (no parent → null last); `a7` is
///   p5's CLOSED child, so it neither counts nor ranks;
/// - persons, by the mentions of projects plus those of open actions labelled `waiting`
///   (desc), then the best rank among the projects they contribute to:
///   `u2` (p3 + waiting a2 = 2, rank 0), `u1` (p1 + p2 = 2, rank 4), `u3` (0: its mentioners are
///   the closed waiting `a7` and the deleted project `px`; rank 1), `u6` (0, rank 4 — p4),
///   `u4` (0, mentioned by the unlabelled `a3`; no project → null), `u5` (0, contributes to the
///   closed `pc` → null: outside `next`'s scope). `u4` before `u5` by id.
fn spec_board(dir: &Path) {
    let mut s = writer(dir);
    for (g, order) in [
        ("zz12.g1", Some("2")),
        ("zz12.g2", Some("1")),
        ("zz12.g3", None),
    ] {
        item(&mut s, g, "goal", "1");
        if let Some(o) = order {
            custom(&mut s, g, "order", o);
        }
    }
    for (p, prio) in [
        ("zz12.p1", "0"),
        ("zz12.p2", "3"),
        ("zz12.p3", "1"),
        ("zz12.p4", "0"),
        ("zz12.p5", "0"),
        ("zz12.pc", "0"),
        ("zz12.px", "0"),
    ] {
        item(&mut s, p, "project", prio);
    }
    close(&mut s, "zz12.pc");
    edge(&mut s, "zz12.p1", "zz12.g1", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p2", "zz12.g2", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p3", "zz12.g1", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p3", "zz12.g2", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p5", "zz12.g1", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p4", "zz12.g3", EdgeKind::ContributesTo);
    for (a, prio, parent) in [
        ("zz12.a1", "0", Some("zz12.p1")),
        ("zz12.a2", "3", Some("zz12.p3")),
        ("zz12.a3", "1", Some("zz12.p3")),
        ("zz12.a4", "0", None),
        ("zz12.a5", "2", Some("zz12.p5")),
        ("zz12.a6", "2", Some("zz12.p5")),
        ("zz12.a7", "0", Some("zz12.p5")),
    ] {
        item(&mut s, a, "action", prio);
        if let Some(p) = parent {
            s.set_parent(a, p, "t").unwrap();
        }
    }
    close(&mut s, "zz12.a7");
    for u in ["u1", "u2", "u3", "u4", "u5", "u6"] {
        item(&mut s, &format!("zz12.{u}"), "person", "1");
    }
    edge(&mut s, "zz12.p1", "zz12.u1", EdgeKind::Mentions);
    edge(&mut s, "zz12.p2", "zz12.u1", EdgeKind::Mentions);
    edge(&mut s, "zz12.u1", "zz12.p4", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.p3", "zz12.u2", EdgeKind::Mentions);
    edge(&mut s, "zz12.a2", "zz12.u2", EdgeKind::Mentions);
    s.add_label("zz12.a2", "waiting", "t");
    edge(&mut s, "zz12.u2", "zz12.p3", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.a7", "zz12.u3", EdgeKind::Mentions);
    s.add_label("zz12.a7", "waiting", "t");
    edge(&mut s, "zz12.px", "zz12.u3", EdgeKind::Mentions);
    s.delete_item("zz12.px", "t");
    edge(&mut s, "zz12.u3", "zz12.p2", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.a3", "zz12.u4", EdgeKind::Mentions);
    edge(&mut s, "zz12.u5", "zz12.pc", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.u6", "zz12.p4", EdgeKind::ContributesTo);
}

const PROJECTS: [&str; 5] = ["zz12.p3", "zz12.p2", "zz12.p5", "zz12.p1", "zz12.p4"];
const ACTIONS: [&str; 6] = [
    "zz12.a3", "zz12.a2", "zz12.a5", "zz12.a6", "zz12.a1", "zz12.a4",
];
const PERSONS: [&str; 6] = [
    "zz12.u2", "zz12.u1", "zz12.u3", "zz12.u6", "zz12.u4", "zz12.u5",
];

// ---- the spec's orders at the Engine seam (§9 "chains") -----------------------------------

#[test]
fn next_query_ranks_actions_by_project_rank_and_projects_by_goal_order() {
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let page = engine(&tmp).next_query(&NextQuery::new(NOW)).unwrap();
    assert_eq!(
        of_type(&page.items, "project"),
        PROJECTS,
        "goal order, priority, children"
    );
    assert_eq!(
        of_type(&page.items, "action"),
        ACTIONS,
        "the parent's rank, across three types"
    );
}

#[test]
fn next_query_filters_after_ranking_and_pages_in_the_same_order() {
    // The rank is computed over all of `next`'s candidates: a type filter removes rows, it does not
    // change the project ranks the actions read.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let engine = engine(&tmp);
    let q = || NextQuery::new(NOW).filter(NextFilter::new().with_type("action"));
    assert_eq!(ids(&engine.next_query(&q()).unwrap().items), ACTIONS);
    let p1 = engine.next_query(&q().limit(4).paginate()).unwrap();
    let token = p1.next_token.clone().expect("rows remain");
    let p2 = engine.next_query(&q().limit(4).token(token)).unwrap();
    assert!(!p2.restarted);
    let walked: Vec<String> = ids(&p1.items).into_iter().chain(ids(&p2.items)).collect();
    assert_eq!(walked, ACTIONS);
}

#[test]
fn list_query_ranks_persons_by_a_sum_of_mentions_then_by_project_rank() {
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let q = ListQuery::new()
        .sort(SortKey::Rank)
        .filter(NextFilter::new().with_type("person"))
        .now(NOW);
    let page = engine(&tmp).list_query(&q).unwrap();
    assert_eq!(ids(&page.items), PERSONS);
}

#[test]
fn a_list_order_reading_a_next_rank_needs_now() {
    // A person reads the rank of projects, which `next` ranks over its candidates at an instant.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let err = engine(&tmp)
        .list_query(&ListQuery::new().sort(SortKey::Rank))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("pass `now`"), "{}", err.msg);
    // The id order reads no rank, and needs no `now`.
    engine(&tmp).list_query(&ListQuery::new()).unwrap();
}

#[test]
fn a_type_filter_whose_order_reads_no_next_rank_needs_no_now() {
    // Review of PR #48, Code Quality #1: the filters narrow the rows before they are ranked, so
    // the chain covers only the orders the kept rows rank by. Goals and projects rank by the
    // `[ranking.list]` default order (field keys only) and need no `now`; persons read the rank
    // of projects, which `next` ranks, and still do.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let engine = engine(&tmp);
    let ranked = |ty: &str| {
        ListQuery::new()
            .sort(SortKey::Rank)
            .filter(NextFilter::new().with_type(ty))
    };
    let goals = engine.list_query(&ranked("goal")).unwrap();
    assert_eq!(ids(&goals.items), ["zz12.g1", "zz12.g2", "zz12.g3"]);
    let projects = engine.list_query(&ranked("project")).unwrap();
    assert_eq!(
        projects.total, 6,
        "every live project: all seven but the deleted px"
    );
    let err = engine.list_query(&ranked("person")).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("pass `now`"), "{}", err.msg);
    // The rank scope stays every live item: a filter does not change the ranks persons read.
    let persons = engine.list_query(&ranked("person").now(NOW)).unwrap();
    assert_eq!(ids(&persons.items), PERSONS);
}

#[test]
fn a_malformed_now_is_refused() {
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    for q in [ListQuery::new(), ListQuery::new().sort(SortKey::Rank)] {
        let err = engine(&tmp).list_query(&q.now("yesterday")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    }
}

#[test]
fn a_page_restarts_when_an_op_outside_the_snapshot_reorders_it() {
    // Review of PR #48, Test Quality #4 — resume check 2. The snapshot holds actions only; the op
    // sets a GOAL's declared order, so it targets no item of the snapshot (check 1 passes), but it
    // reorders the projects and with them the actions. g2 → 9: p5 (g1, two open children), p1,
    // p3 (now min 2), p2, p4 — so a5, a6, a1, a3, a2, a4.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let engine = engine(&tmp);
    let q = || {
        NextQuery::new(NOW)
            .filter(NextFilter::new().with_type("action"))
            .limit(2)
    };
    let p1 = engine.next_query(&q().paginate()).unwrap();
    assert_eq!(ids(&p1.items), ACTIONS[..2]);
    let token = p1.next_token.expect("rows remain");
    {
        let mut s = writer(tmp.path());
        custom(&mut s, "zz12.g2", "order", "9");
    }
    let p2 = engine.next_query(&q().token(token)).unwrap();
    assert!(p2.restarted, "an outside op moved the snapshot's items");
    assert_eq!(ids(&p2.items), ["zz12.a5", "zz12.a6"]);
    assert_eq!(p2.start, 0);
}

#[test]
fn the_rank_follows_the_board_on_the_next_query() {
    // Moving a3 to the bottom-ranked project moves it; nothing is cached across queries.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let engine = engine(&tmp);
    {
        let mut s = writer(tmp.path());
        s.set_parent("zz12.a3", "zz12.p4", "t").unwrap();
    }
    let page = engine.next_query(&NextQuery::new(NOW)).unwrap();
    assert_eq!(
        of_type(&page.items, "action"),
        ["zz12.a2", "zz12.a5", "zz12.a6", "zz12.a1", "zz12.a3", "zz12.a4"]
    );
}

#[test]
fn the_store_free_next_ranks_from_the_ranking_graph_as_the_store_does() {
    // 6j6v.1c5s: the board a server reads carries the graph the keys read, and the store-free
    // `next` ranks from it exactly as the store path does — the spec's three-type chain.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let cfg = plugin::load("derived-rank-fixture").unwrap();
    let s = writer(tmp.path());
    let reads = read::next_ranking_reads(&cfg)
        .unwrap()
        .expect("derived next keys");
    assert_eq!(
        reads.kinds.iter().copied().collect::<Vec<_>>(),
        [EdgeKind::ContributesTo, EdgeKind::Parent],
        "the edges the next orders follow or count"
    );
    assert!(reads.custom && !reads.labels, "{reads:?}");
    let board = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
    let got = read::next_active(&cfg, &board, NOW).unwrap();
    assert_eq!(ids(&got), ids(&read::next(&cfg, &s, NOW, None).unwrap()));
    assert_eq!(of_type(&got, "project"), PROJECTS);
    assert_eq!(of_type(&got, "action"), ACTIONS);
    assert_eq!(
        read::next_active_value(&cfg, &board, NOW).unwrap(),
        read::next_to_value_with_custom(&cfg, &s, &read::next(&cfg, &s, NOW, None).unwrap())
            .unwrap()
    );
}

#[test]
fn the_store_free_next_refuses_a_board_read_without_the_graph_its_keys_read() {
    // Ranking without the edges would order apart from the store path in silence.
    let tmp = workspace("derived-rank-fixture");
    spec_board(tmp.path());
    let cfg = plugin::load("derived-rank-fixture").unwrap();
    let s = writer(tmp.path());
    let board = read::active_board(&s).unwrap();
    let err = read::next_active(&cfg, &board, NOW).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("derived ranking keys"), "{}", err.msg);
    // A graph read for less than the keys need is refused too: here without the custom values
    // the goals' `order` lives in.
    let mut reads = read::next_ranking_reads(&cfg).unwrap().unwrap();
    reads.custom = false;
    let short = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
    let err = read::next_active(&cfg, &short, NOW).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    // `blocked` and `deferred` rank by no derived key and need no graph.
    read::blocked_active(&cfg, &board);
    read::deferred_active(&cfg, &board, NOW).unwrap();
}

/// A plugin ranking `next` by its neighbours with no status filter, so closed, archived and
/// deleted neighbours each decide something (owner, 2026-10-11: a key reads every LIVE neighbour).
const LIVE_ORDERS: &str = r#"
name = "live-neighbours-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["goal", "project", "action"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
types = ["project", "action"]
order = [ { field = "priority", dir = "asc" } ]
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order" }, pick = "min", nulls = "last" },
  { count = "children", dir = "desc" },
  { count = "mentioned_by", dir = "desc" },
]
[ranking.next.action]
order = [ { related = "parent", type = "project", of = "rank", nulls = "last" } ]
[presentation.list]
columns = ["id"]
[fields.order]
type = "text"
on = ["goal"]
"#;

#[test]
fn a_closed_archived_or_deleted_neighbour_reads_alike_with_and_without_a_store() {
    // Goals: `gc` closed (order 1), `ga` archived (order 2), `go` open (order 3), `gd` deleted
    // (order 0). Projects `pa`..`pd` contribute to one each: a closed or archived goal still gives
    // its order, a deleted one gives none (null, last). Projects `q1`..`q4` have no goal: `q1` has
    // three children — two closed, one archived — `q2` one open child, `q3` is mentioned by a
    // closed project and `q4` by nothing. Every child and mentioner but `q2`'s is out of `next`.
    let cfg: plugin::PluginConfig = toml::from_str(LIVE_ORDERS).unwrap();
    let mut s = Store::open_in_memory(1);
    for (g, order) in [
        ("zz12.gc", "1"),
        ("zz12.ga", "2"),
        ("zz12.go", "3"),
        ("zz12.gd", "0"),
    ] {
        item(&mut s, g, "goal", "1");
        custom(&mut s, g, "order", order);
    }
    close(&mut s, "zz12.gc");
    s.set_field("zz12.ga", "archived", Some("2026-06-01".into()), "t");
    for (p, g) in [
        ("zz12.pa", "zz12.gc"),
        ("zz12.pb", "zz12.ga"),
        ("zz12.pc", "zz12.gd"),
        ("zz12.pd", "zz12.go"),
    ] {
        item(&mut s, p, "project", "1");
        edge(&mut s, p, g, EdgeKind::ContributesTo);
    }
    s.delete_item("zz12.gd", "t");
    for q in ["zz12.q1", "zz12.q2", "zz12.q3", "zz12.q4"] {
        item(&mut s, q, "project", "1");
    }
    for (a, parent) in [
        ("zz12.a1", "zz12.q1"),
        ("zz12.a2", "zz12.q1"),
        ("zz12.a3", "zz12.q1"),
        ("zz12.a4", "zz12.q2"),
    ] {
        item(&mut s, a, "action", "1");
        s.set_parent(a, parent, "t").unwrap();
    }
    close(&mut s, "zz12.a1");
    close(&mut s, "zz12.a2");
    s.set_field("zz12.a3", "archived", Some("2026-06-01".into()), "t");
    item(&mut s, "zz12.px", "project", "1");
    edge(&mut s, "zz12.px", "zz12.q3", EdgeKind::Mentions);
    close(&mut s, "zz12.px");

    let want = [
        "zz12.pa", "zz12.pb", "zz12.pd", "zz12.q1", "zz12.q2", "zz12.q3", "zz12.pc", "zz12.q4",
    ];
    let store = read::next(&cfg, &s, NOW, None).unwrap();
    assert_eq!(of_type(&store, "project"), want, "the store path");
    let reads = read::next_ranking_reads(&cfg).unwrap().unwrap();
    let board = read::with_ranking(&s, read::active_board(&s).unwrap(), &reads).unwrap();
    let graph = board.ranking.as_ref().unwrap();
    for id in ["zz12.gc", "zz12.ga", "zz12.a1", "zz12.a3", "zz12.px"] {
        assert!(
            graph.neighbours.contains_key(id),
            "{id} is a live neighbour"
        );
    }
    assert!(
        !graph.neighbours.contains_key("zz12.gd"),
        "a deleted one is not"
    );
    assert!(
        graph.edges.iter().all(|e| e.to != "zz12.gd"),
        "nor an edge to it"
    );
    assert_eq!(
        ids(&read::next_active(&cfg, &board, NOW).unwrap()),
        ids(&store)
    );
}

#[test]
fn a_next_order_reading_the_rank_of_a_type_only_list_orders_is_refused_without_a_store() {
    // Spec §12 point 1 lets a `next` key read the rank `[ranking.list]` gives a type. That rank is
    // computed over the type's live items, which a board of active tickets does not hold: the
    // store-free `next` refuses the plugin (owner, 2026-10-11), the store path ranks it. Any other
    // derived `next` key ranks without a store.
    let cfg: plugin::PluginConfig = toml::from_str(
        r#"
        name = "list-rank-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P0", "P1"]
        [types]
        list = ["action", "person"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [ { field = "priority" } ]
        [ranking.next.action]
        order = [ { related = "mentions", type = "person", of = "rank", nulls = "last" } ]
        [ranking.list.person]
        order = [ { field = "priority", dir = "asc" } ]
        [presentation.list]
        columns = ["id"]
        "#,
    )
    .unwrap();
    let mut s = Store::open_in_memory(1);
    item(&mut s, "zz12.a1", "action", "1");
    item(&mut s, "zz12.u1", "person", "0");
    edge(&mut s, "zz12.a1", "zz12.u1", EdgeKind::Mentions);
    let err = read::next_ranking_reads(&cfg).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg
            .contains("[ranking.next.action] reads the rank of 'person'"),
        "{}",
        err.msg
    );
    let board = read::active_board(&s).unwrap();
    assert_eq!(
        read::next_active(&cfg, &board, NOW).unwrap_err().kind,
        ErrorKind::Validation
    );
    assert!(
        read::next(&cfg, &s, NOW, None).is_ok(),
        "the store path ranks it"
    );
}

// ---- each kind over hand-built boards (§9 "per kind") -------------------------------------

/// A plugin whose `[ranking.list.item]` order is `order` — three types: `item` (ranked), `box`
/// and `note` (its neighbours). `box` ranks by priority under `[ranking.list.box]`, so a key may
/// read its rank. Parsed directly: the load-time rules are the plugin tests' business.
fn kinds_cfg(order: &str) -> PluginConfig {
    toml::from_str(&format!(
        r#"
        name = "kinds-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P0", "P1", "P2", "P3"]
        [types]
        list = ["item", "box", "note"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [ {{ field = "priority" }} ]
        [ranking.list]
        types = ["item", "box"]
        [ranking.list.item]
        order = {order}
        [ranking.list.box]
        order = [ {{ field = "priority", dir = "asc" }} ]
        [presentation.list]
        columns = ["id"]
        [fields.stage]
        type = "text"
        "#
    ))
    .unwrap()
}

/// The items of `s`, ranked by `cfg`'s list order.
fn ranked_items(cfg: &PluginConfig, s: &Store) -> Vec<String> {
    let rows = read::list_at(cfg, s, None, None, Some(SortKey::Rank), None).unwrap();
    of_type(&rows, "item")
}

/// Four items `i1`..`i4` (all P0) and boxes `b1` P2, `b2` P0, `b3` P1, plus note `n1` P3.
fn kinds_board() -> Store {
    let mut s = Store::open_in_memory(1);
    for i in ["zz12.i1", "zz12.i2", "zz12.i3", "zz12.i4"] {
        item(&mut s, i, "item", "0");
    }
    item(&mut s, "zz12.b1", "box", "2");
    item(&mut s, "zz12.b2", "box", "0");
    item(&mut s, "zz12.b3", "box", "1");
    item(&mut s, "zz12.n1", "note", "3");
    s
}

#[test]
fn related_reads_one_neighbours_field_and_places_the_items_without_one() {
    let mut s = kinds_board();
    s.set_parent("zz12.i1", "zz12.b1", "t").unwrap(); // P2
    s.set_parent("zz12.i2", "zz12.b2", "t").unwrap(); // P0
    s.set_parent("zz12.i3", "zz12.b3", "t").unwrap(); // P1
    let last = kinds_cfg(r#"[ { related = "parent", of = { field = "priority" } } ]"#);
    assert_eq!(
        ranked_items(&last, &s),
        ["zz12.i2", "zz12.i3", "zz12.i1", "zz12.i4"],
        "no parent → null, last by default"
    );
    let first = kinds_cfg(
        r#"[ { related = "parent", of = { field = "priority" }, dir = "desc", nulls = "first" } ]"#,
    );
    assert_eq!(
        ranked_items(&first, &s),
        ["zz12.i4", "zz12.i1", "zz12.i3", "zz12.i2"]
    );
}

#[test]
fn related_picks_the_min_or_max_of_several_neighbours() {
    let mut s = kinds_board();
    // i1 → b1 (P2) + b2 (P0); i2 → b3 (P1); i3 → b1 (P2).
    edge(&mut s, "zz12.i1", "zz12.b1", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.i1", "zz12.b2", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.i2", "zz12.b3", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.i3", "zz12.b1", EdgeKind::ContributesTo);
    let min = kinds_cfg(r#"[ { related = "contributes_to", of = { field = "priority" } } ]"#);
    assert_eq!(
        ranked_items(&min, &s),
        ["zz12.i1", "zz12.i2", "zz12.i3", "zz12.i4"],
        "min: i1 reads 0"
    );
    let max = kinds_cfg(
        r#"[ { related = "contributes_to", of = { field = "priority" }, pick = "max" } ]"#,
    );
    assert_eq!(
        ranked_items(&max, &s),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "max: i1 reads 2, ties i3 and falls to the id"
    );
}

#[test]
fn related_honours_its_type_filter_a_precedence_and_a_custom_field() {
    let mut s = kinds_board();
    edge(&mut s, "zz12.i1", "zz12.n1", EdgeKind::Mentions); // a note, P3
    edge(&mut s, "zz12.i2", "zz12.b1", EdgeKind::Mentions); // a box, P2
    let any =
        kinds_cfg(r#"[ { related = "mentions", of = { field = "priority" }, dir = "desc" } ]"#);
    assert_eq!(ranked_items(&any, &s)[0], "zz12.i1", "the note's P3 counts");
    let boxes = kinds_cfg(
        r#"[ { related = "mentions", type = "box", of = { field = "priority" }, dir = "desc" } ]"#,
    );
    assert_eq!(
        ranked_items(&boxes, &s),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "only box neighbours: i1 reads null"
    );
    custom(&mut s, "zz12.n1", "stage", "late");
    custom(&mut s, "zz12.b1", "stage", "early");
    let stage = kinds_cfg(
        r#"[ { related = "mentions", of = { field = "stage", precedence = ["early", "late"] } } ]"#,
    );
    assert_eq!(
        ranked_items(&stage, &s),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "the custom field's precedence position: early before late; no neighbour → null"
    );
}

#[test]
fn related_reads_a_neighbours_rank_in_its_own_order() {
    let mut s = kinds_board();
    // Box ranks by priority: b2 (P0) 0, b3 (P1) 1, b1 (P2) 2.
    s.set_parent("zz12.i1", "zz12.b1", "t").unwrap();
    s.set_parent("zz12.i2", "zz12.b3", "t").unwrap();
    s.set_parent("zz12.i3", "zz12.b2", "t").unwrap();
    let cfg = kinds_cfg(r#"[ { related = "parent", type = "box", of = "rank" } ]"#);
    assert_eq!(
        ranked_items(&cfg, &s),
        ["zz12.i3", "zz12.i2", "zz12.i1", "zz12.i4"]
    );
    // The rank is dense over the box order's keys: b2 and b3 tie on priority, so they share a
    // rank and their items fall through to the next key (here: the id).
    s.set_field("zz12.b3", "priority", Some("0".into()), "t");
    assert_eq!(
        ranked_items(&cfg, &s),
        ["zz12.i2", "zz12.i3", "zz12.i1", "zz12.i4"]
    );
    // A deleted neighbour is no neighbour.
    s.delete_item("zz12.b1", "t");
    assert_eq!(ranked_items(&cfg, &s)[2..], ["zz12.i1", "zz12.i4"]);
}

#[test]
fn count_counts_children_by_type_status_and_label() {
    let mut s = kinds_board();
    // i1: three children — two notes (one closed), one box. i2: one open note labelled `x`.
    for (id, ty) in [
        ("zz12.c1", "note"),
        ("zz12.c2", "note"),
        ("zz12.c3", "box"),
        ("zz12.c4", "note"),
    ] {
        item(&mut s, id, ty, "0");
    }
    s.set_parent("zz12.c1", "zz12.i1", "t").unwrap();
    s.set_parent("zz12.c2", "zz12.i1", "t").unwrap();
    s.set_parent("zz12.c3", "zz12.i1", "t").unwrap();
    s.set_parent("zz12.c4", "zz12.i2", "t").unwrap();
    close(&mut s, "zz12.c2");
    s.add_label("zz12.c4", "x", "t");
    let order = |s: &Store, key: &str| ranked_items(&kinds_cfg(&format!("[ {key} ]")), s);
    assert_eq!(
        order(&s, r#"{ count = "children", dir = "desc" }"#),
        ["zz12.i1", "zz12.i2", "zz12.i3", "zz12.i4"],
        "3, 1, 0, 0"
    );
    assert_eq!(
        order(&s, r#"{ count = "children", dir = "asc" }"#),
        ["zz12.i3", "zz12.i4", "zz12.i2", "zz12.i1"],
        "a count is never null: zero sorts as zero"
    );
    assert_eq!(
        order(&s, r#"{ count = "children", type = "box", dir = "desc" }"#)[0],
        "zz12.i1"
    );
    // Open notes only: i1 1, i2 1 → the id.
    assert_eq!(
        order(
            &s,
            r#"{ count = "children", type = "note", status = ["open"], dir = "desc" }"#
        ),
        ["zz12.i1", "zz12.i2", "zz12.i3", "zz12.i4"]
    );
    assert_eq!(
        order(&s, r#"{ count = "children", label = "x", dir = "desc" }"#),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"]
    );
    // A deleted child is no child.
    s.delete_item("zz12.c1", "t");
    s.delete_item("zz12.c3", "t");
    assert_eq!(
        order(
            &s,
            r#"{ count = "children", status = ["open"], dir = "desc" }"#
        ),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "i1 has only its closed child left"
    );
}

#[test]
fn count_reads_every_direction_and_kind() {
    let mut s = kinds_board();
    // Inbound: b1 → i3 contributes; b2, b3 → i3 dep (i3 has two dependents); n1 → i4 mentions.
    edge(&mut s, "zz12.b1", "zz12.i3", EdgeKind::ContributesTo);
    edge(&mut s, "zz12.b2", "zz12.i3", EdgeKind::Dep);
    edge(&mut s, "zz12.b3", "zz12.i3", EdgeKind::Dep);
    edge(&mut s, "zz12.n1", "zz12.i4", EdgeKind::Mentions);
    // Outbound: i2 → b1, b2 dep; i2 → n1 mentions; i1 → b3 contributes.
    edge(&mut s, "zz12.i2", "zz12.b1", EdgeKind::Dep);
    edge(&mut s, "zz12.i2", "zz12.b2", EdgeKind::Dep);
    edge(&mut s, "zz12.i2", "zz12.n1", EdgeKind::Mentions);
    edge(&mut s, "zz12.i1", "zz12.b3", EdgeKind::ContributesTo);
    let first = |s: &Store, kind: &str| {
        ranked_items(
            &kinds_cfg(&format!(r#"[ {{ count = "{kind}", dir = "desc" }} ]"#)),
            s,
        )[0]
        .clone()
    };
    assert_eq!(first(&s, "contributors"), "zz12.i3");
    assert_eq!(first(&s, "dependents"), "zz12.i3");
    assert_eq!(first(&s, "mentioned_by"), "zz12.i4");
    assert_eq!(first(&s, "dep"), "zz12.i2");
    assert_eq!(first(&s, "mentions"), "zz12.i2");
    assert_eq!(first(&s, "contributes_to"), "zz12.i1");
    assert_eq!(first(&s, "parent"), "zz12.i1", "all zero: the id decides");
    // One item with a parent: an outbound `parent` counts it (review of PR #48, Test Quality #7).
    s.set_parent("zz12.i4", "zz12.b1", "t").unwrap();
    assert_eq!(first(&s, "parent"), "zz12.i4");
}

#[test]
fn count_mentioned_by_keeps_closed_and_archived_mentioners_and_drops_deleted_ones() {
    // The reverse mention lookup is q29p's `mentioners_of` (spec §11): live is its one rule.
    let mut s = kinds_board();
    for n in ["zz12.m1", "zz12.m2", "zz12.m3"] {
        item(&mut s, n, "note", "0");
    }
    edge(&mut s, "zz12.m1", "zz12.i2", EdgeKind::Mentions);
    edge(&mut s, "zz12.m2", "zz12.i2", EdgeKind::Mentions);
    edge(&mut s, "zz12.m3", "zz12.i1", EdgeKind::Mentions);
    edge(&mut s, "zz12.n1", "zz12.i1", EdgeKind::Mentions);
    close(&mut s, "zz12.m1");
    s.set_field("zz12.m2", "archived", Some(NOW.into()), "t");
    s.delete_item("zz12.m3", "t");
    let cfg = kinds_cfg(r#"[ { count = "mentioned_by", dir = "desc" } ]"#);
    assert_eq!(
        ranked_items(&cfg, &s),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "i2: 2 (closed + archived), i1: 1 (the deleted one is gone)"
    );
    // Exactly what `mentioners_of` reads for the same targets.
    let m = read::mentioners_of(&s, &["zz12.i1", "zz12.i2"]).unwrap();
    assert_eq!(m["zz12.i2"].len(), 2);
    assert_eq!(m["zz12.i1"].len(), 1);
}

#[test]
fn sum_adds_its_counts_with_equal_weight() {
    let mut s = kinds_board();
    // i1: 2 box mentions + 0 children; i2: 1 box mention + 2 children; i3: 0 + 1.
    edge(&mut s, "zz12.b1", "zz12.i1", EdgeKind::Mentions);
    edge(&mut s, "zz12.b2", "zz12.i1", EdgeKind::Mentions);
    edge(&mut s, "zz12.b3", "zz12.i2", EdgeKind::Mentions);
    for c in ["zz12.c1", "zz12.c2", "zz12.c3"] {
        item(&mut s, c, "note", "0");
    }
    s.set_parent("zz12.c1", "zz12.i2", "t").unwrap();
    s.set_parent("zz12.c2", "zz12.i2", "t").unwrap();
    s.set_parent("zz12.c3", "zz12.i3", "t").unwrap();
    let cfg = kinds_cfg(
        r#"[ { sum = [ { count = "mentioned_by", type = "box" }, { count = "children" } ], dir = "desc" } ]"#,
    );
    assert_eq!(
        ranked_items(&cfg, &s),
        ["zz12.i2", "zz12.i1", "zz12.i3", "zz12.i4"],
        "3, 2, 1, 0 — not mentions first"
    );
}

#[test]
fn a_concurrently_reparented_child_counts_under_its_projected_parent_only() {
    // Review of PR #48, Integrity #1: two replicas re-point one child at once. The OR-set then
    // holds both `parent` edges, but the derived keys read the `present_parent` projection — the
    // one parent `belongs_to`, `--in` and `show` read (site 2 wins at equal lamport).
    let mut a = kinds_board();
    let mut b = Store::open_in_memory(2);
    b.apply(&a.export());
    // The note n1 under i2 (site 1) and under i3 (site 2); the item i1 under b1 P2 (site 1) and
    // under b2 P0 (site 2); i2 under b3 P1 everywhere.
    a.set_parent("zz12.n1", "zz12.i2", "alice").unwrap();
    b.set_parent("zz12.n1", "zz12.i3", "bob").unwrap();
    a.set_parent("zz12.i1", "zz12.b1", "alice").unwrap();
    b.set_parent("zz12.i1", "zz12.b2", "bob").unwrap();
    let (ea, eb) = (a.export(), b.export());
    a.apply(&eb);
    b.apply(&ea);
    a.set_parent("zz12.i2", "zz12.b3", "t").unwrap();
    assert_eq!(a.parents_of("zz12.n1").len(), 2, "precondition: two edges");
    assert_eq!(
        a.get_item("zz12.n1")
            .unwrap()
            .unwrap()
            .belongs_to
            .as_deref(),
        Some("zz12.i3")
    );
    assert_eq!(
        a.get_item("zz12.i1")
            .unwrap()
            .unwrap()
            .belongs_to
            .as_deref(),
        Some("zz12.b2")
    );
    let children = kinds_cfg(r#"[ { count = "children", dir = "desc" } ]"#);
    assert_eq!(
        ranked_items(&children, &a),
        ["zz12.i3", "zz12.i1", "zz12.i2", "zz12.i4"],
        "n1 counts under i3 alone, not under i2 as well"
    );
    let max = kinds_cfg(r#"[ { related = "parent", of = { field = "priority" }, pick = "max" } ]"#);
    assert_eq!(
        ranked_items(&max, &a),
        ["zz12.i1", "zz12.i2", "zz12.i3", "zz12.i4"],
        "i1 reads b2's P0 — max over the one projected parent, not b1's P2 as well"
    );
}

#[test]
fn a_list_rank_scope_ignores_the_status_filter() {
    // spec §12, decision 2, pinned (review of PR #48, Test Quality #2): under `--status open` the
    // closed box b2 is filtered out of the rows, but it is still in the scope its type ranks over,
    // so its rank (0, P0) is still read by the item under it.
    let mut s = kinds_board();
    close(&mut s, "zz12.b2");
    s.set_parent("zz12.i3", "zz12.b2", "t").unwrap(); // rank 0, closed
    s.set_parent("zz12.i1", "zz12.b1", "t").unwrap(); // rank 2
    let cfg = kinds_cfg(r#"[ { related = "parent", type = "box", of = "rank" } ]"#);
    let rows = read::list_at(&cfg, &s, Some("open"), None, Some(SortKey::Rank), None).unwrap();
    assert!(!ids(&rows).contains(&"zz12.b2".to_string()), "filtered out");
    assert_eq!(
        of_type(&rows, "item"),
        ["zz12.i3", "zz12.i1", "zz12.i2", "zz12.i4"]
    );
}

#[test]
fn a_derived_key_ranks_in_the_list_default_order_without_per_type_tables() {
    // Review of PR #48, Test Quality #5: the `[ranking.list]` default order takes derived keys
    // (spec §12) and ranks every type by them when no per-type order is declared.
    let cfg: PluginConfig = toml::from_str(
        r#"
        name = "list-default-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P0", "P1", "P2", "P3"]
        [types]
        list = ["item", "box", "note"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [ { field = "priority" } ]
        [ranking.list]
        order = [ { count = "children", dir = "desc" }, { field = "priority" } ]
        [presentation.list]
        columns = ["id"]
        "#,
    )
    .unwrap();
    assert!(cfg.ranking.list.has_derived() && !cfg.ranking.list.splits_by_type());
    let mut s = kinds_board();
    // b3 two children, i4 one; everyone else none, then priority (P0 items, b2 P0, b1 P2, n1 P3).
    s.set_parent("zz12.i1", "zz12.b3", "t").unwrap();
    s.set_parent("zz12.i2", "zz12.b3", "t").unwrap();
    s.set_parent("zz12.n1", "zz12.i4", "t").unwrap();
    let rows = read::list_at(&cfg, &s, None, None, Some(SortKey::Rank), None).unwrap();
    assert_eq!(
        ids(&rows),
        ["zz12.b3", "zz12.i4", "zz12.b2", "zz12.i1", "zz12.i2", "zz12.i3", "zz12.b1", "zz12.n1"]
    );
}

/// A plugin whose list ranks items by the rank of their parent box, and whose `next` ranks boxes
/// by `box_order` — the cross-section read whose fingerprint a list token carries.
fn cross_cfg(box_order: &str) -> PluginConfig {
    toml::from_str(&format!(
        r#"
        name = "cross-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P0", "P1", "P2", "P3"]
        [types]
        list = ["item", "box", "note"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        types = ["box"]
        order = [ {{ field = "priority" }} ]
        [ranking.next.box]
        order = {box_order}
        [ranking.list]
        types = ["item"]
        [ranking.list.item]
        order = [ {{ related = "parent", type = "box", of = "rank" }} ]
        [presentation.list]
        columns = ["id"]
        "#
    ))
    .unwrap()
}

#[test]
fn a_derived_list_token_restarts_when_only_the_next_order_changes() {
    // Review of PR #48, Test Quality #3: a list order that reads a `next` rank binds its token to
    // `[ranking.next]` too. The changed `next` order adds a key that decides nothing here (every
    // box is open), so the list order is the same and only the fingerprint can restart the token.
    let mut s = kinds_board();
    s.set_parent("zz12.i1", "zz12.b1", "t").unwrap();
    s.set_parent("zz12.i2", "zz12.b2", "t").unwrap();
    s.set_parent("zz12.i3", "zz12.b3", "t").unwrap();
    let before = cross_cfg(r#"[ { field = "priority", dir = "asc" } ]"#);
    let after =
        cross_cfg(r#"[ { field = "priority", dir = "asc" }, { field = "status", dir = "asc" } ]"#);
    let q = || {
        ListQuery::new()
            .sort(SortKey::Rank)
            .filter(NextFilter::new().with_type("item"))
            .now(NOW)
            .limit(2)
    };
    let p1 = read::list_page(&before, &s, &q().paginate()).unwrap();
    assert_eq!(
        ids(&p1.items),
        ["zz12.i2", "zz12.i3"],
        "b2 P0, b3 P1, b1 P2"
    );
    let token = p1.next_token.expect("rows remain");
    let same = read::list_page(&before, &s, &q().token(token.clone())).unwrap();
    assert!(!same.restarted, "the same plugin resumes");
    assert_eq!(ids(&same.items), ["zz12.i1", "zz12.i4"]);
    let fresh = read::list_page(&after, &s, &q()).unwrap();
    assert_eq!(ids(&fresh.items), ids(&p1.items), "the order did not move");
    let changed = read::list_page(&after, &s, &q().token(token)).unwrap();
    assert!(
        changed.restarted,
        "a changed [ranking.next.box] restarts the list token"
    );
}

#[test]
fn a_plugin_without_derived_keys_ranks_list_and_next_as_before() {
    let cfg = plugin::load("issue-tracker").unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("zz12.e1", "epic", "e", "t");
    s.create_item("zz12.t1", "task", "t", "t");
    let before = s.edge_read_stats();
    read::list_at(&cfg, &s, None, None, Some(SortKey::Rank), None).unwrap();
    read::next(&cfg, &s, NOW, None).unwrap();
    assert_eq!(
        s.edge_read_stats().since(before),
        EdgeReadStats::default(),
        "no derived key, no edge pass"
    );
}

// ---- determinism (§9) -----------------------------------------------------------------

#[test]
fn another_edge_order_ranks_alike_and_ties_fall_to_the_id() {
    let cfg = kinds_cfg(
        r#"[ { count = "mentioned_by", dir = "desc" }, { related = "contributes_to", type = "box", of = "rank", pick = "min" } ]"#,
    );
    let edges = [
        ("zz12.b1", "zz12.i3", EdgeKind::Mentions),
        ("zz12.n1", "zz12.i3", EdgeKind::Mentions),
        ("zz12.b2", "zz12.i2", EdgeKind::Mentions),
        ("zz12.i2", "zz12.b1", EdgeKind::ContributesTo),
        ("zz12.i1", "zz12.b3", EdgeKind::ContributesTo),
        ("zz12.i1", "zz12.b1", EdgeKind::ContributesTo),
        ("zz12.b3", "zz12.i1", EdgeKind::Mentions),
        // i4 ties i2 on both keys (one mention, best box b1): a FULL tie.
        ("zz12.n1", "zz12.i4", EdgeKind::Mentions),
        ("zz12.i4", "zz12.b1", EdgeKind::ContributesTo),
    ];
    let build = |order: &[usize]| {
        let mut s = kinds_board();
        for &n in order {
            let (from, to, kind) = edges[n];
            edge(&mut s, from, to, kind);
        }
        ranked_items(&cfg, &s)
    };
    // The edge reads are ordered, so the insertion order cannot leak into the rank today; this
    // pins that against a future read that is not (a hash map, an unordered scan).
    let forward = build(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(build(&[8, 7, 6, 5, 4, 3, 2, 1, 0]), forward);
    assert_eq!(build(&[3, 7, 0, 6, 1, 8, 5, 2, 4]), forward);
    // i3: 2 mentions; i1, i2 and i4: 1 each, then the best box rank (b3 rank 1 vs b1 rank 2): i1;
    // i2 and i4 tie on every key and fall to the id, as every order does.
    assert_eq!(forward, ["zz12.i3", "zz12.i1", "zz12.i2", "zz12.i4"]);
    // The full tie, from the other side: renamed so that the id order is reversed, the tied pair
    // swaps, and nothing else moves.
    let mut s = kinds_board();
    item(&mut s, "zz12.i0", "item", "0");
    for &(from, to, kind) in &edges {
        let swap = |id: &'static str| if id == "zz12.i4" { "zz12.i0" } else { id };
        edge(&mut s, swap(from), swap(to), kind);
    }
    s.delete_item("zz12.i4", "t");
    assert_eq!(
        ranked_items(&cfg, &s),
        ["zz12.i3", "zz12.i1", "zz12.i0", "zz12.i2"]
    );
}

// ---- bounded cost (§9: a ratio guard in the style of xn8s) ----------------------------

/// A board of `n` projects under the spec's orders, each with two open actions, a goal, and a
/// person it mentions — enough graph for every key kind and the full three-type chain.
fn cost_board(n: usize) -> Store {
    let mut s = Store::open_in_memory(1);
    for g in 0..4 {
        let id = format!("zz12.g{g}");
        item(&mut s, &id, "goal", "1");
        custom(&mut s, &id, "order", &g.to_string());
    }
    for p in 0..n {
        let pid = format!("zz12.p{p:04}");
        item(&mut s, &pid, "project", &(p % 4).to_string());
        edge(
            &mut s,
            &pid,
            &format!("zz12.g{}", p % 4),
            EdgeKind::ContributesTo,
        );
        let uid = format!("zz12.u{p:04}");
        item(&mut s, &uid, "person", "1");
        edge(&mut s, &pid, &uid, EdgeKind::Mentions);
        edge(&mut s, &uid, &pid, EdgeKind::ContributesTo);
        for a in 0..2 {
            let aid = format!("zz12.a{p:04}{a}");
            item(&mut s, &aid, "action", &((p + a) % 4).to_string());
            s.set_parent(&aid, &pid, "t").unwrap();
        }
    }
    s
}

/// The reads one ranked `next` and one ranked `list` run on a board of `n` projects.
fn reads(n: usize) -> (usize, EdgeReadStats, ItemReadStats) {
    let cfg = plugin::load("derived-rank-fixture").unwrap();
    let s = cost_board(n);
    let (edges, items) = (s.edge_read_stats(), s.item_read_stats());
    let next = read::next(&cfg, &s, NOW, None).unwrap();
    read::list_at(&cfg, &s, None, None, Some(SortKey::Rank), Some(NOW)).unwrap();
    (
        next.len(),
        s.edge_read_stats().since(edges),
        s.item_read_stats().since(items),
    )
}

/// The edge pass runs ONCE per query, not once per candidate (spec §5, §9). Asserts the query
/// shape — the store's own counts of edge and item reads — not the clock, for the reason the xn8s
/// guard gives (a timing ratio reports the machine's load as readily as the code's): the counts
/// are identical on a board of 12 projects and on one five times its size, so the ratio of reads
/// to candidates falls with the board instead of staying flat. A per-candidate edge read, or a
/// per-type rank recomputed per neighbour, turns them into numbers that grow with `n`.
#[test]
fn the_edge_pass_runs_once_per_query_however_many_candidates() {
    let (small_n, small_edges, small_items) = reads(12);
    let (large_n, large_edges, large_items) = reads(60);
    assert!(
        large_n > 4 * small_n && small_n > 20,
        "fixture regressed: {small_n} / {large_n} candidates"
    );
    // `next`: one pass; `list`: one pass, plus `next`'s candidates for the project rank (no new
    // edge read) and one reverse-mention lookup for the persons' `sum`.
    assert_eq!(
        small_edges,
        EdgeReadStats {
            passes: 2,
            mention_lookups: 1,
            ..small_edges
        },
        "one edge pass per query, one mention lookup for the one query that counts mentions"
    );
    // Every other read of edges, labels or custom fields is counted too (review of PR #48, Test
    // Quality #1): none runs per item, and the bulk ones do not grow with the board (the equality
    // below), so a per-candidate regression through any of them goes red here.
    assert_eq!(
        small_edges.per_item, 0,
        "no per-item edge read: {small_edges:?}"
    );
    assert_eq!(
        large_edges, small_edges,
        "the edge reads do not grow with the board"
    );
    assert_eq!(small_items.single, 0, "no per-item read: {small_items:?}");
    assert_eq!(
        large_items, small_items,
        "the item reads do not grow with the board"
    );
}
