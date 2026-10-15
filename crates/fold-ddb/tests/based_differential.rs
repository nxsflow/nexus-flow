//! **A replica that starts from a BASED snapshot and pulls only the tail holds exactly what a
//! replica that replayed the whole history holds** (6j6v.0cfa, spec §8) — the acceptance criterion
//! of the format, and the only test that can establish it.
//!
//! Why a differential and not a set of assertions about the format: the claim is not "the snapshot
//! contains the right things", which is a guess about what matters, but "nothing is missing", which
//! only a comparison against the full fold can establish. So two replicas are built from ONE
//! generated stream — one from a snapshot at relay position P plus the tail, one from every op from
//! 0 — and every view table of all three products, every derived lane, and the log above P must
//! come out equal.
//!
//! Four things are held beyond that equality, each of which a plausible implementation gets wrong:
//!
//! - **No op at or below P is ever fetched again.** The relay double records every pull; the based
//!   replica's lowest cursor must stand at P. A replica that re-pulls the prefix would be *correct*
//!   and would have saved nothing, which is the whole point of the format.
//! - **Arrival order does not decide the result.** The tail reaches a third replica shuffled and
//!   with ops delivered twice, as a relay without deduplication hands it over.
//! - **The hosted half starts from the SAME bytes.** The fold table is started from the snapshot
//!   through `snapshot::from_image` and folds the tail, and must hold what the replica holds.
//! - **The cap a hosted replica counts is met.** A stream past the 30,000-op budget is served to a
//!   based replica inside it, and refused to a replaying one.

use nexus_flow_core::derive;
use nexus_flow_core::store::Store;
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_foundation::model::Op;
use nxs_sync::engine::{self, Transport, TransportError, Unbounded, Watermarks};
use nxs_sync::protocol::{Cursor, RegisterOutcome, StreamId};
use nxs_sync::snapshot;
use nxs_sync::wire::WireOp;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::platform::{platform_log, platform_rows, platform_store, platform_view_tables, NOW};
use common::{served_rows, sqlite_rows, Lcg};

/// How big the acceptance stream is. The owner asked for at least 60,000 ops: nexflow.it's
/// Personal-CRM import measured a minimum of 35,551 for ONE user stream and is expected to land
/// near 57,700, and the hosted engine refuses a stream at 35,000 today — so the number this is
/// proven at has to be above the one that will actually arrive, not above the cap.
const ACCEPTANCE_OPS: usize = 60_000;

fn stream() -> StreamId {
    StreamId("based-differential".into())
}

/// The relay double: an append-only log, seq = index + 1, which **records every pull** so a test
/// can assert what was never asked for.
struct Relay {
    log: Vec<WireOp>,
    /// The `since` cursor of every pull, in order.
    pulls: RefCell<Vec<i64>>,
    /// The log as it stood at this length, or all of it — what a reader that synced at P saw.
    visible: usize,
}

impl Relay {
    fn holding(ops: &[Op]) -> Relay {
        Relay {
            log: ops.iter().map(nxs_sync::wire::to_wire).collect(),
            pulls: RefCell::new(Vec::new()),
            visible: ops.len(),
        }
    }

    /// The same log, as it stood when it held only its first `through` entries.
    fn at(&self, through: usize) -> Relay {
        Relay {
            log: self.log.clone(),
            pulls: RefCell::new(Vec::new()),
            visible: through,
        }
    }

    /// The lowest position any pull asked for — the proof that a prefix was never re-fetched.
    fn lowest_pull(&self) -> i64 {
        self.pulls.borrow().iter().copied().min().unwrap_or(i64::MAX)
    }
}

impl Transport for Relay {
    fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
        // Every replica in this test reads. A push would mean the test generated an op itself,
        // which would put it outside the stream both sides are being compared on.
        unreachable!("no replica in this differential writes")
    }
    fn pull(
        &self,
        _: &StreamId,
        since: Cursor,
        limit: usize,
    ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
        self.pulls.borrow_mut().push(since.0);
        let start = (since.0.max(0) as usize).min(self.visible);
        let page: Vec<WireOp> = self.log[start..self.visible]
            .iter()
            .take(limit)
            .cloned()
            .collect();
        Ok((page, Cursor((start + page.len()) as i64)))
    }
    fn register(&self, _: &StreamId, _: &str, _: &str) -> Result<RegisterOutcome, TransportError> {
        Ok(RegisterOutcome::Registered)
    }
}

