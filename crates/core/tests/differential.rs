//! Differential oracle. Two complementary properties are validated against Automerge,
//! which is a dev-dependency only and never ships in the binary (port of E0.10):
//!
//!   TEST 1 (`conflict_free_core_matches_automerge_oracle`): drive the SAME
//!   CONFLICT-FREE op sequence (no two replicas ever touch the same cell) through a
//!   pair of nexus-flow-core replicas AND a pair of Automerge replicas. Because the
//!   result is independent of any tiebreak rule, all four observations must be equal —
//!   so we CAN cross-check `core == automerge`. We observe status, title, priority,
//!   deps, and a clean (net-absent) edge remove.
//!
//!   TEST 2 (`concurrent_conflict_converges_within_each_substrate`): a genuine CONFLICT
//!   (two replicas write the SAME cell concurrently, plus a concurrent edge
//!   add-vs-remove). Here the two substrates use DIFFERENT conflict-resolution rules
//!   (Automerge: actor id; core: `(lamport, site)`), so they may pick DIFFERENT winners.
//!   We therefore assert ONLY intra-substrate convergence (replica A == replica B within
//!   each substrate), NOT cross-substrate agreement.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, Value};
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;

const IDS: [&str; 3] = ["g.A", "g.B", "g.C"];

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    statuses: Vec<(String, Option<String>)>,
    titles: Vec<(String, Option<String>)>,
    priorities: Vec<(String, Option<String>)>,
    // 8qv.1: the new LWW longtext fields ride the same substrate, so the oracle observes them
    // alongside the other cells. `design` is exercised via the same per-id offline edits.
    descriptions: Vec<(String, Option<String>)>,
    designs: Vec<(String, Option<String>)>,
    // C1 (#916.1): the `archived` tombstone is an LWW timestamp cell on the same substrate, so the
    // oracle must model it too — otherwise the diff test is blind to it (or silently diverges).
    archived: Vec<(String, Option<String>)>,
    // C3 (#916.4): the `closed_at` instant is another LWW timestamp cell on the same substrate.
    closed_at: Vec<(String, Option<String>)>,
    deps_a: Vec<String>,
    // deps of g.B carries the clean (net-absent) edge: A adds g.B->g.C then removes it,
    // so both substrates must observe it gone.
    deps_b: Vec<String>,
    // sp6.3: the `parent` edge rides the same observed-remove OR-set, so the oracle models it too.
    // parents of g.C — exercised conflict-free (TEST 1, cross-checked vs Automerge) and under a
    // concurrent add-vs-remove (TEST 2, intra-substrate convergence).
    parents_c: Vec<String>,
}

fn observe_core(s: &Store) -> Observation {
    Observation {
        statuses: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.status),
                )
            })
            .collect(),
        titles: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.title),
                )
            })
            .collect(),
        priorities: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.priority),
                )
            })
            .collect(),
        descriptions: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.description),
                )
            })
            .collect(),
        designs: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.design),
                )
            })
            .collect(),
        archived: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.archived),
                )
            })
            .collect(),
        closed_at: IDS
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    s.get_item(id).unwrap().and_then(|i| i.closed_at),
                )
            })
            .collect(),
        deps_a: s.deps_of("g.A").unwrap(),
        deps_b: s.deps_of("g.B").unwrap(),
        parents_c: s.parents_of("g.C"),
    }
}

// ============================================================================
// TEST 1 — conflict-free: core observation must equal the Automerge oracle.
// ============================================================================

/// Conflict-free scenario: no two replicas touch the same cell, so the result is
/// independent of any tiebreak rule — a cross-substrate difference is a genuine bug.
fn run_core_conflict_free() -> (Observation, Observation) {
    let (a, b) = build_core_conflict_free();
    (observe_core(&a), observe_core(&b))
}

