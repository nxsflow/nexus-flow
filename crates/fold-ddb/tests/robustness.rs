//! What a server fold does when things go wrong (review of PR #24): a run that dies at any write
//! and folds its batch again, an op the table cannot hold, a clear that dies half way, a release
//! with other fold revisions, a lane cut by a limit, a snapshot that does not fit — and the `typed`
//! index kept through retypes, archives and deletes (6j6v.xkr7); a stream being refolded, a row
//! with no room for its index attributes and the escaping of a `typed` partition (review of PR
//! #55).

mod common;

use common::{delivered, head, merged, NOW};
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nexus_flow_facade::read;
use nxs_fold_ddb::board;
use nxs_fold_ddb::fold::{Folded, Folder};
use nxs_fold_ddb::layout::{typed_partition, MAX_ITEM_BYTES, MAX_PARTITION_BYTES, WATERMARK_KEY};
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::records::{self, RecordsError};
use nxs_fold_ddb::snapshot::{self, ImportError};
use nxs_fold_ddb::table::{text, Dated, Outcome, Row, Table};
use nxs_fold_ddb::write::attribute_bytes;
use nxs_fold_ddb::write::{Cond, Write};
use nxs_foundation::change::Cell;
use nxs_foundation::model::Op;
use std::collections::BTreeMap;
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

    async fn get_many(&self, sks: &[String]) -> Result<BTreeMap<String, Row>, Injected> {
        Ok(ok(self.inner.get_many(sks).await))
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

    async fn query_typed(&self, item_type: &str) -> Result<Vec<Row>, Injected> {
        Ok(ok(self.inner.query_typed(item_type).await))
    }
}

/// How many writes and deletes folding `second` after `first` makes — every index at which a
/// [`Failing`] table can stop it.
fn writes_and_deletes(first: &[(i64, Op)], second: &[(i64, Op)]) -> usize {
    let folder = Folder::platform().unwrap();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, first)).unwrap();
    let before = t.requests();
    ready(folder.fold_batch(&t, second)).unwrap();
    let after = t.requests();
    (after.writes - before.writes + after.deletes - before.deletes) as usize
}

/// The reverse-reference triples `(source, field, value)` the stored `.radj#` hints say, and those
/// their `.rsrc#` mirrors say (6j6v.9tx3). Equal on every place the fold finished.
fn hint_triples(t: &MemTable) -> [std::collections::BTreeSet<(String, String, String)>; 2] {
    [".radj#", ".rsrc#"].map(|prefix| {
        t.rows()
            .iter()
            .filter(|(sk, _)| sk.starts_with(prefix))
            .map(|(_, r)| {
                let cell = |c: &str| text(r, c).unwrap_or_default().to_string();
                (cell("source_id"), cell("field"), cell("target_id"))
            })
            .collect()
    })
}

/// How a run that died left the reverse-reference hints, before anything repaired them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct HalfReconciled {
    /// A hint without its mirror: the run died between the `.radj#` put and the `.rsrc#` put.
    put_crashes: usize,
    /// A mirror without its hint: the run died between the `.radj#` delete and the `.rsrc#` delete.
    delete_crashes: usize,
}

impl HalfReconciled {
    fn of(t: &MemTable) -> HalfReconciled {
        let [hints, mirrors] = hint_triples(t);
        HalfReconciled {
            put_crashes: hints.difference(&mirrors).count(),
            delete_crashes: mirrors.difference(&hints).count(),
        }
    }

    fn add(&mut self, other: HalfReconciled) {
        self.put_crashes += other.put_crashes;
        self.delete_crashes += other.delete_crashes;
    }
}

