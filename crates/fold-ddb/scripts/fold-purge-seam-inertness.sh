#!/usr/bin/env bash
# The shipped nxf-fold-purge carries no purge test seam (6j6v.dbmd DoD; MF-2: CI runs this against
# the binary `cargo build -p nxs-fold-ddb --bin nxf-fold-purge --features dynamodb --release`
# makes, and the release workflow runs it against the same build of the tagged commit).
#
# A build with the `test-seams` feature reads NXF_FOLD_TEST_PURGE_*; a shipped one never does. So:
# make a board-fold table and a deny table on DynamoDB Local, seed one stream's partition and put its
# account on the deny list, then purge it with EVERY seam variable set to something that would make
# a seam-carrying build abort or inject. The shipped binary must exit 0 with the partition empty —
# and confirm it.
#
# Usage: fold-purge-seam-inertness.sh <path to nxf-fold-purge> <DynamoDB Local endpoint>
#        (needs the aws CLI and jq; any credentials do for DynamoDB Local)
set -euo pipefail

purge="${1:?usage: fold-purge-seam-inertness.sh <nxf-fold-purge> <endpoint>}"
endpoint="${2:?usage: fold-purge-seam-inertness.sh <nxf-fold-purge> <endpoint>}"
export AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID:-test}" AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY:-test}"
export AWS_REGION=us-east-1 AWS_DEFAULT_REGION=us-east-1 AWS_PAGER=""
suffix="$$_${RANDOM}"
fold="nxf_seam_fold_${suffix}"
deny="nxf_seam_deny_${suffix}"
h="5ea15ea15ea15ea15ea15ea15ea15ea1"
stream="n-${h}-1"
work="$(mktemp -d)"
ddb() { aws dynamodb --endpoint-url "$endpoint" "$@"; }
cleanup() {
  ddb delete-table --table-name "$fold" >/dev/null 2>&1 || true
  ddb delete-table --table-name "$deny" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
fail() { echo "fold-purge-seam-inertness: $*" >&2; exit 1; }

gsi() {
  printf '{"IndexName":"%s","KeySchema":[{"AttributeName":"%s","KeyType":"HASH"},{"AttributeName":"%s","KeyType":"RANGE"}],"Projection":{"ProjectionType":"ALL"}}' "$1" "$2" "$3"
}
# The board-fold table exactly as nxs_fold_ddb::ddb::create_table makes it.
ddb create-table --table-name "$fold" --billing-mode PAY_PER_REQUEST \
  --attribute-definitions AttributeName=stream_id,AttributeType=S AttributeName=sk,AttributeType=S \
    AttributeName=nxf_active,AttributeType=S AttributeName=nxf_dated,AttributeType=S \
    AttributeName=nxf_dated_at,AttributeType=S AttributeName=nxf_typed,AttributeType=S \
  --key-schema AttributeName=stream_id,KeyType=HASH AttributeName=sk,KeyType=RANGE \
  --global-secondary-indexes "[$(gsi active nxf_active sk),$(gsi dated nxf_dated nxf_dated_at),$(gsi typed nxf_typed sk)]" \
  >/dev/null
# The deny table exactly as nxs_fold_ddb::ddb::create_deny_table makes it.
ddb create-table --table-name "$deny" --billing-mode PAY_PER_REQUEST \
  --attribute-definitions AttributeName=h,AttributeType=S --key-schema AttributeName=h,KeyType=HASH \
  >/dev/null
ddb wait table-exists --table-name "$fold"
ddb wait table-exists --table-name "$deny"

put() { ddb put-item --table-name "$fold" --item "$1" >/dev/null; }
for i in $(seq 1 30); do
  id=$(printf 'ab12.%04d' "$i")
  put "{\"stream_id\":{\"S\":\"$stream\"},\"sk\":{\"S\":\"items#$id\"},\"id\":{\"S\":\"$id\"},\"title\":{\"S\":\"seam\"}}"
  put "{\"stream_id\":{\"S\":\"$stream\"},\"sk\":{\"S\":\".adj#$id#t$i\"}}"
done
put "{\"stream_id\":{\"S\":\"$stream\"},\"sk\":{\"S\":\".meta#watermark\"},\"folded_through\":{\"N\":\"60\"},\"revision_index\":{\"N\":\"1\"}}"
ddb put-item --table-name "$deny" \
  --item "{\"h\":{\"S\":\"$h\"},\"v\":{\"N\":\"1\"},\"since\":{\"S\":\"2026-10-23T00:00:00Z\"}}" >/dev/null

export NXF_FOLD_DDB_TABLE="$fold" NXF_FOLD_DDB_REGION=us-east-1 NXF_FOLD_DDB_ENDPOINT="$endpoint"
export NXS_DENY_TABLE="$deny"
# Every seam variable, each set to act: a seam-carrying build would abort after the first batch,
# fold one more item after the rows, and read the plan a key a page.
export NXF_FOLD_TEST_PURGE_STOP_AFTER=RowsBatch:1
export NXF_FOLD_TEST_PURGE_INJECT_AT=Rows
export NXF_FOLD_TEST_PURGE_PAGE=1

"$purge" "$stream" --dry-run >"$work/dry.json" 2>"$work/dry.err" \
  || fail "the dry run exited $?: $(cat "$work/dry.json" "$work/dry.err")"
found=$(jq '[.kinds[].found] | add' "$work/dry.json")
[ "$found" = "61" ] || fail "the dry run found $found keys, expected 61"
digest=$(jq -r .plan_digest "$work/dry.json")
pin=$(jq -r .store.identity_digest "$work/dry.json")

set +e
"$purge" "$stream" --yes --plan "$digest" >"$work/yes.json" 2>"$work/yes.err"
code=$?
set -e
[ "$code" = "0" ] || fail "--yes exited $code with every seam variable set: $(cat "$work/yes.json" "$work/yes.err")"
[ "$(jq -r .complete "$work/yes.json")" = "true" ] || fail "--yes not complete"
left=$(ddb query --table-name "$fold" --consistent-read --key-condition-expression "stream_id = :s" \
  --expression-attribute-values "{\":s\":{\"S\":\"$stream\"}}" --select COUNT --query Count)
[ "$left" = "0" ] || fail "$left items of the stream are left"

"$purge" "$stream" --dry-run --expect-empty --expect-identity "$pin" >"$work/confirm.json" 2>&1 \
  || fail "the confirm exited $?: $(cat "$work/confirm.json")"
echo "fold-purge-seam-inertness: the shipped nxf-fold-purge ignored every NXF_FOLD_TEST_PURGE_* variable"
