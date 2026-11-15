# 2026-11-15 · no ticket · A full cargo test --all occasionally reds on a facade delete-and-recreate test — that is a CLASS, not one test; isolate, repeat, then judge.

- Decided by: recorded in the project memory (`nxm`, key `facade-watch-test-flaky`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, flaky-tests, facade

## The record

A full `cargo test --all` (debug OR release) occasionally goes red on a `nexus-flow-facade` test
from the **delete-and-recreate family** — a test that throws away the workspace db at the SAME path
and recreates it, and therefore depends on both file-system event timing and SQLite locking under
parallel load.

THIS IS A CLASS, NOT A SINGLE TEST. This entry named only one until 2026-09-01, and that is
precisely what it nearly failed on: the next occurrence was a different test.

Proven occurrences:

- `--test watch` :: `detects_same_path_db_delete_and_recreate` (2026-07-10). A file-system watch
  test; foreign changes were in the tree, so not change-induced.
- `--test engine` :: `subscribed_reads_repoint_after_a_recreate_on_the_watcher_cadence`
  (2026-09-01, `cargo test --all --release`). DIFFERENT SYMPTOM, same causal family:
  `NxfError { kind: Io, msg: "failed to set sqlite pragmas: database is locked" }` out of the
  `ws.open_store()` in the test's OWN helper `recreate_db_with` (`crates/facade/tests/engine.rs`),
  not out of the subject. Searching for the word "watch" does not find this one.

**Why it is a flake, and how to PROVE that rather than assert it:** green when repeated in
isolation (3/3), green for the whole crate alone (`cargo test -p nexus-flow-facade --release`), and
green in CI `quality-gates`. Both times the red run happened in a tree whose change did not touch
`crates/facade` at all.

**How to apply:** if a full run reds on ONLY a test from this family, then (1) repeat that test in
isolation, (2) run the whole crate in isolation, (3) only then judge — and repeat the full run once
if you intend to report a clean gate result ([gate-evidence-is-per-commit](2026-11-15-no-ticket-gate-evidence-is-per-commit.md): a red run out of a
race proves nothing). Do not attribute it to your own diff without having done those three steps. A
real fix would be to serialise the delete-and-recreate tests instead of letting them race against
the same file in parallel.
