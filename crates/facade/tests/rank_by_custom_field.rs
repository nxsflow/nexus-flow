//! A per-type ranking order ranks by a plugin-declared custom field as a plain field key
//! (6j6v.z19s): `[ranking.list.note] order = [ { field = "occurred_on", dir = "desc", nulls =
//! "last" } ]` lists notes newest first, nulls last, id breaking ties. The value is the item's own
//! custom value under the field key's compare rule — numeric when both values are integers,
//! lexical otherwise — so a `datetime` field, normalised on write to `YYYY-MM-DDTHH:MM:SSZ`, ranks
//! in time order. Held at the Engine seam (`list_query`, `next_query`) and on the store-free path
//! (`list_board_page`, `next_active`) over a board a server builds without any ranking graph. Its
//! own test binary, so the fixture plugin does not pollute the facade lib tests' plugin set.

use nexus_flow_core::model::ItemRow;
use nexus_flow_core::store::Store;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::plugin::{self, PluginRegistration};
use nexus_flow_facade::read::{self, ListQuery, NextFilter, NextQuery, SortKey};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";
const ACTOR: &str = "alice";

/// Notes rank by `occurred_on` newest first, nulls last, in both sections; tasks by the text
/// field `seq` ascending — nulls last under `list`, first under `next` — so the numeric and the
/// lexical half of the compare rule and the per-key null placement all decide somewhere.
const CUSTOM_RANKED: &str = r#"
name = "custom-rank-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3"]
[types]
list = ["note", "task"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[fields.occurred_on]
type = "datetime"
on = ["note"]
[fields.seq]
type = "text"
on = ["task"]
[ranking.next]
types = ["note", "task"]
order = [ { field = "priority", dir = "asc" } ]
[ranking.next.note]
order = [ { field = "occurred_on", dir = "desc", nulls = "last" } ]
[ranking.next.task]
order = [ { field = "seq", dir = "asc", nulls = "first" } ]
[ranking.list]
types = ["note", "task"]
[ranking.list.note]
order = [ { field = "occurred_on", dir = "desc", nulls = "last" } ]
[ranking.list.task]
order = [ { field = "seq", dir = "asc" } ]
[presentation.list]
columns = ["id"]
"#;

inventory::submit!(PluginRegistration {
    name: "custom-rank-fixture",
    toml: CUSTOM_RANKED,
    order: 130,
});

fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

/// The board. Every id order disagrees with the ranked one:
///
/// - notes: `n001` 2026-03-01T10:00Z, `n002` and `n004` 2026-05-01T00:00Z (a tie, id breaks it),
///   `n003` without a value, `n005` 2026-01-15T08:30Z, `n006` 2026-04-01T12:00+02:00 (stored as
///   10:00Z);
/// - tasks: `t001` seq "10", `t002` "9" (numeric: 9 before 10, though "10" < "9" bytewise),
///   `t003` without a value, `t004` "abc" (text after every number).
fn board(tmp: &TempDir) -> Engine {
    let mut s = writer(tmp.path());
    for (id, ty) in [
        ("zz12.n001", "note"),
        ("zz12.n002", "note"),
        ("zz12.n003", "note"),
        ("zz12.n004", "note"),
        ("zz12.n005", "note"),
        ("zz12.n006", "note"),
        ("zz12.t001", "task"),
        ("zz12.t002", "task"),
        ("zz12.t003", "task"),
        ("zz12.t004", "task"),
    ] {
        s.create_item(id, ty, id, "t");
    }
    drop(s);
    let e = Engine::open(None, tmp.path()).unwrap();
    for (id, set) in [
        ("zz12.n001", "occurred_on=2026-03-01T10:00:00Z"),
        ("zz12.n002", "occurred_on=2026-05-01T00:00:00Z"),
        ("zz12.n004", "occurred_on=2026-05-01T00:00:00Z"),
        ("zz12.n005", "occurred_on=2026-01-15T08:30:00Z"),
        ("zz12.n006", "occurred_on=2026-04-01T12:00:00+02:00"),
        ("zz12.t001", "seq=10"),
        ("zz12.t002", "seq=9"),
        ("zz12.t004", "seq=abc"),
    ] {
        e.update(NOW, ACTOR, id, &[set.to_string()]).unwrap();
    }
    e
}

/// Notes newest first, the tie by id, the note without a value last.
const NOTES_NEWEST_FIRST: [&str; 6] = [
    "zz12.n002",
    "zz12.n004",
    "zz12.n006",
    "zz12.n001",
    "zz12.n005",
    "zz12.n003",
];

/// Tasks under `[ranking.list.task]`: numbers ascending, then text, then the null.
const TASKS_LIST: [&str; 4] = ["zz12.t002", "zz12.t001", "zz12.t004", "zz12.t003"];

/// Tasks under `[ranking.next.task]`: the null first, then numbers ascending, then text.
const TASKS_NEXT: [&str; 4] = ["zz12.t003", "zz12.t002", "zz12.t001", "zz12.t004"];

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn of_type(ty: &str) -> NextFilter {
    NextFilter::new().with_type(ty)
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "custom-rank-fixture").unwrap();
    tmp
}

