//! The server fold answers as the local one (6j6v.vvw6 points 1, 4 and 5, 6j6v.k7w7).
//!
//! Two replicas write generated boards — tickets in every status, archived and deleted ones, defer
//! dates, dependencies and parents with removes, labels, notes and redactions, custom fields,
//! thread links — and merge. The merged log is then folded twice: locally, through the SQLite
//! applier, in the order the replica took it in; and through this crate into a [`MemTable`], in a
//! SHUFFLED order with ops delivered twice, as a relay without deduplication hands them over. Every
//! row of every view table, every lane of the library and both dated lanes must come out the same,
//! and the incrementally kept indexes must be exactly what a full reindex computes.

use nexus_flow_core::graph;
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nexus_flow_core::task_reducer::TaskReducer;
use nxs_fold_ddb::board;
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::table::{text, Dated};
use nxs_foundation::reducer::Reducer;
use std::collections::BTreeMap;

mod common;
use common::{merged, serve, served_rows, sqlite_rows, NOW};

fn dated_ids(t: &MemTable, lane: Dated) -> Vec<String> {
    ready(board::dated(t, lane, None))
        .unwrap()
        .iter()
        .map(|r| text(r, "id").unwrap().to_string())
        .collect()
}

fn sql_ids(s: &Store, sql: &str) -> Vec<String> {
    let conn = s.connection();
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn the_server_fold_in_any_order_answers_as_the_local_fold() {
    let folder = Folder::platform().unwrap();
    let mut active_seen = 0;
    for seed in 0..120 {
        let s = merged(seed);
        let t = serve(&s, seed, &folder);
        let ctx = format!("seed {seed}");

        for table in TaskReducer.view_tables() {
            assert_eq!(
                served_rows(&t, &folder, table),
                sqlite_rows(&s, table),
                "{table}, {ctx}"
            );
        }

        let local = graph::select(s.connection()).unwrap().lanes(NOW);
        let served = ready(board::select(&t)).unwrap();
        assert_eq!(served.board.lanes(NOW), local, "lanes, {ctx}");
        active_seen += served.items.len();

        assert_eq!(
            dated_ids(&t, Dated::Closed),
            sql_ids(
                &s,
                "SELECT id FROM items WHERE COALESCE(deleted,'0')<>'1' AND archived IS NULL
                   AND status='closed'
                 ORDER BY CASE WHEN COALESCE(closed_at,'')='' THEN '0' ELSE closed_at END DESC, id"
            ),
            "closed lane, {ctx}"
        );
        assert_eq!(
            dated_ids(&t, Dated::Archived),
            sql_ids(
                &s,
                "SELECT id FROM items WHERE COALESCE(deleted,'0')<>'1' AND archived IS NOT NULL
                 ORDER BY CASE WHEN archived='' THEN '0' ELSE archived END DESC, id"
            ),
            "archived lane, {ctx}"
        );

        // The flags kept op by op are exactly what recomputing them from the rows gives.
        let kept = t.rows();
        ready(board::reindex(&t)).unwrap();
        let recomputed = t.rows();
        let differ: Vec<_> = recomputed
            .iter()
            .filter(|(sk, row)| kept.get(*sk) != Some(*row))
            .map(|(sk, row)| (sk, kept.get(sk), row))
            .collect();
        assert!(
            differ.is_empty(),
            "incremental index vs reindex, {ctx}: {differ:#?}"
        );
        assert_eq!(recomputed.len(), kept.len(), "{ctx}");
    }
    assert!(active_seen > 0, "the generated boards have active tickets");
}

#[test]
fn closing_a_ticket_moves_its_edges_out_of_the_active_index_and_reopening_brings_them_back() {
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "Work", "u");
    s.create_item("ab12.0002", "task", "Blocker", "u");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "u");
    s.set_field("ab12.0001", "status", Some("closed".into()), "u");
    s.set_field("ab12.0002", "status", Some("closed".into()), "u");
    let t = MemTable::new("stream-1");
    let fold_all = |t: &MemTable, s: &Store, from: usize| {
        let batch: Vec<(i64, _)> = s
            .export()
            .into_iter()
            .enumerate()
            .skip(from)
            .map(|(i, op)| (i as i64 + 1, op))
            .collect();
        ready(folder.fold_batch(t, &batch)).unwrap();
    };
    fold_all(&t, &s, 0);
    let active = ready(board::select(&t)).unwrap();
    assert!(active.items.is_empty());
    assert_eq!(
        ready(t_rows_on_index(&t)),
        0,
        "nothing is on the active index"
    );

    // Reopening the blocker flips it, and its edge comes back with it: the work is blocked again.
    let before = t.requests();
    let seen = s.export().len();
    s.set_field("ab12.0002", "status", Some("open".into()), "u");
    s.set_field("ab12.0001", "status", Some("open".into()), "u");
    fold_all(&t, &s, seen);
    let lanes = ready(board::select(&t)).unwrap().board.lanes(NOW);
    assert_eq!(lanes.blocked, vec!["ab12.0001"]);
    assert_eq!(lanes.ready, vec!["ab12.0002"]);
    let cost = t.requests();
    // Two field ops, each three changes; two flips, each one adjacency Query; the watermark.
    assert_eq!(cost.queries - before.queries, 2, "{cost:?}");
    assert_eq!(cost.index_queries - before.index_queries, 1);
}