/// Build the conflict-free scenario and return both fully-merged replicas (so a caller can also
/// export the converged op-log — used by the aye.36 refold-on-open oracle check).
fn build_core_conflict_free() -> (Store, Store) {
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);

    a.create_item("g.A", "task", "A", "x");
    a.create_item("g.B", "task", "B", "x");
    a.create_item("g.C", "task", "C", "x");
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "x");
    b.apply(&a.export());

    // Disjoint offline edits (no shared cell).
    // Status: each item's status touched by exactly one replica.
    a.set_field("g.A", "status", Some("in_progress".into()), "x");
    b.set_field("g.B", "status", Some("closed".into()), "x");
    // Title: each item titled on exactly one replica.
    a.set_field("g.A", "title", Some("A-from-a".into()), "x");
    b.set_field("g.B", "title", Some("B-from-b".into()), "x");
    a.set_field("g.C", "title", Some("C-from-a".into()), "x");
    // Priority: each item's priority set on exactly one replica.
    a.set_field("g.A", "priority", Some("1".into()), "x");
    b.set_field("g.B", "priority", Some("2".into()), "x");
    a.set_field("g.C", "priority", Some("3".into()), "x");
    // Description + design (8qv.1): each item's longtext cells touched by exactly one replica.
    a.set_field("g.A", "description", Some("why-A".into()), "x");
    b.set_field("g.B", "description", Some("why-B".into()), "x");
    a.set_field("g.A", "design", Some("plan-A".into()), "x");
    b.set_field("g.B", "design", Some("plan-B".into()), "x");
    // Archived timestamp (C1): each item's `archived` cell touched by exactly one replica.
    a.set_field("g.A", "archived", Some("arch-A".into()), "x");
    b.set_field("g.B", "archived", Some("arch-B".into()), "x");
    // Close instant (C3): each item's `closed_at` cell touched by exactly one replica.
    a.set_field("g.A", "closed_at", Some("close-A".into()), "x");
    b.set_field("g.B", "closed_at", Some("close-B".into()), "x");
    // New dep edge, only on a.
    a.add_edge("g.A", "g.C", EdgeKind::Dep, "x");
    // Clean (net-absent) edge: the SAME replica adds then removes g.B->g.C, so both
    // substrates must agree it is gone.
    a.add_edge("g.B", "g.C", EdgeKind::Dep, "x");
    a.remove_edge("g.B", "g.C", EdgeKind::Dep, "x");
    // sp6.3: a `parent` edge (g.C's parent is g.A), only on a — conflict-free, so it must converge
    // to the same observation on both substrates (cross-checked against Automerge in TEST 1).
    a.add_edge("g.C", "g.A", EdgeKind::Parent, "x");

    let ea = a.export();
    let eb = b.export();
    a.apply(&eb);
    b.apply(&ea);
    (a, b)
}

/// aye.36 — refold-on-open correctness, pinned to the oracle. The converged op-log, when it lands
/// in a shared db via a foundation-only writer (a raw substrate store registers NO task reducer,
/// so every op defers) and is then folded by flow's refold-on-open, must rebuild EXACTLY the
/// inline-folded views. Since the inline path already equals Automerge (TEST 1), this proves
/// refold-on-open == inline fold == Automerge: a multi-device view-refresh can never change the
/// converged value.
#[test]
fn refold_on_open_matches_inline_fold_against_the_oracle() {
    let (a, _) = build_core_conflict_free();
    let inline = observe_core(&a);
    let ops = a.export();

    let path = std::env::temp_dir().join(format!("nxf-aye36-oracle-{}.db", ulid::Ulid::new()));
    let p = path.to_str().unwrap();
    {
        // Foundation-only pull: the ops land unfolded in the shared log (no product reducer).
        let mut sub = nxs_foundation::store::Store::open(p, 9).unwrap();
        sub.apply(&ops);
    }
    let refolded = {
        let s = Store::open(p, 9).unwrap(); // refold-on-open rebuilds the views from the log
        observe_core(&s)
    };
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        refolded, inline,
        "refold-on-open rebuilds exactly the inline-folded views"
    );

    let (oracle, _) = run_automerge_conflict_free();
    assert_eq!(
        refolded, oracle,
        "refold-on-open converges to the Automerge oracle"
    );
}

