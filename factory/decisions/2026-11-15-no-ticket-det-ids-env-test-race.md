# 2026-11-15 · no ticket · NXF_DETERMINISTIC_IDS is process-global — never toggle it in a test that asserts id order; capture the ids and assert against sorted(captured).

- Decided by: recorded in the project memory (`nxm`, key `det-ids-env-test-race`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, flaky-tests

## The record

`NXF_DETERMINISTIC_IDS` (read by `nxs_foundation::workspace::deterministic_ids_enabled()`, used by
chat/flow/memory `mint_id`) is a **process-global env var**. A unit test that does
`set_var`/`remove_var` on it (for example chat's
`deterministic_ids_are_sortable_and_stable_under_the_switch`) races any **parallel** test in the
same test binary that mints ids and asserts a **positional** order — the flag can flip between two
`post_message` calls, so one id is a ULID and the next an `m-<seq>`, inverting their sort order.

**Why it hides:** it passed under `cargo test` (debug) by scheduling luck but went red under
`cargo test --release` (different parallel timing) — a classic release-only flake. It was hit
during the nexus-chat app-facade work: `messages_in_channel_returns_full_rows_in_id_order` asserted
`[m1, m2]` and failed once in the release gate.

**How to apply:** in tests that check "ordered by message_id", never toggle the env and never assert
a hardcoded position — capture the returned ids and assert against `sorted(captured)` (which holds
under any minting scheme), and check row content by `.find(|r| r.body == …)`, not by index. Only
the single-purpose determinism test should touch the env, and it must assert against its own
store's ids (per-store counts do not cross-contaminate). Always run `cargo test --release` before
trusting an order-sensitive test — see [slow-tests-check-target-deps-and-swap](2026-11-15-no-ticket-slow-tests-check-target-deps-and-swap.md) and
[facade-watch-test-flaky](2026-11-15-no-ticket-facade-watch-test-flaky.md).
