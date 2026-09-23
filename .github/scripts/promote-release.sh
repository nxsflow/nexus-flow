#!/usr/bin/env bash
#
# promote-release.sh — promote a beta-ring release to stable by SERVER-SIDE COPY
# (nexus-flow-85y.15, spec §5.5). build-once-promote: the very same bytes AND signatures that
# release.yml published to beta are copied to stable — nothing is rebuilt or re-signed. AWS
# creds are already assumed by the caller (the OIDC release role, which carries Read+List on
# `download/*` for exactly this copy + Write for the manifest); this script is pure S3 +
# manifest assembly, no policy.
#
# Inputs (env):
#   VERSION          plain SemVer X.Y.Z (the just-released version)
#   ARTIFACT_BUCKET  target S3 bucket (name embeds the account id ⇒ masked in CI logs)
#   DOMAIN           public origin, for log context only
#   NOTES            aggregated stable notes (computed by the caller via `cargo xtask`)
#   NOTES_FILE       a FILE holding those notes; wins over NOTES when set. Use this from CI —
#                    see the `notes_arg` block below for why the env var alone cannot carry them.
#   FROM_CHANNEL     source ring                                          (default: beta)
#   TO_CHANNEL       target ring                                          (default: stable)
#
# Order is the whole point (the reference's hard-won lesson): the head-object GUARDS run FIRST
# and fail CLOSED — every platform tarball AND both sidecars must already exist in the beta
# ring, distinguishing 404 (object genuinely missing ⇒ beta is incomplete) from 403 (access
# denied ⇒ an IAM regression, NOT a missing object). A single missing `.minisig` aborts the
# whole promotion before one byte is copied; without the sidecars the stable manifest could
# not be signed, so promoting would strand an unverifiable stable ring.
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${ARTIFACT_BUCKET:?ARTIFACT_BUCKET required}"
: "${DOMAIN:?DOMAIN required}"
NOTES="${NOTES:-}"
NOTES_FILE="${NOTES_FILE:-}"
FROM_CHANNEL="${FROM_CHANNEL:-beta}"
TO_CHANNEL="${TO_CHANNEL:-stable}"

# The four shipped platforms (spec §5.2). Manifest keys == tarball platform suffixes.
# SINGLE SOURCE OF TRUTH: release/platforms; `cargo xtask platforms check` (CI) asserts this
# array matches it (nexus-flow-4sr). Keep the two in lockstep.
PLATFORMS=(darwin-aarch64 darwin-x86_64 linux-x86_64 linux-aarch64)
from_prefix="download/${FROM_CHANNEL}/${VERSION}"
to_prefix="download/${TO_CHANNEL}/${VERSION}"

# head_object_status <key>: prints one of ok|missing|denied|error:<msg>. `head-object`, never
# `s3 ls`: a 403 must be observable as a permission failure, never masquerade as "not found"
# (the 404≠403 distinction the spec calls out). Returns 0 only for `ok`.
head_object_status() {
  local key="$1" err code
  if err="$(aws s3api head-object --bucket "$ARTIFACT_BUCKET" --key "$key" 2>&1 >/dev/null)"; then
    echo ok
    return 0
  fi
  # AWS CLI: "An error occurred (404) when calling the HeadObject operation: Not Found".
  # Prefer the parenthesized HTTP status — it is the bare numeric code, stable across CLI
  # versions and not localized; only fall back to the English phrase if that is absent.
  code="$(printf '%s' "$err" | sed -n 's/.*(\([0-9][0-9][0-9]\)).*/\1/p' | head -n1)"
  case "${code:-}" in
    404) echo missing; return 1 ;;
    403) echo denied; return 1 ;;
  esac
  case "$err" in
    *'Not Found'*) echo missing ;;
    *'Forbidden'*) echo denied ;;
    *) echo "error: ${err}" ;;
  esac
  return 1
}

echo "Promoting nxf ${VERSION}: ${from_prefix}/ → ${to_prefix}/ (bucket masked, domain=${DOMAIN})"

# Build the full object list once (4 tarballs × {tarball, .sha256, .minisig}); reuse for guard+copy.
files=()
for p in "${PLATFORMS[@]}"; do
  base="nxf_${VERSION}_${p}.tar.gz"
  files+=("$base" "$base.sha256" "$base.minisig")
done

# 1. GUARDS — fail closed, 404≠403, BEFORE any copy.
for f in "${files[@]}"; do
  status="$(head_object_status "${from_prefix}/${f}")" || true
  case "$status" in
    ok) ;;
    missing)
      echo "::error::beta artifact missing (404) — promotion fails closed: ${from_prefix}/${f}"
      exit 1
      ;;
    denied)
      echo "::error::cannot read beta artifact (403 — IAM regression, not a missing object): ${from_prefix}/${f}"
      exit 1
      ;;
    *)
      echo "::error::unexpected head-object failure for ${from_prefix}/${f}: ${status}"
      exit 1
      ;;
  esac
done
echo "Guard ok: all ${#files[@]} beta objects (4 tarballs + .sha256 + .minisig) present."

