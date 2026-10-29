//! A server ranks by a custom-field key as a replica does (6j6v.z19s): `[ranking.list.note]` and
//! `[ranking.next.note]` order notes by their `occurred_on` newest first, nulls last, id breaking
//! ties. The folded tables' boards (`records::list_board`, `records::active_board`) carry each
//! item's own custom values, so the server ranks without a ranking graph — and gives the rows the
//! store path gives, in the same order.

use nexus_flow_core::model::{ItemRow, MergeStrategy};
use nexus_flow_core::store::Store;
use nexus_flow_facade::plugin::{self, PluginConfig, PluginRegistration};
use nexus_flow_facade::read::{self, ListQuery, NextFilter, SortKey};
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::{board, records};

mod common;
use common::{delivered, NOW};

const NOTES: &str = r#"
name = "custom-rank-server-fixture"
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
[ranking.next]
types = ["note", "task"]
order = [ { field = "priority", dir = "asc" } ]
[ranking.next.note]
order = [ { field = "occurred_on", dir = "desc", nulls = "last" } ]
[ranking.list]
types = ["note", "task"]
[ranking.list.note]
order = [ { field = "occurred_on", dir = "desc", nulls = "last" } ]
[presentation.list]
columns = ["id"]
"#;

inventory::submit!(PluginRegistration {
    name: "custom-rank-server-fixture",
    toml: NOTES,
    order: 150,
});

fn cfg() -> PluginConfig {
    plugin::load("custom-rank-server-fixture").expect("the fixture is a valid plugin")
}

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

/// Notes whose id order disagrees with their time order — a tie on `n002`/`n004`, `n003` without
/// a value, `n005` re-dated to the newest — and two tasks.
fn store() -> Store {
    let mut s = Store::open_in_memory(1);
    for id in ["n001", "n002", "n003", "n004", "n005", "t001", "t002"] {
        let ty = if id.starts_with('n') { "note" } else { "task" };
        s.create_item(&format!("ab12.{id}"), ty, id, "u");
    }
    for (id, at) in [
        ("n001", "2026-03-01T10:00:00Z"),
        ("n002", "2026-05-01T00:00:00Z"),
        ("n004", "2026-05-01T00:00:00Z"),
        ("n005", "2026-01-15T08:30:00Z"),
        ("n005", "2026-09-30T23:59:59Z"),
    ] {
        let id = format!("ab12.{id}");
        s.set_custom_field_merge(&id, "occurred_on", Some(at.into()), "u", MergeStrategy::Lww);
    }
    s
}

const NOTES_NEWEST_FIRST: [&str; 5] = [
    "ab12.n005",
    "ab12.n002",
    "ab12.n004",
    "ab12.n001",
    "ab12.n003",
];

#[test]
fn the_server_lists_notes_newest_first_as_the_replica_does_without_a_ranking_graph() {
    let cfg = cfg();
    let s = store();
    let t = MemTable::new("stream-1");
    ready(
        Folder::platform()
            .unwrap()
            .fold_batch(&t, &delivered(&s, 1)),
    )
    .unwrap();
    for q in [
        ListQuery::new()
            .sort(SortKey::Rank)
            .filter(NextFilter::new().with_type("note")),
        ListQuery::new().sort(SortKey::Rank),
    ] {
        let reads = read::list_reads(&cfg, &q);
        assert_eq!(reads.ranking, None, "no graph to read: {q:?}");
        let server = ready(records::list_board(&t, &reads)).unwrap();
        assert!(server.ranking.is_none());
        let via_server = read::list_board_page(&cfg, &server, &q).unwrap();
        let via_store = read::list_page(&cfg, &s, &q).unwrap();
        assert_eq!(ids(&via_server.items), ids(&via_store.items), "{q:?}");
        assert_eq!(via_server.total, via_store.total, "{q:?}");
    }
    let q = ListQuery::new()
        .sort(SortKey::Rank)
        .filter(NextFilter::new().with_type("note"));
    let server = ready(records::list_board(&t, &read::list_reads(&cfg, &q))).unwrap();
    assert_eq!(
        ids(&read::list_board_page(&cfg, &server, &q).unwrap().items),
        NOTES_NEWEST_FIRST
    );
}

#[test]
fn the_server_ranks_next_by_the_custom_field_as_the_replica_does_without_a_ranking_graph() {
    let cfg = cfg();
    let s = store();
    let t = MemTable::new("stream-1");
    ready(
        Folder::platform()
            .unwrap()
            .fold_batch(&t, &delivered(&s, 1)),
    )
    .unwrap();
    assert_eq!(read::next_ranking_reads(&cfg).unwrap(), None);
    let selection = ready(board::select(&t)).unwrap();
    let server = ready(records::active_board(&t, selection)).unwrap();
    let via_server = read::next_active(&cfg, &server, NOW).unwrap();
    let via_store = read::next(&cfg, &s, NOW, None).unwrap();
    assert_eq!(ids(&via_server), ids(&via_store));
    let notes: Vec<String> = ids(&via_server)
        .into_iter()
        .filter(|id| id.contains(".n"))
        .collect();
    assert_eq!(notes, NOTES_NEWEST_FIRST);
}