/// Fold `first` cleanly, then `second` with a run that dies at each write or delete in turn and
/// folds `second` again; every such stream must end as the unbroken one, and as `board::reindex`
/// leaves it. Returns how many of those deaths left a place half-reconciled — so a caller knows the
/// harness reached the reverse-reference reconcile (6j6v.1w3m, Must fix 1): a run with no reconcile
/// in its failing half proves nothing about it.
fn dies_anywhere_in_the_second_batch(
    first: &[(i64, Op)],
    second: &[(i64, Op)],
    ctx: &str,
) -> HalfReconciled {
    let folder = Folder::platform().unwrap();
    let clean = MemTable::new("stream-1");
    ready(folder.fold_batch(&clean, first)).unwrap();
    ready(folder.fold_batch(&clean, second)).unwrap();
    let indexes = writes_and_deletes(first, second);
    assert!(indexes > 0, "{ctx}");
    let mut half = HalfReconciled::default();
    for fail_at in 0..indexes {
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
        half.add(HalfReconciled::of(&t));
        ready(folder.fold_batch(&t, second)).unwrap();
        assert!(
            t.rows() == clean.rows(),
            "{ctx}: a run that died at write {fail_at} of the second batch and folded it again differs"
        );
        let replayed = t.rows();
        ready(board::reindex(&t)).unwrap();
        assert!(
            t.rows() == replayed,
            "{ctx}: after the replay of a death at write {fail_at}, reindex changes a row"
        );
    }
    half
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

    // The shared generator writes id-shaped custom values (6j6v.1w3m): both seeds' second halves
    // hold reconciles, and a death lands between a hint and its mirror — seed 2's both on a put
    // and on a delete, seed 9's on puts.
    let mut both = HalfReconciled::default();
    for seed in [2, 9] {
        let ops = delivered(&merged(seed), seed);
        let (first, second) = ops.split_at(ops.len() / 2);
        let half = dies_anywhere_in_the_second_batch(first, second, &format!("seed {seed}"));
        assert!(half.put_crashes > 0, "seed {seed}: {half:?}");
        both.add(half);
    }
    assert!(both.delete_crashes > 0, "{both:?}");
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
    // description; and the design that would grow ab12.0002 past the limit — weighed with the row
    // it lands on before anything is written (review of PR #55, Integrity #4).
    assert_eq!(refused.len(), 6, "{reasons:#?}");
    assert_eq!(
        reasons.iter().filter(|r| r.contains("a key of")).count(),
        4,
        "{reasons:#?}"
    );
    assert!(reasons.iter().any(|r| r.contains("a row of")));
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("item ab12.0002 would weigh")),
        "{reasons:#?}"
    );
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
fn a_type_too_long_for_the_typed_index_is_refused_and_the_item_folds_on_without_it() {
    // 6j6v.xkr7: the `typed` index partition carries the type; DynamoDB refuses an index key past
    // 2,048 bytes, and a refused write would fail every run at the same op. So the op that sets
    // such a type is refused before it writes, as an op with a key too long is, and the item keeps
    // the type it had.
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "T", "u");
    s.set_field("ab12.0001", "type", Some("t".repeat(2100)), "u");
    s.create_item("ab12.0002", &"n".repeat(2030), "Just fits", "u");
    let t = MemTable::new("stream-1");
    let refused = ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    let reasons: Vec<&str> = refused.iter().map(|r| r.reason.as_str()).collect();
    assert_eq!(refused.len(), 1, "{reasons:#?}");
    assert!(
        reasons[0].contains("`typed` index partition"),
        "{reasons:#?}"
    );
    let ids = |ty: &str| -> Vec<String> {
        ready(t.query_typed(ty))
            .unwrap()
            .iter()
            .map(|r| text(r, "id").unwrap().to_string())
            .collect()
    };
    assert_eq!(ids("task"), ["ab12.0001"]);
    assert_eq!(ids(&"n".repeat(2030)), ["ab12.0002"]);
}

#[test]
fn retyping_archiving_and_deleting_move_an_item_on_the_typed_index() {
    // 6j6v.xkr7: an item is listed under its type while it is live — archived or not — moves with a
    // retype, and leaves when it is deleted. A type with `#` in it gets its own partition.
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "person", "Ada", "u");
    s.create_item("ab12.0002", "person", "Bob", "u");
    s.create_item("ab12.0003", "a#b", "Odd", "u");
    let t = MemTable::new("stream-1");
    let mut seen = 0;
    let mut step = |s: &Store| {
        let ops = numbered(s, seen);
        seen += ops.len();
        ready(folder.fold_batch(&t, &ops)).unwrap();
        ["person", "project", "a#b", "a"].map(|ty| {
            ready(t.query_typed(ty))
                .unwrap()
                .iter()
                .map(|r| text(r, "id").unwrap().to_string())
                .collect::<Vec<_>>()
        })
    };
    let v = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let none = v(&[]);

    assert_eq!(
        step(&s),
        [
            v(&["ab12.0001", "ab12.0002"]),
            none.clone(),
            v(&["ab12.0003"]),
            none.clone()
        ]
    );
    s.set_field("ab12.0002", "archived", Some("2026-10-08".into()), "u");
    s.set_field("ab12.0001", "status", Some("closed".into()), "u");
    assert_eq!(
        step(&s),
        [
            v(&["ab12.0001", "ab12.0002"]),
            none.clone(),
            v(&["ab12.0003"]),
            none.clone()
        ],
        "closed and archived items stay listed"
    );
    s.set_field("ab12.0001", "type", Some("project".into()), "u");
    assert_eq!(
        step(&s),
        [
            v(&["ab12.0002"]),
            v(&["ab12.0001"]),
            v(&["ab12.0003"]),
            none.clone()
        ]
    );
    s.delete_item("ab12.0002", "u");
    s.delete_item("ab12.0003", "u");
    assert_eq!(
        step(&s),
        [none.clone(), v(&["ab12.0001"]), none.clone(), none.clone()]
    );
    // Created again under a deleted id, and a type cleared to none: the index lists what the store
    // holds live, by type — whatever the store makes of the re-create — and an item without a type
    // is on no partition (review of PR #55, Test #6).
    s.create_item("ab12.0002", "person", "Bob again", "u");
    s.set_field("ab12.0001", "type", None, "u");
    let live = |s: &Store| {
        ["person", "project", "a#b", "a"].map(|ty| {
            let mut ids: Vec<String> = s
                .list_items()
                .unwrap()
                .into_iter()
                .filter(|i| i.deleted.as_deref() != Some("1"))
                .filter(|i| i.item_type.as_deref() == Some(ty))
                .map(|i| i.id)
                .collect();
            ids.sort();
            ids
        })
    };
    let want = live(&s);
    assert_eq!(want[1], none, "the type cleared to none leaves `project`");
    assert_eq!(step(&s), want);
    assert!(!t.rows()["items#ab12.0001"].contains_key("nxf_typed"));
    // The attribute is on the item rows only, never on an adjacency entry or another row.
    assert!(t
        .rows()
        .iter()
        .filter(|(_, r)| r.contains_key("nxf_typed"))
        .all(|(sk, _)| sk.starts_with("items#")));
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

    // A clear that dies half way leaves a watermark that is not current — the next run clears it
    // again.
    assert!(ready(Folder::clear(&Failing::new(&t, 3), head(&ops))).is_err());
    let after = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert!(!folder.is_current(&after));

    // Cleared for good, only the sentinel watermark is left; folded from position 0, the stream
    // holds what a fresh fold holds, its watermark current and caught up with the clear's head.
    ready(Folder::clear(&t, head(&ops))).unwrap();
    assert_eq!(t.rows().keys().collect::<Vec<_>>(), [WATERMARK_KEY]);
    ready(folder.fold_batch(&t, &ops)).unwrap();
    let without_mark = |t: &MemTable| {
        let mut rows = t.rows();
        rows.remove(WATERMARK_KEY);
        rows
    };
    assert_eq!(without_mark(&t), without_mark(&clean));
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert!(folder.is_current(&mark));
    assert_eq!(mark.folded_through, head(&ops));
    assert_eq!(mark.refold_until, Some(head(&ops)));
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

    // An import that died after some rows is refused to readers — its watermark is the sentinel
    // it wrote first, naming no revision — and is simply run again.
    let t = MemTable::new("stream-1");
    let partial = ready(snapshot::import(&Failing::new(&t, 10), &folder, &good));
    assert!(partial.is_err());
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert!(mark.revisions.is_empty() && !folder.is_current(&mark));
    assert!(matches!(
        ready(records::list_board(&t, &read::ListReads::default())),
        Err(RecordsError::NotCurrent {
            domain: "index",
            found: None
        })
    ));
    ready(snapshot::import(&t, &folder, &good)).unwrap();
    let whole = MemTable::new("stream-1");
    ready(snapshot::import(&whole, &folder, &good)).unwrap();
    assert_eq!(t.rows(), whole.rows());
    assert!(matches!(
        ready(folder.fold(&t, &s.export()[0])).unwrap(),
        Folded::Folded
    ));
}

