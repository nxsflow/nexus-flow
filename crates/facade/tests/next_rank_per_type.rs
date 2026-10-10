//! Per-type `next` orders (6j6v.z9jk): `[ranking.next.<type>]` replaces the default
//! `[ranking.next]` order for one item type, a type without one falls back to the default, and
//! `types = [...]` says how items of different types stand against each other. Tested at the seam
//! the products take: `Engine::next`, the store-free `read::next_active`, and the paging of
//! `read::next_page`. Its own test binary, so the fixture plugins do not pollute the facade lib
//! tests' "exactly two OSS plugins" assertion (mirrors `external_plugin_registration.rs`).

use nexus_flow_core::model::ItemRow;
use nexus_flow_core::store::Store;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::plugin::{self, PluginRegistration};
use nexus_flow_facade::read::{self, NextQuery};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";

/// The fixture: three types. `action` ranks by due date, `project` by priority DESCENDING (the
/// reverse of the default), and `note` has no order of its own, so it takes the default (priority
/// ascending). Across types: actions, then projects, then the unlisted `note`.
const PER_TYPE: &str = r#"
name = "per-type-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["project", "action", "note"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
types = ["action", "project"]
order = [
    { field = "status", precedence = ["in_progress", "open"] },
    { field = "priority", dir = "asc" },
]
[ranking.next.action]
order = [ { field = "due", dir = "asc", nulls = "last" } ]
[ranking.next.project]
order = [ { field = "priority", dir = "desc" } ]
[presentation.list]
columns = ["id"]
"#;

inventory::submit!(PluginRegistration {
    name: "per-type-rank-fixture",
    toml: PER_TYPE,
    order: 110,
});

// The same types under ONE default order — the plugin a workspace might switch to. It ranks the
// fixture's board differently, so a token handed out under the fixture must not page through it.
inventory::submit!(PluginRegistration {
    name: "flat-rank-fixture",
    toml: r#"
name = "flat-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["project", "action", "note"]
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
    order: 111,
});

fn workspace(plugin: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), plugin).unwrap();
    tmp
}

fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

fn item(s: &mut Store, id: &str, ty: &str, priority: &str, due: Option<&str>) {
    s.create_item(id, ty, id, "t");
    s.set_field(id, "priority", Some(priority.into()), "t");
    if let Some(due) = due {
        s.set_field(id, "due", Some(due.into()), "t");
    }
}

