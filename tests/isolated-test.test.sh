#!/usr/bin/env bash
# Hermetic harness for the isolated test run (6j6v.jmxk): `.github/scripts/isolated-test.sh` against
# a stubbed `cargo`, and the three things the run must never be able to do, read from the files that
# would have to change for it to do them.
#
# Run: tests/isolated-test.test.sh   (needs bash, jq)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/.github/scripts/isolated-test.sh"
WORKFLOW="$ROOT/.github/workflows/isolated-test.yml"
CI="$ROOT/.github/workflows/ci.yml"
GATE="$ROOT/.github/scripts/require-green-ci.sh"

command -v jq >/dev/null 2>&1 || { echo "jq is required to run this harness"; exit 1; }

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-isolated-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin"

# The stub `cargo`. `STUB_LIST` is what `--list` prints; each real run takes the next word of
# `STUB_RUNS` (the last one repeats): pass, fail, ignored, none (a filter that matched nothing), or
# die (the process ended before its summary). Every call's arguments go to `$WORK/calls`.
cat > "$WORK/bin/cargo" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "$STUB_DIR/calls"
case "$*" in
  *--no-run*) exit "${STUB_BUILD_EXIT:-0}" ;;
  *--list*) printf '%b' "$STUB_LIST"; exit 0 ;;
esac
n="$(cat "$STUB_DIR/count" 2>/dev/null || echo 0)"
n=$((n + 1))
echo "$n" > "$STUB_DIR/count"
read -r -a runs <<< "$STUB_RUNS"
i=$((n <= ${#runs[@]} ? n - 1 : ${#runs[@]} - 1))
case "${runs[$i]}" in
  pass)    echo "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 0.01s" ;;
  fail)    echo "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 41 filtered out; finished in 0.01s"; exit 101 ;;
  ignored) echo "test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 41 filtered out; finished in 0.01s" ;;
  none)    echo "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s" ;;
  die)     exit 134 ;;
esac
STUB
chmod +x "$WORK/bin/cargo"

# run <list> <runs> <repeats> [VAR=value ...]: sets $code and $out, from a fresh stub state.
run() {
  local list="$1" runs="$2" repeats="$3"
  shift 3
  rm -f "$WORK/calls" "$WORK/count"
  code=0
  out="$(cd "$ROOT" && env PATH="$WORK/bin:$PATH" STUB_DIR="$WORK" STUB_LIST="$list" STUB_RUNS="$runs" \
    RUNNER_OS=Linux ImageOS=ubuntu24 GITHUB_STEP_SUMMARY="$WORK/summary" \
    ISOLATED_PACKAGE=nexus-flow-facade ISOLATED_TARGET=engine ISOLATED_TEST=the_test \
    ISOLATED_PROFILE=debug ISOLATED_REPEATS="$repeats" "$@" \
    bash "$SCRIPT" 2>&1)" || code=$?
}

expect() { # <name> <exit> <grep -E pattern on the output>
  if [ "$code" = "$2" ] && printf '%s\n' "$out" | grep -qE -- "$3"; then
    ok "$1"
  else
    bad "$1 (exit $code)"
    printf '%s\n' "$out" | tail -5 | sed 's/^/       /'
  fi
}

ONE='the_test: test\n'
HEAD="$(git -C "$ROOT" rev-parse HEAD)"

echo "isolated-test.sh"

run "$ONE" "pass" 5
expect "every repeat executed and green passes, and the record names all of it" 0 \
  "^isolated-test-result commit=$HEAD test=the_test package=nexus-flow-facade target=engine system=Linux/ubuntu24 profile=debug repeats=5 executed=5 reds=0 verdict=pass$"
if grep -q "verdict=pass" "$WORK/summary"; then ok "the record reaches the job summary too"; else bad "the record reaches the job summary too"; fi

run "$ONE" "pass" 3
expect "three repeats count" 0 "repeats=3 executed=3 reds=0 verdict=pass"

run "$ONE" "pass" 2
expect "two repeats do not count" 2 "fewer than three does not count"
if [ ! -e "$WORK/calls" ]; then ok "a refused repeat count runs nothing"; else bad "a refused repeat count runs nothing"; fi

run "$ONE" "pass" 0
expect "zero repeats do not count" 2 "fewer than three does not count"

run "$ONE" "pass" "three"
expect "repeats that are not a number do not count" 2 "not a number"

run "" "none" 5
expect "a filter matching nothing is refused before any repeat" 2 "does not select exactly"

run 'the_test: test\nmod::the_test: test\n' "pass" 5
expect "a selection of several tests is refused" 2 "does not select exactly"

run 'other_test: test\n' "pass" 5
expect "a selection of another test is refused" 2 "does not select exactly"

