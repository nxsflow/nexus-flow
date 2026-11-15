# 2026-11-15 · no ticket · The trycmd golden harness runs in crates/cli with a pinned $HOME — a command that persists state outside the workspace cannot be a golden case.

- Decided by: recorded in the project memory (`nxm`, key `golden-harness-runs-in-crate-dir`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, golden

## The record

The trycmd golden harness (`crates/cli/tests/golden.rs`) runs `nxf` in the **`crates/cli` working
directory, not a sandbox**. That is why `crates/cli/.nxs` exists on disk: a real workspace, seeded
with the fixed identity `NXF_DETERMINISTIC_IDS` hands out (prefix `ab12`), ignored twice over — by
the repo-level `.gitignore` entry `.nxs` and by its own generated `.nxs/.gitignore` containing
`*`. (It was called `.nexusflow` before the workspace directory was renamed; anything still saying
so is stale.)

CONSEQUENCE: a golden case for a command that PERSISTS state OUTSIDE the workspace and has no
reset step is not reproducible across runs — `nxf setup claude` writes `.claude/settings.json`, so
the first run reports `hook_added: true` and the next `false` — and it also leaves an un-ignored
artifact behind. Such host-config commands are listed in `EXEMPT` in `xtask/src/docs.rs` (as of
2026-09-05 that list is exactly `["setup claude"]`) and are covered by hermetic `TempDir`
integration tests instead. Note that host setup now lives on the umbrella — `nxs setup` is the
canonical verb and `nxf setup claude` delegates to it.

TWO GUARDS THE HARNESS GREW SINCE, both of which a new golden suite needs as well:

- **It pins `$HOME`.** A golden corpus asserts stdout AND stderr byte for byte, so anything the
  binary reports about the developer's own machine — a dangling program alias, a background
  service registration — would fail every case, and under `TRYCMD=overwrite` would be burned into
  the committed goldens, developer paths and all. `nxs_test_support::pinned_home_env()` supplies
  the pairs.
- **It refuses a stale binary FIRST.** `nxs_test_support::assert_multicall_binary_fresh()` runs
  before trycmd touches a single case, because a package-scoped run leaves `nxs` unbuilt and an
  old binary under `TRYCMD=overwrite` would rewrite the goldens from stale output. See
  [build-test](2026-11-15-no-ticket-build-test.md) for the whole stale-binary trap.

The clock is pinned too (`NXF_NOW`), because `close`/`archive` stamp their timestamps from `now`
and would otherwise capture the live wall clock.
