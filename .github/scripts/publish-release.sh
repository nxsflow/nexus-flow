#!/usr/bin/env bash
#
# publish-release.sh — upload the signed nexus-flow artifacts + channel manifest to S3.
# (nexus-flow-85y.13, spec §5.4). AWS creds are already assumed by the caller (the OIDC
# release role); this script is pure S3 publish + manifest assembly, no policy.
#
# Inputs (env):
#   VERSION          plain SemVer X.Y.Z
#   CHANNEL          promotion ring (beta on a fresh release; stable is promotion-only)
#   ARTIFACT_BUCKET  target S3 bucket (name embeds the account id ⇒ masked in CI logs)
#   DOMAIN           public origin, for log context only
#   ARTIFACT_DIR     dir holding nxf_<v>_<platform>.tar.gz{,.sha256,.minisig}  (default: dist)
#   NOTES            human-readable manifest notes                            (default: Beta-ring build.)
#
# Writes (spec §5.4):
#   s3://<bucket>/download/<channel>/<version>/nxf_<v>_<platform>.tar.gz  (+ .sha256, + .minisig)
#   s3://<bucket>/manifests/<channel>.json
#   s3://<bucket>/release-notes.json   (max-age 300, best-effort — only if the feed exists)
#   s3://<bucket>/install.sh           (max-age 300, best-effort — owned by 85y.16)
#
# The .minisig sidecars MUST land next to the tarballs: the beta→stable promotion (85y.15)
# reads them from there to sign the stable manifest. The manifest `signature` field carries
# the FULL .minisig text so `nxs self-update` can verify without a second fetch.
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${CHANNEL:?CHANNEL required}"
: "${ARTIFACT_BUCKET:?ARTIFACT_BUCKET required}"
: "${DOMAIN:?DOMAIN required}"
ARTIFACT_DIR="${ARTIFACT_DIR:-dist}"
NOTES="${NOTES:-Beta-ring build.}"

# The four shipped platforms (spec §5.2). Manifest keys == tarball platform suffixes.
# SINGLE SOURCE OF TRUTH: release/platforms; `cargo xtask platforms check` (CI) asserts this
# array matches it (nexus-flow-4sr). Keep the two in lockstep.
PLATFORMS=(darwin-aarch64 darwin-x86_64 linux-x86_64 linux-aarch64)
prefix="download/${CHANNEL}/${VERSION}"

echo "Publishing nxf ${VERSION} → s3://${ARTIFACT_BUCKET}/${prefix}/ (channel=${CHANNEL}, domain=${DOMAIN})"

# 1. Upload every platform's tarball + its two sidecars.
for p in "${PLATFORMS[@]}"; do
  base="nxf_${VERSION}_${p}.tar.gz"
  tar="${ARTIFACT_DIR}/${base}"
  for f in "$tar" "$tar.sha256" "$tar.minisig"; do
    [ -f "$f" ] || { echo "::error::missing artifact $f"; exit 1; }
  done
  aws s3 cp "$tar"         "s3://${ARTIFACT_BUCKET}/${prefix}/${base}"         --content-type application/gzip
  aws s3 cp "$tar.sha256"  "s3://${ARTIFACT_BUCKET}/${prefix}/${base}.sha256"  --content-type text/plain
  aws s3 cp "$tar.minisig" "s3://${ARTIFACT_BUCKET}/${prefix}/${base}.minisig" --content-type text/plain
done

# 2. Assemble the channel manifest (spec §5.4 shape, extended with sha256). `path` is the S3
#    key == public URL path 1:1; `signature` is the full .minisig text; `sha256` the hex digest.
platforms='{}'
for p in "${PLATFORMS[@]}"; do
  base="nxf_${VERSION}_${p}.tar.gz"
  sha="$(awk '{print $1}' "${ARTIFACT_DIR}/${base}.sha256")"
  platforms="$(jq \
    --arg key "$p" \
    --arg path "${prefix}/${base}" \
    --arg sha "$sha" \
    --rawfile sig "${ARTIFACT_DIR}/${base}.minisig" \
    '.[$key] = {path: $path, sha256: $sha, signature: $sig}' <<<"$platforms")"
done

manifest="$(mktemp)"
jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg notes "$NOTES" \
  --argjson platforms "$platforms" \
  '{version: $version, pub_date: $pub_date, notes: $notes, enabled: true, rollout: 100, platforms: $platforms}' \
  > "$manifest"
aws s3 cp "$manifest" "s3://${ARTIFACT_BUCKET}/manifests/${CHANNEL}.json" --content-type application/json
rm -f "$manifest"

# 3. Feed + install script: short TTL so a promotion/update is visible fast. Best-effort —
#    the feed only exists after a `version set`, and install.sh is owned by 85y.16.
if [ -f release-notes.json ]; then
  aws s3 cp release-notes.json "s3://${ARTIFACT_BUCKET}/release-notes.json" \
    --content-type application/json --cache-control "max-age=300"
fi
for cand in install.sh .github/scripts/install.sh; do
  if [ -f "$cand" ]; then
    aws s3 cp "$cand" "s3://${ARTIFACT_BUCKET}/install.sh" \
      --content-type text/x-shellscript --cache-control "max-age=300"
    break
  fi
done

echo "Published ${#PLATFORMS[@]} platforms + manifests/${CHANNEL}.json"
