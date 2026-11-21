# 2026-11-21 · NEX-204 · v2's seam and binary use temporary names until the cut-over

- Decided by: Carsten
- Why: One workspace cannot hold two packages named `nexus-flow-facade` or two `nxs` binaries, and v1's are still on `v2`. v2's seam is built as `crates/seam` and its binary as `crates/nxs-v2`; one cut-over ticket (NEX-194) renames them to `nexus-flow-facade` and `nxs` and removes v1's. v1 keeps working on `v2` until then, and the weekly merge of `main` into `v2` stays easy. Written into RFC 0001, "The library seam".
- Instead of: removing v1's facade and binary from `v2` first, which leaves `v2` without a working `nxs` and turns every merge of `main` into modify/delete conflicts.
- Topics: architecture, crates
