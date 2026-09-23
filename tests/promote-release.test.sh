#!/usr/bin/env bash
# Hermetic test harness for .github/scripts/promote-release.sh (nexus-flow-85y.15).
#
# No AWS, no network: a fake `aws` shim earlier on PATH simulates head-object / copy-object /
# s3 cp and records every call. The control file $FAKE_STATE selects which beta key is
# "missing" (404) or "denied" (403). Covers: the happy promotion (guard → 12 copies → stable
# manifest), and the two fail-closed guards (a missing .minisig and a 403 must each abort
# BEFORE any copy, with distinct messages). Mirrors the install.sh harness; gates CI.
#
# Run: tests/promote-release.test.sh   (needs bash, jq)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/.github/scripts/promote-release.sh"

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-promote-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# --- fake aws ---------------------------------------------------------------------------------
# Records each call to $CALLS; honours $FAKE_STATE ("missing:<key>" or "denied:<key>") for
# head-object so guard behaviour is exercised; emits plausible sidecar bytes for `s3 cp … -`.
BIN="$WORK/bin"
mkdir -p "$BIN"
cat >"$BIN/aws" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >>"$CALLS"
sub="$1"; shift
arg() { # arg <flag> <args...> → echoes the value after <flag>
  local want="$1"; shift
  while [ "$#" -gt 0 ]; do [ "$1" = "$want" ] && { echo "$2"; return; }; shift; done
}
if [ "$sub" = "s3api" ]; then
  op="$1"; shift
  case "$op" in
    head-object)
      key="$(arg --key "$@")"
      state="${FAKE_STATE:-}"
      if [ "$state" = "missing:$key" ]; then
        echo "An error occurred (404) when calling the HeadObject operation: Not Found" >&2; exit 255
      elif [ "$state" = "denied:$key" ]; then
        echo "An error occurred (403) when calling the HeadObject operation: Forbidden" >&2; exit 255
      fi
      echo '{"ContentLength": 123}'; exit 0 ;;
    copy-object) exit 0 ;;
    *) echo "fake aws: unknown s3api op $op" >&2; exit 2 ;;
  esac
fi
if [ "$sub" = "s3" ]; then
  op="$1"; shift
  if [ "$op" = "cp" ]; then
    src="$1"; dst="$2"
    case "$src" in
      # Reading a sidecar back from S3 to a file (or stdout): emit deterministic fake content.
      *.sha256)
        line="deadbeefcafefeed00000000000000000000000000000000000000000000abcd  $(basename "${src%.sha256}")"
        if [ "$dst" = "-" ]; then echo "$line"; else printf '%s\n' "$line" >"$dst"; fi ;;
      *.minisig)
        if [ "$dst" != "-" ]; then printf 'untrusted comment: x\nRWZ==\ntrusted comment: y\nZ2==\n' >"$dst"; fi ;;
      # Uploading a local file to S3: capture the stable manifest body so the test can assert it.
      *)
        case "$dst" in
          */manifests/stable.json) [ -n "${MANIFEST_OUT:-}" ] && cp "$src" "$MANIFEST_OUT" ;;
        esac ;;
    esac
    exit 0
  fi
  echo "fake aws: unknown s3 op $op" >&2; exit 2
fi
echo "fake aws: unknown command $sub" >&2; exit 2
FAKE
chmod +x "$BIN/aws"

VERSION="0.2.0"
BUCKET="nxs-staging-artifacts-FAKEACCT"
run_promote() { # FAKE_STATE value (may be empty) → runs the script with the fake aws on PATH
  : >"$WORK/manifest.json"
  ( cd "$WORK"
    CALLS="$WORK/calls.log" FAKE_STATE="${1:-}" MANIFEST_OUT="$WORK/manifest.json" \
    PATH="$BIN:$PATH" \
    VERSION="$VERSION" ARTIFACT_BUCKET="$BUCKET" DOMAIN="https://staging.nxsflow.com/nxs" \
    NOTES="### Added"$'\n'"- thing" \
      bash "$SCRIPT" )
}

echo "promote-release.sh (fake aws)"

