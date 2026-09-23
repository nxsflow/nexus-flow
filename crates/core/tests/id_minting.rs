//! Minting short ids through the real store predicate. The suffix space is small
//! (32^4 ≈ 1.05M), so without the collision-retry in `mint_unique` a few thousand
//! creates would alias. This guards that the CLI's mint predicate
//! (`store.get_item(id).unwrap().is_some()`) actually secures within-replica uniqueness.

use nexus_flow_core::id::ReplicaIds;
use nexus_flow_core::store::Store;

#[test]
fn minting_through_the_store_predicate_yields_distinct_ids() {
    let mut store = Store::open_in_memory(1);
    let ids = ReplicaIds::new("ab12");

    let mut created = Vec::new();
    for n in 0..3000 {
        let id = ids
            .mint_unique(|cand| store.get_item(cand).unwrap().is_some())
            .expect("suffix space not exhausted at 3k items");
        store.create_item(&id, "task", &format!("t{n}"), "u1");
        created.push(id);
    }

    // Not flaky despite random suffixes: `mint_unique` only ever returns a locally-free
    // suffix (or None), so distinctness holds by construction. The sole statistical event
    // is the `expect` above firing — all 64 tries colliding at ~0.3% occupancy
    // (3k/1.05M) is ~0.003^64 ≈ 0. No seed needed.
    let distinct: std::collections::HashSet<_> = created.iter().collect();
    assert_eq!(distinct.len(), created.len(), "every minted id is unique");
    // A freshly minted id is genuinely seen as taken afterwards.
    assert!(store.get_item(&created[0]).unwrap().is_some());
}