/// The same conflict-free scenario on Automerge maps: items as a map of
/// id -> {status, title, priority}, deps as a map of "from->to" -> bool (false = a
/// removed/absent edge). Conflict-free, so order-independent.
fn run_automerge_conflict_free() -> (Observation, Observation) {
    let mut a = mk_automerge();
    am_set(&mut a, "g.A", "status", "open");
    am_set(&mut a, "g.B", "status", "open");
    am_set(&mut a, "g.C", "status", "open");
    am_set(&mut a, "g.A", "title", "A");
    am_set(&mut a, "g.B", "title", "B");
    am_set(&mut a, "g.C", "title", "C");
    am_add_dep(&mut a, "g.A", "g.B");
    let mut b = a.fork();

    // Disjoint offline edits mirroring run_core_conflict_free.
    am_set(&mut a, "g.A", "status", "in_progress");
    am_set(&mut b, "g.B", "status", "closed");
    am_set(&mut a, "g.A", "title", "A-from-a");
    am_set(&mut b, "g.B", "title", "B-from-b");
    am_set(&mut a, "g.C", "title", "C-from-a");
    am_set(&mut a, "g.A", "priority", "1");
    am_set(&mut b, "g.B", "priority", "2");
    am_set(&mut a, "g.C", "priority", "3");
    am_set(&mut a, "g.A", "description", "why-A");
    am_set(&mut b, "g.B", "description", "why-B");
    am_set(&mut a, "g.A", "design", "plan-A");
    am_set(&mut b, "g.B", "design", "plan-B");
    am_set(&mut a, "g.A", "archived", "arch-A");
    am_set(&mut b, "g.B", "archived", "arch-B");
    am_set(&mut a, "g.A", "closed_at", "close-A");
    am_set(&mut b, "g.B", "closed_at", "close-B");
    am_add_dep(&mut a, "g.A", "g.C");
    // Clean (net-absent) edge: model the dep key then delete it, all on a.
    am_add_dep(&mut a, "g.B", "g.C");
    am_del_dep(&mut a, "g.B", "g.C");
    // sp6.3: the parent edge g.C->g.A, only on a (conflict-free).
    am_add_parent(&mut a, "g.C", "g.A");

    a.merge(&mut b).unwrap();
    b.merge(&mut a.clone()).unwrap();
    (observe_automerge(&a), observe_automerge(&b))
}

#[test]
fn conflict_free_core_matches_automerge_oracle() {
    let (ca, cb) = run_core_conflict_free();
    assert_eq!(ca, cb, "core replicas must converge (conflict-free)");

    let (aa, ab) = run_automerge_conflict_free();
    assert_eq!(aa, ab, "automerge replicas must converge (conflict-free)");

    // Conflict-free ⇒ tiebreak-independent ⇒ the substrates must agree.
    assert_eq!(ca, aa, "core must agree with the automerge oracle");
}

// ============================================================================
// TEST 2 — concurrent conflict: assert convergence WITHIN each substrate only.
// ============================================================================

/// Conflict scenario in core: after a shared baseline, both replicas concurrently set
/// the SAME cell (`g.A` title) to different values, and one replica adds an edge that
/// the other concurrently removes. This exercises the LWW tiebreak and observed-remove
/// under genuine conflict. Returns both replicas' views after a bidirectional merge.
fn run_core_conflict() -> (Observation, Observation) {
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);

    // Shared baseline, then sync to b.
    a.create_item("g.A", "task", "A", "x");
    a.create_item("g.B", "task", "B", "x");
    a.create_item("g.C", "task", "C", "x");
    a.add_edge("g.A", "g.B", EdgeKind::Dep, "x"); // baseline edge, observed by both
    a.add_edge("g.C", "g.A", EdgeKind::Parent, "x"); // baseline parent edge, observed by both
    b.apply(&a.export());

    // Concurrent same-cell LWW conflict: both write g.A.title differently.
    a.set_field("g.A", "title", Some("title-from-a".into()), "alice");
    b.set_field("g.A", "title", Some("title-from-b".into()), "bob");

    // Concurrent edge add-vs-remove on the baseline edge g.A->g.B:
    // a removes the (observed) edge, b concurrently leaves it — and additionally a
    // and b race a second edge g.A->g.C (a adds, b adds then removes its own add).
    a.remove_edge("g.A", "g.B", EdgeKind::Dep, "alice");
    b.add_edge("g.A", "g.C", EdgeKind::Dep, "bob");
    a.add_edge("g.A", "g.C", EdgeKind::Dep, "alice");

    // sp6.3: concurrent parent add-vs-remove. a removes the observed baseline parent (g.C->g.A);
    // a and b both race a new parent g.C->g.B (b unseen by a's remove) — the add must win on both.
    a.remove_edge("g.C", "g.A", EdgeKind::Parent, "alice");
    b.add_edge("g.C", "g.B", EdgeKind::Parent, "bob");
    a.add_edge("g.C", "g.B", EdgeKind::Parent, "alice");

    let ea = a.export();
    let eb = b.export();
    a.apply(&eb);
    b.apply(&ea);
    (observe_core(&a), observe_core(&b))
}

