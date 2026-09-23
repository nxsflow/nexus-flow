#!/usr/bin/env bash
# Hermetic test harness for .github/scripts/require-green-ci.sh (nexus-flow 6j6v.31rs).
#
# That script is the release's fail-closed quality gate: it decides whether a tagged commit is
# allowed to be built, signed, notarized and published, on the strength of a CI run it did not
# perform. It replaced a job that ran the test suite a second time — a duplicate that was at least
# obviously honest. What replaces it has to be provably fail-closed instead of merely written to be,
# so every way OUT of the gate is asserted here, not just the happy one.
#
# Hermetic: `gh` is a stub on PATH that serves a canned `workflow_runs` payload from a file, and the
# timeouts are shrunk to milliseconds-worth of polling. No network, no token, no GitHub.
#
# MUTATION-CHECKED, recorded rather than asserted (PR #300 review, Test Quality #2 — this repo's own
# bar for the phrase is `docs/specs/E5c-chat-orchestration-api.md`'s "four deliberate perturbations,
# each seen red"). Five perturbations were planted in `.github/scripts/require-green-ci.sh`, each
# seen RED here, each reverted:
#
#   M1  drop `select(.event == "push" or .event == "workflow_dispatch")`
#       → "a green PULL_REQUEST run does not count" flipped to exit 0.          2 cases red
#   M2  `select(.status == "completed" and .conclusion == "success")`
#         → `select(.status == "completed")`
#       → "a red push run is a RED release" and the cancelled twin both passed. 2 cases red
#   M3  `exit 0` in the "nothing found" branch (the classic silent pass)
#       → "no run at all is a RED release" flipped to exit 0.                   3 cases red
#   M4  an API failure optimistically answered with a synthetic green run
#       → "a failing `gh` fails the gate" flipped to exit 0.                    1 case red
#   M5  `pending=0` hardcoded (never wait for an in-flight run)
#       → the queued-then-green case failed, and the never-finishing one lost
#         its "still running" wording.                                          2 cases red
#
# One perturbation that did NOT produce a red is worth recording too, because it says something
# about the gate rather than the harness: appending `|| echo "[]"` to the `gh` pipeline still ends
# in a non-zero exit. Swallowing the API error there cannot manufacture a pass — it can only
# mis-diagnose one. That is why the `gh`-failure case below asserts the MESSAGE, not just the code.
#
# Run: tests/require-green-ci.test.sh   (needs bash, jq)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GATE="$ROOT/.github/scripts/require-green-ci.sh"

command -v jq >/dev/null 2>&1 || { echo "jq is required to run this harness"; exit 1; }

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-ci-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin"

# The stub `gh`. It ignores its arguments except for honouring `--jq`, and instead replays the
# response bodies listed in $GH_REPLIES, one per invocation, so a test can model "still running,
# then green". The last body is reused once the list runs out. `$GH_EXIT` (default 0) makes the
# stub fail instead, which is how "GitHub could not be asked" is tested.
cat > "$WORK/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [ "${GH_EXIT:-0}" != "0" ]; then
  echo "gh: simulated API failure" >&2
  exit "$GH_EXIT"
fi
n="$(cat "$GH_CALL_COUNT" 2>/dev/null || echo 0)"
# shellcheck disable=SC2206  # deliberate word splitting: GH_REPLIES is a space-separated list
replies=($GH_REPLIES)
idx="$n"
[ "$idx" -ge "${#replies[@]}" ] && idx=$(( ${#replies[@]} - 1 ))
echo $(( n + 1 )) > "$GH_CALL_COUNT"
# The real `gh api --jq` applies the filter itself; mirror just enough of that.
filter=""
want_filter=0
for arg in "$@"; do
  if [ "$want_filter" = 1 ]; then filter="$arg"; want_filter=0; continue; fi
  [ "$arg" = "--jq" ] && want_filter=1
done
if [ -n "$filter" ]; then
  jq -c "$filter" < "${replies[$idx]}"
else
  cat "${replies[$idx]}"
fi
STUB
chmod +x "$WORK/bin/gh"

# One `workflow_runs` body. Each argument is `status:conclusion:event`.
body() { # <file> <run…>
  local out="$1"; shift
  local runs="" r
  for r in "$@"; do
    IFS=: read -r status conclusion event <<< "$r"
    runs="$runs{\"id\":1,\"status\":\"$status\",\"conclusion\":$( [ "$conclusion" = "null" ] && echo null || echo "\"$conclusion\"" ),\"event\":\"$event\",\"html_url\":\"https://example.invalid/run\"},"
  done
  printf '{"total_count":%d,"workflow_runs":[%s]}\n' "$#" "${runs%,}" > "$out"
}

# Run the gate against a list of canned bodies. Sets $RC and $OUT in THIS shell (never through a
# command substitution, which would run the function in a subshell and lose both).
#
# The four knobs a case may pre-set are read and reset here, so a case can never leak one into the
# next: $GH_EXIT, $RUN_TIMEOUT, $APPEAR_TIMEOUT.
RC=0
OUT=""
GH_EXIT=0
RUN_TIMEOUT=0
APPEAR_TIMEOUT=0
run_gate() { # <body-file…>
  RC=0
  : > "$WORK/calls"
  OUT="$(
    PATH="$WORK/bin:$PATH" \
    GH_REPLIES="$*" \
    GH_CALL_COUNT="$WORK/calls" \
    GH_EXIT="$GH_EXIT" \
    GH_REPO="nxsflow/nexus-flow" \
    SHA="deadbeef" \
    CI_GATE_POLL=0 \
    CI_GATE_RUN_TIMEOUT="$RUN_TIMEOUT" \
    CI_GATE_APPEAR_TIMEOUT="$APPEAR_TIMEOUT" \
    bash "$GATE" 2>&1
  )" || RC=$?
  GH_EXIT=0
  RUN_TIMEOUT=0
  APPEAR_TIMEOUT=0
}

case_exit() { # <name> <expected-rc> <body-file…>
  local name="$1" want="$2"; shift 2
  run_gate "$@"
  if [ "$RC" = "$want" ]; then ok "$name"; else bad "$name (expected exit $want, got $RC)
$OUT"; fi
}

says() { # <name> <needle>
  case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 — output was:
$OUT" ;; esac
}

never_says() { # <name> <needle>
  case "$OUT" in *"$2"*) bad "$1 — output was:
$OUT" ;; *) ok "$1" ;; esac
}

