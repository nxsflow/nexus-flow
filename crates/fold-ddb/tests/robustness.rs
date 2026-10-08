//! What a server fold does when things go wrong (review of PR #24): a run that dies at any write
//! and folds its batch again, an op the table cannot hold, a clear that dies half way, a release
//! with other fold revisions, a lane cut by a limit, and a snapshot that does not fit.

mod common;

use common::{delivered, merged, NOW};
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nxs_fold_ddb::board;
use nxs_fold_ddb::fold::{Folded, Folder};
use nxs_fold_ddb::layout::WATERMARK_KEY;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::snapshot::{self, ImportError};
use nxs_fold_ddb::table::{text, Dated, Outcome, Row, Table};
use nxs_fold_ddb::write::{Cond, Write};
use nxs_foundation::change::Cell;
use nxs_foundation::model::Op;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Injected;

impl std::fmt::Display for Injected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "injected failure")
    }
}

impl std::error::Error for Injected {}

/// A [`MemTable`] whose `fail_at`-th write (or delete) fails, as a run that dies there would.
struct Failing<'a> {
    inner: &'a MemTable,
    fail_at: usize,
    seen: AtomicUsize,
}

impl<'a> Failing<'a> {
    fn new(inner: &'a MemTable, fail_at: usize) -> Failing<'a> {
        Failing {
            inner,
            fail_at,
            seen: AtomicUsize::new(0),
        }
    }

    fn tick(&self) -> Result<(), Injected> {
        if self.seen.fetch_add(1, Ordering::SeqCst) == self.fail_at {
            Err(Injected)
        } else {
            Ok(())
        }
    }
}

fn ok<T>(r: Result<T, std::convert::Infallible>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => match e {},
    }
}

impl Table for Failing<'_> {
    type Error = Injected;

    fn stream(&self) -> &str {
        self.inner.stream()
    }

    async fn get(&self, sk: &str) -> Result<Option<Row>, Injected> {
        Ok(ok(self.inner.get(sk).await))
    }

    async fn write(&self, write: &Write) -> Result<Outcome, Injected> {
        self.tick()?;
        Ok(ok(self.inner.write(write).await))
    }

    async fn delete(&self, sk: &str) -> Result<(), Injected> {
        self.tick()?;
        ok(self.inner.delete(sk).await);
        Ok(())
    }

    async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, Injected> {
        Ok(ok(self.inner.query_prefix(prefix).await))
    }

    async fn query_active(&self) -> Result<Vec<Row>, Injected> {
        Ok(ok(self.inner.query_active().await))
    }

    async fn query_dated(&self, lane: Dated, limit: Option<usize>) -> Result<Vec<Row>, Injected> {
        Ok(ok(self.inner.query_dated(lane, limit).await))
    }
}

/// Fold `first` cleanly, then `second` with a run that dies at each write in turn and folds
/// `second` again; every such stream must end as the unbroken one.
fn dies_anywhere_in_the_second_batch(first: &[(i64, Op)], second: &[(i64, Op)], ctx: &str) {
    let folder = Folder::platform().unwrap();
    let clean = MemTable::new("stream-1");
    ready(folder.fold_batch(&clean, first)).unwrap();
    let before = clean.requests().writes;
    ready(folder.fold_batch(&clean, second)).unwrap();
    let writes = (clean.requests().writes - before) as usize;
    assert!(writes > 0, "{ctx}");
    for fail_at in 0..writes {
        let t = MemTable::new("stream-1");
        ready(folder.fold_batch(&t, first)).unwrap();
        let mark = ready(Folder::watermark(&t)).unwrap();
        assert!(
            ready(folder.fold_batch(&Failing::new(&t, fail_at), second)).is_err(),
            "{ctx}: write {fail_at} fails"
        );
        assert_eq!(
            ready(Folder::watermark(&t)).unwrap(),
            mark,
            "{ctx}: a failed batch does not move the watermark"
        );
        ready(folder.fold_batch(&t, second)).unwrap();
        assert!(
            t.rows() == clean.rows(),
            "{ctx}: a run that died at write {fail_at} of the second batch and folded it again differs"
        );
    }
}

#[test]
fn a_run_that_dies_at_any_write_and_folds_its_batch_again_ends_where_an_unbroken_run_ends() {
    // The flip in its own batch, after the edge's: the replay has nothing but the flip to go on.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "Work", "u");
    s.create_item("ab12.0002", "task", "Blocker", "u");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "u");
    s.set_field("ab12.0002", "status", Some("closed".into()), "u");
    let first = numbered(&s, 0);
    s.set_field("ab12.0001", "status", Some("closed".into()), "u");
    s.set_field("ab12.0002", "status", Some("open".into()), "u");
    s.set_field("ab12.0002", "status", Some("closed".into()), "u");
    let second = numbered(&s, first.len());
    dies_anywhere_in_the_second_batch(&first, &second, "two tickets");

    for seed in [2, 9] {
        let ops = delivered(&merged(seed), seed);
        let (first, second) = ops.split_at(ops.len() / 2);
        dies_anywhere_in_the_second_batch(first, second, &format!("seed {seed}"));
    }
}