/// The same conflict scenario on Automerge. Winners may differ from core (Automerge
/// resolves by actor id, core by `(lamport, site)`), so this is only used to prove
/// Automerge converges within itself.
fn run_automerge_conflict() -> (Observation, Observation) {
    let mut a = mk_automerge();
    am_set(&mut a, "g.A", "status", "open");
    am_set(&mut a, "g.B", "status", "open");
    am_set(&mut a, "g.C", "status", "open");
    am_set(&mut a, "g.A", "title", "A");
    am_set(&mut a, "g.B", "title", "B");
    am_set(&mut a, "g.C", "title", "C");
    am_add_dep(&mut a, "g.A", "g.B"); // baseline edge
    am_add_parent(&mut a, "g.C", "g.A"); // baseline parent edge
    let mut b = a.fork();

    // Concurrent same-cell conflict on g.A.title.
    am_set(&mut a, "g.A", "title", "title-from-a");
    am_set(&mut b, "g.A", "title", "title-from-b");

    // Concurrent edge add-vs-remove.
    am_del_dep(&mut a, "g.A", "g.B");
    am_add_dep(&mut b, "g.A", "g.C");
    am_add_dep(&mut a, "g.A", "g.C");

    // sp6.3: concurrent parent add-vs-remove (mirrors the core conflict scenario).
    am_del_parent(&mut a, "g.C", "g.A");
    am_add_parent(&mut b, "g.C", "g.B");
    am_add_parent(&mut a, "g.C", "g.B");

    a.merge(&mut b).unwrap();
    b.merge(&mut a.clone()).unwrap();
    (observe_automerge(&a), observe_automerge(&b))
}

#[test]
fn concurrent_conflict_converges_within_each_substrate() {
    let (ca, cb) = run_core_conflict();
    // Core's LWW tiebreak and observed-remove must be CONVERGENT even under conflict.
    assert_eq!(ca, cb, "core replicas must converge under conflict");

    let (aa, ab) = run_automerge_conflict();
    assert_eq!(aa, ab, "automerge replicas must converge under conflict");

    // NOTE: we deliberately do NOT assert `ca == aa` here. Under a genuine conflict the
    // two substrates resolve concurrent same-cell writes by DIFFERENT rules (Automerge:
    // actor id; core: `(lamport, site)`), so they may pick different winners. The
    // property that actually matters — and that this test proves — is that EACH
    // substrate converges to a single value, not that they agree on which one.
}

// ============================================================================
// Automerge oracle helpers (Automerge 0.5.x). Items live under a root "items" map
// (id -> {status, title, priority}); deps under a root "deps" map ("from->to" -> bool,
// false meaning a removed/absent edge).
// ============================================================================

fn mk_automerge() -> AutoCommit {
    let mut d = AutoCommit::new();
    d.put_object(automerge::ROOT, "items", ObjType::Map)
        .unwrap();
    d.put_object(automerge::ROOT, "deps", ObjType::Map).unwrap();
    // sp6.3: parent edges modelled exactly like deps — a "child->parent" -> bool map.
    d.put_object(automerge::ROOT, "parents", ObjType::Map)
        .unwrap();
    d
}

fn am_items(d: &AutoCommit) -> automerge::ObjId {
    d.get(automerge::ROOT, "items").unwrap().unwrap().1
}

