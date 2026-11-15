#!/usr/bin/env bash
# Hermetic test harness for .github/scripts/docs-only.sh (NEX-36).
#
# That script decides, at the start of every heavy CI job, whether the change is DOCS-ONLY, and the
# job then skips its builds and tests. Every wrong "yes" is a gate that silently did not run, so
# every way the script can answer is asserted here, the "no"s above all: an outside path, a rename
# out of code, a push to `main` or a maintenance line, `full=true`, an unknown event, an empty
# change, a base commit git cannot fetch.
#
# Hermetic: the "GitHub remote" is a bare repository on disk, reached over `file://` so that the
# shallow fetches the script makes behave as they do against github.com. No network, no token.
#
# Run: tests/docs-only.test.sh   (needs bash and git)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/.github/scripts/docs-only.sh"

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-docs-only-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK/home"
mkdir -p "$HOME"
git config --global init.defaultBranch v2
git config --global advice.detachedHead false

# ---- classify: the allowlist itself -----------------------------------------------------------

# Feeds NUL-separated paths to `classify`. Sets $RC and $OUT in this shell.
RC=0
OUT=""
classify() { # <path…>
  RC=0
  OUT="$( { [ "$#" -gt 0 ] && printf '%s\0' "$@"; } | bash "$SCRIPT" classify 2>&1)" || RC=$?
}

expect_docs_only() { # <case> <path…>
  local name="$1"; shift
  classify "$@"
  if [ "$RC" = 0 ]; then ok "$name"; else bad "$name (exit $RC: $OUT)"; fi
}
expect_full() { # <case> <path…>
  local name="$1"; shift
  classify "$@"
  if [ "$RC" = 1 ]; then ok "$name"; else bad "$name (exit $RC: $OUT)"; fi
}

echo "classify"
expect_docs_only "an RFC alone is docs-only" docs/rfc/0001-x.md
expect_docs_only "research notes are docs-only" docs/research/notes.md
expect_docs_only "factory files and decisions are docs-only" factory/GENESIS.md factory/decisions/2026-11-15-NEX-36-x.md
expect_docs_only "a change fragment is docs-only" changes/a-fragment.md
expect_docs_only "all four together are docs-only" docs/rfc/a.md docs/research/b.md factory/c.md changes/d.md
expect_full "one code file next to docs is a full run" docs/rfc/a.md crates/core/src/lib.rs
expect_full "a workflow is a full run" .github/workflows/ci.yml
expect_full "this script is a full run" .github/scripts/docs-only.sh
expect_full "the generated docs are a full run (CI checks them against the source)" docs/generated/nxf.md
expect_full "a spec under docs/ outside the allowlist is a full run" docs/specs/x.md
expect_full "a near-miss prefix is a full run (docs/rfcs/)" docs/rfcs/x.md
expect_full "a near-miss prefix is a full run (factoryx/)" factoryx/a.md
expect_full "a file named like a directory is a full run (docs/rfc)" docs/rfc
expect_full "a file named like a directory is a full run (changes)" changes
expect_full "case matters (Factory/)" Factory/a.md
expect_full "a root file is a full run" README.md
expect_full "AGENTS.md is a full run" AGENTS.md
expect_full "no path at all is a full run"
if [ "$RC" = 1 ] && grep -q "no changed path" <<<"$OUT"; then
  ok "an empty change says why it runs in full"
else
  bad "an empty change says why it runs in full ($OUT)"
fi
expect_full "one outside path is named" docs/rfc/a.md install.sh
if grep -q "install.sh" <<<"$OUT"; then ok "the outside path is named in the verdict"; else bad "the outside path is named in the verdict ($OUT)"; fi

# ---- decide: the events, against a real repository ------------------------------------------

ORIGIN="$WORK/origin.git"
SEED="$WORK/seed"
git init -q --bare "$ORIGIN"
git -C "$ORIGIN" config uploadpack.allowAnySHA1InWant true
git init -q "$SEED"
git -C "$SEED" remote add origin "file://$ORIGIN"

commit_files() { # <message> <path…> — writes each path with fresh content and commits in $SEED
  local msg="$1"; shift
  local p
  for p in "$@"; do
    mkdir -p "$SEED/$(dirname "$p")"
    printf '%s %s\n' "$msg" "$p" > "$SEED/$p"
    git -C "$SEED" add -- "$p"
  done
  git -C "$SEED" commit -q -m "$msg"
  git -C "$SEED" rev-parse HEAD
}

# The base branch `v2`: some code and some docs.
git -C "$SEED" checkout -q -b v2
BASE="$(commit_files base crates/core/src/lib.rs crates/core/src/gone.rs docs/rfc/0001-a.md factory/GENESIS.md)"
git -C "$SEED" push -q origin v2