/// `s`'s log from op `from` on, numbered by position.
fn numbered(s: &Store, from: usize) -> Vec<(i64, Op)> {
    s.export()
        .into_iter()
        .enumerate()
        .skip(from)
        .map(|(i, op)| (i as i64 + 1, op))
        .collect()
}

#[test]
fn an_op_the_table_cannot_hold_is_refused_and_the_stream_folds_on() {
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    let long_id = format!("ab12.{}", "x".repeat(2000));
    s.create_item(&long_id, "task", "T", "u");
    s.create_item("ab12.0001", "task", "Fine", "u");
    s.set_field(
        "ab12.0001",
        "description",
        Some("d".repeat(500 * 1024)),
        "u",
    );
    s.create_item("ab12.0002", "task", "Grows", "u");
    s.set_field(
        "ab12.0002",
        "description",
        Some("d".repeat(250 * 1024)),
        "u",
    );
    s.set_field("ab12.0002", "design", Some("e".repeat(250 * 1024)), "u");
    s.add_edge(&long_id, "ab12.0001", EdgeKind::Dep, "u");
    let ops: Vec<(i64, _)> = s
        .export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    let t = MemTable::new("stream-1");
    let refused = ready(folder.fold_batch(&t, &ops)).unwrap();
    let reasons: Vec<&str> = refused.iter().map(|r| r.reason.as_str()).collect();
    // The long id's own three field ops and its edge (whose adjacency key is too long); the 500 KB
    // description; and the design that would grow ab12.0002 past the limit.
    assert_eq!(refused.len(), 6, "{reasons:#?}");
    assert_eq!(
        reasons.iter().filter(|r| r.contains("a key of")).count(),
        4,
        "{reasons:#?}"
    );
    assert!(reasons.iter().any(|r| r.contains("a row of")));
    assert!(reasons.iter().any(|r| r.contains("kept their state")));
    // The stream folded on: the watermark moved past every op, and what fits is there.
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert_eq!(mark.folded_through, ops.len() as i64);
    let rows = t.rows();
    let fine = &rows["items#ab12.0001"];
    assert_eq!(text(fine, "title"), Some("Fine"));
    assert!(fine.get("description").is_none());
    let grows = &rows["items#ab12.0002"];
    assert_eq!(text(grows, "description").map(str::len), Some(250 * 1024));
    assert!(grows.get("design").is_none());
    let lanes = ready(board::select(&t)).unwrap().board.lanes(NOW);
    assert_eq!(lanes.ready, vec!["ab12.0001", "ab12.0002"]);
}

#[test]
fn a_clear_that_dies_keeps_the_watermark_and_a_release_with_other_rules_refolds_from_zero() {
    let folder = Folder::platform().unwrap();
    let s = merged(4);
    let ops = delivered(&s, 4);
    let clean = MemTable::new("stream-1");
    ready(folder.fold_batch(&clean, &ops)).unwrap();

    // The stream was folded by an older release: its watermark names other revisions.
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &ops)).unwrap();
    ready(t.write(&Write {
        sk: WATERMARK_KEY.into(),
        set: vec![("revision_task".into(), Cell::Int(0))],
        remove: Vec::new(),
        condition: Some(Cond::RowPresent),
    }))
    .unwrap();
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert!(!folder.is_current(&mark));

    // A clear that dies half way leaves the watermark — the next run sees the old revisions again.
    assert!(ready(Folder::clear(&Failing::new(&t, 3))).is_err());
    let after = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert!(!folder.is_current(&after));

    // Cleared for good and folded from position 0, it holds what a fresh fold holds.
    ready(Folder::clear(&t)).unwrap();
    assert!(t.rows().is_empty());
    ready(folder.fold_batch(&t, &ops)).unwrap();
    assert_eq!(t.rows(), clean.rows());
}

#[test]
fn a_dated_lane_cut_by_a_limit_is_the_head_of_the_whole_lane() {
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    // Eight closed tickets, four of them without a close date (they all sort as "0").
    for i in 0..8 {
        let id = format!("ab12.{i:04}");
        s.create_item(&id, "task", "T", "u");
        s.set_field(&id, "status", Some("closed".into()), "u");
        if i % 2 == 0 {
            s.set_field(
                &id,
                "closed_at",
                Some(format!("2026-09-0{}", 1 + i / 4)),
                "u",
            );
        }
    }
    let ops: Vec<(i64, _)> = s
        .export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &ops)).unwrap();
    let ids = |limit| -> Vec<String> {
        ready(board::dated(&t, Dated::Closed, limit))
            .unwrap()
            .iter()
            .map(|r| text(r, "id").unwrap().to_string())
            .collect()
    };
    let whole = ids(None);
    assert_eq!(
        whole,
        [
            "ab12.0006",
            "ab12.0004",
            "ab12.0002",
            "ab12.0000",
            "ab12.0007",
            "ab12.0005",
            "ab12.0003",
            "ab12.0001"
        ]
    );
    for n in 1..=whole.len() {
        assert_eq!(ids(Some(n)), whole[..n], "limit {n}");
    }
}