/// What the three readers of a hosted board answer for `t`: the records, the list, the ranking
/// graph (`plain` read before).
fn served(
    t: &MemTable,
    plain: &read::ActiveBoard,
) -> [Result<(), RecordsError<std::convert::Infallible>>; 3] {
    let selection = ready(board::select(t)).unwrap();
    [
        ready(records::active_board(t, selection)).map(drop),
        ready(records::list_board(t, &read::ListReads::default())).map(drop),
        ready(records::with_ranking(
            t,
            plain.clone(),
            &read::RankingReads::default(),
        ))
        .map(drop),
    ]
}

#[test]
fn a_refolding_stream_is_refused_from_its_clear_until_the_refold_reaches_the_head() {
    // Review of PR #55, Integrity #2: between a clear and the end of its refold the tables hold part
    // of the history. Every reader refuses the stream for that whole window — `NotCurrent` while
    // the watermark names no revision, `Refolding` once the first batch wrote them — and serves it
    // again exactly when the refold reaches the head the clear named.
    let folder = Folder::platform().unwrap();
    let s = merged(4);
    let ops = delivered(&s, 4);
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &ops)).unwrap();
    let plain = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
    assert!(!plain.tickets.is_empty());
    assert!(served(&t, &plain).iter().all(Result::is_ok));
    let whole = t.rows();

    // A clear that dies after its sentinel: refused, and not current, so the next run clears again.
    assert!(ready(Folder::clear(&Failing::new(&t, 1), head(&ops))).is_err());
    for answer in served(&t, &plain) {
        assert!(
            matches!(
                answer,
                Err(RecordsError::NotCurrent {
                    domain: "index",
                    found: None
                })
            ),
            "{answer:?}"
        );
    }
    assert!(!folder.is_current(&ready(Folder::watermark(&t)).unwrap().unwrap()));

    ready(Folder::clear(&t, head(&ops))).unwrap();
    for answer in served(&t, &plain) {
        assert!(
            matches!(
                answer,
                Err(RecordsError::NotCurrent {
                    domain: "index",
                    found: None
                })
            ),
            "{answer:?}"
        );
    }

    // Half the history refolded: this build's revisions are written, the head is not reached.
    let half = ops.len() / 2;
    ready(folder.fold_batch(&t, &ops[..half])).unwrap();
    assert!(folder.is_current(&ready(Folder::watermark(&t)).unwrap().unwrap()));
    for answer in served(&t, &plain) {
        match answer {
            Err(RecordsError::Refolding {
                folded_through,
                until,
            }) => assert_eq!((folded_through, until), (head(&ops[..half]), head(&ops))),
            other => panic!("a half-refolded stream must be refused, got {other:?}"),
        }
    }

    // Caught up: served again, the same tables as before the clear; a batch folded twice changes
    // nothing.
    ready(folder.fold_batch(&t, &ops[half..])).unwrap();
    assert!(served(&t, &plain).iter().all(Result::is_ok));
    ready(folder.fold_batch(&t, &ops[half..])).unwrap();
    assert!(served(&t, &plain).iter().all(Result::is_ok));
    let without_mark = |mut rows: BTreeMap<String, Row>| {
        rows.remove(WATERMARK_KEY);
        rows
    };
    assert_eq!(without_mark(t.rows()), without_mark(whole));
}

