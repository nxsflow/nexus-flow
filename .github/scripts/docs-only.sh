#!/usr/bin/env bash
# Is this CI run's change DOCS-ONLY? (NEX-36)
#
# Every heavy job in `ci.yml` and `facade-semver.yml` calls this as its first step after checkout.
# On a docs-only change the job's later steps are skipped and the job ends GREEN UNDER ITS OWN NAME,
# with a `::notice::` and a job-summary line that name what was not run and why. A job-level `if:`
# is not used: GitHub reports a skipped required check as green with no reason attached, a silent
# pass nobody can tell from a real one.
#
#   docs-only.sh classify              NUL-separated paths on stdin; exit 0 docs-only, 1 not
#   docs-only.sh decide "<what>"       the CI step: writes docs_only=true|false to $GITHUB_OUTPUT
#
# THE ALLOWLIST. A path is docs-only when it sits under one of the prefixes below, and nothing a
# heavy job builds, tests or checks reads it. Checked when the list was written (NEX-36; the PR
# lists the evidence): no `include_str!`/`include_bytes!`/`include_dir!`, no build script, no test,
# no golden harness and no xtask check run by a heavy job reads them. Two cheap checks DO read two
# of them, and both keep running on a docs-only change: `lint`'s `cargo xtask decisions --check`
# (factory/decisions/) and `changelog-check` (changes/). `docs/generated/` is NOT here: quality-
# gates checks it against the clap tree. When in doubt a path stays out — a wrong "yes" is a gate
# that silently did not run, a wrong "no" only costs runner minutes.
#
# THE EVENTS. Only these may take the docs-only path, and every other case runs in full:
#   pull_request        the merge commit against its first parent (the base tip GitHub merged onto)
#   push to v2          the pushed commit against `github.event.before`
#   workflow_dispatch   the dispatched commit against the tip of `v2`, on any ref but `main` and
#                       the `release/` lines; a branch behind `v2` therefore also counts `v2`'s own
#                       changes, which can only turn a "yes" into a "no"
# `full=true` ($NXF_FULL) always runs in full; so does a push to `main` or a maintenance line, an
# empty change, and any git failure on the way to an answer (fail toward the full run).
#
# Bash 3.2 on purpose: `platform-gates-macos` runs this on the org's own Mac, whose /bin/bash is 3.2.
set -uo pipefail

# The prefixes, with their trailing slash so a near-miss (`docs/rfcs/`, `factoryx/`) never matches.
ALLOWLIST="docs/rfc/ docs/research/ factory/ changes/"

# The base branch (factory/CONFIG.md): the one branch whose pushes may be docs-only, and what a
# dispatch is compared with. Judged by the FULL ref, so a pushed tag of the same name never counts.
BASE_BRANCH="v2"

ZERO_SHA="0000000000000000000000000000000000000000"

# ---- classify ---------------------------------------------------------------------------------

# Reads NUL-separated paths on stdin. Prints the verdict; exits 0 when every path is allowlisted
# and there is at least one, 1 otherwise.
classify() {
  local path prefix allowed count=0
  # `|| [ -n "$path" ]` keeps a last path that lacks its terminating NUL.
  while IFS= read -r -d '' path || [ -n "$path" ]; do
    count=$((count + 1))
    allowed=0
    for prefix in $ALLOWLIST; do
      case "$path" in
        "$prefix"?*) allowed=1; break ;;
      esac
    done
    if [ "$allowed" = 0 ]; then
      echo "full run: $path is outside the docs-only allowlist ($ALLOWLIST)"
      return 1
    fi
  done
  if [ "$count" = 0 ]; then
    echo "full run: no changed path to judge"
    return 1
  fi
  echo "docs-only: all $count changed paths are under $ALLOWLIST"
  return 0
}

# ---- decide -----------------------------------------------------------------------------------

