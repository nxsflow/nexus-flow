//! `list` with a plugin-declared order per type, the type and container filters, and paging
//! (6j6v.n698): `[ranking.list.<type>]` ranks `list --sort rank`, `types = [...]` orders the types
//! against each other, a type without a table falls back to the `[ranking.next]` default order, and
//! `Engine::list_query` narrows and pages exactly as `next_query` does — through the same snapshot
//! cache. Tested at the seam the products take. Its own test binary, so the fixture plugins do not
//! pollute the facade lib tests' "exactly two OSS plugins" assertion.

use nexus_flow_core::model::{EdgeKind, ItemRow, MergeStrategy};
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

// ---- the ref filter: `list --ref` (6j6v.9tx3, cut 2) ------------------------------------

/// The ref fixture: meetings hold their `participants` (a list ref to persons), stations a single
/// `person` and a single `company`, persons a free-text `note`. Under `list --sort rank` a station
/// ranks by its PARENT person's rank — computed over the whole person scope — then by title, so a
/// filter that narrowed the scope instead of the rows would reorder the stations.
const LIST_REF: &str = r#"
name = "list-ref-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["meeting", "person", "company", "station"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [ { field = "priority", dir = "asc" } ]
[ranking.list]
types = ["station", "person"]
[ranking.list.person]
order = [ { field = "title", dir = "asc" } ]
[ranking.list.station]
order = [
  { related = "parent", type = "person", of = "rank", nulls = "last" },
  { field = "title", dir = "asc" },
]
[presentation.list]
columns = ["id"]
[fields.participants]
type = "ref"
targets = ["person"]
list = true
on = ["meeting"]
[fields.person]
type = "ref"
targets = ["person"]
on = ["station"]
[fields.company]
type = "ref"
targets = ["company"]
on = ["station"]
[fields.note]
type = "text"
on = ["person"]
"#;

inventory::submit!(PluginRegistration {
    name: "list-ref-fixture",
    toml: LIST_REF,
    order: 122,
});

const P1: &str = "ab12.p001";
const P2: &str = "ab12.p002";
const C1: &str = "ab12.c001";

/// The one source `ref_place_ok` excludes: a station whose own id is 129 bytes.
fn long_station() -> String {
    format!("ab12.{}", "s".repeat(124))
}

