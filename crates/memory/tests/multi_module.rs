//! P2 acceptance (nexus-flow-aye.23): the first real test of the "multiple products, ONE DB"
//! thesis (spec §6/§7). nxm and nxf share ONE `.nxs/` op-log; fact and task ops coexist; each
//! reducer folds only its domain (foreign domain: store-don't-fold, no cross-contamination); and a
//! fact op survives the SHARED sync wire with its domain intact and folds at the peer.
//!
//! These are the only tests in this crate that reference flow — as DEV-deps. The memory LIBRARY
//! stays flow-free (spec §4.4); only this cross-product proof references both products.

use nexus_flow_core::store::Store as FlowStore;
use nexus_memory::model::Scope;
use nexus_memory::store::{MemoryQuery, MemoryStore};
use nxs_foundation::model::Op;
use nxs_sync::wire::WireOp;

fn tmp_db() -> String {
    std::env::temp_dir()
        .join(format!("nxs-p2-{}.db", ulid::Ulid::new()))
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn fact_and_task_coexist_in_one_op_log_without_cross_contamination() {
    let path = tmp_db();

    // Process 1 — flow writes a task into the shared workspace db.
    {
        let mut flow = FlowStore::open(&path, 1).unwrap();
        flow.create_item("t.1", "task", "Write the CLI", "alice");
    }
    // Process 2 — memory writes a fact into the SAME db (a separate `nxm` invocation; it recovers
    // the Lamport clock past flow's ops on open, so the two products never collide).
    {
        let mut mem = MemoryStore::open(&path, 1).unwrap();
        mem.set_wall_clock("2026-06-20T10:00:00Z");
        mem.remember("auth-jwt", "auth uses JWT", "alice");
    }

    // Both domains live in ONE op-log.
    {
        let flow = FlowStore::open(&path, 1).unwrap();
        assert_eq!(
            flow.op_count(),
            4,
            "task (type+title+status = 3) + fact (1) coexist in one log"
        );
        // flow's view holds its task — and the fact's key never leaks into flow's items
        // (flow's task reducer store-don't-folds the fact domain).
        assert!(flow.get_item("t.1").unwrap().is_some(), "the task survives");
        assert!(
            flow.get_item("auth-jwt").unwrap().is_none(),
            "no fact key contaminates flow's items"
        );
    }
    // memory's view holds its fact — and the task id never leaks into memories.
    {
        let mem = MemoryStore::open(&path, 1).unwrap();
        assert_eq!(
            mem.recall("auth-jwt").unwrap().unwrap().body.as_deref(),
            Some("auth uses JWT")
        );
        assert!(
            mem.get("t.1").unwrap().is_none(),
            "no task id contaminates the memories view"
        );
        assert_eq!(
            mem.memories(&MemoryQuery::default()).unwrap().len(),
            1,
            "exactly the one fact — no task cross-contamination"
        );
    }

    std::fs::remove_file(&path).ok();
}

#[test]
fn a_fact_op_survives_the_shared_sync_wire_and_folds_at_the_peer() {
    // Replica A writes a fact (memory) and a task (flow) — a mixed-domain log.
    let mut a_mem = MemoryStore::open_in_memory(1);
    a_mem.set_wall_clock("2026-06-20T10:00:00Z");
    a_mem.remember("auth-jwt", "auth uses JWT", "alice");

    let task_ops = {
        let mut f = FlowStore::open_in_memory(1);
        f.create_item("t.1", "task", "T", "alice");
        f.export()
    };
    let mut all_ops = a_mem.export();
    all_ops.extend(task_ops);

    // Round-trip every op through the ACTUAL shared sync wire (the JSON envelope the relay
    // forwards): Op → WireOp → JSON → WireOp → Op. domain rides the wire since aye.1.4.
    let round_tripped: Vec<Op> = all_ops
        .iter()
        .map(|op| {
            let wire = WireOp::from(op);
            let json = serde_json::to_string(&wire).expect("wire serializes");
            let back: WireOp = serde_json::from_str(&json).expect("wire deserializes");
            Op::from(back)
        })
        .collect();

    // The fact op kept its domain across the wire — never coerced to task, never dropped.
    assert!(
        round_tripped
            .iter()
            .any(|o| o.domain == "fact" && o.target_id == "auth-jwt"),
        "the fact op survives the wire with domain=fact"
    );

    // Peer replica B (a fresh memory store) merges the wire ops → folds the fact into its memories
    // view. This is the nxs edge over bd's per-repo Dolt: one shared sync, synchronized across
    // machines (spec §6).
    let mut b_mem = MemoryStore::open_in_memory(2);
    b_mem.apply(&round_tripped);
    assert_eq!(
        b_mem.recall("auth-jwt").unwrap().unwrap().body.as_deref(),
        Some("auth uses JWT"),
        "the fact folded at the peer after a sync round-trip"
    );
    // The task op rode the same wire but is store-don't-fold for memory — no contamination.
    assert!(
        b_mem.get("t.1").unwrap().is_none(),
        "the task never folds into memories"
    );
    assert_eq!(b_mem.memories(&MemoryQuery::default()).unwrap().len(), 1);
}

#[test]
fn the_peer_memory_view_converges_with_the_origin() {
    // A fuller convergence check: A remembers, updates in place, and forgets across keys; B receives
    // the whole log over the wire and must materialize the identical memories view.
    let mut a = MemoryStore::open_in_memory(1);
    a.remember("auth-jwt", "auth uses JWT", "alice");
    a.remember("auth-jwt", "auth uses JWT (HS256)", "alice"); // in-place update
    a.remember("race", "run tests with -race", "alice");
    a.forget("race", "alice"); // reversible tombstone

    let wire: Vec<Op> = a
        .export()
        .iter()
        .map(|op| Op::from(WireOp::from(op)))
        .collect();

    let mut b = MemoryStore::open_in_memory(2);
    b.apply(&wire);
    assert_eq!(
        b.memories(&MemoryQuery::default()).unwrap(),
        a.memories(&MemoryQuery::default()).unwrap(),
        "the peer's memories view converges with the origin"
    );
    assert_eq!(
        b.recall("auth-jwt").unwrap().unwrap().body.as_deref(),
        Some("auth uses JWT (HS256)")
    );
    assert!(
        b.recall("race").unwrap().is_none(),
        "the forget converged too"
    );
}

#[test]
fn a_memorys_classification_and_order_cross_the_wire_too() {
    // 6j6v.e0z6: the four new registers are ordinary fact ops that differ only in their `field`,
    // which the wire carries verbatim. So filing a memory on one machine has to show up filed on
    // the next — otherwise the classification would be a local annotation rather than part of the
    // memory, and a second device would quietly disagree about where a memory belongs.
    let mut a = MemoryStore::open_in_memory(1);
    a.remember(
        "two-levels",
        "the two board levels are the load-bearing idea",
        "alice",
    );
    a.set_category("two-levels", "architecture", "alice");
    a.set_scope("two-levels", Scope::Global, "alice");
    a.set_refs(
        "two-levels",
        &["6j6v.5gvj".to_string(), "6j6v.e0z6".to_string()],
        "alice",
    );
    a.set_ordinal("two-levels", 1, "alice");

    let wire: Vec<Op> = a
        .export()
        .iter()
        .map(|op| Op::from(WireOp::from(op)))
        .collect();
    assert!(
        wire.iter()
            .any(|o| o.field == "category" && o.domain == "fact"),
        "a classification op keeps its field AND its domain across the wire"
    );

    let mut b = MemoryStore::open_in_memory(2);
    b.apply(&wire);
    assert_eq!(
        b.memories(&MemoryQuery::default()).unwrap(),
        a.memories(&MemoryQuery::default()).unwrap(),
        "the peer agrees about where the memory is filed, not just what it says"
    );
    let peer = b.recall("two-levels").unwrap().unwrap();
    assert_eq!(peer.category, "architecture");
    assert_eq!(peer.scope, "global");
    assert_eq!(peer.refs, ["6j6v.5gvj", "6j6v.e0z6"]);
    assert_eq!(peer.ordinal, Some(1));
}