#[test]
fn a_typed_partition_escapes_the_type_so_no_stream_and_type_share_one() {
    // Review of PR #55, Test #2: the stream is the partition's prefix and the type its rest, so a
    // `#` in the type must not read as the separator.
    assert_ne!(typed_partition("a", "b#c"), typed_partition("a#b", "c"));
    assert_eq!(typed_partition("a", "b#c"), "a#b%23c");
    assert_ne!(typed_partition("a", "b%23c"), typed_partition("a", "b#c"));

    // A type of `#`s triples when escaped: it is weighed escaped, so a type whose own bytes fit is
    // refused once its partition would not.
    let folder = Folder::platform().unwrap();
    let hashes = "#".repeat(700);
    assert!("stream-1#".len() + hashes.len() <= MAX_PARTITION_BYTES);
    assert!(typed_partition("stream-1", &hashes).len() > MAX_PARTITION_BYTES);
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", &hashes, "Hashes", "u");
    s.create_item("ab12.0002", &"x".repeat(700), "Plain", "u");
    let t = MemTable::new("stream-1");
    let refused = ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    let reasons: Vec<&str> = refused.iter().map(|r| r.reason.as_str()).collect();
    assert_eq!(refused.len(), 1, "{reasons:#?}");
    assert!(
        reasons[0].contains("`typed` index partition"),
        "{reasons:#?}"
    );
    assert!(ready(t.query_typed(&hashes)).unwrap().is_empty());
    let plain: Vec<String> = ready(t.query_typed(&"x".repeat(700)))
        .unwrap()
        .iter()
        .map(|r| text(r, "id").unwrap().to_string())
        .collect();
    assert_eq!(plain, ["ab12.0002"]);
}

#[test]
fn an_item_with_no_room_for_its_index_attributes_is_refused_rather_than_left_off_an_index() {
    // Review of PR #55, Integrity #4: closing a ticket swaps its `active` attribute for the two
    // `dated` ones, which weigh more. A row the close itself fits, but not with those, used to take
    // the close and drop the attribute write in silence — closed in its cells, still on `active`,
    // on no `dated` lane. The op is refused before it writes anything instead.
    let folder = Folder::platform().unwrap();
    let stream = "stream-1";
    let ops_with = |description: usize| {
        let mut s = Store::open_in_memory(1);
        s.create_item("ab12.0001", "task", "T", "u");
        s.set_field(
            "ab12.0001",
            "description",
            Some("d".repeat(description)),
            "u",
        );
        s.set_field("ab12.0001", "status", Some("closed".into()), "u");
        numbered(&s, 0)
    };
    let weigh = |r: &Row| r.iter().map(|(n, c)| attribute_bytes(n, c)).sum::<usize>();

    // Measured on a small row: the closed row with its index attributes (`typed`, `dated`), and as
    // it would weigh still carrying the open row's (`typed`, `active`).
    let small = MemTable::new(stream);
    ready(folder.fold_batch(&small, &ops_with(10))).unwrap();
    let row = small.rows()["items#ab12.0001"].clone();
    let dated: usize = ["nxf_dated", "nxf_dated_at"]
        .iter()
        .map(|a| attribute_bytes(a, &row[*a]))
        .sum();
    let active = attribute_bytes("nxf_active", &Cell::Text(stream.into()));
    assert!(dated > active, "the close makes the row heavier");
    let with_open_attributes = weigh(&row) - dated + active;

    // The description that leaves the closed row exactly at the limit with the open row's
    // attributes: the close itself fits, its index attributes do not.
    let ops = ops_with(10 + MAX_ITEM_BYTES - with_open_attributes);
    let t = MemTable::new(stream);
    let (close, before) = ops.split_last().unwrap();
    assert!(ready(folder.fold_batch(&t, before)).unwrap().is_empty());
    match ready(folder.fold(&t, &close.1)).unwrap() {
        Folded::Refused(r) => assert!(r.reason.contains("index attributes"), "{r:?}"),
        other => panic!("the close must be refused, got {other:?}"),
    }
    // Refused whole: still open in its cells and on its indexes, on no `dated` lane.
    let row = &t.rows()["items#ab12.0001"];
    assert_eq!(text(row, "status"), Some("open"));
    assert!(row.contains_key("nxf_active") && row.contains_key("nxf_typed"));
    assert!(weigh(row) <= MAX_ITEM_BYTES);
    assert!(ready(board::dated(&t, Dated::Closed, None))
        .unwrap()
        .is_empty());
    assert_eq!(
        ready(board::select(&t)).unwrap().board.lanes(NOW).ready,
        vec!["ab12.0001"]
    );
}

// ---------------------------------------------------------------------------------------------
// Reverse-reference hints (6j6v.9tx3 cut 3, 6j6v.1w3m): `.radj#<value>#<field>#<source>` and its
// mirror `.rsrc#<source>#<field>#<value>`, a function of the rows `ref_place_ok` admits.
// ---------------------------------------------------------------------------------------------

/// A neutral test plugin with one single and one list ref field, so `records::list_board` can be
/// asked `list --ref` (the reads come from the facade's `list_reads`).
const REFS: &str = r#"
name = "robustness-ref-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]
[types]
list = ["project", "person"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [ { field = "priority", dir = "asc" } ]
[presentation.list]
columns = ["id"]
[fields.owner]
type = "ref"
targets = ["person"]
on = ["project"]
[fields.members]
type = "ref"
targets = ["person"]
list = true
on = ["project"]
"#;

inventory::submit!(nexus_flow_facade::plugin::PluginRegistration {
    name: "robustness-ref-fixture",
    toml: REFS,
    order: 142,
});

/// What a hosted `list --ref field=target` answers from `t`, unranked: the ids of its page.
fn hosted_ref(t: &MemTable, field: &str, target: &str) -> Vec<String> {
    let cfg = nexus_flow_facade::plugin::load("robustness-ref-fixture").unwrap();
    let q = read::ListQuery::new().referencing(field, target);
    let reads = read::list_reads(&cfg, &q);
    assert!(reads.select.is_some(), "an unranked query");
    let board = ready(records::list_board(t, &reads)).unwrap();
    read::list_board_page(&cfg, &board, &q)
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.id)
        .collect()
}