async fn t_rows_on_index(t: &MemTable) -> usize {
    use nxs_fold_ddb::table::Table;
    t.query_active().await.unwrap().len()
}

#[test]
fn a_snapshot_is_tables_plus_watermark_and_a_stream_started_from_it_folds_on_as_any_other() {
    let folder = Folder::platform().unwrap();
    let s = merged(7);
    let ops = s.export();
    let half = ops.len() / 2;
    let numbered: Vec<(i64, _)> = ops
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();

    // The source folded the first half and moved its watermark, then folded a few ops more before
    // the export read its tables: the snapshot holds them under the older watermark, and the
    // stream started from it folds them again.
    let source = MemTable::new("stream-1");
    ready(folder.fold_batch(&source, &numbered[..half])).unwrap();
    for (_, op) in &numbered[half..half + 3] {
        ready(folder.fold(&source, op)).unwrap();
    }
    let snapshot = ready(nxs_fold_ddb::snapshot::export(&source, &folder))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.folded_through, half as i64);
    assert!(!snapshot
        .tables
        .values()
        .flatten()
        .any(|r| r.contains_key("nxf_active")));

    let started = MemTable::new("stream-1");
    ready(nxs_fold_ddb::snapshot::import(&started, &folder, &snapshot)).unwrap();
    // The log comes from the relay, in batches from the watermark on.
    for chunk in numbered[half..].chunks(5) {
        ready(folder.fold_batch(&started, chunk)).unwrap();
    }
    let whole = MemTable::new("stream-1");
    ready(folder.fold_batch(&whole, &numbered)).unwrap();
    assert_eq!(started.rows(), whole.rows());

    // A second import into a started stream is refused, and so is one of other rules.
    assert!(ready(nxs_fold_ddb::snapshot::import(&started, &folder, &snapshot)).is_err());
    let mut other = snapshot.clone();
    other.revisions.insert("task".into(), 99);
    assert!(ready(nxs_fold_ddb::snapshot::import(
        &MemTable::new("stream-1"),
        &folder,
        &other
    ))
    .is_err());
}

#[test]
fn a_snapshot_of_a_local_replica_starts_a_server_stream_with_the_same_rows() {
    let folder = Folder::platform().unwrap();
    let s = merged(11);
    let snapshot =
        nxs_fold_ddb::snapshot::from_sqlite(s.connection(), &folder, "stream-1", 40).unwrap();
    let t = MemTable::new("stream-1");
    ready(nxs_fold_ddb::snapshot::import(&t, &folder, &snapshot)).unwrap();
    for table in TaskReducer.view_tables() {
        assert_eq!(
            served_rows(&t, &folder, table),
            sqlite_rows(&s, table),
            "{table}"
        );
    }
    let local = graph::select(s.connection()).unwrap().lanes(NOW);
    assert_eq!(ready(board::select(&t)).unwrap().board.lanes(NOW), local);
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert_eq!(mark.folded_through, 40);
    assert!(folder.is_current(&mark));
}

#[test]
fn the_watermark_only_moves_forward() {
    let folder = Folder::platform().unwrap();
    let t = MemTable::new("stream-1");
    ready(folder.advance(&t, 10)).unwrap();
    ready(folder.advance(&t, 4)).unwrap();
    assert_eq!(
        ready(Folder::watermark(&t))
            .unwrap()
            .unwrap()
            .folded_through,
        10
    );
    let rows: BTreeMap<_, _> = t.rows();
    assert_eq!(rows.len(), 1);
    ready(Folder::clear(&t)).unwrap();
    assert!(t.rows().is_empty());
}

#[test]
fn a_flip_costs_one_query_and_three_reads_per_edge_and_no_more() {
    let folder = Folder::platform().unwrap();
    let n = 40u64;
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0000", "task", "Blocker", "u");
    for i in 1..=n {
        let id = format!("ab12.{i:04}");
        s.create_item(&id, "task", "Dependent", "u");
        s.add_edge(&id, "ab12.0000", EdgeKind::Dep, "u");
    }
    let t = MemTable::new("stream-1");
    let all = |s: &Store, from: usize| -> Vec<(i64, _)> {
        s.export()
            .into_iter()
            .enumerate()
            .skip(from)
            .map(|(i, op)| (i as i64 + 1, op))
            .collect()
    };
    ready(folder.fold_batch(&t, &all(&s, 0))).unwrap();
    let seen = s.export().len();
    s.set_field("ab12.0000", "status", Some("closed".into()), "u");
    let before = t.requests();
    ready(folder.fold_batch(&t, &all(&s, seen))).unwrap();
    let after = t.requests();
    // Three changes, the ticket's index write, the watermark; one read for the ticket and three
    // per edge; one adjacency Query. Nothing grows with the board beyond the closed ticket's edges.
    assert_eq!(after.writes - before.writes, 5);
    assert_eq!(after.gets - before.gets, 1 + 3 * n);
    assert_eq!(after.queries - before.queries, 1);
    assert_eq!(
        ready(board::select(&t))
            .unwrap()
            .board
            .lanes(NOW)
            .ready
            .len(),
        n as usize
    );
}