# 2. Server-side copy beta→stable. `--metadata-directive COPY` (explicit) keeps the content-type
#    and the bytes/signatures byte-for-byte identical — no rebuild, no re-sign. If a copy fails
#    mid-loop, `set -e` aborts BEFORE the manifest is written (step 4) — and the manifest is the
#    only activation switch, so a partial copy never goes live. Re-running is safe idempotent
#    recovery: the keys are immutable and every copy is byte-identical.
for f in "${files[@]}"; do
  aws s3api copy-object \
    --bucket "$ARTIFACT_BUCKET" \
    --key "${to_prefix}/${f}" \
    --copy-source "${ARTIFACT_BUCKET}/${from_prefix}/${f}" \
    --metadata-directive COPY \
    >/dev/null
done
echo "Copied ${#files[@]} objects to ${to_prefix}/."

# 2b. Verify every stable object landed BEFORE assembling the manifest — the manifest must never
#     reference (or activate a channel over) an incompletely-copied ring.
for f in "${files[@]}"; do
  [ "$(head_object_status "${to_prefix}/${f}")" = ok ] \
    || { echo "::error::stable object missing after copy (incomplete promotion): ${to_prefix}/${f}"; exit 1; }
done
echo "Verified all ${#files[@]} stable objects present."

# 3. Assemble manifests/<to_channel>.json from the now-present STABLE sidecars (reading them
#    back also confirms the copy landed). Same shape as publish-release.sh: `path` is the S3
#    key == public URL path 1:1; `signature` is the full .minisig text; `sha256` the hex digest.
platforms='{}'
for p in "${PLATFORMS[@]}"; do
  base="nxf_${VERSION}_${p}.tar.gz"
  # Read the sidecar to a file and VALIDATE the hash before it enters the manifest: a wrong or
  # empty sha256 (a warning line, a truncated body) would DoS every updater/installer that trusts
  # the manifest. Accept only a bare 64-char lowercase hex digest.
  shafile="$(mktemp)"
  aws s3 cp "s3://${ARTIFACT_BUCKET}/${to_prefix}/${base}.sha256" "$shafile" >/dev/null
  sha="$(awk 'NR==1 {print $1}' "$shafile")"
  rm -f "$shafile"
  printf '%s' "$sha" | grep -Eq '^[0-9a-f]{64}$' \
    || { echo "::error::invalid sha256 read back for ${base}: '${sha}'"; exit 1; }
  sigfile="$(mktemp)"
  aws s3 cp "s3://${ARTIFACT_BUCKET}/${to_prefix}/${base}.minisig" "$sigfile" >/dev/null
  [ -s "$sigfile" ] || { echo "::error::empty minisig read back for ${base}"; exit 1; }
  platforms="$(jq \
    --arg key "$p" \
    --arg path "${to_prefix}/${base}" \
    --arg sha "$sha" \
    --rawfile sig "$sigfile" \
    '.[$key] = {path: $path, sha256: $sha, signature: $sig}' <<<"$platforms")"
  rm -f "$sigfile"
done

# The notes come from a FILE where the caller can give us one, and that is not a style choice:
# Linux caps a single environment variable at MAX_ARG_STRLEN (32 pages = 128 KiB), and the
# aggregate for a stable jump is "everything since the last stable" — it grows with the size of
# the jump, not with the release. The v0.63.0 promotion measured 201 KiB (EN), so
# `NOTES="$(cat …)" ./promote-release.sh` died with E2BIG ("Argument list too long") before the
# script ever started. That is a promotion that gets HARDER the longer stable lags behind, which
# is exactly backwards. `jq --rawfile` reads the file directly and has no such ceiling.
#
# `NOTES` stays for the hand-run against staging, where the notes are a line or two.
notes_arg=(--arg notes "$NOTES")
if [ -n "$NOTES_FILE" ]; then
  [ -r "$NOTES_FILE" ] || { echo "NOTES_FILE is set but not readable: $NOTES_FILE" >&2; exit 1; }
  notes_arg=(--rawfile notes "$NOTES_FILE")
fi

manifest="$(mktemp)"
jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${notes_arg[@]}" \
  --argjson platforms "$platforms" \
  '{version: $version, pub_date: $pub_date, notes: $notes, enabled: true, rollout: 100, platforms: $platforms}' \
  >"$manifest"
aws s3 cp "$manifest" "s3://${ARTIFACT_BUCKET}/manifests/${TO_CHANNEL}.json" --content-type application/json
rm -f "$manifest"
echo "Wrote manifests/${TO_CHANNEL}.json"

# 4. Feed → S3 (best-effort): the caller regenerated release-notes.json via `xtask changelog
#    promote` before invoking us. Short TTL so the new stable notes surface fast.
if [ -f release-notes.json ]; then
  aws s3 cp release-notes.json "s3://${ARTIFACT_BUCKET}/release-notes.json" \
    --content-type application/json --cache-control "max-age=300"
  echo "Published release-notes.json"
fi

echo "Promotion complete: ${VERSION} → ${TO_CHANNEL}"