fn am_deps(d: &AutoCommit) -> automerge::ObjId {
    d.get(automerge::ROOT, "deps").unwrap().unwrap().1
}

fn am_parents(d: &AutoCommit) -> automerge::ObjId {
    d.get(automerge::ROOT, "parents").unwrap().unwrap().1
}

fn am_set(d: &mut AutoCommit, id: &str, field: &str, val: &str) {
    let it = am_items(d);
    let obj = match d.get(&it, id).unwrap() {
        Some((Value::Object(ObjType::Map), o)) => o,
        _ => d.put_object(&it, id, ObjType::Map).unwrap(),
    };
    d.put(&obj, field, val).unwrap();
}

fn am_add_dep(d: &mut AutoCommit, from: &str, to: &str) {
    let dp = am_deps(d);
    d.put(&dp, format!("{from}->{to}"), true).unwrap();
}

/// Model a clean/observed remove: set the dep key to false. A concurrent add (true) on
/// another replica is itself a same-cell conflict — but TEST 2 never cross-checks the
/// edge against core, and TEST 1's remove is conflict-free (same replica add+remove).
fn am_del_dep(d: &mut AutoCommit, from: &str, to: &str) {
    let dp = am_deps(d);
    d.put(&dp, format!("{from}->{to}"), false).unwrap();
}

fn am_add_parent(d: &mut AutoCommit, child: &str, parent: &str) {
    let pp = am_parents(d);
    d.put(&pp, format!("{child}->{parent}"), true).unwrap();
}

fn am_del_parent(d: &mut AutoCommit, child: &str, parent: &str) {
    let pp = am_parents(d);
    d.put(&pp, format!("{child}->{parent}"), false).unwrap();
}

fn am_str(d: &AutoCommit, obj: &automerge::ObjId, key: &str) -> Option<String> {
    match d.get(obj, key).unwrap() {
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(v) => Some(v.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn am_field(d: &AutoCommit, id: &str, field: &str) -> Option<String> {
    let it = am_items(d);
    match d.get(&it, id).unwrap() {
        Some((Value::Object(ObjType::Map), o)) => am_str(d, &o, field),
        _ => None,
    }
}

fn am_dep_present(d: &AutoCommit, from: &str, to: &str) -> bool {
    let dp = am_deps(d);
    matches!(
        d.get(&dp, format!("{from}->{to}")).unwrap(),
        Some((Value::Scalar(s), _)) if matches!(s.as_ref(), ScalarValue::Boolean(true))
    )
}

fn am_deps_of(d: &AutoCommit, from: &str) -> Vec<String> {
    let mut out = Vec::new();
    for to in IDS {
        if to != from && am_dep_present(d, from, to) {
            out.push(to.to_string());
        }
    }
    out.sort();
    out
}

fn am_parent_present(d: &AutoCommit, child: &str, parent: &str) -> bool {
    let pp = am_parents(d);
    matches!(
        d.get(&pp, format!("{child}->{parent}")).unwrap(),
        Some((Value::Scalar(s), _)) if matches!(s.as_ref(), ScalarValue::Boolean(true))
    )
}

fn am_parents_of(d: &AutoCommit, child: &str) -> Vec<String> {
    let mut out = Vec::new();
    for parent in IDS {
        if parent != child && am_parent_present(d, child, parent) {
            out.push(parent.to_string());
        }
    }
    out.sort();
    out
}

fn observe_automerge(d: &AutoCommit) -> Observation {
    Observation {
        statuses: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "status")))
            .collect(),
        titles: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "title")))
            .collect(),
        priorities: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "priority")))
            .collect(),
        descriptions: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "description")))
            .collect(),
        designs: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "design")))
            .collect(),
        archived: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "archived")))
            .collect(),
        closed_at: IDS
            .iter()
            .map(|id| (id.to_string(), am_field(d, id, "closed_at")))
            .collect(),
        deps_a: am_deps_of(d, "g.A"),
        deps_b: am_deps_of(d, "g.B"),
        parents_c: am_parents_of(d, "g.C"),
    }
}