/// Sync `store` against `relay` to the end, in pages of 5,000.
fn sync_through(store: &mut Store, site: i64, marks: &mut Watermarks, relay: &Relay) {
    loop {
        let before = marks.pulled_through;
        engine::sync(store, site, &stream(), marks, relay, 5_000, &Unbounded).unwrap();
        if marks.pulled_through == before {
            return;
        }
    }
}

/// A replica that pulled everything — the reference every based replica is compared against.
fn full_replay(ops: &[Op], site: i64) -> Store {
    let relay = Relay::holding(ops);
    let mut store = platform_store(site);
    let mut marks = Watermarks::default();
    sync_through(&mut store, site, &mut marks, &relay);
    store
}

/// A based snapshot of the stream at position `p`, taken by a replica that synced to exactly there.
fn snapshot_at(ops: &[Op], p: usize, site: i64) -> Vec<u8> {
    let relay = Relay::holding(ops);
    let prefix = relay.at(p);
    let mut writer = platform_store(site);
    let mut marks = Watermarks::default();
    sync_through(&mut writer, site, &mut marks, &prefix);
    assert_eq!(
        marks.pulled_through, p as i64,
        "the writer stands exactly at the position it claims"
    );
    snapshot::export_based(
        &writer,
        site,
        &stream(),
        &marks,
        &prefix,
        snapshot::PLATFORM,
    )
    .unwrap()
}

/// Everything about a store that two replicas of one stream must agree on: every view table of all
/// three products, the lanes the board derives, an item's live chunks, and the log's op ids.
fn state(s: &Store) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = platform_rows(s)
        .into_iter()
        .map(|(table, rows)| (format!("view:{table}"), format!("{rows:?}")))
        .collect();
    let c = s.connection();
    out.insert("ready".into(), format!("{:?}", derive::ready(c, NOW).unwrap()));
    out.insert("blocked".into(), format!("{:?}", derive::blocked(c).unwrap()));
    out.insert(
        "in_progress".into(),
        format!("{:?}", derive::in_progress(c, NOW).unwrap()),
    );
    out.insert(
        "next".into(),
        format!("{:?}", derive::next_candidates(c, NOW).unwrap()),
    );
    // The chunks of every item that has any, read the way a reader reads them — a view-row
    // comparison alone would not notice a supersede resolving differently.
    let mut stmt = c
        .prepare("SELECT DISTINCT item_id FROM chunks ORDER BY item_id")
        .unwrap();
    let items: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for item in items {
        for field in ["doc", "doc2"] {
            out.insert(
                format!("chunks:{item}/{field}"),
                format!("{:?}", s.chunks_of(&item, field).unwrap()),
            );
        }
    }
    out
}

/// The op ids a store holds, by domain.
fn op_ids_by_domain(s: &Store) -> BTreeMap<String, BTreeSet<String>> {
    let mut by_domain: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for op in s.export() {
        by_domain
            .entry(op.domain.clone())
            .or_default()
            .insert(op.op_id.clone());
    }
    by_domain
}

/// Compare two stores' whole state, reporting the FIRST key that differs rather than a diff of
/// everything — on a 60,000-op stream a full dump is unreadable.
fn assert_same_state(based: &Store, replayed: &Store, ctx: &str) {
    let (a, b) = (state(based), state(replayed));
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "the two replicas describe different things, {ctx}"
    );
    for (key, left) in &a {
        let right = &b[key];
        assert!(
            left == right,
            "{key} differs, {ctx}\n  based:    {}\n  replayed: {}",
            &left[..left.len().min(2000)],
            &right[..right.len().min(2000)]
        );
    }
}