/// The ref board. Validated writes go through the Engine; the shapes an older binary leaves (a
/// plain `add`, a plain `set` on an over-long source) go through the store directly.
///
/// - persons `p001` "Ada", `p002` "Zed", `x001` "Mia" (its text `note` holds `p001`'s id);
/// - company `c001`;
/// - meetings: `m001` (p001, p002), `m002` (p001, closed), `m003` (p002), `m004` (p001, then
///   removed), `m005` (p001, archived), `m006` (p001 through a plain `add_field_entry`);
/// - stations: `s001` (p001 at c001, "A", parent p002), `s002` (p001 at c001, "B", parent p001),
///   `s003` (p002 at c001, "C", parent p002), `s004` (p001, deleted), and the 129-byte station
///   (p001 through a plain `set`, "D", no parent).
fn ref_board(tmp: &TempDir) -> Engine {
    let mut s = writer(tmp.path());
    for (id, ty, title) in [
        (P1, "person", "Ada"),
        (P2, "person", "Zed"),
        ("ab12.x001", "person", "Mia"),
        (C1, "company", "Acme"),
        ("ab12.m001", "meeting", "Kickoff"),
        ("ab12.m002", "meeting", "Review"),
        ("ab12.m003", "meeting", "Lunch"),
        ("ab12.m004", "meeting", "Call"),
        ("ab12.m005", "meeting", "Old"),
        ("ab12.m006", "meeting", "Legacy"),
        ("ab12.s001", "station", "A"),
        ("ab12.s002", "station", "B"),
        ("ab12.s003", "station", "C"),
        ("ab12.s004", "station", "Gone"),
    ] {
        s.create_item(id, ty, title, "t");
    }
    let long = long_station();
    s.create_item(&long, "station", "D", "t");
    s.set_custom_field_merge(&long, "person", Some(P1.into()), "t", MergeStrategy::Lww);
    s.add_field_entry("ab12.m006", "participants", P1, None, "t");
    s.set_parent("ab12.s001", P2, "t").unwrap();
    s.set_parent("ab12.s002", P1, "t").unwrap();
    s.set_parent("ab12.s003", P2, "t").unwrap();
    drop(s);

    let e = engine(tmp);
    let add = |id: &str, target: &str| {
        e.field_entry_add(NOW, ACTOR, id, "participants", target, None)
            .unwrap();
    };
    add("ab12.m001", P1);
    add("ab12.m001", P2);
    add("ab12.m002", P1);
    add("ab12.m003", P2);
    add("ab12.m004", P1);
    add("ab12.m005", P1);
    e.field_entry_remove(NOW, ACTOR, "ab12.m004", "participants", P1)
        .unwrap();
    e.close(NOW, ACTOR, "ab12.m002", Some("held")).unwrap();
    e.close(NOW, ACTOR, "ab12.m005", Some("held")).unwrap();
    let archived = e.archive(NOW, ACTOR, &["ab12.m005".to_string()]).unwrap();
    assert!(
        archived.outcomes.iter().all(|o| o.ok),
        "{:?}",
        archived.outcomes
    );
    let set = |id: &str, sets: &[&str]| {
        let sets: Vec<String> = sets.iter().map(|x| x.to_string()).collect();
        e.update(NOW, ACTOR, id, &sets).unwrap();
    };
    set("ab12.s001", &["person=ab12.p001", "company=ab12.c001"]);
    set("ab12.s002", &["person=ab12.p001", "company=ab12.c001"]);
    set("ab12.s003", &["person=ab12.p002", "company=ab12.c001"]);
    set("ab12.s004", &["person=ab12.p001"]);
    set("ab12.x001", &["note=ab12.p001"]);
    let mut s = writer(tmp.path());
    s.delete_item("ab12.s004", "t");
    e
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn stations() -> ListQuery {
    ListQuery::new().filter(NextFilter::new().with_type("station"))
}

#[test]
fn list_referencing_lists_live_unarchived_referrers_of_a_declared_ref_field_by_equality() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    // m001, m002 (closed), m006 (a plain add); not m004 (removed), m005 (archived), x001 (the id in
    // a text field), m003 (another person).
    let page = e
        .list_query(&ListQuery::new().referencing("participants", P1))
        .unwrap();
    assert_eq!(ids(&page.items), ["ab12.m001", "ab12.m002", "ab12.m006"]);
    assert_eq!(page.total, 3);
    // A single ref: the deleted s004 is dropped, the 129-byte station is not a referrer anywhere.
    let page = e
        .list_query(&ListQuery::new().referencing("person", P1))
        .unwrap();
    assert_eq!(ids(&page.items), ["ab12.s001", "ab12.s002"]);
}

#[test]
fn two_refs_are_alternatives_and_compose_with_type_status_container_label_and_limit() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    // Stations of p001 OR at c001: s001, s002, s003; s004 is deleted.
    let q = stations()
        .referencing("person", P1)
        .referencing("company", C1);
    assert_eq!(
        sorted(ids(&e.list_query(&q).unwrap().items)),
        ["ab12.s001", "ab12.s002", "ab12.s003"]
    );
    // A relationship read from both ends is the same shape: p002 as participant OR as person.
    let q = ListQuery::new()
        .referencing("participants", P2)
        .referencing("person", P2);
    assert_eq!(
        ids(&e.list_query(&q).unwrap().items),
        ["ab12.m001", "ab12.m003", "ab12.s003"]
    );
    let q = ListQuery::new()
        .referencing("participants", P1)
        .status("closed");
    assert_eq!(ids(&e.list_query(&q).unwrap().items), ["ab12.m002"]);
    // The container and the label filters narrow the same rows.
    let q = stations()
        .referencing("company", C1)
        .filter(NextFilter::new().with_type("station").with_container(P2));
    assert_eq!(
        ids(&e.list_query(&q).unwrap().items),
        ["ab12.s001", "ab12.s003"]
    );
    writer(tmp.path()).add_label("ab12.s003", "vip", "t");
    let q = stations()
        .referencing("company", C1)
        .filter(NextFilter::new().with_label("vip"));
    assert_eq!(ids(&e.list_query(&q).unwrap().items), ["ab12.s003"]);
    // A limit cuts and discloses the filtered total.
    let p = e
        .list_query(&ListQuery::new().referencing("participants", P1).limit(1))
        .unwrap();
    assert_eq!((ids(&p.items), p.total), (vec!["ab12.m001".to_string()], 3));
}

