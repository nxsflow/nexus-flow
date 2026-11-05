#!/usr/bin/env bash
#
# The engine's determinism step of the `lint` job (spec 6j6v.zjyv §11). It runs `decide` over the
# scripted cases in two SEPARATE PROCESSES, under a different clock and environment, and compares the
# decision lists byte for byte. A second run in one process would prove less: the seed of a `HashMap`
# is per process, and so is whatever `decide` might read at startup.
#
# Until nexus-run-engine exists this says so and passes; the moment it exists, it requires the test
# (fail-closed). The contract with that test, crates/run-engine/tests/determinism.rs:
# write every scripted case's decision lists, in script order, to the file named by
# $NXR_DETERMINISM_OUT. Writing nothing fails this step, because it would prove nothing.
#
# The later process runs on a later wall clock, in another time zone and with other variables set.
set -euo pipefail

if [ ! -d crates/run-engine ]; then
  echo "engine determinism: nexus-run-engine is not built yet, nothing to run"
  exit 0
fi

if [ ! -f crates/run-engine/tests/determinism.rs ]; then
  echo "engine determinism: crates/run-engine exists without tests/determinism.rs" >&2
  exit 1
fi

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

TZ=UTC LC_ALL=C NXR_DETERMINISM_PROBE=first \
  NXR_DETERMINISM_OUT="$out/first" \
  cargo test -p nexus-run-engine --test determinism

sleep "${NXR_DETERMINISM_PAUSE_S:-2}"

TZ=Pacific/Auckland LC_ALL=POSIX NXR_DETERMINISM_PROBE=second NXC_ACTOR=another NXF_NOW=2001-01-01 \
  NXR_DETERMINISM_OUT="$out/second" \
  cargo test -p nexus-run-engine --test determinism

if [ ! -s "$out/first" ] || [ ! -s "$out/second" ]; then
  echo "engine determinism: the test wrote nothing to \$NXR_DETERMINISM_OUT, which proves nothing" >&2
  exit 1
fi

if ! cmp "$out/first" "$out/second"; then
  echo "engine determinism: the decision lists of the two processes differ" >&2
  diff "$out/first" "$out/second" | head -40 >&2 || true
  exit 1
fi

echo "engine determinism: two processes, $(wc -c < "$out/first") identical bytes of decisions"
