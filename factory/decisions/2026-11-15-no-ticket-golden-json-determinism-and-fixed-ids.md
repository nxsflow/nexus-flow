# 2026-11-15 · no ticket · nxf JSON determinism needs serde_json WITHOUT preserve_order — xtask must never enable it, or feature unification breaks the goldens.

- Decided by: recorded in the project memory (`nxm`, key `golden-json-determinism-and-fixed-ids`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, golden, json-contract

## The record

The golden-docs harness rests on two determinism properties, and both are easy to break from far
away.

1. KEY ORDER. `nxf` JSON determinism depends on `serde_json` NOT having the `preserve_order`
   feature: the default `BTreeMap` sorts keys lexicographically, which is the documented contract
   in `output.rs`. `xtask` must NOT enable `serde_json/preserve_order`. It would be a no-op for
   xtask's own struct-based artifacts (`commands.json` and the release-notes feed serialise in
   declaration order anyway) — but cargo FEATURE UNIFICATION leaks it into the `nxf` binary under
   `cargo test --all`, flipping `nxf`'s `Value` key order to insertion order and breaking the
   golden trycmd snapshots.

2. IDENTITY AND IDS. The env var `NXF_DETERMINISTIC_IDS` makes `nxf init` take a fixed identity
   (prefix `ab12`, site_id 1) and makes `create` mint sequential ids (`ab12.0001`…) via
   `ReplicaIds::mint_sequential`. Note that op_ids stay random ULIDs — that is CRDT identity and is
   never seeded — so golden lines carrying one are redacted with trycmd's `[..]` wildcard. The wall
   clock is pinned separately with `NXF_NOW`, because `close`/`archive` stamp their timestamps from
   `now`.

See [golden-harness-runs-in-crate-dir](2026-11-15-no-ticket-golden-harness-runs-in-crate-dir.md) for where the harness runs and what else it pins.
