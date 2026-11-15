# 2026-11-15 · no ticket · crates/sync constructs core Op {} literals, so changing the Op struct is not confined to crates/core — grep the whole workspace before touching it.

- Decided by: recorded in the project memory (`nxm`, key `op-struct-external-constructors`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: core, sync, crdt

## The record

Changing the core `Op` struct (`crates/core/src/model.rs`) is NOT confined to `crates/core`:
`crates/sync/src/wire.rs` constructs `nexus_flow_core::model::Op { .. }` literals (in
`From<WireOp> for Op`, plus a test literal), gated behind the sync crate's `engine` feature.
Adding or removing an `Op` field there is a compile break, and `cargo test` builds the WHOLE
workspace — so always `grep -rn 'model::Op\|Op {' crates` (not just `crates/core`) before touching
`Op`.

Facade, CLI and server do NOT construct `core::Op` (the server has its own `WireOp`).

The wire envelope (`WireOp`) is the forward-compatibility seam: it must MIRROR new `Op` fields or
they are lost over sync. Verified 2026-06-20, and re-verified 2026-09-05 that the two `From`
implementations are still there.
