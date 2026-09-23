//! Cross-replica determinism and body convergence (E2.17).
//!
//! These drive `nexus-flow-core` directly to simulate two replicas with distinct
//! site ids exchanging ops — the substrate the CLI writes onto. The body test is a
//! *convergence* test ("both op orders → same materialized value"), deliberately NOT a
//! byte-equality test, so the later Yrs backend (ra0) does not inherit a wrongly-shaped
//! golden test.

use nexus_flow_core::derive;
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;

const NOW: &str = "2030-01-01T00:00:00Z";

#[test]
fn derivation_is_identical_across_replicas_regardless_of_op_order() {
    // Replica 1 builds a small graph: A depends on B.
    let mut r1 = Store::open_in_memory(1);
    r1.create_item("p.A", "bug", "A", "u1");
    r1.create_item("p.B", "bug", "B", "u1");
    r1.add_edge("p.A", "p.B", EdgeKind::Dep, "u1");
    let ops = r1.export();

    // Replicas on other sites apply the same ops in other orders: reversed, and every rotation of
    // both directions. Not all 7! orders (5040 stores) — but no longer the single reversal the name
    // used to rest on (review of PR #482, Test Quality #2).
    let mut orders = Vec::new();
    for shift in 0..ops.len() {
        let mut rotated = ops.clone();
        rotated.rotate_left(shift);
        orders.push(rotated.clone());
        rotated.reverse();
        orders.push(rotated);
    }
    for (n, order) in orders.iter().enumerate() {
        let mut r2 = Store::open_in_memory(2 + n as i64);
        r2.apply(order);
        assert_eq!(
            derive::ready(r1.connection(), NOW),
            derive::ready(r2.connection(), NOW),
            "ready must match across replicas (order #{n})"
        );
        assert_eq!(
            derive::blocked(r1.connection()),
            derive::blocked(r2.connection()),
            "blocked must match across replicas (order #{n})"
        );
        assert_eq!(
            derive::in_progress(r1.connection(), NOW),
            derive::in_progress(r2.connection(), NOW),
            "in_progress must match across replicas (order #{n})"
        );
    }
}

#[test]
fn description_converges_regardless_of_op_order() {
    // Both replicas know the item.
    let mut r1 = Store::open_in_memory(1);
    r1.create_item("p.X", "bug", "X", "u1");
    let base = r1.export();
    let mut r2 = Store::open_in_memory(2);
    r2.apply(&base);

    // Concurrent longtext writes from each replica.
    r1.set_field("p.X", "description", Some("from r1".into()), "u1");
    r2.set_field("p.X", "description", Some("from r2".into()), "u2");
    let from_r1 = r1.export();
    let from_r2 = r2.export();

    // Exchange ops; each replica meets the other's write after its own.
    r1.apply(&from_r2);
    r2.apply(&from_r1);

    let b1 = r1.get_item("p.X").unwrap().unwrap().description;
    let b2 = r2.get_item("p.X").unwrap().unwrap().description;
    assert_eq!(
        b1, b2,
        "LWW description must converge to the same materialized value on both replicas"
    );
    assert!(b1.is_some());

    // …and "regardless of op order" means EVERY order a replica could meet these ops in, not the
    // one exchange above (review of PR #482, Test Quality #2). The union is five ops, so all 120
    // orders are cheap to fold.
    let mut union = from_r1.clone();
    union.extend(from_r2.into_iter().filter(|o| !from_r1.contains(o)));
    let mut outcomes = std::collections::BTreeSet::new();
    for order in permutations(&union) {
        let mut r = Store::open_in_memory(9);
        r.apply(&order);
        outcomes.insert(r.get_item("p.X").unwrap().unwrap().description);
    }
    assert_eq!(
        outcomes.into_iter().collect::<Vec<_>>(),
        vec![b1],
        "every order folds to the one value both replicas agreed on"
    );
}

/// Every ordering of `items` (Heap's algorithm) — for the handful of ops a test exchanges.
fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    fn heap<T: Clone>(k: usize, a: &mut [T], out: &mut Vec<Vec<T>>) {
        if k <= 1 {
            out.push(a.to_vec());
            return;
        }
        heap(k - 1, a, out);
        for i in 0..k - 1 {
            if k.is_multiple_of(2) {
                a.swap(i, k - 1);
            } else {
                a.swap(0, k - 1);
            }
            heap(k - 1, a, out);
        }
    }
    let mut out = Vec::new();
    heap(items.len(), &mut items.to_vec(), &mut out);
    out
}