#[test]
fn archiving_unarchiving_and_deleting_move_a_ticket_and_its_edge_between_the_indexes() {
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "Work", "u");
    s.create_item("ab12.0002", "task", "Blocker", "u");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "u");
    let t = MemTable::new("stream-1");
    let mut seen = 0;
    let mut step = |s: &Store| {
        let ops: Vec<(i64, _)> = s
            .export()
            .into_iter()
            .enumerate()
            .skip(seen)
            .map(|(i, op)| (i as i64 + 1, op))
            .collect();
        seen += ops.len();
        ready(folder.fold_batch(&t, &ops)).unwrap();
        let lanes = ready(board::select(&t)).unwrap().board.lanes(NOW);
        let archived: Vec<String> = ready(board::dated(&t, Dated::Archived, None))
            .unwrap()
            .iter()
            .map(|r| text(r, "id").unwrap().to_string())
            .collect();
        (lanes.blocked, lanes.ready, archived)
    };
    let v = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    assert_eq!(step(&s), (v(&["ab12.0001"]), v(&["ab12.0002"]), v(&[])));
    s.set_field("ab12.0002", "archived", Some("2026-10-08".into()), "u");
    assert_eq!(step(&s), (v(&[]), v(&["ab12.0001"]), v(&["ab12.0002"])));
    s.set_field("ab12.0002", "archived", None, "u");
    assert_eq!(step(&s), (v(&["ab12.0001"]), v(&["ab12.0002"]), v(&[])));
    s.delete_item("ab12.0002", "u");
    assert_eq!(step(&s), (v(&[]), v(&["ab12.0001"]), v(&[])));
    // Deleted, the blocker is off the index; the edge stays, because its other end is active — the
    // same edge set `graph::select` hands the library locally, where the deleted end counts as closed.
    let on_index: Vec<String> = ready(t.query_active())
        .unwrap()
        .iter()
        .filter_map(|r| text(r, "sk").map(str::to_string))
        .collect();
    assert_eq!(on_index.len(), 2, "{on_index:?}");
    assert!(on_index.contains(&"items#ab12.0001".to_string()));
    assert!(on_index.iter().any(|k| k.starts_with("edge_adds#")));
    // Closing the work too takes the last active end away, and the edge leaves with it.
    s.set_field("ab12.0001", "status", Some("closed".into()), "u");
    assert_eq!(step(&s), (v(&[]), v(&[]), v(&[])));
    assert!(ready(t.query_active()).unwrap().is_empty());
}

#[test]
fn a_snapshot_that_does_not_fit_is_refused_before_anything_is_written() {
    let folder = Folder::platform().unwrap();
    let s = merged(6);
    let source = MemTable::new("stream-1");
    assert!(ready(snapshot::export(&source, &folder)).unwrap().is_none());
    ready(folder.fold_batch(&source, &delivered(&s, 6))).unwrap();
    let good = ready(snapshot::export(&source, &folder)).unwrap().unwrap();

    let refused = |snap: &snapshot::Snapshot| {
        let t = MemTable::new("stream-1");
        let err = ready(snapshot::import(&t, &folder, snap)).unwrap_err();
        assert!(t.rows().is_empty(), "nothing written for {err}");
        err
    };
    let mut other = good.clone();
    other.stream_id = "stream-2".into();
    assert!(matches!(refused(&other), ImportError::WrongStream { .. }));
    let mut newer = good.clone();
    newer.format += 1;
    assert!(matches!(refused(&newer), ImportError::Format(_)));
    let mut bad = good.clone();
    bad.tables.get_mut("items").unwrap()[0].insert("title".into(), serde_json::json!([1]));
    assert!(matches!(refused(&bad), ImportError::BadCell { .. }));
    let mut unknown = good.clone();
    unknown.tables.insert("nope".into(), vec![]);
    assert!(matches!(refused(&unknown), ImportError::Layout(_)));
    let mut column = good.clone();
    column.tables.get_mut("items").unwrap()[0].insert("nope".into(), serde_json::json!("x"));
    assert!(matches!(refused(&column), ImportError::Layout(_)));

    // An import that died after some rows — no watermark yet — is simply run again.
    let t = MemTable::new("stream-1");
    let partial = ready(snapshot::import(&Failing::new(&t, 10), &folder, &good));
    assert!(partial.is_err());
    assert!(ready(Folder::watermark(&t)).unwrap().is_none());
    ready(snapshot::import(&t, &folder, &good)).unwrap();
    let whole = MemTable::new("stream-1");
    ready(snapshot::import(&whole, &folder, &good)).unwrap();
    assert_eq!(t.rows(), whole.rows());
    assert!(matches!(
        ready(folder.fold(&t, &s.export()[0])).unwrap(),
        Folded::Folded
    ));
}
