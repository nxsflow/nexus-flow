#!/usr/bin/env bash
# The release's fail-closed quality gate (nexus-flow 6j6v.31rs): demand a GREEN CI run for the
# commit being released, instead of running the test suite a second time.
#
# `release.yml` used to carry a `test` job that ran `cargo test --all` and `cargo test --all
# --release` — byte for byte the two commands `ci.yml`'s `quality-gates` runs, on the same commit,
# minutes apart. This script replaces that duplicate with a LOOK at the verdict that already exists.
#
# Fail-closed is the whole point, so it is spelled out here rather than left to a reader:
#
#   - A green run for this exact commit is the ONLY way out with exit 0.
#   - No run, a red run, a cancelled run, a run that never finishes, or a failure to ASK GitHub at
#     all ⇒ non-zero. There is no fallback that tests the tree itself and no path that shrugs.
#   - Only `push` and `workflow_dispatch` runs count. `ci.yml`'s three release-profile steps carry
#     `if: github.event_name != 'pull_request'`, so a `pull_request` run is a smaller gate — and the
#     release profile catches a class (a side effect inside `debug_assert!`, compiled out of release
#     and silently no-op) that has shipped from this repo once already.
#
# It waits rather than failing instantly when a run for the commit is still going: tagging the
# instant a merge lands is normal, and "come back in fifteen minutes" is not a verdict. Both waits
# are bounded, and running out of either is a RED release.
#
# ── WHAT THIS GATE DOES AND DOES NOT ASSERT (PR #300 review, Integrity & Robustness #2) ─────────
#
# It asserts: THE QUALITY GATE PASSED FOR THIS EXACT COMMIT.
# It does NOT assert: this commit was reviewed, or ever landed on `main`.
#
# Those are different statements, and on this plan they cannot be made the same one — branch
# protection is not configurable (`gh api .../branches/main/protection` answers 403 "Upgrade to
# GitHub Pro"), so there is nothing server-side that says "this SHA is an ancestor of `main` and
# got there through review". Concretely: anyone with write access can push an unreviewed commit to
# a throwaway branch, dispatch `ci.yml` on it, wait for green, and tag THAT commit.
#
# This grants no privilege that did not already exist. The job it replaces was a `test` job that
# ran the suite on whatever commit was tagged, with no prior run required at all — so the demand
# for a pre-existing green run is strictly MORE than before, not less. But "green" and "reviewed"
# should not be conflated by a future reader, hence this paragraph rather than a silence.
#
# If that gap ever needs closing, the shape is `git merge-base --is-ancestor "$SHA" origin/main`
# plus a check that the run's `head_branch` is `main` or a maintenance line — deliberately NOT
# added here, because it would also block the legitimate backport-line and rehearsal paths this
# repo uses, and because on this plan it would still be advisory rather than enforced.
#
# Environment:
#   GH_REPO                    owner/name (required)
#   SHA                        the commit being released (required)
#   GH_TOKEN                   passed through to `gh` (required in CI)
#   CI_WORKFLOW                workflow file to consult (default: ci.yml)
#   CI_GATE_RUN_TIMEOUT        seconds to wait for a RUNNING run to finish (default: 2100 = 35 min)
#   CI_GATE_APPEAR_TIMEOUT     seconds to wait for a run to APPEAR at all (default: 300 = 5 min)
#   CI_GATE_POLL               seconds between polls (default: 20)
#
# Hermetically tested by `tests/require-green-ci.test.sh`, which stubs `gh` and shrinks the
# timeouts — the gate between a red tree and a signed artifact does not get to be untested.
set -euo pipefail

: "${GH_REPO:?GH_REPO (owner/name) is required}"
: "${SHA:?SHA (the commit being released) is required}"
CI_WORKFLOW="${CI_WORKFLOW:-ci.yml}"
CI_GATE_RUN_TIMEOUT="${CI_GATE_RUN_TIMEOUT:-2100}"
CI_GATE_APPEAR_TIMEOUT="${CI_GATE_APPEAR_TIMEOUT:-300}"
CI_GATE_POLL="${CI_GATE_POLL:-20}"

