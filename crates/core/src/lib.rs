//! nexus-flow-core — the engine's meaning-free core.
//! Op-log CRDT substrate, universal slots, pure derivation, invariants. See
//! docs/specs/E1-core-data-model.md.

pub mod derive;
pub mod history;
pub mod id;
pub mod invariant;
pub mod model;
pub mod reducer;
pub mod rewrite;
pub mod schema;
pub mod store;
pub mod task_reducer;