#[test]
fn a_datetime_value_is_stored_normalised_so_lexical_order_is_time_order() {
    // The premise of the ticket, held where the ranking reads it: the offset form is stored UTC.
    let tmp = workspace();
    let _e = board(&tmp);
    let s = writer(tmp.path());
    let custom = s.custom_fields_of_bulk(&["zz12.n006"]).unwrap();
    assert_eq!(
        custom["zz12.n006"]["occurred_on"], "2026-04-01T10:00:00Z",
        "{custom:?}"
    );
}

#[test]
fn engine_list_query_ranks_notes_newest_first_with_nulls_last_and_id_breaking_ties() {
    let tmp = workspace();
    let e = board(&tmp);
    let q = ListQuery::new().sort(SortKey::Rank).filter(of_type("note"));
    assert_eq!(ids(&e.list_query(&q).unwrap().items), NOTES_NEWEST_FIRST);
    let q = ListQuery::new().sort(SortKey::Rank).filter(of_type("task"));
    assert_eq!(ids(&e.list_query(&q).unwrap().items), TASKS_LIST);
    // The whole ranked list: the cross-type order, each type by its own custom key.
    let all: Vec<&str> = NOTES_NEWEST_FIRST
        .iter()
        .chain(&TASKS_LIST)
        .copied()
        .collect();
    let q = ListQuery::new().sort(SortKey::Rank);
    assert_eq!(ids(&e.list_query(&q).unwrap().items), all);
}

#[test]
fn engine_next_query_ranks_by_the_custom_field_with_the_null_placement_of_its_key() {
    let tmp = workspace();
    let e = board(&tmp);
    let q = NextQuery::new(NOW).filter(of_type("note"));
    assert_eq!(ids(&e.next_query(&q).unwrap().items), NOTES_NEWEST_FIRST);
    let q = NextQuery::new(NOW).filter(of_type("task"));
    assert_eq!(ids(&e.next_query(&q).unwrap().items), TASKS_NEXT);
}

#[test]
fn the_store_free_list_ranks_as_the_store_path_without_any_ranking_graph() {
    let tmp = workspace();
    let e = board(&tmp);
    let cfg = plugin::load("custom-rank-fixture").unwrap();
    let s = writer(tmp.path());
    for q in [
        ListQuery::new().sort(SortKey::Rank),
        ListQuery::new().sort(SortKey::Rank).filter(of_type("note")),
        ListQuery::new().sort(SortKey::Rank).filter(of_type("task")),
    ] {
        let reads = read::list_reads(&cfg, &q);
        // An item's own custom values are on every `ListItem` a reader builds: no graph to read.
        assert_eq!(reads.ranking, None, "{q:?}");
        assert!(!reads.next, "{q:?}");
        let board = read::list_board(&s, &reads).unwrap();
        let via_board = read::list_board_page(&cfg, &board, &q).unwrap();
        let via_engine = e.list_query(&q).unwrap();
        assert_eq!(ids(&via_board.items), ids(&via_engine.items), "{q:?}");
        assert_eq!(via_board.total, via_engine.total, "{q:?}");
    }
    let q = ListQuery::new().sort(SortKey::Rank).filter(of_type("note"));
    let board = read::list_board(&s, &read::list_reads(&cfg, &q)).unwrap();
    assert_eq!(
        ids(&read::list_board_page(&cfg, &board, &q).unwrap().items),
        NOTES_NEWEST_FIRST
    );
}

#[test]
fn the_store_free_next_ranks_as_the_store_path_without_any_ranking_graph() {
    let tmp = workspace();
    let e = board(&tmp);
    let cfg = plugin::load("custom-rank-fixture").unwrap();
    let s = writer(tmp.path());
    // The board a server builds: its active tickets with their custom values, and no graph.
    let board = read::active_board(&s).unwrap();
    assert!(board.ranking.is_none());
    let via_board = read::next_active(&cfg, &board, NOW).unwrap();
    let via_engine = e.next_query(&NextQuery::new(NOW)).unwrap();
    assert_eq!(ids(&via_board), ids(&via_engine.items));
    let notes: Vec<String> = ids(&via_board)
        .into_iter()
        .filter(|id| id.contains(".n"))
        .collect();
    assert_eq!(notes, NOTES_NEWEST_FIRST);
}