#[test]
fn a_ranked_ref_list_keeps_the_rank_scope() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    // Unfiltered ranked stations: parent rank first (Ada < Zed), then title → s002 (B, Ada),
    // s001 (A, Zed), s003 (C, Zed); the 129-byte station has no parent and comes last.
    let all = stations().sort(SortKey::Rank);
    assert_eq!(
        ids(&e.list_query(&all).unwrap().items),
        [
            "ab12.s002".to_string(),
            "ab12.s001".to_string(),
            "ab12.s003".to_string(),
            long_station()
        ]
    );
    // Filtered to person = p001: the same relative order. A filter that narrowed the SCOPE would
    // drop the persons from it, null every parent rank and fall back to title: s001, s002.
    let q = all.clone().referencing("person", P1);
    assert_eq!(
        ids(&e.list_query(&q).unwrap().items),
        ["ab12.s002", "ab12.s001"],
        "and never the 129-byte source"
    );
}

#[test]
fn more_than_max_refs_is_validation_and_exactly_max_refs_is_not() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    assert_eq!(read::MAX_REFS, 8);
    let mut q = ListQuery::new();
    for i in 0..read::MAX_REFS {
        q = q.referencing("participants", format!("ab12.none{i}"));
    }
    assert!(e.list_query(&q).unwrap().items.is_empty());
    let q = q.referencing("participants", "ab12.one-too-many");
    let err = e.list_query(&q).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("`--ref` takes at most 8 filters"),
        "{}",
        err.msg
    );
}

#[test]
fn a_ref_filter_on_a_non_ref_field_or_a_non_id_value_is_validation_and_an_unheld_id_is_empty() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    let err = e
        .list_query(&ListQuery::new().referencing("note", P1))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains(
            "`--ref` names 'note', which is not a ref field of this plugin; ref fields: company, \
             participants, person"
        ),
        "{}",
        err.msg
    );
    let err = e
        .list_query(&ListQuery::new().referencing("nosuch", P1))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    let err = e
        .list_query(&ListQuery::new().referencing("participants", "nodot"))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains(
            "`--ref participants`: a ref is a full item id '<prefix>.<suffix>' (got 'nodot')"
        ),
        "{}",
        err.msg
    );
    let page = e
        .list_query(&ListQuery::new().referencing("participants", "ab12.none"))
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.total, 0);
    // The target need not exist: a deleted target's referrers stay findable.
    writer(tmp.path()).delete_item(P2, "t");
    let page = e
        .list_query(&ListQuery::new().referencing("participants", P2))
        .unwrap();
    assert_eq!(ids(&page.items), ["ab12.m001", "ab12.m003"]);
}

#[test]
fn a_page_token_restarts_when_a_held_referrer_changes_and_is_bound_to_its_refs() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    let q = || ListQuery::new().referencing("participants", P1).limit(1);
    let p1 = e.list_query(&q().paginate()).unwrap();
    assert_eq!(ids(&p1.items), ["ab12.m001"]);
    let t = p1.next_token.unwrap();
    // A token used under other refs restarts: the key names them.
    let other = e
        .list_query(
            &ListQuery::new()
                .referencing("participants", P2)
                .limit(1)
                .token(&t),
        )
        .unwrap();
    assert!(other.restarted, "another ref is another list");
    // The same refs spelled in another order and repeated are the same list.
    let mut qq = ListQuery::new()
        .referencing("person", P1)
        .referencing("participants", P1);
    let t2 = e
        .list_query(&qq.clone().limit(1).paginate())
        .unwrap()
        .next_token
        .unwrap();
    qq = ListQuery::new()
        .referencing("participants", P1)
        .referencing("person", P1)
        .referencing("person", P1);
    assert!(!e.list_query(&qq.limit(1).token(&t2)).unwrap().restarted);
    // A new referrer (m003 gains p001) is not in the snapshot: the pager keeps on, as for a new item.
    e.field_entry_add(NOW, ACTOR, "ab12.m003", "participants", P1, None)
        .unwrap();
    let p2 = e.list_query(&q().token(&t)).unwrap();
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), ["ab12.m002"]);
    // A held referrer dropping its ref is an op on that item: the token restarts.
    let t = p2.next_token.unwrap();
    let mut s = writer(tmp.path());
    s.remove_field_entry("ab12.m006", "participants", P1, "t");
    let p3 = e.list_query(&q().token(&t)).unwrap();
    assert!(p3.restarted, "a held referrer changed");
    assert_eq!(
        (ids(&p3.items), p3.total),
        (vec!["ab12.m001".to_string()], 3)
    );
}

