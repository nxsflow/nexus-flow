//! Deeper CRDT properties (nexus-flow-nwu, from the E1 final review):
//!
//!   (1) Generic order-independence over ALL fold paths: a rich op pool exercising
//!       item LWW (incl. concurrent same-cell from two sites), the edge OR-set
//!       (add/add/remove + concurrent re-add), a net-absent contributes_to edge, and
//!       the note stream (add + grow-only redaction) must converge to one state under
//!       any apply order. We sample many seeded shuffles (deterministic, no rng dep) and
//!       assert each equals the reference. Generalizes `store::merge_is_order_independent`
//!       (which only checked forward vs reversed).
//!
//!   (3) OR-set interleavings the unit tests didn't cover: add/add/remove (every observed
//!       add-tag is tombstoned → edge gone) and concurrent remove/remove (both replicas
//!       remove the same observed edge → converges, no resurrection, idempotent).
//!
//! (Gap (2) concurrent same-cell vs Automerge is covered by tests/differential.rs TEST 2;
//! gap (4) belongs_to_missing/_deleted by invariant.rs unit tests. This file fills the
//! two genuinely-uncovered gaps.)

use nexus_flow_core::model::{EdgeKind, LinkRelation, LinkWeight, Op};
use nexus_flow_core::store::Store;

// ---- deterministic shuffle (seeded xorshift64, no external dependency) -------

struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// In-place Fisher–Yates using the seeded rng.
fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

// ---- snapshot of the full converged state -----------------------------------

/// A canonical, order-independent rendering of everything the fold paths materialize:
/// items, present edges, and the (non-redacted) note stream per item.
fn snapshot(s: &Store) -> String {
    let mut out = String::new();
    for it in s.list_items().unwrap() {
        out.push_str(&format!("{it:?}\n"));
    }
    let mut stmt = s
        .connection()
        .prepare("SELECT from_id, to_id, kind FROM present_edges ORDER BY from_id, to_id, kind")
        .unwrap();
    let edges: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    out.push_str(&format!("edges={edges:?}\n"));
    // Thread links (nxf 6j6v.8dbe) fold through their own OR-set plus a causally-projected winner
    // per (thread, item) pair, so they belong in the order-independence snapshot too — the
    // projection is exactly the kind of rule a fold-order bug would quietly break.
    let mut stmt = s
        .connection()
        .prepare(
            "SELECT thread_id, item_id, relation, weight FROM present_thread_links \
             ORDER BY thread_id, item_id",
        )
        .unwrap();
    let links: Vec<(String, String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    out.push_str(&format!("thread_links={links:?}\n"));
    for it in s.list_items().unwrap() {
        out.push_str(&format!(
            "notes[{}]={:?}\n",
            it.id,
            s.notes_of(&it.id).unwrap()
        ));
    }
    out
}

fn dedup_by_op_id(ops: Vec<Op>) -> Vec<Op> {
    let mut seen = std::collections::HashSet::new();
    ops.into_iter()
        .filter(|o| seen.insert(o.op_id.clone()))
        .collect()
}

/// A pool of ops touching every fold path, with genuine cross-site concurrency.
fn rich_pool() -> Vec<Op> {
    let mut a = Store::open_in_memory(1);
    a.create_item("g.A", "task", "A", "u");
    a.create_item("g.B", "task", "B", "u");
    let mut b = Store::open_in_memory(2);
    b.apply(&a.export()); // both replicas observe A and B

    // Concurrent same-cell LWW (two sites write g.A.title without seeing each other),
    // plus disjoint field writes.
    a.set_field("g.A", "title", Some("from-a".into()), "u");
    b.set_field("g.A", "title", Some("from-b".into()), "u");
    a.set_field("g.A", "status", Some("closed".into()), "u");
    b.set_field("g.B", "priority", Some("2".into()), "u");

    // OR-set: two adds of the same element on a, then a remove that observes both;
    // b concurrently re-adds the same element with an unseen tag (add wins).
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "u");
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "u");
    a.remove_edge("g.A", "g.B", EdgeKind::Dep, "u");
    b.add_edge("g.A", "g.B", EdgeKind::Dep, "u");

    // Net-absent contributes_to edge (same replica add then remove).
    a.add_edge("g.A", "g.B", EdgeKind::ContributesTo, "u");
    a.remove_edge("g.A", "g.B", EdgeKind::ContributesTo, "u");

    // Reference (mentions) edge through the same OR-set path: a net-present edge with a
    // concurrent same-element re-add from the other site (distinct tags → survives). Folds
    // the new edge kind under every permutation alongside the rest.
    a.add_edge("g.B", "g.A", EdgeKind::Mentions, "u");
    b.add_edge("g.B", "g.A", EdgeKind::Mentions, "u");

    // Thread links (nxf 6j6v.8dbe): the third OR-set, with the attribute projection on top.
    // `a` attaches the pair and then FIRMS IT UP (a later add on the same pair must win in place);
    // `b` concurrently attaches the same pair with a third combination it never saw — so the
    // converged winner is decided purely by the (lamport, site, tag) projection, which is precisely
    // what has to come out identical under every permutation.
    a.add_thread_link("m-1", "g.A", LinkRelation::Cited, LinkWeight::Passing, "u");
    a.add_thread_link(
        "m-1",
        "g.A",
        LinkRelation::WorkedOn,
        LinkWeight::Bearing,
        "u",
    );
    b.add_thread_link("m-1", "g.A", LinkRelation::Cited, LinkWeight::Bearing, "u");
    // A net-ABSENT link (same replica attach then detach), mirroring the contributes_to case above.
    a.add_thread_link(
        "m-2",
        "g.B",
        LinkRelation::WorkedOn,
        LinkWeight::Bearing,
        "u",
    );
    a.remove_thread_link("m-2", "g.B", "u");

    // Note stream: two adds, one redaction (grow-only tombstone).
    let n1 = a.add_note("g.A", "note-1", "u");
    a.add_note("g.A", "note-2", "u");
    a.redact_note(&n1, "u");

    let mut pool = a.export();
    pool.extend(b.export());
    dedup_by_op_id(pool)
}

