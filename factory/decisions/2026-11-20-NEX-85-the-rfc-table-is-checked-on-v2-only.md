# 2026-11-20 · NEX-85 · The RFC table is generated, and checked on v2 only

- Decided by: coder on NEX-85
- Why: When every RFC pull request added a row to `docs/rfc/README.md`, each merge put the other open RFC pull requests in conflict, and the fix dismissed the owner's approval. Now `cargo xtask rfc-index` writes the table from the RFC files, an RFC pull request leaves README.md alone, and `cargo xtask rfc-index --check` runs in the `lint` job on a push to `v2` only. On a pull request the table is stale by design. After a merge that adds or changes an RFC, `v2` stays red until a pull request with the regenerated table lands.
- Instead of: a CI step that regenerates and commits the table after each merge. The `v2` ruleset lets no account bypass the pull request, and `docs/rfc/` is code-owned, so any regeneration has to be a pull request the owner approves.
- Topics: rfc, ci, factory
