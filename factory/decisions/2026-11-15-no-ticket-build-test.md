# 2026-11-15 · no ticket · What CI actually runs, and what the local loop cannot see: the guide-topic map, the stale-binary symlink trap, and pruning target/ by SIZE.

- Decided by: recorded in the project memory (`nxm`, key `build-test`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: ci, build, testing

## The record

WHAT THIS MEMORY IS, after 2026-09-18: the inventory of what CI runs, and the traps the local loop
cannot see. It no longer carries the local-gate policy — that is [gates-run-in-ci-not-locally](2026-11-15-no-ticket-gates-run-in-ci-not-locally.md),
which states the rule and the measurement (clippy NEVER locally; only the test file you changed,
plus `cargo fmt --all`) — and it no longer carries the engine architecture, which moved to
[architecture-overview](2026-11-15-no-ticket-architecture-overview.md). The four commands `cargo test`, `cargo test --release`,
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are what CI runs FOR you; this
entry used to call them the fast local loop, and on this machine they are 88 to 122 minutes.

## What CI actually runs

`.github/workflows/ci.yml` is the source of truth. The `quality-gates` job, re-read from the file
on 2026-09-18, in order:

```bash
node --test                                 # the agent-sidecar suite; it has no cargo gate at all
# then: build the sidecar bundle (npm ci && npm run build) and export NXF_EMBED_SIDECAR=require
cargo fmt --all --check                     # --all, not just the default member set
cargo clippy --all-targets -- -D warnings
cargo xtask runners check                   # self-hosted routing stays off pull requests
cargo xtask macos-tests check               # every cfg(macos) test is in the macOS lane
cargo xtask platforms check                 # single-source drift gates: they fail on generated
cargo xtask channels check                  # content that somebody edited by hand
cargo xtask docs check
cargo xtask docs coverage
cargo test --no-fail-fast --test verb_seam \
  -p nexus-flow-cli -p nexus-memory -p nexus-chat   # the omission gate, named so it fails by name
cargo build -p nxs --no-default-features && clippy the same        # lean build 1
cargo build -p nexus-chat --no-default-features && clippy the same # lean build 2
cargo test --all                            # --all: `cargo test` alone misses other crates
cargo test --all --release                  # NOT on the pull-request event, see below
```

Seven jobs sit beside it: `platform-gates-windows`, `platform-gates-macos`, `macOS tests
(hosted)`, `postgres-parity`, `postgres-tls`, `managed-postgres`, `dynamodb-local`. Fourteen
workflow files in all; the ones with their own gate are `changelog-check`, `version-check`,
`facade-semver`, `example-check`, `install-sh-ci`, `install-update-smoke`, `npm-mcp-ci`,
`promote-ci`, `cache-sweep`, `content-ci`, plus `release`, `promote` and `publish-content`.
`cargo xtask changelog check origin/main` is the local form of the first.

**The release profile and macOS are staggered off the pull-request event.** `Tests (release)`,
`platform-gates-macos`, `macOS tests (hosted)` and steps inside `postgres-parity` and
`dynamodb-local` all carry `if: github.event_name != 'pull_request'`: they run on every push to
`main` and to every `release/**` line, and on a manual `workflow_dispatch` — but not on the PR
event, where they were about half the machine time. Nothing that ships skips them (a release is
cut by tagging a commit on `main`/`release/**`), but a release-only break — classically a
`debug_assert!` with a side effect — reaches `main` before it reaches a reviewer. The
countermeasure is NOT a local full run; it is one `workflow_dispatch` of `ci.yml` on the branch
before the merge.

Also: `ci.yml` has a `concurrency` group that cancels superseded **PR** runs (never a
`main`/`release/**` push run) — a force-push mid-run cancels the old run by design, not a flake.
What that hides when it happens is in [macos-ci-jobs-are-two-different-machines](2026-11-15-no-ticket-macos-ci-jobs-are-two-different-machines.md).

**The two lean builds are the ones that actually bite**, because nothing in this repo links those
configurations: `crates/nxs` takes chat's default features, so the `default-features = false`
library variant — the one app-foundations embeds — has NO in-repo consumer and NO local gate. It
compiled only in CI, and it once stayed broken for a day behind six other green checks (`sha2`
moved onto the shared orchestration path while still gated behind chat's `cli` feature). Run at
least the lean builds before pushing anything that moves code between `cli.rs` and a shared
module, or that touches a `[features]` block — and never beside a running test suite
([no-concurrent-lean-build-during-tests](2026-11-15-no-ticket-no-concurrent-lean-build-during-tests.md)).

## The gate the local loop does not cover at all: `content-quality-gates`

`.github/workflows/content-ci.yml` is a SEPARATE workflow with no local counterpart, and it is
fail-closed. It bit on 2026-08-07: adding ONE guide topic in English went green through every Rust
gate and red there, because the website federates guides in **both** locales and asserts topic
parity (`assertTopicParity` in `content/lib/docs.mjs`).

There are **FOUR guide trees**, one per building block plus the umbrella — `crates/cli`,
`crates/memory`, `crates/chat`, `crates/nxs` — all served through the ONE mechanism in
`crates/guide` (`nxs-guide`) and fanned out by `nxs guide`. All four carry content. (The crate's own
description still counts three; 6j6v.b5he.)

Adding or renaming a guide topic touches these places (`<crate>` = `cli` | `memory` | `chat` |
`nxs`):

```
crates/<crate>/src/…/guide.rs           TOPICS       (the block's own list)
crates/<crate>/docs/guide/en/<topic>.md the EN page  (compiled into the binary)
crates/<crate>/docs/guide/de/<topic>.md the DE page  (website-only, but REQUIRED for parity)
crates/cli/tests/guide.rs               TOPICS       (FLOW ONLY — see below)
content/lib/docs.mjs                    BUILDING_BLOCKS[].order  (the site's list, by assertion)
crates/{chat,memory}/tests/golden/*.trycmd  (any ```console block the new page shows)
```

CORRECTED 2026-09-18: the fourth line is NOT a per-crate pattern, and reading it as one sends you
looking for a file that does not exist. Only `crates/cli` keeps a second, hand-written `TOPICS`
copy in its test. `memory` and `chat` guard the same drift differently, through
`tests/guide_examples.rs`: every ` ```console ` block in those guides — both locales — is held
against that crate's golden corpus, so changing a golden output without carrying the guides along
is red rather than a false map. memory's suite asserts the reverse direction too: every block in
`tests/golden/guides.trycmd` must be quoted by some guide, or that corpus drifts into a second,
unread test file.

The published slug is **`<binary>-<topic>`** (`nxf-getting-started`), never the bare topic: the
blocks share ONE docs page and its slugs are duplicate-free by contract. A cross-link between
guides must use that slug — `[core concepts](nxf-core-concepts)` — because the landing page
resolves a bare target only if it is a known slug and otherwise degrades the link to plain text
with no error anywhere. `checkCrossLinks` in `content/lib/docs.mjs` is what makes that a red
build instead.

Plus `TRYCMD=overwrite cargo test -p nexus-flow-cli --test golden` for `guide.trycmd` (and
`-p nxs` for the suite fan-out, `-p nexus-memory` / `-p nexus-chat` for that block's guide listing
AND its worked session). Build `nxs` FIRST in every case — the freshness gate refuses to burn a
stale binary's output into a golden, and it fires on a guide `.md` edit too, because the `en` tree
is a build input. Run the content gate locally before pushing:

```bash
(cd content && npm ci && node --test && node assemble.mjs \
   && node validate.mjs --schema schema/content-contract-v1.json)
```

## The stale-binary trap (`cargo test -p <crate>`)

`nxf`/`nxm`/`nxc` are **`argv[0]` symlinks** to the single `nxs` binary (since the multicall
collapse). `nxs` is its own package that depends on the persona crates — not the reverse — so
**`cargo test -p <crate>` never builds it**: the run compiles every test binary, finds
`target/<profile>/nxf`, follows the symlink, and executes an *arbitrarily old* `nxs`. It goes
green while asserting against code you did not write. This is not theoretical — goldens once
regenerated against a pre-rebase binary silently deleted a command from the agent-facing docs and
cost hours of chasing a phantom.

A **gate catches this** (`crates/test-support`): the golden harnesses call
`nxs_test_support::assert_multicall_binary_fresh()` and a `binary_freshness` test in cli/memory/
chat fails if `target/<profile>/nxs` is older than any source that links into it — under
`TRYCMD=overwrite` it aborts *before* trycmd can write. So the rule is enforced, not remembered:

```bash
cargo build -p nxs && cargo test -p nexus-flow-cli   # the fast loop, made honest
cargo test --all                                     # builds nxs itself — always safe
```

One thing the gate does NOT cover: **`cargo test -p <crate>` does not compile other crates' test
files.** A broken callsite in `crates/facade/tests/read.rs` stayed invisible across several green
partial runs. Only `cargo test --all` compiles them all.

(Guide docs — `crates/{cli,memory,chat,nxs}/docs/guide/en/**`, embedded via `include_dir!` — used
to live here as a trap: "touch `guide.rs` to force a rebuild". That was a misdiagnosis. A guide
*content* edit rebuilds on its own — the macro expands to `include_bytes!`, which rustc records in
dep-info — and each of those crates' `build.rs` emits `rerun-if-changed=docs/guide/en` so add and
remove are tracked too. That last part matters most when a tree GROWS: an added or retired topic
is an ADD or a REMOVE, exactly the case dep-info cannot see. The `en` trees are first-class build
inputs the stale-binary gate watches; `de` is website-only and stays out.)

## Build cache

`target/` grows unbounded — cargo never auto-prunes stale incremental and dep artifacts, and it
has reached tens of GB more than once. Two levers:

- **Slimmer debug info** is already wired in the workspace `Cargo.toml`: `[profile.dev] debug =
  "line-tables-only"` roughly halves `target/debug` while keeping file:line backtraces. It does
  not touch `debug_assert!` / `cargo test --release` (those gate on `cfg(debug_assertions)`, a
  separate knob). Nothing to run — it applies on the next build.
- **Third-party crates are OPTIMISED in the dev/test profile** (6j6v.vewv, 2026-11-03):
  `[profile.dev.package."*"] opt-level = 2` in the workspace `Cargo.toml`. `"*"` matches
  non-workspace packages only, so workspace crates still build at opt-level 0 with incremental
  compilation, and debug-assertions/overflow-checks stay on everywhere. Why: CI's
  `Tests (debug)` had grown to 54 min, 52.7 of them EXECUTING tests (warm compile 1m35s, so
  neither linking nor the number of test binaries was the cost). One binary, fold-ddb's
  `based_differential`, took 1731 s against 64 s in release, and a sample put ~35% of its CPU in
  curve25519-dalek/sha2 (ed25519 signing) and ~35% in the bundled SQLite, both at -O0. After the
  change: `Tests (debug)` 14.5 min including the compile (run 41666096148, against 41648455950),
  dynamodb-local's debug tests 8 min instead of 47, the same 335 binaries and 5833 tests. The
  price is one slower dependency build (locally ~10 min once, on this machine), held by the cache
  afterwards. Do not remove it to "speed up the build": the debug suite goes back to an hour. If
  a debug test is slow again, sample it (`sample <pid> 15` on macOS) before reaching for shards.
- **Prune by SIZE, not by age.** `cargo sweep --time 15` is the obvious call and it is the WRONG
  one here: churn on this workspace is roughly 60k files a day (parallel sessions, debug+release
  double builds, feature variants), so almost nothing is older than 15 days and the sweep reclaims
  nothing. Use `cargo sweep --maxsize <GB>` instead — see
  [slow-tests-check-target-deps-and-swap](2026-11-15-no-ticket-slow-tests-check-target-deps-and-swap.md), where that was measured. Never sweep while a build or
  test is running, and prefer it over `cargo clean`, which forces a cold rebuild.