# Every `push`/`workflow_dispatch` run of $CI_WORKFLOW for $SHA, as a JSON array. A failing `gh`
# propagates (`set -e` + `pipefail`): not being able to ASK is not permission to proceed.
runs_for_sha() {
  gh api --paginate \
    "repos/${GH_REPO}/actions/workflows/${CI_WORKFLOW}/runs?head_sha=${SHA}&per_page=100" \
    --jq '.workflow_runs[]
          | select(.event == "push" or .event == "workflow_dispatch")
          | {id, status, conclusion, event, html_url}' \
    | jq -s '.'
}

# One line per run, for a human reading a failed release.
report() {
  printf '%s' "$1" | jq -r '.[] | "  \(.status)/\(.conclusion // "-")  \(.event)  \(.html_url)"'
}

start="$(date +%s)"
while :; do
  runs="$(runs_for_sha)"

  green="$(printf '%s' "$runs" \
    | jq -r '[.[] | select(.status == "completed" and .conclusion == "success")] | first // empty | .html_url')"
  if [ -n "$green" ]; then
    echo "::notice::the quality gate is green for ${SHA} — ${green}"
    echo "the release trusts that run instead of repeating it"
    exit 0
  fi

  pending="$(printf '%s' "$runs" | jq '[.[] | select(.status != "completed")] | length')"
  total="$(printf '%s' "$runs" | jq 'length')"
  waited=$(( $(date +%s) - start ))

  if [ "$pending" -gt 0 ]; then
    if [ "$waited" -ge "$CI_GATE_RUN_TIMEOUT" ]; then
      echo "::error::CI for ${SHA} was still running after ${CI_GATE_RUN_TIMEOUT}s — refusing to guess, and refusing to test the tree here instead. Re-run this release once that run has finished green."
      report "$runs"
      exit 1
    fi
    echo "CI for ${SHA} is still running (${pending} of ${total}) — waited ${waited}s, polling again…"
    sleep "$CI_GATE_POLL"
    continue
  fi

  if [ "$total" -eq 0 ] && [ "$waited" -lt "$CI_GATE_APPEAR_TIMEOUT" ]; then
    # A tag push and the branch push that precedes it can land in the same second; give the run a
    # moment to be created before calling it missing.
    echo "no ${CI_WORKFLOW} run for ${SHA} yet — waited ${waited}s, polling again…"
    sleep "$CI_GATE_POLL"
    continue
  fi

  # Both ways out are named, because the two callers arrive here from opposite directions and the
  # tag-on-`main` advice is useless to one of them (6j6v.rvyp): a STAGING REHEARSAL is dispatched on
  # a feature branch on purpose, and a feature branch produces only `pull_request` runs — `ci.yml`'s
  # push filter is `[main, "release/<major>.<minor>"]` — which this gate does not count. So the
  # rehearser's first attempt always lands here, and without the second sentence they learn only
  # that the gate refuses, not that dispatching `ci.yml` on the same branch first is the way through.
  echo "::error::no green ${CI_WORKFLOW} run for commit ${SHA}. A release trusts the quality gate for the commit it tags and never re-tests it, so this is fail-closed. A \`pull_request\` run does not count: it skips the release-profile tests on purpose. Two ways forward, depending on which you are doing: (1) RELEASING — push the commit to \`main\` (or to a \`release/<major>.<minor>\` line), let CI go green there, and tag THAT commit; (2) REHEARSING the pipeline on a feature branch — that branch has only \`pull_request\` runs, so establish the full gate on it first with \`gh workflow run ${CI_WORKFLOW} --ref <branch>\`, wait for that run to go green (~20-25 min, both profiles), then dispatch this workflow again. Runbook: docs/specs/release-management.md §10.1."
  if [ "$total" -eq 0 ]; then
    echo "  found no push/workflow_dispatch run of ${CI_WORKFLOW} for this commit at all"
  else
    report "$runs"
  fi
  exit 1
done
