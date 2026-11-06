#!/usr/bin/env bash
# Repeat ONE named test and say, in CI's own log, what was measured (6j6v.jmxk). The body of
# `.github/workflows/isolated-test.yml`, which says what the run is for and what it can never do.
#
# Green means exactly this: the named test, and nothing else, executed in every one of at least three
# repeats, and never went red. Everything short of that exits non-zero, so a filter that matched
# nothing cannot pass as a clean result: libtest exits 0 for "0 passed; 0 failed".
#
# The last thing it prints is one line a reader can grep from the run's log:
#
#   isolated-test-result commit=<sha> test=<name> package=<p> target=<t> system=<os> profile=<p> repeats=<n> executed=<n> reds=<n> verdict=<pass|fail>
#
# `executed` counts the repeats whose own summary shows this one test passing or failing; an ignored
# test, a filtered-out test or a process that died before its summary does not count. `reds` counts
# the repeats that did not end green, so a test that failed counts in both and a process that died
# only in `reds`.
#
# Environment: ISOLATED_PACKAGE, ISOLATED_TARGET (`lib`, or the file name of an integration test),
# ISOLATED_TEST (the full name, as libtest prints it), ISOLATED_PROFILE (`debug` or `release`),
# ISOLATED_REPEATS (3 or more).
#
# Hermetically tested by `tests/isolated-test.test.sh`, which stubs `cargo`.
set -euo pipefail

export CARGO_TERM_COLOR=never

refuse() {
  echo "::error::isolated test run refused: $1"
  exit 2
}

package="${ISOLATED_PACKAGE:-}"
target="${ISOLATED_TARGET:-}"
test_name="${ISOLATED_TEST:-}"
profile="${ISOLATED_PROFILE:-}"
repeats="${ISOLATED_REPEATS:-}"

# The inputs reach this script through the environment and are never evaluated, so these checks are
# about refusing a run that could not count, not about quoting.
[[ "$package" =~ ^[A-Za-z0-9_-]+$ ]] || refuse "package '$package' is not a package name"
[[ "$target" =~ ^[A-Za-z0-9_-]+$ ]] || refuse "target '$target' is neither 'lib' nor an integration test's file name"
[[ "$test_name" =~ ^[A-Za-z0-9_:]+$ ]] || refuse "test '$test_name' is not one full test name; a pattern does not count"
case "$profile" in
  debug | release) ;;
  *) refuse "profile '$profile' is neither debug nor release" ;;
esac
[[ "$repeats" =~ ^[0-9]+$ ]] || refuse "repeats '$repeats' is not a number"
[ "$((10#$repeats))" -ge 3 ] || refuse "repeats is $repeats; fewer than three does not count"
repeats=$((10#$repeats))

select_args=(-p "$package")
if [ "$target" = lib ]; then
  select_args+=(--lib)
else
  select_args+=(--test "$target")
fi
if [ "$profile" = release ]; then
  select_args+=(--release)
fi

# The build's own failure is not a measurement of the test: `set -e` stops the run here, with no
# result line.
cargo test "${select_args[@]}" --no-run

# The selection has to be this test alone, and has to exist, before a repeat means anything.
listed="$(cargo test "${select_args[@]}" -- --exact "$test_name" --list --format terse 2>/dev/null | grep -E ': (test|benchmark)$' || true)"
if [ "$listed" != "$test_name: test" ]; then
  echo "selected by this filter:"
  printf '%s\n' "${listed:-(nothing)}"
  refuse "the filter does not select exactly the test '$test_name' in $package/$target"
fi

executed=0
reds=0
for ((i = 1; i <= repeats; i++)); do
  echo "::group::repeat $i of $repeats"
  log="$(mktemp)"
  code=0
  cargo test "${select_args[@]}" -- --exact "$test_name" --color never 2>&1 | tee "$log" || code=$?
  echo "::endgroup::"

  summaries="$(grep -E '^test result: ' "$log" || true)"
  rm -f "$log"
  passed=""
  failed=""
  # One binary is selected, so one summary. Any other number is not this test's verdict.
  if [ "$(printf '%s' "$summaries" | grep -c '^test result: ' || true)" = 1 ]; then
    passed="$(printf '%s\n' "$summaries" | sed -n 's/.* \([0-9][0-9]*\) passed;.*/\1/p')"
    failed="$(printf '%s\n' "$summaries" | sed -n 's/.* \([0-9][0-9]*\) failed;.*/\1/p')"
  fi

  if [ "$((${passed:-0} + ${failed:-0}))" = 1 ]; then
    executed=$((executed + 1))
  fi
  if [ "$code" != 0 ] || [ "${failed:-0}" != 0 ]; then
    reds=$((reds + 1))
    echo "::error::repeat $i of $repeats of $test_name went red (exit $code)"
  fi
done

verdict=fail
if [ "$executed" = "$repeats" ] && [ "$reds" = 0 ]; then
  verdict=pass
fi

result="isolated-test-result commit=$(git rev-parse HEAD) test=$test_name package=$package target=$target system=${RUNNER_OS:-$(uname -s)}/${ImageOS:-local} profile=$profile repeats=$repeats executed=$executed reds=$reds verdict=$verdict"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  printf '%s\n' "$result" >>"$GITHUB_STEP_SUMMARY"
fi
echo "$result"
[ "$verdict" = pass ]
