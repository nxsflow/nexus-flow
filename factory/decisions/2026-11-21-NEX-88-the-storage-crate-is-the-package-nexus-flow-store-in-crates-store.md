# 2026-11-21 · NEX-88 · The storage crate is the package nexus-flow-store, in crates/store

- Decided by: coder on NEX-88
- Why: RFC 0001 names the crate `store` but not its package; `nexus-flow-store` follows the engine's packages `nexus-flow-core` and `nexus-flow-facade`, and the directory `crates/store` keeps the RFC's name, so `cargo build -p nexus-flow-store` builds it and the SDK boundary check (NEX-89) names that package.
- Instead of: a package named plain `store`, which says nothing outside this workspace and collides with common crate names in error messages and `cargo tree`.
- Topics: crates, nocentraldb, v2