/// **The acceptance criterion** (6j6v.0cfa): at least 60,000 ops, three writing sites, every op
/// shape the three products fold and three the build folds not at all — and a replica that starts
/// from the snapshot at P plus the tail is indistinguishable from one that replayed all of it.
#[test]
fn a_based_replica_plus_the_tail_is_identical_to_a_full_replay() {
    let ops = platform_log(0xacce_9742, 3, 8_000);
    assert!(
        ops.len() >= ACCEPTANCE_OPS,
        "the generated stream is {} ops, below the {ACCEPTANCE_OPS} the acceptance asks for",
        ops.len()
    );
    // Every product and the residue are really in it — a differential over a stream that holds
    // only tickets would pass while saying nothing about chat, memory or unfoldable ops.
    let replayed = full_replay(&ops, 1);
    let held = op_ids_by_domain(&replayed);
    for domain in ["task", "message", "fact", "no-reducer-here"] {
        assert!(
            held.get(domain).is_some_and(|ids| !ids.is_empty()),
            "the stream holds no {domain} ops: {:?}",
            held.keys().collect::<Vec<_>>()
        );
    }

    // Mid-stream, with every site's writing still in flight on both sides of the cut.
    let p = ops.len() / 2;
    let bytes = snapshot_at(&ops, p, 2);
    let header = snapshot::peek(&bytes).unwrap();
    assert_eq!(header.format, snapshot::FORMAT_BASED);
    assert_eq!(header.pulled_through, p as i64);
    let base = header.base.clone().expect("a based snapshot carries a base");
    assert_eq!(base.ops_through_p, p, "the writer held every op below P");

    let relay = Relay::holding(&ops);
    let mut based = platform_store(3);
    let imported = snapshot::import(&mut based, &bytes, &stream(), &relay).unwrap();
    assert_eq!(imported.marks.pulled_through, p as i64);
    assert_eq!(
        imported.loaded,
        nxs_foundation::image::Loaded::Views,
        "a based snapshot's views are taken, never folded again"
    );
    let mut marks = imported.marks;
    sync_through(&mut based, 3, &mut marks, &relay);
    assert_eq!(marks.pulled_through, ops.len() as i64);

    // THE claim: a snapshot at P plus the tail is a full replay.
    assert_same_state(&based, &replayed, "at half the stream");

    // And it cost no op below P. The import's own anchor check reads position P − 1, which is the
    // one read of the prefix the format makes — nothing is FETCHED below P.
    assert_eq!(
        relay.lowest_pull(),
        p as i64 - 1,
        "the based replica asked for a position below P"
    );
    assert!(
        relay.pulls.borrow().iter().filter(|&&s| s < p as i64).count() == 1,
        "the anchor check is the only read below P: {:?}",
        &relay.pulls.borrow()[..relay.pulls.borrow().len().min(20)]
    );

    // The log it holds is the point of the format: nothing of the dropped domains below P, every
    // op above P, and chat's whole log (spec D9).
    let below: BTreeSet<String> = ops[..p].iter().map(|o| o.op_id.clone()).collect();
    let above: BTreeSet<String> = ops[p..].iter().map(|o| o.op_id.clone()).collect();
    let mine = op_ids_by_domain(&based);
    let theirs = op_ids_by_domain(&replayed);
    for (domain, ids) in &theirs {
        let kept: BTreeSet<String> = ids.intersection(&above).cloned().collect();
        let got = mine.get(domain).cloned().unwrap_or_default();
        assert!(
            kept.is_subset(&got),
            "the based replica is missing {domain} ops from above P"
        );
    }
    let dropped: BTreeSet<&String> = below
        .iter()
        .filter(|id| {
            mine.get("task").is_some_and(|s| s.contains(*id))
                || mine.get("fact").is_some_and(|s| s.contains(*id))
        })
        .collect();
    assert!(
        dropped.is_empty(),
        "{} task/fact ops below P rode along after all",
        dropped.len()
    );
    let chat_below: BTreeSet<String> = theirs["message"].intersection(&below).cloned().collect();
    assert!(
        !chat_below.is_empty() && chat_below.is_subset(&mine["message"]),
        "chat's log below P must ride along in full (spec D9)"
    );
    // And the state really is smaller than the history it stands for.
    assert!(
        header.ops < p,
        "the snapshot carries {} of the {p} ops below its position — it saved nothing",
        header.ops
    );
}