/// The ids `t`'s rows themselves say hold `target` in `field` — a `custom_fields` row or a present
/// `field_entries` row — what a hosted answer may never exceed.
fn backed_by_rows(t: &MemTable, field: &str, target: &str) -> Vec<String> {
    let mut ids: Vec<String> = t
        .rows()
        .iter()
        .filter(|(sk, r)| {
            (sk.starts_with("custom_fields#")
                || (sk.starts_with("field_entries#")
                    && !matches!(r.get("removed"), Some(Cell::Int(1)))))
                && text(r, "field") == Some(field)
                && text(r, "value") == Some(target)
        })
        .filter_map(|(_, r)| text(r, "item_id").map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn without_mark(mut rows: BTreeMap<String, Row>) -> BTreeMap<String, Row> {
    rows.remove(WATERMARK_KEY);
    rows
}

/// Rows equal to what `board::reindex` makes of them — checked on a copy, so `t` is left as is.
fn rows_equal_reindex(t: &MemTable) -> bool {
    reindexed_rows(t) == t.rows()
}

/// What `board::reindex` makes of `t`'s rows, computed on a copy.
fn reindexed_rows(t: &MemTable) -> BTreeMap<String, Row> {
    let copy = MemTable::new(t.stream());
    for (sk, row) in t.rows() {
        let set = row
            .into_iter()
            .filter(|(n, _)| n != "stream_id" && n != "sk")
            .collect();
        ready(copy.write(&Write::put(sk, set))).unwrap();
    }
    ready(board::reindex(&copy)).unwrap();
    copy.rows()
}

/// Persons p1..p3; project x1 owned by p1, x2 owned by p1, x3 with members p1 and p2 — the first
/// batch. The second: x1 retargeted to p2, x2's owner cleared, p1 removed from x3's members.
fn retarget_clear_remove() -> (Store, Vec<(i64, Op)>, Vec<(i64, Op)>) {
    use nexus_flow_core::model::MergeStrategy::Lww;
    let mut s = Store::open_in_memory(1);
    for p in ["ab12.p001", "ab12.p002", "ab12.p003"] {
        s.create_item(p, "person", "P", "u");
    }
    for x in ["ab12.x001", "ab12.x002", "ab12.x003"] {
        s.create_item(x, "project", "X", "u");
    }
    s.set_custom_field_merge("ab12.x001", "owner", Some("ab12.p001".into()), "u", Lww);
    s.set_custom_field_merge("ab12.x002", "owner", Some("ab12.p001".into()), "u", Lww);
    s.add_field_entry("ab12.x003", "members", "ab12.p001", None, "u");
    s.add_field_entry("ab12.x003", "members", "ab12.p002", None, "u");
    let first = numbered(&s, 0);
    s.set_custom_field_merge("ab12.x001", "owner", Some("ab12.p002".into()), "u", Lww);
    s.set_custom_field_merge("ab12.x002", "owner", None, "u", Lww);
    s.remove_field_entry("ab12.x003", "members", "ab12.p001", "u");
    let second = numbered(&s, first.len());
    (s, first, second)
}

#[test]
fn a_ref_hint_is_written_on_fold_and_rebuilt_by_reindex() {
    use nexus_flow_core::model::MergeStrategy::Lww;
    use nxs_fold_ddb::layout::{ref_adjacency_key, ref_source_key};
    let folder = Folder::platform().unwrap();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.x001", "project", "X", "u");
    // Fields of any name: no declaration is involved, the rule is the value's shape.
    s.set_custom_field_merge("ab12.x001", "sponsor", Some("ab12.p001".into()), "u", Lww);
    s.add_field_entry("ab12.x001", "crew", "ab12.p002", Some("lead"), "u");
    // Not id-shaped: no hint. A source of 129 bytes: no hint, single or list.
    s.set_custom_field_merge("ab12.x001", "colour", Some("dark blue".into()), "u", Lww);
    let long = format!("ab12.{}", "y".repeat(124));
    assert_eq!(long.len(), 129);
    s.create_item(&long, "project", "Long", "u");
    s.set_custom_field_merge(&long, "sponsor", Some("ab12.p001".into()), "u", Lww);
    let ops = numbered(&s, 0);
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &ops)).unwrap();

    let rows = t.rows();
    let refs: Vec<&String> = rows
        .keys()
        .filter(|sk| sk.starts_with(".radj#") || sk.starts_with(".rsrc#"))
        .collect();
    assert_eq!(refs.len(), 4, "{refs:#?}");
    for (field, value) in [("sponsor", "ab12.p001"), ("crew", "ab12.p002")] {
        for sk in [
            ref_adjacency_key(value, field, "ab12.x001"),
            ref_source_key("ab12.x001", field, value),
        ] {
            let row = &rows[&sk];
            assert_eq!(text(row, "target_id"), Some(value), "{sk}");
            assert_eq!(text(row, "field"), Some(field), "{sk}");
            assert_eq!(text(row, "source_id"), Some("ab12.x001"), "{sk}");
        }
    }

    // Reindex changes nothing; a clear and a full refold write the same rows again.
    assert!(rows_equal_reindex(&t));
    ready(Folder::clear(&t, head(&ops))).unwrap();
    assert!(!t.rows().keys().any(|sk| sk.starts_with(".r")));
    ready(folder.fold_batch(&t, &ops)).unwrap();
    assert_eq!(without_mark(t.rows()), without_mark(rows));
}

