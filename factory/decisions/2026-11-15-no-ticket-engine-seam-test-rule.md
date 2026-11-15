# 2026-11-15 · no ticket · A new public Engine::* seam is not done without a test AT THAT SEAM — CLI or facade coverage does not count, because the products speak Engine::*.

- Decided by: recorded in the project memory (`nxm`, key `engine-seam-test-rule`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, library-seam

## The record

A new public library seam (`Engine::*`) is NOT done without a test AT THAT SAME SEAM. CLI coverage
does not count for it: tests going through `facade::*` or an `nxm`/`nxf` subprocess exercise the
path the PRODUCTS DO NOT TAKE — manufakt.io and nexflow.it speak `Engine::*`.

Three review rounds in a row (2026-08-03 and 2026-08-04, across the memory-classification work, the
retrieval-rule work and the MCP/NEXUS_MEMORY.md work) produced exactly this as their heaviest
finding: the CLI thoroughly tested, the engine seam not at all.

Whoever adds a new engine command writes the test for it in the same move —
`crates/memory/tests/embed.rs` is the designated place on the memory side.

The same blindness produced a worse failure elsewhere: a sync driver was proven by parity tests
running in plain `#[test]` threads, which have no async runtime, while the shipped binary ran it
inside one and crashed on the only path there was. See
[sync-postgres-driver-needs-off-runtime](2026-11-15-no-ticket-sync-postgres-driver-needs-off-runtime.md) — "thoroughly tested on a path the product does not
take" is the same shape.

An open board item tracks the gate that would enforce this mechanically; until it stands, this rule
carries the load. (The related verb-seam gate already exists and catches a different omission — a
CLI verb with no seam counterpart at all — see [conventions-patterns](2026-11-15-no-ticket-conventions-patterns.md).)