echo "require-green-ci.sh — every way out of the gate"

# --- the one way to pass ------------------------------------------------------------------------
body "$WORK/green.json"        completed:success:push
case_exit "a completed, successful push run passes" 0 "$WORK/green.json"
says "…and says it is trusting that run rather than repeating it" "trusts that run instead of repeating it"

body "$WORK/dispatch.json"     completed:success:workflow_dispatch
case_exit "a workflow_dispatch run passes too (it runs the full both-profile gate)" 0 "$WORK/dispatch.json"

body "$WORK/green_among.json"  completed:failure:push completed:success:push
case_exit "a green run passes even next to an earlier red attempt on the same commit" 0 "$WORK/green_among.json"

# --- every way to fail --------------------------------------------------------------------------
body "$WORK/red.json"          completed:failure:push
case_exit "a red push run is a RED release" 1 "$WORK/red.json"

body "$WORK/cancelled.json"    completed:cancelled:push
case_exit "a cancelled run is not a green one" 1 "$WORK/cancelled.json"

body "$WORK/pr_only.json"      completed:success:pull_request
case_exit "a green PULL_REQUEST run does not count (it skips the release profile)" 1 "$WORK/pr_only.json"
says "…and the refusal says why" "pull_request"
# This is the shape a staging rehearsal hits (6j6v.rvyp): a feature branch has only `pull_request`
# runs, so the gate refuses — correctly — and the reader needs to be told the way OUT of it, not
# just the tag-on-`main` path they are not on. Costing every rehearser a failed run plus a five-
# minute wait to discover a two-line workaround is how a facility acquires a reputation for being
# "a bit odd", which is the erosion 6j6v.xk2t was about.
says "…and names the branch-dispatch route out, not only the tag-on-main one" "--ref <branch>"

body "$WORK/none.json"
case_exit "no run at all is a RED release, never a pass" 1 "$WORK/none.json"
says "…and names what it looked for" "no push/workflow_dispatch run"

body "$WORK/running.json"      in_progress:null:push
case_exit "a run that never finishes runs out of the wait and goes red" 1 "$WORK/running.json"
says "…and does not claim the tree is untested, only unfinished" "still running"

# --- waiting, and what it is for ----------------------------------------------------------------
body "$WORK/queued.json"       queued:null:push
RUN_TIMEOUT=60
case_exit "a queued run is waited for, then passes when it goes green" 0 \
  "$WORK/queued.json" "$WORK/running.json" "$WORK/green.json"

APPEAR_TIMEOUT=60
case_exit "a run that has not been created yet is waited for" 0 \
  "$WORK/none.json" "$WORK/none.json" "$WORK/green.json"

# --- not being able to ask is not permission to proceed -----------------------------------------
#
# The appear-window is deliberately WIDE here. A gate that treated an API failure as "no runs found"
# would still end up red, but only after burning the whole window and then blaming the wrong thing —
# a diagnosis that sends the next reader looking for a missing CI run instead of a broken token or a
# rate limit. So this asserts the error PROPAGATES: exit non-zero carrying `gh`'s own message, and
# never the "there is no run" wording.
GH_EXIT=1
APPEAR_TIMEOUT=60
case_exit "a failing \`gh\` fails the gate rather than falling through" 1 "$WORK/green.json"
says "…carrying the API failure itself" "simulated API failure"
never_says "…and never mis-reports it as a missing CI run" "no push/workflow_dispatch run"

echo
echo "passed: $PASS   failed: $FAIL"
[ "$FAIL" -eq 0 ]