# P1 happy path: guard passes, 12 server-side copies (the EXACT expected stable keys), and a
# well-formed stable manifest.
: >"$WORK/calls.log"
if out="$(run_promote "" 2>&1)"; then
  copies="$(grep -c '^s3api copy-object' "$WORK/calls.log" || true)"
  if [ "$copies" -eq 12 ]; then ok "P1 issued 12 server-side copies"; else bad "P1 expected 12 copies, got $copies"; fi
  # Assert WHICH keys were copied — every platform × {tarball,.sha256,.minisig} to the stable ring.
  missing_keys=0
  for p in darwin-aarch64 darwin-x86_64 linux-x86_64 linux-aarch64; do
    for ext in "" .sha256 .minisig; do
      key="download/stable/${VERSION}/nxf_${VERSION}_${p}.tar.gz${ext}"
      grep -q -- "--key $key " "$WORK/calls.log" || { missing_keys=$((missing_keys + 1)); }
    done
  done
  if [ "$missing_keys" -eq 0 ]; then ok "P1 copied the exact 12 expected stable keys"; else bad "P1 $missing_keys expected stable keys never copied"; fi
  # Assert WHICH BUCKET every call addressed. Without this the harness checks only the keys, so
  # `promote-release.sh` could address the wrong (or a stale, since-deleted) bucket with perfectly
  # correct keys and every assertion above would still pass — the gap that let the nxf-* → nxs-*
  # rename land unverified (review of PR #307, Test Quality #3).
  stray="$(grep -oE -- '--bucket [^ ]+|s3://[^/ ]+' "$WORK/calls.log" \
    | sed -E 's#^--bucket ##; s#^s3://##' | sort -u | grep -vFx "$BUCKET" || true)"
  if [ -z "$stray" ]; then
    ok "P1 every call addressed $BUCKET and no other bucket"
  else
    bad "P1 addressed unexpected bucket(s): $(printf '%s' "$stray" | tr '\n' ' ')"
  fi
  # Assert the MANIFEST BODY, not just that the upload happened.
  m="$WORK/manifest.json"
  if [ -s "$m" ] && jq -e '.enabled == true and .rollout == 100 and (.version | type == "string")' "$m" >/dev/null; then
    ok "P1 stable.json: enabled=true, rollout=100, version present"
  else
    bad "P1 stable.json missing/!enabled/!rollout100: $(cat "$m" 2>/dev/null)"
  fi
  if jq -e '(.platforms | keys | sort) == ["darwin-aarch64","darwin-x86_64","linux-aarch64","linux-x86_64"]' "$m" >/dev/null 2>&1; then
    ok "P1 stable.json carries all 4 platform entries"
  else
    bad "P1 stable.json platform keys wrong: $(jq -c '.platforms|keys' "$m" 2>/dev/null)"
  fi
  if jq -e '[.platforms[] | select((.path|startswith("download/stable/")) and (.sha256|test("^[0-9a-f]{64}$")) and (.signature|length>0))] | length == 4' "$m" >/dev/null 2>&1; then
    ok "P1 every platform entry has stable path + 64-hex sha256 + non-empty signature"
  else
    bad "P1 a platform entry has a malformed path/sha256/signature"
  fi
  if printf '%s\n' "$out" | grep -q 'Guard ok'; then ok "P1 guard ran and passed"; else bad "P1 no guard-ok line"; fi
  if printf '%s\n' "$out" | grep -q 'Promotion complete'; then ok "P1 completed"; else bad "P1 did not complete"; fi
else
  bad "P1 promotion failed unexpectedly: $out"
fi

# P2 fail-closed on a missing .minisig (404): abort, distinct 404 message, NO copies.
: >"$WORK/calls.log"
miss="download/beta/${VERSION}/nxf_${VERSION}_linux-aarch64.tar.gz.minisig"
if out="$(run_promote "missing:$miss" 2>&1)"; then
  bad "P2 should have failed on a missing .minisig"
else
  if printf '%s\n' "$out" | grep -q '404'; then ok "P2 missing sidecar → 404 fail-closed message"; else bad "P2 no 404 message: $out"; fi
  copies="$(grep -c '^s3api copy-object' "$WORK/calls.log" || true)"
  if [ "$copies" -eq 0 ]; then ok "P2 aborted before any copy"; else bad "P2 copied $copies objects despite missing sidecar"; fi
fi

# P3 a 403 is reported distinctly from a 404 (not swallowed as "missing").
: >"$WORK/calls.log"
den="download/beta/${VERSION}/nxf_${VERSION}_darwin-aarch64.tar.gz"
if out="$(run_promote "denied:$den" 2>&1)"; then
  bad "P3 should have failed on a 403"
else
  if printf '%s\n' "$out" | grep -q '403'; then ok "P3 denied → distinct 403 message"; else bad "P3 no 403 message: $out"; fi
  if printf '%s\n' "$out" | grep -qi 'IAM regression'; then ok "P3 names the IAM-regression cause"; else bad "P3 403 not distinguished from 404"; fi
  copies="$(grep -c '^s3api copy-object' "$WORK/calls.log" || true)"
  if [ "$copies" -eq 0 ]; then ok "P3 aborted before any copy (no side effects)"; else bad "P3 copied $copies objects despite a 403"; fi
  if [ ! -s "$WORK/manifest.json" ]; then ok "P3 wrote no manifest"; else bad "P3 wrote a manifest despite a 403"; fi
fi

echo ""
echo "promote-release.sh tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
