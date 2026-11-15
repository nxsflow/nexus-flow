# 2026-11-15 · no ticket · CI is the gate: push, dispatch ci.yml (debug-only, cancels older), check headSha; full run once pre-merge, never on main; locally only fmt, touched tests, scoped clippy.

- Decided by: recorded in the project memory (`nxm`, key `gates-run-in-ci-not-locally`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: gates, ci, testing

## The record

## THE GATE PROCEDURE (migrated from CLAUDE.md, 2026-11-02)

The project's gate is the CI workflow, not a local run. Locally run ONLY: `cargo fmt --all -- --check`; the test files your change touched (`cargo test -p <crate> --test <file>`); and clippy scoped to the TARGETS your change touched, once before the push. Never `--all-targets`: nexus-chat alone has 134 test binaries (104k lines of tests), and `--all-targets` type-checks every one of them. Name the targets instead:

    cargo clippy -p <crate> --lib --test <touched_test_file> [--test <another>] [--bin <touched_bin>] -- -D warnings

`--lib` when you edited a file under `src/`, one `--test <name>` per file under `tests/` you created or edited (the name is the file name without `.rs`), `--bin` only for a touched binary. Clippy cannot lint a single file; the target is the smallest scope it has. Never `cargo test --all`, `cargo test --workspace`, an unscoped clippy (no `-p`), or `--all-targets` locally. Then:

    git push origin HEAD
    gh workflow run ci.yml --ref "$(git branch --show-current)"
    run=$(gh run list --workflow ci.yml --branch "$(git branch --show-current)" --limit 1 --json databaseId -q '.[0].databaseId')
    gh run watch "$run" --exit-status

WHAT THAT DISPATCH RUNS (since 6j6v.ee3e, 2026-11-03): on a feature branch it is a DEBUG-ONLY run by default, the same jobs a pull_request run gets: no `Tests (release)`, no release steps in postgres-parity/dynamodb-local, no hosted macOS lane. Its title says so: `CI on <branch>: debug only`. It also CANCELS a run still in progress on the same branch, so a superseded dispatch no longer runs to its end while the next one queues behind it: dispatch after your last push and watch the newest run, never an older one. fmt and `clippy --all-targets` run first in their own `lint` job; every long job (quality-gates, postgres-*, managed-postgres, dynamodb-local, macOS tests) has `needs: lint`, so a lint or compile error turns the run red within minutes and the long jobs show as skipped.

`gh workflow run ci.yml --ref <branch> -f full=true` asks for the FULL both-profile gate (title `CI on <branch>: full gate`). That is not a coder's per-push gate: see "The one full run belongs BEFORE the merge" below.

Check that the run's `headSha` is `git rev-parse HEAD` before you trust it: a run proves only the commit it ran on ([gate-evidence-is-per-commit](2026-11-15-no-ticket-gate-evidence-is-per-commit.md)). If you hand over a gate result, give that commit and the run URL. If a job is red, the failing job's log (`gh run view "$run" --log-failed`) is the evidence.

In the coding channel "you" is the coder: it pushes and dispatches, and the verifier reads that run on HEAD, dispatches nothing, and does not repeat the local commands, which the run covers.

TO JUDGE A REGISTERED FLAKE WHERE IT WENT RED (6j6v.jmxk): the workflow `isolated-test.yml` repeats one named test on Linux in a chosen profile, `gh workflow run isolated-test.yml --ref <branch> -f package=<package> -f target=<lib or test file> -f test=<full test name> -f profile=<debug|release> -f repeats=20`. The evidence is the last log line, `isolated-test-result commit=... test=... system=... profile=... repeats=... executed=... reds=... verdict=...`. It cannot cancel a ci.yml run and cannot count toward a release. It reaches a red in `quality-gates` only: no service containers, no cargo features, no doctests, so a red in postgres, DynamoDB or a macOS lane has no isolated run (6j6v.wf8p). First real run: 42258414565.

MEASURED 2026-09-03 against a real pull request and the transcript of the session that built it.
Not a feeling — numbers.

## The finding

    CI, wall clock until everything is green   7.5 min   (second run 7.2 min — stable)
    Locally, one full gate cycle               88 / 100 / 27 / 122 min
    Factor                                     12 to 16

Four full local cycles in one night = 5.8 h. CI would have done the same work four times in half
an hour, and not on this machine.

The machine is the floor: Intel i5-8257U @ 1.4 GHz (4 cores, a 15 W mobile chip from 2019), 16 GB
RAM, swap in use, and the checkout sits on an external USB disk formatted HFS+ rather than APFS.
Creating 2000 small files — the shape of a Rust build — takes 30.4 s there against 5.6 s
internally.

## The rule

1. WHILE WORKING, test narrowly: `cargo test -p <crate>`, `--test <file>`. In the measured
   transcript such runs had a median of ~30 s. That is also exactly the TDD inner loop.
2. PUSH EARLY and let CI be the gate. 7.5 min, in the background, on somebody else's hardware.
3. AT MOST ONE full local run, and only for what CI does not see on a pull request.

This agrees with [gate-evidence-is-per-commit](2026-11-15-no-ticket-gate-evidence-is-per-commit.md): "a gate runs ONCE, at the end, against the tree
you are reporting — not behind every commit."

## What CI does NOT see on a pull request (or on a default branch dispatch)

Everything carrying `if: needs.lint.outputs.full == 'true'`: `Tests (release)`
(`cargo test --all --release`), the `macOS tests (hosted)` job, and the release steps inside
`postgres-parity` and `dynamodb-local`. Since 6j6v.ee3e a branch dispatch without `-f full=true`
skips the same set. `platform-gates-macos` (a short clippy on the org's own Mac) still carries
`if: github.event_name != 'pull_request'` - that line is a security boundary pinned by
`cargo xtask runners check` - so it runs on every dispatch, debug or full. Verified against
`ci.yml` on 2026-11-03.

CORRECTED 2026-09-05: this memory used to say that `#[cfg(target_os = "macos")]` tests run
NOWHERE in CI. That is no longer true. There is now a dedicated `macOS tests (hosted)` job
(`cargo build -p nxs`, then `cargo test --no-fail-fast -p nxs -p nexus-chat -p nxs-service` on a
hosted macOS runner) and a `cargo xtask macos-tests check` gate in `quality-gates` that fails when
a `cfg(macos)` test is not in that lane. What remains true is only the staggering: that job, like
the release profile, is skipped on the pull-request event.

Everything else needs no local run. CI does considerably MORE than the local four: fmt, clippy
-D warnings, `cargo test --all`, six xtask drift gates, the sidecar suite under node, two lean
builds, Windows platform gates, postgres-tls, postgres-parity, managed-postgres and
dynamodb-local. The full inventory, in order, is in [build-test](2026-11-15-no-ticket-build-test.md).

## The evidence that is stronger than the arithmetic

The session behind that pull request ran `cargo test --all` FOUR times, all green — and CI still
found a real bug: a test asserted a bound that only holds on macOS and fell over on Linux. Four
full local runs did not prevent that and COULD not, because they never covered the missing
operating system. Locally green says nothing here about CI green.

## Relationship to the TDD skill

No conflict at the core, but two sentences that are easy to misread:

- "Verify RED — watch it fail" is UNTOUCHED and still mandatory. It is a targeted run of seconds,
  not a suite run. This memory is NOT a licence to skip the red step.
- "Verify GREEN — other tests still pass" and the checklist item "All tests pass" must NOT be read
  as a local `cargo test --all` per cycle. CI supplies that assurance here.

## The one full run belongs BEFORE the merge — via workflow_dispatch -f full=true, by the merging step

UPDATED 2026-11-03 (6j6v.ee3e). The coder's dispatches are debug-only now. The full both-profile
gate runs ONCE per work order, before the merge, dispatched by the MERGING step - not by the coder
on every push:

    gh workflow run ci.yml --ref <branch> -f full=true

and the merge waits for that run to go green (check its headSha, and that its quality-gates step
`Tests (release)` concluded success - the title `: full gate` is only a label; the release gate
`require-green-ci.sh` reads the step, never the title). A release-only or macOS-only break then still surfaces while it belongs to a pull
request.

ON MAIN, THE PUSH RUN IS THE PROOF - NEVER DISPATCH ANOTHER RUN THERE. Measured 2026-11-02: right
after a merge the merging step dispatched an extra full run on main while main's push run had just
started. Nothing is cancelled on main (a push run there is main's standing record of green), so the
dispatch queued behind the push run for an hour - and the push run alone already proved main green.
After the merge, watch main's PUSH run for the merge commit; that run is always full.

The rest of this section is the 2026-09-03 history of why the full run moved before the merge.

MEASURED 2026-09-03 over the last 20 ci.yml runs:

    PR run      7.2 - 10.2 min   (quality-gates is the pole: 7.4 min, of which 281 s `Tests (debug)`)
    main run   17.1 - 28.1 min   (quality-gates 17.0 min — almost all of the difference is `Tests (release)`)

Because of the pull-request staggering, **the first time the full set runs is AFTER the merge.**
That was defensible while the house rule was "run `cargo test --all --release` locally first" —
and that countermeasure is exactly what this memory repeals. Without a replacement a release-only
break goes red on `main`, blocks the version cut through `ci-green` in `release.yml`, and needs a
fresh pull request.

THE REPLACEMENT COSTS NO YAML: `ci.yml` already has `workflow_dispatch`, and a run triggered that
way is NOT `pull_request` — so the release tests and the macOS lane come along.

    After mitigating the review findings, after the last push, BEFORE the merge:
    dispatch ci.yml on the branch once and wait for green.

Once per work order, not per push. Cost ~13 min, once (2026-09 figure; a full run took 80+ min by
November). Gain: `main` does not go red, the release
is not blocked, and a release-only break surfaces while it still belongs to a pull request.

A FULL dispatch is also the only way to get the hosted macOS test lane on a feature branch, because a
PR run and a default dispatch render it `skipping`. CANCELLING (since 6j6v.ee3e): `ci.yml`'s
`cancel-in-progress` is true for a `pull_request` run and for a `workflow_dispatch` on any ref that
is not `main` or a `release/<major>.<minor>` line, evaluated on the run being QUEUED, and the group is
`ci-<ref>`. So a dispatch never cancels the PR run (refs/pull/N/merge is another group), but a newer
dispatch on the same branch DOES cancel an older one, full or not - so do not dispatch a debug run on
a branch while the merging step's full run is still going there.

This does not work on a fork PR (nobody can dispatch there) — which becomes relevant the moment
the repository is made public.

## `cargo clippy` LOCALLY: CRATE-SCOPED ONLY, ONCE BEFORE THE PUSH (owner, 2026-11-02; was: never, 2026-09-18)

The rule above ("test narrowly, CI is the gate") was in place and STILL a subagent-driven run of
eight tasks spent about fifteen hours of wall clock on local gates, because "the four commands"
was read as a list to shorten rather than a habit to drop, and because the CONTROLLER's dispatch
prompts told every implementer to re-run a list of neighbouring suites plus clippy.

    cargo clippy -p nexus-chat --all-targets -- -D warnings    15 - 25 min, EVERY time
    the same clippy in CI's quality-gates job                   part of a 7.5 min run

It is not a shorter run than `cargo test`; it type-checks the crate AND every test target again.
On one task it ran five times — once per fix round, twice more because the 10-minute tool timeout
moved it to the background and it was reissued.

THE RULE, owner instruction of 2026-09-18:

1. SUPERSEDED 2026-11-02 by owner decision. The rule used to be "no clippy locally at all". By
   November a CI cycle had grown to 80+ min (quality-gates `Tests (debug)` alone 61 min), and on
   6j6v.t7xp three red CI runs, each for a lint or compile error, cost ~70 min. So now: run
   clippy ONCE before the push, scoped to the touched TARGETS (`-p <crate> --lib --test <file>`,
   see THE GATE PROCEDURE above). NEVER without `-p`, never `--all-targets`/`--all`/`--workspace`.
   Clippy cannot lint a single file; the crate is its smallest scope. Not after every edit, not
   per fix round, not in the background-and-reissue loop described below. The 15-25 min
   measurement below still holds; it is now weighed against an 80+ min CI cycle.
   (Since 6j6v.vewv, 2026-11-03, `Tests (debug)` is ~14.5 min again: third-party crates are
   optimised in the dev profile, see [build-test](2026-11-15-no-ticket-build-test.md). The 61 min was -O0 crypto and SQLite.)
2. Run ONLY the test file(s) covering the code you just changed (`cargo test -p <crate> --test
   <file>` / `--lib <module>`), plus `cargo fmt --all`, which costs seconds.
3. Do NOT re-run neighbouring suites "to be safe". A cross-suite break is what CI is for.
4. A DELEGATING SESSION PUTS THIS POLICY IN EVERY SUBAGENT PROMPT, VERBATIM — not in some of them,
   not only in the ones carrying a quality-gates envelope. A fresh implementer with Bash reaches
   for clippy on its own because every code-quality habit it has says to, and a general-purpose
   subagent told only to REVIEW will run cargo unprompted "to corroborate" a source-level finding
   unless it is told not to. The same applies to the no-cargo constraint in a SHARED checkout,
   where a concurrent run also destroys the other session's build
   ([no-concurrent-lean-build-during-tests](2026-11-15-no-ticket-no-concurrent-lean-build-during-tests.md)).

WHAT THIS BUYS, measured on the same run: after the policy changed, a fix round went from hours to
the length of its own test file.

(Absorbed `no-cargo-constraint-must-reach-every-subagent` on 2026-09-18 — it was point 4 above in a
memory of its own, and this one already quoted it.)