#[test]
fn a_snapshot_import_carries_the_hints() {
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let source = MemTable::new("stream-1");
    ready(folder.fold_batch(&source, &numbered(&s, 0))).unwrap();
    let snap = ready(snapshot::export(&source, &folder)).unwrap().unwrap();
    // The task fold revision is the one before this cut: an earlier snapshot still imports.
    assert_eq!(snap.revisions.get("task"), Some(&3));
    let started = MemTable::new("stream-1");
    ready(snapshot::import(&started, &folder, &snap)).unwrap();
    for (field, p) in [
        ("owner", "ab12.p001"),
        ("owner", "ab12.p002"),
        ("members", "ab12.p002"),
    ] {
        assert_eq!(
            ready(records::referrers(&started, field, p)).unwrap(),
            ready(records::referrers(&source, field, p)).unwrap(),
            "{field} {p}"
        );
    }
    assert_eq!(
        ready(records::referrers(&started, "owner", "ab12.p002")).unwrap(),
        ["ab12.x001"]
    );
    assert!(started.rows() == source.rows());
}

#[test]
fn a_retarget_a_clear_and_a_removed_entry_prune_their_hints() {
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    let p1: Vec<String> = t
        .rows()
        .into_keys()
        .filter(|sk| {
            (sk.starts_with(".radj#") && sk.contains("ab12.p001"))
                || (sk.starts_with(".rsrc#") && sk.ends_with("ab12.p001"))
        })
        .collect();
    assert!(
        p1.is_empty(),
        "no hint and no mirror for p1 is left: {p1:?}"
    );
    assert!(hosted_ref(&t, "owner", "ab12.p001").is_empty());
    assert!(hosted_ref(&t, "members", "ab12.p001").is_empty());
    assert_eq!(hosted_ref(&t, "owner", "ab12.p002"), ["ab12.x001"]);
    assert_eq!(hosted_ref(&t, "members", "ab12.p002"), ["ab12.x003"]);
    let kept = t.rows();
    ready(board::reindex(&t)).unwrap();
    assert!(t.rows() == kept, "reindex changes no row");
}

#[test]
fn an_import_after_a_retarget_plus_the_tail_equals_the_whole_fold() {
    use nexus_flow_core::model::MergeStrategy::Lww;
    let folder = Folder::platform().unwrap();
    let (mut s, _, _) = retarget_clear_remove();
    // A tail that touches the same places again: the snapshot sits after the retarget.
    s.set_custom_field_merge("ab12.x001", "owner", Some("ab12.p003".into()), "u", Lww);
    s.add_field_entry("ab12.x003", "members", "ab12.p001", None, "u");
    s.set_custom_field_merge("ab12.x002", "owner", Some("ab12.p002".into()), "u", Lww);
    let ops = numbered(&s, 0);
    let retarget_at = ops
        .iter()
        .position(|(_, op)| op.value.as_deref() == Some("ab12.p002") && op.field == "owner")
        .expect("the retarget");
    // The snapshot is taken right after the retarget; the tail touches the same places again.
    let half = retarget_at + 1;
    assert!(
        half < ops.len() - 2,
        "a tail of three ops follows the snapshot"
    );
    let source = MemTable::new("stream-1");
    ready(folder.fold_batch(&source, &ops[..half])).unwrap();
    let snap = ready(snapshot::export(&source, &folder)).unwrap().unwrap();
    let started = MemTable::new("stream-1");
    ready(snapshot::import(&started, &folder, &snap)).unwrap();
    ready(folder.fold_batch(&started, &ops[half..])).unwrap();
    let whole = MemTable::new("stream-1");
    ready(folder.fold_batch(&whole, &ops)).unwrap();
    assert!(started.rows() == whole.rows());
    assert_eq!(
        ready(records::referrers(&whole, "owner", "ab12.p003")).unwrap(),
        ["ab12.x001"]
    );
}

#[test]
fn a_run_that_dies_at_any_write_of_a_reconcile_is_repaired_by_the_replay() {
    // Must fix 1 (6j6v.1w3m): the failing table ticks on every write AND delete, and a field op
    // writes its row and two `item_timestamps` changes before its reconcile — so no index is
    // counted by hand: the harness fails at every one of them and replays.
    let (_, first, second) = retarget_clear_remove();
    let half = dies_anywhere_in_the_second_batch(&first, &second, "retarget, clear, remove");
    // The retarget puts a pair and deletes one, the clear and the remove delete one each: deaths
    // between a hint and its mirror happen on both sides.
    assert_eq!(
        half,
        HalfReconciled {
            put_crashes: 1,
            delete_crashes: 3
        }
    );
    // After the replay, referrers answer from what the rows hold.
    let folder = Folder::platform().unwrap();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &first)).unwrap();
    ready(folder.fold_batch(&t, &second)).unwrap();
    assert!(ready(records::referrers(&t, "owner", "ab12.p001"))
        .unwrap()
        .is_empty());
    assert_eq!(
        ready(records::referrers(&t, "owner", "ab12.p002")).unwrap(),
        ["ab12.x001"]
    );
    assert!(ready(records::referrers(&t, "members", "ab12.p001"))
        .unwrap()
        .is_empty());
}

