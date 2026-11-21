# 2026-11-21 · NEX-212 · The v2 skeletons are the packages nexus-flow-seam, nxs-v2, nexus-flow-merge-gate and nexus-flow-migrate

- Decided by: coder on NEX-212
- Why: RFC 0001 and NEX-204 name the directories `crates/seam` and `crates/nxs-v2` but not the packages. The libraries follow `nexus-flow-store` (NEX-88): `nexus-flow-seam`, `nexus-flow-merge-gate`, `nexus-flow-migrate`, each in the directory its RFC names. The binary's package and binary are both `nxs-v2`, as v1's `nxs` package builds the binary `nxs`. NEX-194 renames the seam and the binary at the cut-over.
- Instead of: plain `seam`, `merge-gate` and `migrate`, which say nothing outside this workspace and collide with common crate names in `cargo tree`.
- Topics: crates, v2
