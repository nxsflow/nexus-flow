//! `list` with a plugin-declared order per type, the type and container filters, and paging
//! (6j6v.n698): `[ranking.list.<type>]` ranks `list --sort rank`, `types = [...]` orders the types
//! against each other, a type without a table falls back to the `[ranking.next]` default order, and
//! `Engine::list_query` narrows and pages exactly as `next_query` does — through the same snapshot
//! cache. Tested at the seam the products take. Its own test binary, so the fixture plugins do not
//! pollute the facade lib tests' "exactly two OSS plugins" assertion.

use nexus_flow_core::model::{EdgeKind, ItemRow};
use nexus_flow_core::store::Store;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginRegistration};
use nexus_flow_facade::read::{self, ListQuery, MentionedByQuery, NextFilter, NextQuery, SortKey};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";
const ACTOR: &str = "alice";

/// The fixture: four types. Under `list --sort rank`, persons rank by title, projects by priority
/// DESCENDING, and both stand before the types without a list table (`action`, `goal`), which take
/// the `[ranking.next]` default order. `next` never sees `[ranking.list]`.
const LIST_RANKED: &str = r#"
name = "list-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["goal", "project", "person", "action"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [
    { field = "status", precedence = ["in_progress", "open"] },
    { field = "priority", dir = "asc" },
]
[ranking.list]
types = ["person", "project"]
[ranking.list.person]
order = [ { field = "title", dir = "asc" } ]
[ranking.list.project]
order = [ { field = "priority", dir = "desc" } ]
[presentation.list]
columns = ["id"]
"#;

inventory::submit!(PluginRegistration {
    name: "list-rank-fixture",
    toml: LIST_RANKED,
    order: 120,
});

// The same plugin without `[ranking.list]`: `list --sort rank` ranks by the `[ranking.next]`
// default order alone, as before n698.
inventory::submit!(PluginRegistration {
    name: "list-flat-rank-fixture",
    toml: r#"
name = "list-flat-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["goal", "project", "person", "action"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [
    { field = "status", precedence = ["in_progress", "open"] },
    { field = "priority", dir = "asc" },
]
[presentation.list]
columns = ["id"]
"#,
    order: 121,
});

fn workspace(plugin: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), plugin).unwrap();
    tmp
}

/// Another writer on the workspace — what a second process, or a sync pull, is to the engine.
fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

fn item(s: &mut Store, id: &str, ty: &str, title: &str, priority: &str) {
    s.create_item(id, ty, title, "t");
    s.set_field(id, "priority", Some(priority.into()), "t");
}

/// Seven items whose per-type list orders DISAGREE with both the id order and the flat rank:
///
/// - persons: `m001` "Zoe" P1, `m002` "Adam" P2 (label `vip`) → by title: m002, m001;
/// - projects: `p001` P0, `p002` P3 → by priority desc: p002, p001;
/// - actions (no table): `a001` P3 child of p001, `a002` P0 in progress, contributes to p001 →
///   the default order: a002, a001;
/// - goal (no table): `g001` P1.
///
/// `m001` contributes to `p001` as well, so the container `p001` holds a001, a002 and m001.
fn board(dir: &Path) {
    let mut s = writer(dir);
    item(&mut s, "zz12.m001", "person", "Zoe", "1");
    item(&mut s, "zz12.m002", "person", "Adam", "2");
    item(&mut s, "zz12.p001", "project", "Build", "0");
    item(&mut s, "zz12.p002", "project", "Archive", "3");
    item(&mut s, "zz12.a001", "action", "Write", "3");
    item(&mut s, "zz12.a002", "action", "Read", "0");
    item(&mut s, "zz12.g001", "goal", "Grow", "1");
    s.set_parent("zz12.a001", "zz12.p001", "t").unwrap();
    s.add_edge("zz12.a002", "zz12.p001", EdgeKind::ContributesTo, "t");
    s.add_edge("zz12.m001", "zz12.p001", EdgeKind::ContributesTo, "t");
    s.set_field("zz12.a002", "status", Some("in_progress".into()), "t");
    s.add_label("zz12.m002", "vip", "t");
}

/// `list --sort rank` under the fixture: persons by title, projects by priority descending, then
/// the unlisted types by name (`action` before `goal`), each by the `[ranking.next]` default order.
const RANKED: [&str; 7] = [
    "zz12.m002",
    "zz12.m001",
    "zz12.p002",
    "zz12.p001",
    "zz12.a002",
    "zz12.a001",
    "zz12.g001",
];

const BY_ID: [&str; 7] = [
    "zz12.a001",
    "zz12.a002",
    "zz12.g001",
    "zz12.m001",
    "zz12.m002",
    "zz12.p001",
    "zz12.p002",
];

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn engine(tmp: &TempDir) -> Engine {
    Engine::open(None, tmp.path()).unwrap()
}

fn page(engine: &Engine, q: ListQuery) -> read::NextPage {
    engine.list_query(&q).unwrap()
}

fn ranked() -> ListQuery {
    ListQuery::new().sort(SortKey::Rank)
}