/// The same differential at SEVERAL positions, and with the tail arriving in an order the relay
/// never promised: shuffled and with ops delivered twice, as a relay without deduplication hands
/// them over. Smaller, so the positions can be swept.
#[test]
fn the_differential_holds_at_every_position_and_in_any_arrival_order() {
    for seed in [0x51ice_u64 ^ 0x11, 0x51ice ^ 0x22] {
        let ops = platform_log(seed, 3, 900);
        assert!(ops.len() > 2_000, "{} ops", ops.len());
        let replayed = full_replay(&ops, 1);
        for fraction in [10, 35, 50, 77, 95] {
            let p = ops.len() * fraction / 100;
            let ctx = format!("seed {seed:#x}, P at {fraction}% ({p} of {})", ops.len());
            let bytes = snapshot_at(&ops, p, 2);
            let relay = Relay::holding(&ops);

            // In order, through the pull loop.
            let mut based = platform_store(3);
            let imported = snapshot::import(&mut based, &bytes, &stream(), &relay).unwrap();
            let mut marks = imported.marks;
            sync_through(&mut based, 3, &mut marks, &relay);
            assert_same_state(&based, &replayed, &ctx);

            // Out of order, with duplicates — applied straight to the store, because a cursor
            // cannot deliver a page it has already passed.
            let mut shuffled = platform_store(4);
            snapshot::import(&mut shuffled, &bytes, &stream(), &relay).unwrap();
            let mut tail: Vec<Op> = ops[p..].to_vec();
            let mut r = Lcg(seed ^ p as u64);
            for i in (1..tail.len()).rev() {
                tail.swap(i, r.next(i as u64 + 1) as usize);
            }
            let twice: Vec<Op> = tail.iter().filter(|_| r.next(5) == 0).cloned().collect();
            tail.extend(twice);
            shuffled.apply(&tail);
            assert_same_state(&shuffled, &replayed, &format!("{ctx}, shuffled"));
        }
    }
}

/// The hosted half (spec §4.5): the fold table starts from the SAME bytes, through the projection
/// in this crate, and folds the tail — and holds what the replica holds.
#[test]
fn a_fold_table_started_from_the_same_bytes_holds_what_the_replica_holds() {
    let folder = Folder::platform().unwrap();
    let ops = platform_log(0xf01d, 3, 700);
    let p = ops.len() * 60 / 100;
    let bytes = snapshot_at(&ops, p, 2);
    let (_, image) = snapshot::unpack_image(&bytes).unwrap();
    let projected = nxs_fold_ddb::snapshot::from_image(&image, "based-differential").unwrap();
    assert_eq!(projected.folded_through, p as i64);

    let table = MemTable::new("based-differential");
    ready(nxs_fold_ddb::snapshot::import(
        &table, &folder, &projected,
    ))
    .unwrap();
    // The tail, shuffled and doubled, as a relay hands it over.
    let mut r = Lcg(0xf01d);
    let mut tail: Vec<(i64, Op)> = ops[p..]
        .iter()
        .enumerate()
        .map(|(i, op)| ((p + i) as i64 + 1, op.clone()))
        .collect();
    for i in (1..tail.len()).rev() {
        tail.swap(i, r.next(i as u64 + 1) as usize);
    }
    let twice: Vec<(i64, Op)> = tail.iter().filter(|_| r.next(5) == 0).cloned().collect();
    tail.extend(twice);
    ready(folder.fold_batch(&table, &tail)).unwrap();

    let replayed = full_replay(&ops, 1);
    for view in platform_view_tables() {
        assert_eq!(
            served_rows(&table, &folder, view),
            sqlite_rows(&replayed, view),
            "{view}"
        );
    }
}