#[test]
fn list_query_value_json_is_the_list_record_with_custom_and_the_envelope() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    let v = e
        .list_query_value(
            &ListQuery::new()
                .referencing("participants", P1)
                .limit(1)
                .paginate(),
        )
        .unwrap();
    assert_eq!(v["total"], 3);
    assert_eq!(v["restarted"], false);
    assert!(v["next_token"].is_string());
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "ab12.m001");
    assert_eq!(
        items[0]["custom"]["participants"],
        serde_json::json!([{ "value": P1 }, { "value": P2 }])
    );
    // The record is `list_value`'s.
    let listed = e.list_value(None, None).unwrap();
    let same = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "ab12.m001")
        .unwrap();
    assert_eq!(&items[0], same);
}

#[test]
fn the_store_board_answers_the_same_page_as_the_sql_path_and_narrows_only_when_unranked() {
    let tmp = workspace("list-ref-fixture");
    let e = ref_board(&tmp);
    let cfg = plugin::load("list-ref-fixture").unwrap();
    let s = writer(tmp.path());
    let unranked = stations().referencing("person", P1);
    let ranked = unranked.clone().sort(SortKey::Rank);
    let participants = ListQuery::new().referencing("participants", P1);
    for q in [&unranked, &ranked, &participants] {
        let reads = read::list_reads(&cfg, q);
        assert_eq!(reads.refs, q.refs, "the reads carry the refs");
        if let Some(select) = &reads.select {
            assert_eq!(select.refs, q.refs);
            assert!(select.reads_refs());
        }
        let board = read::list_board(&s, &reads).unwrap();
        let via_board = read::list_board_page(&cfg, &board, q).unwrap();
        let via_sql = e.list_query(q).unwrap();
        assert_eq!(ids(&via_board.items), ids(&via_sql.items), "{q:?}");
        assert_eq!(via_board.total, via_sql.total, "{q:?}");
        assert!(!ids(&via_board.items).contains(&long_station()));
    }
    // Unranked: the board holds the candidates only, cut by `list_select`.
    let board = read::list_board(&s, &read::list_reads(&cfg, &unranked)).unwrap();
    assert_eq!(
        board.items.keys().cloned().collect::<Vec<_>>(),
        ["ab12.s001", "ab12.s002"]
    );
    // Ranked: the whole station scope, the 129-byte station included — and still in no page.
    let board = read::list_board(&s, &read::list_reads(&cfg, &ranked)).unwrap();
    assert_eq!(
        sorted(board.items.keys().cloned().collect()),
        sorted(vec![
            "ab12.s001".to_string(),
            "ab12.s002".to_string(),
            "ab12.s003".to_string(),
            long_station()
        ])
    );
}

#[test]
fn ref_filter_keeps_is_the_reference_rule_in_memory() {
    use nexus_flow_facade::FieldEntry;
    use std::collections::BTreeMap;
    let refs = [read::RefFilter::new("participants", P1)];
    let custom = BTreeMap::new();
    let mut lists = BTreeMap::new();
    lists.insert(
        "participants".to_string(),
        vec![FieldEntry {
            value: P1.to_string(),
            label: None,
        }],
    );
    assert!(read::ref_filter_keeps(
        &[],
        "ab12.m001",
        &custom,
        &BTreeMap::new()
    ));
    assert!(read::ref_filter_keeps(&refs, "ab12.m001", &custom, &lists));
    assert!(!read::ref_filter_keeps(
        &refs,
        &long_station(),
        &custom,
        &lists
    ));
    let mut single = BTreeMap::new();
    single.insert("person".to_string(), P1.to_string());
    let person = [read::RefFilter::new("person", P1)];
    assert!(read::ref_filter_keeps(
        &person,
        "ab12.s001",
        &single,
        &BTreeMap::new()
    ));
    assert!(!read::ref_filter_keeps(
        &person,
        "ab12.s001",
        &BTreeMap::new(),
        &lists
    ));
}