// ---- the order ----------------------------------------------------------------------

#[test]
fn the_default_order_stays_id_whatever_the_plugin_declares() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let p = page(&e, ListQuery::new());
    assert_eq!(ids(&p.items), BY_ID);
    assert_eq!((p.total, p.start, p.restarted), (7, 0, false));
    assert_eq!(p.next_token, None);
    assert_eq!(ids(&e.list(None, None).unwrap()), BY_ID, "Engine::list too");
}

#[test]
fn sort_rank_takes_the_per_type_list_orders_and_the_cross_type_order() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let p = page(&engine(&tmp), ranked());
    assert_eq!(ids(&p.items), RANKED);
    // The free read the CLI's plain `list --sort rank` and `flow_list` take ranks alike.
    let s = writer(tmp.path());
    let cfg = plugin::load("list-rank-fixture").unwrap();
    let free = read::list(&cfg, &s, None, None, Some(SortKey::Rank)).unwrap();
    assert_eq!(ids(&free), RANKED);
}

#[test]
fn without_ranking_list_sort_rank_ranks_by_the_next_default_order_as_before() {
    // Status precedence first (a002 is in progress), then priority ascending, then id — across
    // all types, with no type split.
    let tmp = workspace("list-flat-rank-fixture");
    board(tmp.path());
    assert_eq!(
        ids(&page(&engine(&tmp), ranked()).items),
        [
            "zz12.a002",
            "zz12.p001",
            "zz12.g001",
            "zz12.m001",
            "zz12.m002",
            "zz12.a001",
            "zz12.p002"
        ]
    );
}

#[test]
fn the_bundled_plugins_rank_list_exactly_as_before() {
    // issue-tracker: status → priority → type → due → id — the `next` default order, as
    // `list --sort rank` ranked before n698.
    let tmp = workspace("issue-tracker");
    {
        let mut s = writer(tmp.path());
        item(&mut s, "it12.0001", "epic", "e", "2");
        item(&mut s, "it12.0002", "feature", "f", "1");
        item(&mut s, "it12.0003", "bug", "b", "1");
        item(&mut s, "it12.0004", "chore", "c", "0");
    }
    assert_eq!(
        ids(&page(&engine(&tmp), ranked()).items),
        ["it12.0004", "it12.0003", "it12.0002", "it12.0001"]
    );
}

#[test]
fn ranking_list_never_changes_next() {
    let ranked_ws = workspace("list-rank-fixture");
    board(ranked_ws.path());
    let flat_ws = workspace("list-flat-rank-fixture");
    board(flat_ws.path());
    assert_eq!(
        ids(&engine(&ranked_ws).next(NOW).unwrap()),
        ids(&engine(&flat_ws).next(NOW).unwrap()),
        "both plugins share [ranking.next]; [ranking.list] is list's alone"
    );
}

// ---- the filters ----------------------------------------------------------------------

#[test]
fn list_query_filters_by_type_container_label_and_status_keeping_the_order() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let q = |q: ListQuery| ids(&page(&e, q).items);

    assert_eq!(
        q(ranked().filter(NextFilter::new().with_type("person"))),
        ["zz12.m002", "zz12.m001"]
    );
    assert_eq!(
        q(ranked().filter(NextFilter::new().with_type("project").with_type("person"))),
        ["zz12.m002", "zz12.m001", "zz12.p002", "zz12.p001"],
        "the types are alternatives"
    );
    // In p001: its child a001, and its contributors a002 and m001.
    assert_eq!(
        q(ranked().filter(NextFilter::new().with_container("zz12.p001"))),
        ["zz12.m001", "zz12.a002", "zz12.a001"]
    );
    assert_eq!(
        q(ListQuery::new().filter(NextFilter::new().with_container("zz12.p001"))),
        ["zz12.a001", "zz12.a002", "zz12.m001"],
        "id order without a sort"
    );
    assert_eq!(
        q(ListQuery::new().filter(NextFilter::new().with_label("vip"))),
        ["zz12.m002"]
    );
    assert_eq!(q(ListQuery::new().status("in_progress")), ["zz12.a002"]);
    assert!(q(ListQuery::new().filter(NextFilter::new().with_container("zz12.g001"))).is_empty());
}

// ---- paging, as `next` pages ----------------------------------------------------------

#[test]
fn a_limit_alone_cuts_and_discloses_but_hands_out_no_token() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let p = page(&engine(&tmp), ranked().limit(3));
    assert_eq!(ids(&p.items), RANKED[0..3]);
    assert_eq!(p.total, 7);
    assert!(p.truncated());
    assert_eq!(p.next_token, None);
}

#[test]
fn paginate_walks_the_whole_ranked_list_in_order() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let mut seen = Vec::new();
    let mut p = page(&e, ranked().limit(3).paginate());
    loop {
        assert!(!p.restarted);
        assert_eq!(p.start, seen.len());
        assert_eq!(p.total, 7);
        seen.extend(ids(&p.items));
        match p.next_token.clone() {
            Some(t) => p = page(&e, ranked().limit(3).token(t)),
            None => break,
        }
    }
    assert_eq!(seen, RANKED);
}