/// A with-log snapshot cannot start a fold run: it names no fold revision per domain, so there is
/// nothing to compare the reader's rules against.
#[test]
fn the_fold_projection_refuses_a_with_log_snapshot() {
    let ops = platform_log(0xf01e, 2, 120);
    let relay = Relay::holding(&ops);
    let mut writer = platform_store(1);
    let mut marks = Watermarks::default();
    sync_through(&mut writer, 1, &mut marks, &relay);
    let bytes = snapshot::export(&writer, 1, &stream(), &marks, &relay).unwrap();
    let (_, image) = snapshot::unpack_image(&bytes).unwrap();
    let err = nxs_fold_ddb::snapshot::from_image(&image, "based-differential").unwrap_err();
    assert!(
        matches!(err, nxs_fold_ddb::snapshot::FromImageError::NotBased),
        "{err}"
    );
}

/// **The cap** (spec §4.6, §8): the hosted engine counts every op it pulls since hydration against
/// 30,000, and today a snapshot does not lower that count because it carries the whole log. A based
/// start does — so a stream the engine refuses today is served inside the budget.
#[test]
fn a_based_start_meets_the_hosted_cap_a_full_replay_exceeds() {
    /// What engine-server's `Capped` does: add each page's op count to a running total and refuse
    /// the page that would cross the cap.
    struct Capped<'a> {
        inner: &'a Relay,
        counted: RefCell<usize>,
        cap: usize,
    }
    impl Transport for Capped<'_> {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            unreachable!()
        }
        fn pull(
            &self,
            s: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            let (page, next) = self.inner.pull(s, since, limit)?;
            let mut counted = self.counted.borrow_mut();
            if *counted + page.len() > self.cap {
                return Err(TransportError(format!(
                    "stream too large: {} ops counted, {} more would cross {}",
                    *counted,
                    page.len(),
                    self.cap
                )));
            }
            *counted += page.len();
            Ok((page, next))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Registered)
        }
    }

    const CAP: usize = 30_000;
    let ops = platform_log(0xcapp, 3, 4_700);
    assert!(
        ops.len() > 35_000,
        "the stream must be past the size the engine refuses today: {} ops",
        ops.len()
    );
    let p = ops.len() - 2_000;
    let bytes = snapshot_at(&ops, p, 2);
    let relay = Relay::holding(&ops);

    // Replaying it is refused, exactly as the hosted engine refuses it today.
    let mut replaying = platform_store(3);
    let mut marks = Watermarks::default();
    let capped = Capped {
        inner: &relay,
        counted: RefCell::new(0),
        cap: CAP,
    };
    let refused = engine::sync(
        &mut replaying,
        3,
        &stream(),
        &mut marks,
        &capped,
        5_000,
        &Unbounded,
    );
    assert!(
        refused.is_err(),
        "a full replay of {} ops must cross the {CAP}-op budget",
        ops.len()
    );

    // Starting from the snapshot, what is counted is the residue, the kept log and the tail.
    let header = snapshot::peek(&bytes).unwrap();
    let base = header.base.clone().unwrap();
    let mut based = platform_store(4);
    let imported = snapshot::import(&mut based, &bytes, &stream(), &relay).unwrap();
    let counted_at_start = base.residue + header.ops - base.residue; // every op the file carries
    let capped = Capped {
        inner: &relay,
        counted: RefCell::new(counted_at_start),
        cap: CAP,
    };
    let mut marks = imported.marks;
    loop {
        let before = marks.pulled_through;
        engine::sync(&mut based, 4, &stream(), &mut marks, &capped, 5_000, &Unbounded)
            .unwrap_or_else(|e| panic!("a based start must stay inside the budget: {e}"));
        if marks.pulled_through == before {
            break;
        }
    }
    assert_eq!(marks.pulled_through, ops.len() as i64);
    assert!(
        *capped.counted.borrow() <= CAP,
        "{} ops counted against a cap of {CAP}",
        capped.counted.borrow()
    );
    assert_same_state(&based, &full_replay(&ops, 1), "at the cap");
}