run "$ONE" "pass" 5 ISOLATED_TEST="the_"
expect "a partial name that lists nothing under that name is refused" 2 "does not select exactly"

run "$ONE" "pass" 5 ISOLATED_TEST="the_test --include-ignored"
expect "a name carrying more arguments is refused" 2 "not one full test name"

run "$ONE" "none" 5
expect "a repeat that executed nothing fails the run even though it exited 0" 1 "executed=0 reds=0 verdict=fail"

run "$ONE" "ignored" 5
expect "an ignored test did not execute" 1 "executed=0 reds=0 verdict=fail"

run "$ONE" "pass fail pass" 5
expect "one red fails the run, is counted, and the later repeats still run" 1 "repeats=5 executed=5 reds=1 verdict=fail"
if [ "$(cat "$WORK/count")" = 5 ]; then ok "a red does not stop the repeats"; else bad "a red does not stop the repeats"; fi

run "$ONE" "pass die pass" 5
expect "a process that died is a red and did not execute to a verdict" 1 "repeats=5 executed=4 reds=1 verdict=fail"

run "$ONE" "pass" 3 STUB_BUILD_EXIT=101
if [ "$code" != 0 ] && ! printf '%s\n' "$out" | grep -q isolated-test-result; then
  ok "a build that fails is red and leaves no result line"
else
  bad "a build that fails is red and leaves no result line (exit $code)"
fi

run "$ONE" "pass" 3
if grep -q -- '--test engine -- --exact the_test --color never' "$WORK/calls" && ! grep -q -- '--release' "$WORK/calls"; then
  ok "debug selects the target and the exact name, without --release"
else
  bad "debug selects the target and the exact name, without --release"
fi

run "$ONE" "pass" 3 ISOLATED_PROFILE=release ISOLATED_TARGET=lib
if grep -q -- '-p nexus-flow-facade --lib --release -- --exact the_test' "$WORK/calls"; then
  ok "release and lib select --lib --release"
else
  bad "release and lib select --lib --release"
fi

run "$ONE" "pass" 3 ISOLATED_PROFILE=fast
expect "an unknown profile is refused" 2 "neither debug nor release"

echo "what the workflow must never be able to do"

if grep -qE '^[[:space:]]*concurrency:' "$WORKFLOW"; then
  bad "it declares no concurrency group, so it cannot cancel a ci.yml run"
else
  ok "it declares no concurrency group, so it cannot cancel a ci.yml run"
fi

triggers="$(sed -n '/^on:/,/^[a-z-]*:/p' "$WORKFLOW" | grep -E '^  [a-z_]+:' || true)"
if [ "$triggers" = "  workflow_dispatch:" ]; then
  ok "it starts only by dispatch"
else
  bad "it starts only by dispatch (found: $triggers)"
fi

if grep -q 'quality-gates:' "$WORKFLOW" || grep -q 'name: Tests (release)' "$WORKFLOW"; then
  bad "it has no job or step a release gate looks for by name"
else
  ok "it has no job or step a release gate looks for by name"
fi

# The release gate, against a `gh` that shows a green dispatch of THIS workflow and nothing of ci.yml.
cat > "$WORK/bin/gh" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "$STUB_DIR/gh-calls"
case "$*" in
  *workflows/isolated-test.yml/runs*)
    echo '{"id":7,"status":"completed","conclusion":"success","event":"workflow_dispatch","html_url":"https://example.invalid/7"}' ;;
esac
exit 0
STUB
chmod +x "$WORK/bin/gh"
rm -f "$WORK/gh-calls"
gate_code=0
env PATH="$WORK/bin:$PATH" STUB_DIR="$WORK" GH_REPO=o/r SHA=abc CI_GATE_APPEAR_TIMEOUT=0 CI_GATE_POLL=0 \
  bash "$GATE" >/dev/null 2>&1 || gate_code=$?
if [ "$gate_code" != 0 ] && grep -q 'workflows/ci.yml/runs' "$WORK/gh-calls" && ! grep -q 'isolated-test' "$WORK/gh-calls"; then
  ok "the release gate reads ci.yml's runs only, and a green isolated run does not open it"
else
  bad "the release gate reads ci.yml's runs only, and a green isolated run does not open it (exit $gate_code)"
fi

echo "ci.yml"

# Every test command that gates a job measures all its targets. Comments mention `cargo test` too.
missing="$(grep -nE '^[[:space:]]*(run:[[:space:]]*)?cargo test( |$)' "$CI" | grep -v -- '--no-fail-fast' || true)"
if [ -z "$missing" ]; then
  ok "every cargo test in ci.yml passes --no-fail-fast"
else
  bad "every cargo test in ci.yml passes --no-fail-fast"
  printf '%s\n' "$missing" | sed 's/^/       /'
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