#[test]
fn a_delete_crash_is_finished_by_a_later_op_and_a_put_crash_leaves_one_orphan_until_reindex() {
    // Must fix 2 (6j6v.1w3m): after a death between a `.radj#` put and its mirror, the orphan hint
    // is invisible to every later reconcile — they read the mirrors — so a DIFFERENT later op does
    // not remove it; only the replay of the same op and `reindex` do. A death between a `.radj#`
    // delete and its mirror is finished by any later op on the place.
    use nexus_flow_core::model::MergeStrategy::Lww;
    let folder = Folder::platform().unwrap();
    let (s, first, second) = retarget_clear_remove();
    // Two kinds of later op on every place of the second batch: an id-shaped value, and a plain
    // one — each a later op than the second batch's, so it wins.
    let later = |id_shaped: bool| -> Vec<(i64, Op)> {
        let mut s2 = Store::open_in_memory(3);
        s2.apply(&s.export());
        let v = if id_shaped { "ab12.p003" } else { "plain" };
        s2.set_custom_field_merge("ab12.x001", "owner", Some(v.into()), "u", Lww);
        s2.set_custom_field_merge("ab12.x002", "owner", Some(v.into()), "u", Lww);
        s2.add_field_entry("ab12.x003", "members", v, None, "u");
        let seen: std::collections::BTreeSet<String> =
            s.export().into_iter().map(|op| op.op_id).collect();
        let fresh: Vec<Op> = s2
            .export()
            .into_iter()
            .filter(|op| !seen.contains(&op.op_id))
            .collect();
        assert_eq!(fresh.len(), 3);
        fresh
            .into_iter()
            .enumerate()
            .map(|(i, op)| (head(&second) + 1 + i as i64, op))
            .collect()
    };
    let indexes = writes_and_deletes(&first, &second);
    let mut orphans_seen = 0;
    let mut delete_crashes_seen = 0;
    for id_shaped in [true, false] {
        let later = later(id_shaped);
        for fail_at in 0..indexes {
            let ctx = format!("death at {fail_at}, later op id-shaped: {id_shaped}");
            let t = MemTable::new("stream-1");
            ready(folder.fold_batch(&t, &first)).unwrap();
            assert!(ready(folder.fold_batch(&Failing::new(&t, fail_at), &second)).is_err());
            let half = HalfReconciled::of(&t);
            // After any death, before any repair: the read answers from the rows, never from a
            // hint alone.
            for p in ["ab12.p001", "ab12.p002"] {
                for field in ["owner", "members"] {
                    let backed = backed_by_rows(&t, field, p);
                    for id in hosted_ref(&t, field, p) {
                        assert!(backed.contains(&id), "{ctx}: {field} {p} answered {id}");
                    }
                }
            }
            ready(folder.fold_batch(&t, &later)).unwrap();
            let reindexed = reindexed_rows(&t);
            let rows = t.rows();
            if half.put_crashes > 0 {
                orphans_seen += 1;
                // Exactly one `.radj#` orphan: the hint of p2 under x1, its mirror never put.
                let differ: Vec<&String> = rows
                    .keys()
                    .filter(|sk| reindexed.get(*sk) != rows.get(*sk))
                    .chain(reindexed.keys().filter(|sk| !rows.contains_key(*sk)))
                    .collect();
                assert_eq!(differ.len(), 1, "{ctx}: {differ:#?}");
                assert!(
                    !reindexed.contains_key(differ[0]),
                    "{ctx}: reindex removes it"
                );
                assert!(
                    differ[0].starts_with(".radj#ab12.p002#owner#"),
                    "{ctx}: {differ:?}"
                );
                assert!(
                    !hosted_ref(&t, "owner", "ab12.p002").contains(&"ab12.x001".to_string()),
                    "{ctx}: the read does not answer the orphan"
                );
                ready(board::reindex(&t)).unwrap();
                assert!(!t.rows().contains_key(differ[0]), "{ctx}");
            } else {
                delete_crashes_seen += usize::from(half.delete_crashes > 0);
                assert!(rows == reindexed, "{ctx}: rows equal reindex");
            }
        }
    }
    assert_eq!(
        orphans_seen, 2,
        "the one put-crash index, under both later ops"
    );
    assert_eq!(
        delete_crashes_seen,
        2 * 3,
        "the three delete-crash indexes, under both later ops"
    );
}

#[test]
fn an_orphan_hint_is_ignored_by_the_read_and_removed_by_reindex() {
    // A `.radj#` row for a value no row holds and with no mirror — a state the reconcile itself
    // cannot reach, kept as the belt-and-braces case.
    use nxs_fold_ddb::layout::ref_adjacency_key;
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    let orphan = ref_adjacency_key("ab12.p003", "owner", "ab12.x002");
    ready(t.write(&Write::put(
        orphan.clone(),
        vec![
            ("target_id".into(), Cell::Text("ab12.p003".into())),
            ("field".into(), Cell::Text("owner".into())),
            ("source_id".into(), Cell::Text("ab12.x002".into())),
        ],
    )))
    .unwrap();
    assert_eq!(
        ready(records::referrers(&t, "owner", "ab12.p003")).unwrap(),
        ["ab12.x002"],
        "the hint names it"
    );
    assert!(
        hosted_ref(&t, "owner", "ab12.p003").is_empty(),
        "the read does not"
    );
    ready(board::reindex(&t)).unwrap();
    assert!(!t.rows().contains_key(&orphan));
}

#[test]
fn reindex_prunes_what_the_rows_do_not_back() {
    use nxs_fold_ddb::layout::{ref_adjacency_key, ref_source_key};
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    let clean = t.rows();
    let cells = vec![
        ("target_id".into(), Cell::Text("ab12.p009".into())),
        ("field".into(), Cell::Text("owner".into())),
        ("source_id".into(), Cell::Text("ab12.x001".into())),
    ];
    let stray_mirror = ref_source_key("ab12.x001", "owner", "ab12.p009");
    let stray_hint = ref_adjacency_key("ab12.p009", "owner", "ab12.x001");
    ready(t.write(&Write::put(stray_hint.clone(), cells.clone()))).unwrap();
    ready(t.write(&Write::put(stray_mirror.clone(), cells))).unwrap();
    ready(board::reindex(&t)).unwrap();
    assert!(!t.rows().contains_key(&stray_mirror));
    assert!(!t.rows().contains_key(&stray_hint));
    assert!(t.rows() == clean);
}