# Prints the commit to compare HEAD with, fetching it first; returns 1 (with the reason on stdout)
# when this run may not take the docs-only path or the base cannot be had.
base_commit() {
  local event="${GITHUB_EVENT_NAME:-}" ref="${GITHUB_REF:-}" sha="${GITHUB_SHA:-}"
  if [ "${NXF_FULL:-}" = "true" ]; then
    echo "full run: full=true was asked for"; return 1
  fi
  if [ -z "$sha" ]; then
    echo "full run: GITHUB_SHA is not set"; return 1
  fi
  case "$event" in
    pull_request)
      # The merge commit and its parents; the checkout fetched the commit alone.
      git fetch -q --no-tags --depth=2 origin "$sha" >&2 || { echo "full run: could not fetch $sha with its parents"; return 1; }
      git rev-parse -q --verify "$sha^1^{commit}" || { echo "full run: $sha has no first parent"; return 1; }
      ;;
    push)
      if [ "$ref" != "refs/heads/$BASE_BRANCH" ]; then
        echo "full run: a push to $ref always runs in full"; return 1
      fi
      local before="${NXF_PUSH_BEFORE:-}"
      if [ -z "$before" ] || [ "$before" = "$ZERO_SHA" ]; then
        echo "full run: the push names no previous commit"; return 1
      fi
      git fetch -q --no-tags --depth=1 origin "$before" >&2 || { echo "full run: could not fetch the previous commit $before"; return 1; }
      git rev-parse -q --verify "$before^{commit}" || { echo "full run: $before is not a commit"; return 1; }
      ;;
    workflow_dispatch)
      case "$ref" in
        refs/heads/main|refs/heads/release/*) echo "full run: a dispatch on $ref always runs in full"; return 1 ;;
      esac
      git fetch -q --no-tags --depth=1 origin "refs/heads/$BASE_BRANCH" >&2 || { echo "full run: could not fetch $BASE_BRANCH"; return 1; }
      git rev-parse -q --verify "FETCH_HEAD^{commit}" || { echo "full run: $BASE_BRANCH did not resolve"; return 1; }
      ;;
    *)
      echo "full run: the event '$event' never takes the docs-only path"; return 1
      ;;
  esac
}

decide() {
  local what="${1:-}"
  if [ -z "$what" ]; then
    echo "::error::docs-only.sh decide needs the work this job skips, as its argument"; return 2
  fi
  if [ -z "${GITHUB_OUTPUT:-}" ]; then
    echo "::error::GITHUB_OUTPUT is not set: nowhere to write the verdict"; return 2
  fi

  local verdict="false" reason base diff_rc
  if reason="$(base_commit)"; then
    base="$reason"
    # --no-renames: a rename lists BOTH paths, so moving code into docs/rfc/ still names the code.
    reason="$(git diff --no-renames --name-only -z "$base" "${GITHUB_SHA}" | classify)"
    diff_rc=$?
    if [ "$diff_rc" = 0 ]; then
      verdict="true"
    fi
  fi

  echo "$reason"
  echo "docs_only=$verdict" >> "$GITHUB_OUTPUT" || return 2
  if [ "$verdict" = "true" ]; then
    echo "::notice title=Docs-only change::Not run: $what. Every changed path is under $ALLOWLIST, which nothing this job builds or tests reads (.github/scripts/docs-only.sh)."
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
      # shellcheck disable=SC2016  # the backticks are Markdown for the job summary, not a command
      printf '**Skipped as docs-only:** %s. Every changed path is under `%s`, which nothing this job builds or tests reads (`.github/scripts/docs-only.sh`).\n' \
        "$what" "$ALLOWLIST" >> "$GITHUB_STEP_SUMMARY"
    fi
  fi
}

case "${1:-}" in
  classify) classify ;;
  decide) shift; decide "$@" ;;
  *) echo "usage: docs-only.sh classify | decide \"<what this job skips>\"" >&2; exit 2 ;;
esac
