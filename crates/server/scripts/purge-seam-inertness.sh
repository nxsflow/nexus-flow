#!/usr/bin/env bash
# The shipped nxf-relay carries no purge test seam (6j6v.f6yb DoD; MF-4: CI runs this against the
# binary `cargo build -p nxs-server --bin nxf-relay --release` makes, and the release workflow runs
# it against the release artefact itself).
#
# A relay built with the `test-seams` feature reads NXF_RELAY_TEST_PURGE_*; a shipped one never
# does. So: seed a SQLite relay store through the binary's own server (which also proves that
# `nxf-relay` with no arguments still serves), then purge one stream with EVERY seam variable set
# to something that would make a seam-carrying build abort, fail, pause or inject. The shipped
# binary must exit 0 with the target empty — and confirm it.
#
# Usage: purge-seam-inertness.sh <path to nxf-relay>    (needs curl and jq)
set -euo pipefail

relay="${1:?usage: purge-seam-inertness.sh <path to nxf-relay>}"
work="$(mktemp -d)"
pid=""
cleanup() {
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "purge-seam-inertness: $*" >&2
  [ -f "$work/relay.log" ] && sed 's/^/  relay: /' "$work/relay.log" >&2
  exit 1
}

db="$work/relay.sqlite"
port=$(( 20000 + RANDOM % 20000 ))
base="http://127.0.0.1:$port"
stream="n-seaminertness0000000000000000000-1"

# 1. The server, exactly as shipped: no arguments.
NXF_RELAY_DB="$db" NXF_RELAY_ADDR="127.0.0.1:$port" "$relay" >"$work/relay.log" 2>&1 &
pid=$!
for _ in $(seq 1 100); do
  if curl -s -o /dev/null "$base/streams/probe/ops"; then break; fi
  sleep 0.1
done
ops=$(jq -nc '{ops: [range(0;20) | {envelope_version: 1, op_id: ("seam-op-\(.)"), lamport: 1,
  site: 1, target_kind: "item", target_id: "ab12.0001", field: "title", op_type: "set",
  value: "seam value", author: "seam", wall_clock: "2026-10-18T20:00:00Z"}]}')
curl -sf -X POST -H 'content-type: application/json' -d "$ops" "$base/streams/$stream/ops" \
  >/dev/null || fail "the relay did not take a push"
curl -sf -X POST -H 'content-type: application/json' \
  -d '{"prefix":"ab12","replica_uuid":"seam-uuid"}' "$base/streams/$stream/register" \
  >/dev/null || fail "the relay did not take a register"
kill "$pid"
wait "$pid" 2>/dev/null || true
pid=""

# 2. The purge, with every seam variable set.
export NXF_RELAY_BACKEND=sqlite NXF_RELAY_DB="$db" NXF_RELAY_REGISTRY_DB="$db"
export NXF_RELAY_TEST_PURGE_STOP_AFTER=S0
export NXF_RELAY_TEST_PURGE_INJECT_AT=V0:op
export NXF_RELAY_TEST_PURGE_FAIL_AT=S1-tx
export NXF_RELAY_TEST_PURGE_PAUSE_AT=S0:600
export NXF_RELAY_TEST_PURGE_PAGE=1
export NXF_RELAY_TEST_PURGE_LEASE_SECS=1

"$relay" purge "$stream" --dry-run >"$work/dry.json" || fail "the dry run exited $?"
found=$(jq -r '.kinds[] | select(.kind == "ops") | .found' "$work/dry.json")
[ "$found" = "20" ] || fail "the dry run found $found ops, expected 20"
digest=$(jq -r .plan_digest "$work/dry.json")
pin=$(jq -r .store.identity_digest "$work/dry.json")

# A seam-carrying build would abort at S0 (the stop seam is checked first), so this never waits
# out the pause.
set +e
"$relay" purge "$stream" --yes --plan "$digest" >"$work/yes.json"
code=$?
set -e
[ "$code" = "0" ] || fail "--yes exited $code with every seam variable set: $(cat "$work/yes.json")"
[ "$(jq -r .complete "$work/yes.json")" = "true" ] || fail "--yes not complete"

"$relay" purge "$stream" --dry-run --expect-empty --expect-identity "$pin" >"$work/confirm.json" \
  || fail "the confirm exited $?: $(cat "$work/confirm.json")"
echo "purge-seam-inertness: the shipped nxf-relay ignored every NXF_RELAY_TEST_PURGE_* variable"
