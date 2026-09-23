//! **Start from a snapshot against a full fold, measured on a real board** (nxf 6j6v.mxt2's DoD) —
//! at the seam an embedding host calls, in-process, so the numbers are the fold's and the import's
//! and not a network's.
//!
//! Ignored by default: it needs a board to measure (`NXS_SNAPSHOT_MEASURE_DB`, the path of a
//! workspace's `db.sqlite`; the file is copied first and never opened in place) and only means
//! something in a release build:
//!
//! ```text
//! NXS_SNAPSHOT_MEASURE_DB=/path/to/.nxs/db.sqlite \
//!   cargo test --release -p nxs-sync --features engine --test snapshot_measure -- --ignored --nocapture
//! ```
//!
//! The board's ops are served in their own rowid order by an in-process relay that counts the bytes
//! its pages would put on the wire, exactly the JSON the real relay sends.
#![cfg(feature = "engine")]

use std::cell::Cell;
use std::time::Instant;

use nexus_flow_core::store::Store;
use nxs_sync::engine::{self, Transport, TransportError, Unbounded, Watermarks};
use nxs_sync::protocol::{Cursor, PullResponse, RegisterOutcome, StreamId};
use nxs_sync::snapshot;
use nxs_sync::wire::WireOp;

struct Relay {
    log: Vec<WireOp>,
    bytes: Cell<usize>,
}

impl Transport for Relay {
    fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
        Ok(())
    }
    fn pull(
        &self,
        _: &StreamId,
        since: Cursor,
        limit: usize,
    ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
        let start = (since.0.max(0) as usize).min(self.log.len());
        let ops: Vec<WireOp> = self.log[start..].iter().take(limit).cloned().collect();
        let next = Cursor((start + ops.len()) as i64);
        let page = PullResponse {
            ops: ops.clone(),
            next,
        };
        self.bytes
            .set(self.bytes.get() + serde_json::to_vec(&page).unwrap().len());
        Ok((ops, next))
    }
    fn register(&self, _: &StreamId, _: &str, _: &str) -> Result<RegisterOutcome, TransportError> {
        Ok(RegisterOutcome::Registered)
    }
}

/// flow's folded views, order-free, plus the op set — the equality the promise is about.
fn state(s: &Store) -> Vec<String> {
    let mut out: Vec<String> = s
        .image()
        .unwrap()
        .views
        .into_iter()
        .map(|t| {
            let mut rows: Vec<String> = t.rows.iter().map(|r| format!("{r:?}")).collect();
            rows.sort();
            format!("{} {rows:?}", t.name)
        })
        .collect();
    let mut ids: Vec<String> = s.export().into_iter().map(|o| o.op_id).collect();
    ids.sort();
    out.push(format!("{ids:?}"));
    out
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "needs NXS_SNAPSHOT_MEASURE_DB and a release build"]
fn start_from_a_snapshot_against_a_full_fold_on_a_real_board() {
    let Ok(db) = std::env::var("NXS_SNAPSHOT_MEASURE_DB") else {
        eprintln!("set NXS_SNAPSHOT_MEASURE_DB to a board's db.sqlite");
        return;
    };
    let dir = std::env::temp_dir().join(format!("nxs-snapshot-measure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("source.sqlite");
    std::fs::copy(db, &copy).unwrap();
    let source = Store::open(copy.to_str().unwrap(), 1).unwrap();
    let log: Vec<WireOp> = {
        let mut stmt = source
            .connection()
            .prepare("SELECT op_id FROM ops ORDER BY rowid")
            .unwrap();
        let order: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let by_id: std::collections::HashMap<String, WireOp> = source
            .export()
            .iter()
            .map(|op| (op.op_id.clone(), WireOp::from(op)))
            .collect();
        order.into_iter().map(|id| by_id[&id].clone()).collect()
    };
    let relay = Relay {
        log,
        bytes: Cell::new(0),
    };
    let stream = StreamId("measured".into());
    let n = relay.log.len();
    println!("board: {n} ops");

    for (label, file_backed) in [("in memory", false), ("on disk", true)] {
        let open = |name: &str| {
            if file_backed {
                let path = dir.join(format!("{name}.sqlite"));
                let _ = std::fs::remove_file(&path);
                let store = Store::open(path.to_str().unwrap(), 7).unwrap();
                // WAL, as every workspace the product opens is (`facade::workspace`) — without it
                // each op's commit waits on a journal fsync and the full fold measures the disk.
                store
                    .connection()
                    .execute_batch("PRAGMA journal_mode=WAL;")
                    .unwrap();
                store
            } else {
                Store::open_in_memory(7)
            }
        };

        // A full fold: an empty replica pulls and folds the whole history.
        relay.bytes.set(0);
        let started = Instant::now();
        let mut full = open("full");
        let mut marks = Watermarks::default();
        engine::sync(&mut full, 7, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        let full_ms = ms(started);
        let full_bytes = relay.bytes.get();

        // A snapshot of that replica…
        let started = Instant::now();
        let bytes = snapshot::export(&full, 7, &stream, &marks, &relay).unwrap();
        let export_ms = ms(started);

        // …and an empty replica started from it: import, then the pass that pulls the rest.
        relay.bytes.set(0);
        let started = Instant::now();
        let mut fresh = open("fresh");
        let imported = snapshot::import(&mut fresh, &bytes, &stream, &relay).unwrap();
        let mut resume = imported.marks;
        let rest =
            engine::sync(&mut fresh, 7, &stream, &mut resume, &relay, 500, &Unbounded).unwrap();
        let start_ms = ms(started);
        let start_bytes = bytes.len() + relay.bytes.get();

        assert_eq!(rest.pulled, 0, "nothing came after the snapshot");
        assert_eq!(state(&fresh), state(&full), "{label}: the same state");
        println!(
            "{label}: full fold {full_ms:.0} ms, {full_bytes} bytes pulled | snapshot: export \
             {export_ms:.0} ms, {} bytes; start from it (import + the pass that pulls the rest) \
             {start_ms:.0} ms, {start_bytes} bytes | views {:?}, position {:?}",
            bytes.len(),
            imported.loaded,
            imported.anchor
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
