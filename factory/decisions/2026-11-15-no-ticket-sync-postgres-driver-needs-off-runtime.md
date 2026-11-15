# 2026-11-15 · no ticket · The sync postgres driver owns its own runtime: inside async it panics, in a Drop it aborts — wrap every entry point, the Drop included.

- Decided by: recorded in the project memory (`nxm`, key `sync-postgres-driver-needs-off-runtime`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: relay, postgres, async

## The record

TRAP: the SYNCHRONOUS postgres driver (crate `postgres` 0.19, with r2d2/r2d2_postgres) owns its OWN
tokio runtime and calls `block_on` on the CALLING thread. In an async context tokio then panics
with "Cannot start a runtime from within a runtime" — and in a DESTRUCTOR that becomes an
unwindable abort (SIGABRT).

CONSEQUENCE, discovered 2026-08-07: the relay's Postgres backend had NEVER worked in the `nxf-relay`
binary. `main` is `#[tokio::main]`, so `NXF_RELAY_BACKEND=postgres` already broke at boot in
`backends()`, and every axum handler would have done the same. Not on a rare path — on the only
one.

WHY NO TEST SAW IT: `crates/server/tests/postgres_parity.rs` calls the trait from plain `#[test]`
threads, which have NO runtime at all. The parity was genuine and the binary was dead anyway.
Exactly the same blind spot as the engine-seam findings: thoroughly tested on a path the product
does not take (see [engine-seam-test-rule](2026-11-15-no-ticket-engine-seam-test-rule.md)). Anyone hanging a sync driver behind an async server
MUST write a test that runs INSIDE a runtime context — `tests/postgres_in_runtime.rs` is the
template (multi-thread, current-thread, and an embedded relay over real HTTP).

THE SOLUTION IN THE REPO: `store_pg::off_runtime(f)` picks the context in which blocking is allowed
— `block_in_place` on multi-thread, a scoped OS thread on current-thread (where `block_in_place` is
itself forbidden), and directly when there is no runtime. From a `spawn_blocking` thread
`block_in_place` is demonstrably fine (unit-tested). EVERY entry point is wrapped: `connect`,
`append`, `read_since`, `register`. AND — easy to forget — the DROP: `PoolHandle(Option<PgPool>)`
hands the pool to `off_runtime` in `Drop`, because closing the connections is the same blocking
driver call, and a clean shutdown otherwise became a SIGABRT.

The DynamoDB backend never had this problem: `store_ddb::block_on` bridges via its own bridge
runtime plus a channel, with the same flavour switch. Anyone building a third backend copies one of
the two patterns — not neither.
