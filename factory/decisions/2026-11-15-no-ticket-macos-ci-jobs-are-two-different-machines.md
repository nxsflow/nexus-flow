# 2026-11-15 · no ticket · macOS tests (hosted) is a GitHub 3-core runner, not the org Mac — a hosted red is often a race, and a green check list can hide a cancelled run.

- Decided by: recorded in the project memory (`nxm`, key `macos-ci-jobs-are-two-different-machines`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: ci, macos, flaky-tests

## The record

`macOS tests (hosted)` in ci.yml is the GitHub-HOSTED `macos-latest` runner (three cores), NOT the
org's own Mac. The self-hosted one is `Platform gates (macos)` (routed through the repository
variable `NXF_MACOS_RUNS_ON`, today `["self-hosted","macOS","ARM64"]`), and it runs `cargo clippy`
only — no tests at all.

WHY THIS MATTERS WHEN A RUN GOES RED. The two jobs fail for entirely different reasons and only one
of them can be reproduced by running cargo on the owner's machine:

- `macOS tests (hosted)`: three cores, shared, slow, and it runs
  `cargo test --no-fail-fast -p nxs -p nexus-chat -p nxs-service`. A test with a latent timing
  assumption fails HERE and nowhere else — 20 repeats on the owner's 10-core machine prove nothing
  about it. First measured case: nxf 6j6v.1zs2, where one tree answered PASS/FAIL/FAIL/PASS across
  four runs of this job.
- `Platform gates (macos)`: the org's own Mac, `if: github.event_name != 'pull_request'`, clippy over
  the `#[cfg(target_os = "macos")]` code. A red here is a lint or a compile error, never a race.

CONSEQUENCE: when the hosted job reds on a test the diff does not touch, the first question is not
"what did I break" but "what is the failure RATE" — rerun that one job (`gh run rerun <id> --failed`)
before either accepting or blaming it, and compare against the same job's history on main
(`gh run view <id> --json jobs`). A single green run is what a 50% flake looks like half the time
([gate-evidence-is-per-commit](2026-11-15-no-ticket-gate-evidence-is-per-commit.md)).

AND THE THING THAT IS NOT INNOCENT: adding tests to an existing test BINARY changes which tests run
beside each other on a three-core machine. A change that touches none of the failing code can still
raise a latent race's rate from never to often, and the honest way to say that is a hypothesis with
the run history beside it — not "not my change" and not a fix applied on a guess.

## A VERDICT CAN ALSO BE WRONG ABOUT ITSELF — read the rollup, not the check list

Absorbed here on 2026-09-18 from `ci-dispatch-cancels-the-open-prs-run`, a memory whose own body
recorded that its key named the direction the configuration does NOT support. What is real:

`git push` a new branch, `gh pr create` seconds later, and the arriving PULL_REQUEST run cancels the
in-flight PUSH run (measured 2026-09-11, PR #461). There `gh pr checks` shows every job PASSING — it
reports the surviving run — while `gh pr view --json mergeStateStatus` reads UNSTABLE and the merge
looks unsafe. The cancelled entry is visible ONLY in the rollup:

    gh pr view <n> --json statusCheckRollup --jq '.statusCheckRollup[]
      | select((.conclusion // .state) != "SUCCESS")'

A cancelled run whose successor SUCCEEDED seconds later is benign — compare `completedAt` of the
cancelled one with `startedAt` of the good one. UNSTABLE with every check green is that, and not a
reason to hold a merge.

And when a check reads `fail`, ask the API for the job's real conclusion before believing it:
`gh api repos/<owner>/<repo>/actions/jobs/<id> -q .conclusion` tells `cancelled` from `failure`,
which the PR check list does not. That is what a 2026-09-10 observation on PR #458 turned out to be
— a `managed-postgres` job reading `fail 35m1s` whose real conclusion was `cancelled`.

(Why a dispatch is safe beside an open PR, and why it is the only way to get macOS coverage on a
branch, is in [gates-run-in-ci-not-locally](2026-11-15-no-ticket-gates-run-in-ci-not-locally.md).)