#[test]
fn a_change_to_a_cached_item_restarts_at_the_first_page() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let p1 = page(&e, ranked().limit(3).paginate());
    // p001 is on the page not yet served: the op bears on the loaded list.
    e.update(NOW, ACTOR, "zz12.p001", &["title=Built".to_string()])
        .unwrap();
    let p = page(&e, ranked().limit(3).token(p1.next_token.unwrap()));
    assert!(p.restarted, "the snapshot is stale");
    assert_eq!(
        (ids(&p.items), p.start),
        (RANKED[0..3].iter().map(|s| s.to_string()).collect(), 0)
    );
}

#[test]
fn a_new_item_does_not_invalidate_and_waits_for_a_fresh_query() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let p1 = page(&e, ranked().limit(4).paginate());
    // A person titled "Aaron" would rank first; it is not in the snapshot, so this pager keeps on.
    item(&mut writer(tmp.path()), "zz12.m003", "person", "Aaron", "1");
    let p2 = page(&e, ranked().limit(4).token(p1.next_token.unwrap()));
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), RANKED[4..]);
    let fresh = page(&e, ranked());
    assert_eq!((fresh.total, fresh.items[0].id.as_str()), (8, "zz12.m003"));
}

#[test]
fn a_token_of_another_sort_filter_status_or_read_restarts() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let t = page(&e, ranked().limit(2).paginate()).next_token.unwrap();

    let by_id = page(&e, ListQuery::new().limit(2).token(&t));
    assert!(by_id.restarted, "another sort");
    assert_eq!(ids(&by_id.items), BY_ID[0..2]);
    let filtered = page(
        &e,
        ranked()
            .filter(NextFilter::new().with_type("person"))
            .limit(2)
            .token(&t),
    );
    assert!(filtered.restarted, "another filter");
    assert!(
        page(&e, ranked().status("open").limit(2).token(&t)).restarted,
        "another status"
    );
    let as_next = e
        .next_query(&NextQuery::new(NOW).limit(2).token(&t))
        .unwrap();
    assert!(as_next.restarted, "a list token is not a next token");
    let as_mentioned_by = e
        .mentioned_by("zz12.p001", &MentionedByQuery::new().limit(2).token(&t))
        .unwrap();
    assert!(as_mentioned_by.restarted, "nor a mentioned-by token");

    // And the other way round: a next token does not page a list.
    let next_token = e
        .next_query(&NextQuery::new(NOW).limit(2).paginate())
        .unwrap()
        .next_token
        .unwrap();
    assert!(page(&e, ranked().limit(2).token(next_token)).restarted);

    let garbage = page(&e, ranked().limit(2).token("nope"));
    assert!(garbage.restarted);
    assert_eq!(ids(&garbage.items), RANKED[0..2]);
}

#[test]
fn a_token_from_another_plugin_list_order_restarts() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let s = writer(tmp.path());
    let ranked_cfg = plugin::load("list-rank-fixture").unwrap();
    let flat_cfg = plugin::load("list-flat-rank-fixture").unwrap();
    let p1 = read::list_page(&ranked_cfg, &s, &ranked().limit(2).paginate()).unwrap();
    let token = p1.next_token.unwrap();
    let under_flat = read::list_page(&flat_cfg, &s, &ranked().limit(2).token(&token)).unwrap();
    assert!(under_flat.restarted, "another [ranking.list] restarts");
    let p2 = read::list_page(&ranked_cfg, &s, &ranked().limit(2).token(&token)).unwrap();
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), RANKED[2..4]);
}

#[test]
fn paginate_needs_a_limit_and_a_token_needs_paginate() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    for q in [
        ListQuery::new().paginate(),
        ListQuery::new().limit(0).paginate(),
    ] {
        assert_eq!(e.list_query(&q).unwrap_err().kind, ErrorKind::Validation);
    }
    let mut q = ListQuery::new().limit(2);
    q.token = Some("x.2".into());
    assert_eq!(e.list_query(&q).unwrap_err().kind, ErrorKind::Validation);
}

#[test]
fn list_query_value_is_the_paging_envelope_of_list_records() {
    let tmp = workspace("list-rank-fixture");
    board(tmp.path());
    let e = engine(&tmp);
    let v = e.list_query_value(&ranked().limit(2).paginate()).unwrap();
    assert_eq!(v["total"], 7);
    assert_eq!(v["restarted"], false);
    assert!(v["next_token"].is_string());
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "zz12.m002");
    assert_eq!(items[0]["type"], "person");
    // The records are `list_value`'s.
    let listed = e.list_value(None, None).unwrap();
    let same = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "zz12.m002")
        .unwrap();
    assert_eq!(&items[0], same);
    // Without paginate: no token keys, as for `next`.
    let plain = e.list_query_value(&ListQuery::new()).unwrap();
    assert_eq!(plain["total"], 7);
    assert!(plain.get("next_token").is_none());
}