#[test]
fn a_stream_at_index_revision_three_is_not_current() {
    // Index revision 4 (6j6v.9tx3) adds the hints: a stream folded by revision 3 has none, and its
    // `list --ref` would answer nothing — refused until refolded.
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &numbered(&s, 0))).unwrap();
    ready(t.write(&Write {
        sk: WATERMARK_KEY.into(),
        set: vec![("revision_index".into(), Cell::Int(3))],
        remove: Vec::new(),
        condition: Some(Cond::RowPresent),
    }))
    .unwrap();
    assert!(!folder.is_current(&ready(Folder::watermark(&t)).unwrap().unwrap()));
    match ready(records::list_board(&t, &read::ListReads::default())) {
        Err(RecordsError::NotCurrent {
            domain: "index",
            found: Some(3),
        }) => {}
        other => panic!("a revision-3 stream must be refused, got {other:?}"),
    }
}

#[test]
fn a_hint_is_never_a_reason_to_refuse_an_op() {
    use nexus_flow_core::model::MergeStrategy::Lww;
    let folder = Folder::platform().unwrap();
    let value = format!("ab12.{}", "v".repeat(123));
    assert_eq!(value.len(), 128);
    let field = "f".repeat(60);
    let mut s = Store::open_in_memory(1);
    let fits = format!("ab12.{}", "i".repeat(115));
    assert_eq!(fits.len(), 120);
    let long = format!("ab12.{}", "l".repeat(124));
    assert_eq!(long.len(), 129);
    s.create_item(&fits, "project", "X", "u");
    s.create_item(&long, "project", "Y", "u");
    let base = s.export().len();
    s.set_custom_field_merge(&fits, &field, Some(value.clone()), "u", Lww);
    s.set_custom_field_merge(&long, &field, Some(value.clone()), "u", Lww);
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &numbered(&s, 0)[..base])).unwrap();
    for (_, op) in &numbered(&s, base) {
        assert_eq!(ready(folder.fold(&t, op)).unwrap(), Folded::Folded);
    }
    assert_eq!(
        ready(records::referrers(&t, &field, &value)).unwrap(),
        [fits.clone()],
        "the 120-byte source is hinted, the 129-byte one is not"
    );
    assert_eq!(
        text(&t.rows()[&format!("custom_fields#{long}#{field}")], "value"),
        Some(value.as_str()),
        "the 129-byte source's value folds all the same"
    );
}

/// A [`MemTable`] that answers [`Outcome::TooLarge`] for every reverse-reference hint it is asked
/// to write — a row that cannot occur (the `layout` unit test), to show what the fold does if it
/// ever did.
struct Cramped<'a>(&'a MemTable);

impl Table for Cramped<'_> {
    type Error = std::convert::Infallible;

    fn stream(&self) -> &str {
        self.0.stream()
    }

    async fn get(&self, sk: &str) -> Result<Option<Row>, Self::Error> {
        self.0.get(sk).await
    }

    async fn get_many(&self, sks: &[String]) -> Result<BTreeMap<String, Row>, Self::Error> {
        self.0.get_many(sks).await
    }

    async fn write(&self, write: &Write) -> Result<Outcome, Self::Error> {
        if write.sk.starts_with(".radj#") {
            return Ok(Outcome::TooLarge);
        }
        self.0.write(write).await
    }

    async fn delete(&self, sk: &str) -> Result<(), Self::Error> {
        self.0.delete(sk).await
    }

    async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, Self::Error> {
        self.0.query_prefix(prefix).await
    }

    async fn query_active(&self) -> Result<Vec<Row>, Self::Error> {
        self.0.query_active().await
    }

    async fn query_dated(
        &self,
        lane: Dated,
        limit: Option<usize>,
    ) -> Result<Vec<Row>, Self::Error> {
        self.0.query_dated(lane, limit).await
    }

    async fn query_typed(&self, item_type: &str) -> Result<Vec<Row>, Self::Error> {
        self.0.query_typed(item_type).await
    }
}

#[test]
fn a_hint_that_does_not_fit_refuses_its_op_and_never_fails_the_batch() {
    // Low (c) of the last design pass: a `TooLarge` on a bookkeeping row must not become a fold
    // error — that would fail `fold_batch`, hold the watermark, and wedge the stream on this op.
    // It is reported as the op's refusal, as an item row past its size is; the mirror is not put,
    // so the place does not read as hinted.
    let folder = Folder::platform().unwrap();
    let (s, _, _) = retarget_clear_remove();
    let ops = numbered(&s, 0);
    let t = MemTable::new("stream-1");
    let refused = ready(folder.fold_batch(&Cramped(&t), &ops)).unwrap();
    assert!(!refused.is_empty());
    assert!(
        refused
            .iter()
            .all(|r| r.reason.contains("the reverse-reference row .radj#")),
        "{refused:#?}"
    );
    assert_eq!(
        ready(Folder::watermark(&t))
            .unwrap()
            .unwrap()
            .folded_through,
        head(&ops),
        "the watermark moved past every op"
    );
    assert!(!t.rows().keys().any(|sk| sk.starts_with(".r")));
}