# A pull request's merge commit, as GitHub builds `refs/pull/<n>/merge`: the head branch merged into
# the base tip, first parent the base. Echoes the merge commit.
pr_merge() { # <branch> <path…>
  local branch="$1"; shift
  git -C "$SEED" checkout -q -b "$branch" "$BASE"
  commit_files "$branch" "$@" >/dev/null
  git -C "$SEED" checkout -q --detach "$BASE"
  git -C "$SEED" merge -q --no-ff --no-edit "$branch" >/dev/null
  local merge
  merge="$(git -C "$SEED" rev-parse HEAD)"
  git -C "$SEED" push -q origin "$merge:refs/pull/$branch/merge" "$branch:$branch"
  echo "$merge"
}

# The checkout actions/checkout makes: one commit, depth 1, nothing else fetched.
checkout_shallow() { # <sha> → sets $REPO
  REPO="$WORK/checkout-$1"
  rm -rf "$REPO"
  git init -q "$REPO"
  git -C "$REPO" remote add origin "file://$ORIGIN"
  git -C "$REPO" fetch -q --no-tags --depth=1 origin "$1"
  git -C "$REPO" checkout -q --detach FETCH_HEAD
}

# Runs `decide` in $REPO. Sets $RC, $OUT, $DOCS_ONLY (the value written to $GITHUB_OUTPUT) and
# $SUMMARY. Pre-set knobs, read and reset here so no case leaks into the next: $EVENT, $REF, $SHA,
# $FULL, $BEFORE.
EVENT="" REF="" SHA="" FULL="" BEFORE=""
DOCS_ONLY=""
SUMMARY=""
decide() {
  RC=0
  : > "$WORK/output"
  : > "$WORK/summary"
  OUT="$(cd "$REPO" && env GITHUB_EVENT_NAME="$EVENT" GITHUB_REF_NAME="$REF" GITHUB_SHA="$SHA" \
    NXF_FULL="$FULL" NXF_PUSH_BEFORE="$BEFORE" \
    GITHUB_OUTPUT="$WORK/output" GITHUB_STEP_SUMMARY="$WORK/summary" \
    bash "$SCRIPT" decide "the test suites" 2>&1)" || RC=$?
  DOCS_ONLY="$(sed -n 's/^docs_only=//p' "$WORK/output")"
  SUMMARY="$(cat "$WORK/summary")"
  EVENT="" REF="" SHA="" FULL="" BEFORE=""
}

expect_decision() { # <case> <true|false>
  if [ "$RC" = 0 ] && [ "$DOCS_ONLY" = "$2" ]; then
    ok "$1"
  else
    bad "$1 (exit $RC, docs_only='$DOCS_ONLY')"
    printf '%s\n' "$OUT" | sed 's/^/       | /'
  fi
}

echo "decide: pull_request"
M="$(pr_merge pr-docs docs/rfc/0002-b.md docs/research/r.md factory/decisions/d.md changes/c.md)"
checkout_shallow "$M"
EVENT=pull_request REF=7/merge SHA="$M" decide
expect_decision "a pull request that changes only allowlisted paths is docs-only" true
if grep -q '^::notice' <<<"$OUT" && grep -q "the test suites" <<<"$OUT"; then
  ok "a docs-only job says so in a ::notice:: that names what it did not run"
else
  bad "a docs-only job says so in a ::notice:: that names what it did not run ($OUT)"
fi
if grep -q "the test suites" <<<"$SUMMARY" && grep -qi "docs-only" <<<"$SUMMARY"; then
  ok "a docs-only job writes a job-summary line naming what it did not run"
else
  bad "a docs-only job writes a job-summary line naming what it did not run ($SUMMARY)"
fi

M="$(pr_merge pr-mixed docs/rfc/0003-c.md crates/core/src/lib.rs)"
checkout_shallow "$M"
EVENT=pull_request REF=8/merge SHA="$M" decide
expect_decision "a pull request that also changes one code file runs in full" false
if ! grep -q '^::notice' <<<"$OUT" && [ -z "$SUMMARY" ]; then
  ok "a full run carries no docs-only notice and no summary line"
else
  bad "a full run carries no docs-only notice and no summary line ($OUT / $SUMMARY)"
fi

git -C "$SEED" checkout -q -b pr-rename "$BASE"
git -C "$SEED" mv crates/core/src/gone.rs docs/rfc/gone.rs
git -C "$SEED" commit -q -m rename
git -C "$SEED" checkout -q --detach "$BASE"
git -C "$SEED" merge -q --no-ff --no-edit pr-rename >/dev/null
M="$(git -C "$SEED" rev-parse HEAD)"
git -C "$SEED" push -q origin "$M:refs/pull/rename/merge"
checkout_shallow "$M"
EVENT=pull_request REF=9/merge SHA="$M" decide
expect_decision "moving a code file into docs/rfc/ runs in full (the code file is gone)" false