#[test]
fn apply_is_order_independent_over_all_fold_paths() {
    let pool = rich_pool();
    let reference = {
        let mut s = Store::open_in_memory(99);
        s.apply(&pool);
        snapshot(&s)
    };

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for n in 0..256 {
        let mut shuffled = pool.clone();
        shuffle(&mut shuffled, &mut rng);
        let mut s = Store::open_in_memory(99);
        s.apply(&shuffled);
        assert_eq!(
            snapshot(&s),
            reference,
            "permutation {n} diverged from the reference state"
        );
    }
}

#[test]
fn add_add_then_remove_clears_the_edge() {
    let mut a = Store::open_in_memory(1);
    a.create_item("g.A", "task", "A", "u");
    a.create_item("g.B", "task", "B", "u");
    // Two adds of the same element → two distinct live add-tags.
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "u");
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "u");
    // A single remove must tombstone BOTH observed tags, not just one.
    a.remove_edge("g.A", "g.B", EdgeKind::Dep, "u");
    assert!(
        a.deps_of("g.A").unwrap().is_empty(),
        "remove must clear every observed add-tag"
    );

    // And it stays gone after delivery to a fresh replica.
    let mut b = Store::open_in_memory(2);
    b.apply(&a.export());
    assert!(
        b.deps_of("g.A").unwrap().is_empty(),
        "edge stays gone after merge"
    );
}

#[test]
fn concurrent_remove_remove_converges_without_resurrection() {
    let mut a = Store::open_in_memory(1);
    a.create_item("g.A", "task", "A", "u");
    a.create_item("g.B", "task", "B", "u");
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "u");

    let mut b = Store::open_in_memory(2);
    b.apply(&a.export()); // both observe the same single add-tag

    // Both replicas remove the same observed edge concurrently.
    a.remove_edge("g.A", "g.B", EdgeKind::Dep, "alice");
    b.remove_edge("g.A", "g.B", EdgeKind::Dep, "bob");

    let ea = a.export();
    let eb = b.export();
    a.apply(&eb);
    b.apply(&ea);

    assert!(a.deps_of("g.A").unwrap().is_empty(), "no resurrection on a");
    assert!(b.deps_of("g.A").unwrap().is_empty(), "no resurrection on b");

    // Re-applying the foreign remove is idempotent — still gone.
    a.apply(&eb);
    assert!(a.deps_of("g.A").unwrap().is_empty(), "remove is idempotent");
}