/// Six ready items, two per type, each pair seeded so its per-type order DISAGREES with the
/// default (priority ascending, then id):
///
/// - actions: `a001` P0 due Sep, `a002` P3 due Jul → by due: a002, a001 (default: a001, a002);
/// - projects: `p001` P0, `p002` P3 → by priority desc: p002, p001 (default: p001, p002);
/// - notes: `n001` P2, `n002` P1 → the default: n002, n001 (an id order would be n001, n002).
fn board(dir: &Path) {
    let mut s = writer(dir);
    item(&mut s, "zz12.a001", "action", "0", Some("2026-09-01"));
    item(&mut s, "zz12.a002", "action", "3", Some("2026-07-01"));
    item(&mut s, "zz12.p001", "project", "0", None);
    item(&mut s, "zz12.p002", "project", "3", None);
    item(&mut s, "zz12.n001", "note", "2", None);
    item(&mut s, "zz12.n002", "note", "1", None);
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

#[test]
fn a_per_type_order_ranks_the_items_of_its_type() {
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let next = Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap();
    assert_eq!(
        of_type(&next, "action"),
        ["zz12.a002", "zz12.a001"],
        "actions by due date, not by priority"
    );
    assert_eq!(
        of_type(&next, "project"),
        ["zz12.p002", "zz12.p001"],
        "projects by priority descending"
    );
}

#[test]
fn a_type_without_its_own_order_falls_back_to_the_default() {
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let next = Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap();
    assert_eq!(
        of_type(&next, "note"),
        ["zz12.n002", "zz12.n001"],
        "notes by the default order (priority ascending), not by id"
    );
}

#[test]
fn items_of_different_types_compare_by_the_declared_type_order() {
    // The whole list: the type order first (action, project, then the unlisted note), each type by
    // its own order. Without per-type orders this board ranks by priority across types.
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let next = Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap();
    assert_eq!(
        ids(&next),
        [
            "zz12.a002",
            "zz12.a001",
            "zz12.p002",
            "zz12.p001",
            "zz12.n002",
            "zz12.n001"
        ]
    );
}

#[test]
fn the_finish_first_tiers_still_come_before_the_per_type_order() {
    // A claimed note with no children is Tier 1 (closeable now). The note type ranks LAST across
    // types, yet the tier puts it first: per-type orders decide order within a tier only.
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let mut s = writer(tmp.path());
    s.set_field("zz12.n001", "status", Some("in_progress".into()), "t");
    let next = Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap();
    assert_eq!(
        ids(&next),
        [
            "zz12.n001",
            "zz12.a002",
            "zz12.a001",
            "zz12.p002",
            "zz12.p001",
            "zz12.n002"
        ]
    );
}

#[test]
fn the_store_free_path_ranks_by_the_same_per_type_orders() {
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let s = writer(tmp.path());
    let cfg = plugin::load("per-type-rank-fixture").unwrap();
    let store_free = read::next_active(&cfg, &read::active_board(&s).unwrap(), NOW).unwrap();
    assert_eq!(
        ids(&store_free),
        ids(&Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap())
    );
    assert_eq!(
        of_type(&store_free, "action"),
        ["zz12.a002", "zz12.a001"],
        "the store-free path applies the per-type order itself"
    );
}

#[test]
fn a_token_from_another_plugin_order_restarts_instead_of_paging_on() {
    // A page under the per-type fixture, then the same token under the flat plugin, which ranks
    // this board differently: the reader gets the first page of the flat order, `restarted`.
    let tmp = workspace("per-type-rank-fixture");
    board(tmp.path());
    let s = writer(tmp.path());
    let per_type = plugin::load("per-type-rank-fixture").unwrap();
    let flat = plugin::load("flat-rank-fixture").unwrap();

    let p1 = read::next_page(&per_type, &s, &NextQuery::new(NOW).limit(2).paginate()).unwrap();
    assert_eq!(ids(&p1.items), ["zz12.a002", "zz12.a001"]);
    let token = p1.next_token.expect("rows remain");

    let under_flat =
        read::next_page(&flat, &s, &NextQuery::new(NOW).limit(2).token(&token)).unwrap();
    assert!(under_flat.restarted, "a token from another order restarts");
    assert_eq!(under_flat.start, 0);
    let fresh_flat = read::next(&flat, &s, NOW, None).unwrap();
    assert_eq!(ids(&under_flat.items), ids(&fresh_flat)[..2].to_vec());

    // The same token under the order it came from still pages on.
    let p2 = read::next_page(&per_type, &s, &NextQuery::new(NOW).limit(2).token(&token)).unwrap();
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), ["zz12.p002", "zz12.p001"]);
}

#[test]
fn the_bundled_plugins_rank_next_exactly_as_before() {
    // issue-tracker: status → priority → type → due → id. Priority beats type, so a P0 chore leads
    // and the P2 epic trails; a type-first order would put the epic first.
    let tmp = workspace("issue-tracker");
    {
        let mut s = writer(tmp.path());
        item(&mut s, "it12.0001", "epic", "2", None);
        item(&mut s, "it12.0002", "feature", "1", None);
        item(&mut s, "it12.0003", "bug", "1", None);
        item(&mut s, "it12.0004", "chore", "0", None);
    }
    assert_eq!(
        ids(&Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap()),
        ["it12.0004", "it12.0003", "it12.0002", "it12.0001"]
    );

    // personal-todo: status → priority → id, no type key. A `now` todo leads a `soon` project.
    let tmp = workspace("personal-todo");
    {
        let mut s = writer(tmp.path());
        item(&mut s, "pt12.0001", "project", "1", None);
        item(&mut s, "pt12.0002", "todo", "0", None);
        item(&mut s, "pt12.0003", "termin", "1", None);
    }
    assert_eq!(
        ids(&Engine::open(None, tmp.path()).unwrap().next(NOW).unwrap()),
        ["pt12.0002", "pt12.0001", "pt12.0003"]
    );
}