M="$(pr_merge pr-full docs/rfc/0004-d.md)"
checkout_shallow "$M"
EVENT=pull_request REF=10/merge SHA="$M" FULL=true decide
expect_decision "full=true runs in full even on a docs-only change" false

echo "decide: push"
git -C "$SEED" checkout -q v2
V2_DOCS="$(commit_files v2-docs docs/rfc/0005-e.md factory/CONFIG.md)"
git -C "$SEED" push -q origin v2
checkout_shallow "$V2_DOCS"
EVENT=push REF=v2 SHA="$V2_DOCS" BEFORE="$BASE" decide
expect_decision "a docs-only push to v2 is docs-only" true

V2_CODE="$(commit_files v2-code crates/core/src/lib.rs)"
git -C "$SEED" push -q origin v2
checkout_shallow "$V2_CODE"
EVENT=push REF=v2 SHA="$V2_CODE" BEFORE="$V2_DOCS" decide
expect_decision "a push to v2 that changes code runs in full" false

git -C "$SEED" push -q origin "$V2_DOCS:refs/heads/main" "$V2_DOCS:refs/heads/release/0.52"
checkout_shallow "$V2_DOCS"
EVENT=push REF=main SHA="$V2_DOCS" BEFORE="$BASE" decide
expect_decision "a docs-only push to main runs in full" false
checkout_shallow "$V2_DOCS"
EVENT=push REF=release/0.52 SHA="$V2_DOCS" BEFORE="$BASE" decide
expect_decision "a docs-only push to a maintenance line runs in full" false

checkout_shallow "$V2_DOCS"
EVENT=push REF=v2 SHA="$V2_DOCS" BEFORE=0000000000000000000000000000000000000000 decide
expect_decision "a push with no previous commit (a new branch) runs in full" false

checkout_shallow "$V2_DOCS"
EVENT=push REF=v2 SHA="$V2_DOCS" BEFORE=1234567890123456789012345678901234567890 decide
expect_decision "a push whose previous commit cannot be fetched runs in full" false

checkout_shallow "$V2_DOCS"
EVENT=push REF=v2 SHA="$V2_DOCS" BEFORE="" decide
expect_decision "a push with no 'before' at all runs in full" false

echo "decide: workflow_dispatch"
git -C "$SEED" checkout -q -b dispatch-docs "$V2_CODE"
D="$(commit_files dispatch docs/rfc/0006-f.md)"
git -C "$SEED" push -q origin dispatch-docs
checkout_shallow "$D"
EVENT=workflow_dispatch REF=dispatch-docs SHA="$D" decide
expect_decision "a dispatch on a branch that differs from v2 only in docs is docs-only" true

checkout_shallow "$D"
EVENT=workflow_dispatch REF=dispatch-docs SHA="$D" FULL=true decide
expect_decision "a dispatch with full=true runs in full" false

git -C "$SEED" checkout -q -b dispatch-code "$V2_CODE"
D="$(commit_files dispatch crates/core/src/lib.rs)"
git -C "$SEED" push -q origin dispatch-code
checkout_shallow "$D"
EVENT=workflow_dispatch REF=dispatch-code SHA="$D" decide
expect_decision "a dispatch on a branch that changes code runs in full" false

checkout_shallow "$V2_DOCS"
EVENT=workflow_dispatch REF=main SHA="$V2_DOCS" decide
expect_decision "a dispatch on main runs in full" false

checkout_shallow "$V2_CODE"
EVENT=workflow_dispatch REF=v2 SHA="$V2_CODE" decide
expect_decision "a dispatch on v2 itself (nothing changed) runs in full" false

echo "decide: anything else"
checkout_shallow "$V2_DOCS"
EVENT=schedule REF=v2 SHA="$V2_DOCS" BEFORE="$BASE" decide
expect_decision "an event the rule does not know runs in full" false

checkout_shallow "$V2_DOCS"
RC=0
OUT="$(cd "$REPO" && env -u GITHUB_OUTPUT GITHUB_EVENT_NAME=push GITHUB_REF_NAME=v2 \
  GITHUB_SHA="$V2_DOCS" NXF_PUSH_BEFORE="$BASE" bash "$SCRIPT" decide "the test suites" 2>&1)" || RC=$?
if [ "$RC" != 0 ]; then
  ok "with nowhere to write its verdict the step fails rather than let the job run unguarded"
else
  bad "with nowhere to write its verdict the step fails rather than let the job run unguarded"
fi

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
