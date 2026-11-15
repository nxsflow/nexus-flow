# 2026-11-15 · no ticket · Never run the lean build while tests run — it overwrites target/debug/nxs and the MCP tests fail as if the code were broken.

- Decided by: recorded in the project memory (`nxm`, key `no-concurrent-lean-build-during-tests`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: build, testing

## The record

In nexus-flow, never run `cargo build -p nxs --no-default-features` (the lean variant) **at the same
time** as `cargo test`. The lean build overwrites `target/debug/nxs` with an MCP-less binary, and
the MCP integration tests in `crates/nxs/tests/mcp.rs` spawn the REAL binary via
`assert_cmd::cargo_bin("nxs")` over stdio. If the lean binary is on disk when those tests run, they
all fail with `error: unrecognized subcommand 'mcp'` — which looks like a code break but is just a
clobbered artefact.

**Why:** debug and release use separate profile directories (`target/debug` vs `target/release`), so
`cargo test --release` is safe to run alongside `cargo test`. But two `cargo` invocations that both
write `target/debug` (a default-features test plus a no-default-features build) race on the same
`nxs` artefact.

**How to apply:** run feature-variant builds serially with the test suite (let one finish before
starting the other), or build the lean variant into a separate `--target-dir`. After any
`--no-default-features` build, the next plain `cargo test` / `cargo build -p nxs` restores the
MCP-enabled `target/debug/nxs`.

The same collision bites two SESSIONS sharing one checkout, not just two commands — see
[review-session-in-a-shared-checkout-mistakes-the-work-for-its-own](2026-11-15-no-ticket-review-session-in-a-shared-checkout-mistakes-the-work-for-its-own.md).
