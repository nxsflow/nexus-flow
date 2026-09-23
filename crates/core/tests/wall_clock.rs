//! Local ops carry the caller-supplied wall-clock once it is set, and stay blank by default.
//!
//! `wall_clock` is display-only — it never participates in ordering or folding, which is the
//! Lamport/site pair (see `history`). It was reserved-but-unset until the embedding-app write
//! paths (E5 #9t7.5) gained an explicit `now`: the facade threads that `now` into the store so a
//! long-lived host stamps its ops with a deterministic, caller-controlled time rather than
//! leaving them blank. This pins the seam in the core: the default is unchanged (blank), and a
//! set wall-clock lands on every subsequently emitted LOCAL op.

use nexus_flow_core::store::Store;

#[test]
fn local_ops_are_unstamped_by_default() {
    // No wall-clock source wired: every existing caller (the whole test suite, the CLI before
    // the facade threads `now`) keeps emitting blank-stamped ops, byte-for-byte as before.
    let mut store = Store::open_in_memory(1);
    store.create_item("ab12.0001", "task", "A", "u");
    store.set_field("ab12.0001", "priority", Some("1".into()), "u");
    assert!(
        store.export().iter().all(|op| op.wall_clock.is_empty()),
        "with no wall clock set, local ops stay blank (the unchanged default)"
    );
}

#[test]
fn set_wall_clock_stamps_every_subsequent_local_op() {
    const NOW: &str = "2026-06-17T08:30:00Z";
    let mut store = Store::open_in_memory(1);
    store.set_wall_clock(NOW);
    // A single create emits several ops (type/title/status); a follow-up set emits one more.
    store.create_item("ab12.0001", "task", "A", "u");
    store.set_field("ab12.0001", "priority", Some("1".into()), "u");
    store.add_note("ab12.0001", "a note", "u");
    let ops = store.export();
    assert!(ops.len() >= 4, "create + set + note emit several ops");
    assert!(
        ops.iter().all(|op| op.wall_clock == NOW),
        "every local op emitted after set_wall_clock carries that timestamp"
    );
}
