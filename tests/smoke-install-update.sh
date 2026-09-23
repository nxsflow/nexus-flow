#!/usr/bin/env bash
# Real install + self-update smoke (nexus-flow-h1h).
#
# Unlike tests/install.sh.test.sh (hermetic: local static server, throwaway key — it can never
# reach the real origin), this drives the PUBLISHED channel end to end on a FRESH host:
#
#   1. `curl -fsSL <base>/install.sh | sh` from the live origin,
#   2. the mandatory signature verification actually fires (fail-closed), and
#   3. the binary runs — `nxf --version` equals the channel head — and
#   4. `nxs self-update` (the canonical, suite-wide verb) either lifts a deliberately-older install to the published stand
#      (no downgrade, signature-verified) or, on a head install, reports up-to-date.
#
# It is the automation of the previously-manual release verification step (lhg). Henne-Ei: it runs
# against whatever origin it is pointed at, so a staging rehearsal release needs no public release
# to exist. Default target is the new staging /nxs chain (staging previews the beta ring, spec §4).
#
# Configuration (all via env):
#   NXF_SMOKE_BASE_URL        origin to install from (default https://staging.nxsflow.com/nxs — the
#                             staging /nxs chain; set https://nxsflow.com/nxs for a prod check).
#   NXF_SMOKE_CHANNEL         promotion ring: stable | beta | alpha (default beta — the staging ring).
#   NXF_SMOKE_UPDATE_FROM     an older version to install first, then self-update FROM. Empty
#                             (default) installs the channel head and asserts `self-update
#                             --check` is up-to-date; set it in a staging rehearsal where a newer
#                             build exists to exercise a REAL older->newer update.
#   NXF_SMOKE_EXPECT_VERIFIER minisign | openssl | any (default any) — assert WHICH authenticity
#                             path install.sh used. `openssl` pins the 85y.28 path (capable
#                             openssl, no minisign binary); `minisign` pins the binary path.
#
# Requires install.sh's own deps (curl, tar, sha256sum|shasum). Fails closed: any verification
# failure, version mismatch, or unexpected verifier path exits non-zero.
set -euo pipefail

BASE_URL="${NXF_SMOKE_BASE_URL:-https://staging.nxsflow.com/nxs}"
BASE_URL="${BASE_URL%/}" # tolerate a trailing slash
CHANNEL="${NXF_SMOKE_CHANNEL:-beta}"
UPDATE_FROM="${NXF_SMOKE_UPDATE_FROM:-}"
EXPECT_VERIFIER="${NXF_SMOKE_EXPECT_VERIFIER:-any}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
# Canonicalize the arch the same way install.sh's platform_for does, so our /latest probe and
# the actual install agree on a single spelling — rather than relying on the updater Lambda's
# normalizeArch forgiving raw `uname -m` (e.g. arm64 vs aarch64, amd64 vs x86_64).
case "$(uname -m)" in
  arm64 | aarch64) ARCH="aarch64" ;;
  x86_64 | amd64) ARCH="x86_64" ;;
  *) ARCH="$(uname -m)" ;;
esac

say() { printf '\n=== %s ===\n' "$*"; }
fail() {
  printf 'SMOKE FAIL: %s\n' "$*" >&2
  exit 1
}

# A throwaway install root + HOME so the smoke never clobbers a real install or config.
SMOKE_HOME="$(mktemp -d "${TMPDIR:-/tmp}/nxf-smoke.XXXXXX")"
trap 'rm -rf "$SMOKE_HOME"' EXIT INT TERM
INSTALL_DIR="$SMOKE_HOME/bin"
NXF_BIN="$INSTALL_DIR/nxf"
# The real suite binary; `nxf`/`nxm` are symlinks to it (multicall). Self-update is canonically
# driven through `nxs` (nexus-flow-gel), so the smoke exercises that path end to end.
NXS_BIN="$INSTALL_DIR/nxs"

# Resolve the channel head version from /latest: the 302 target's tarball path carries the
# version (.../download/<ch>/<version>/nxf_<version>_<platform>.tar.gz), so we assert `--version`
# against the real published stand without hardcoding it.
resolve_head_version() {
  local redirect
  redirect="$(curl -fsS -o /dev/null -w '%{redirect_url}' \
    "${BASE_URL}/latest?channel=${CHANNEL}&target=${OS}&arch=${ARCH}")" \
    || fail "could not reach ${BASE_URL}/latest"
  [ -n "$redirect" ] \
    || fail "${BASE_URL}/latest returned no redirect (no release on channel '${CHANNEL}' for ${OS}-${ARCH}?)"
  printf '%s\n' "$redirect" | sed -E 's#.*/download/[^/]+/([^/]+)/.*#\1#'
}

installed_version() {
  "$NXF_BIN" --version | awk '{print $NF}'
}

# Install from the live origin, capturing the log so we can assert the verifier path. `pin` is an
# exact version, or empty for the channel head. Runs in a subshell with a sandboxed HOME *and*
# XDG dirs so self-update's persisted config/cache never touches the caller's real home — nxf
# resolves XDG_CONFIG_HOME/XDG_CACHE_HOME ahead of HOME, so sandboxing HOME alone would leak.
run_install() {
  local pin="$1" log="$SMOKE_HOME/install.log"
  say "install from ${BASE_URL} (channel ${CHANNEL}${pin:+, version ${pin}})"
  if ! (
    export HOME="$SMOKE_HOME"
    export XDG_CONFIG_HOME="$SMOKE_HOME/.config" XDG_CACHE_HOME="$SMOKE_HOME/.cache"
    export NXF_INSTALL_DIR="$INSTALL_DIR" NXF_BASE_URL="$BASE_URL" NXF_CHANNEL="$CHANNEL"
    if [ -n "$pin" ]; then export NXF_VERSION="$pin"; fi
    curl -fsSL "${BASE_URL}/install.sh" | sh
  ) >"$log" 2>&1; then
    cat "$log" >&2
    fail "install.sh failed (fail-closed) — ${BASE_URL}, ${pin:-head}"
  fi
  cat "$log"
  # The SIGNATURE (authenticity) gate MUST have fired. install.sh logs exactly this line on a
  # successful verify (minisign path: bare; openssl path: + "(via openssl)"). Crucially, the
  # mandatory "sha256 verified:" INTEGRITY line and the NXF_INSECURE / 404-skip downgrade paths
  # do NOT print it — so requiring this specific string (not a generic "verified") keeps even
  # `any` mode genuinely fail-closed against a silently-skipped signature check.
  grep -qi 'minisign signature verified' "$log" \
    || fail "signature was not verified (authenticity gate did not fire) — see log above"
  case "$EXPECT_VERIFIER" in
    minisign)
      ! grep -qi 'via openssl' "$log" \
        || fail "expected the minisign binary path, but install verified via openssl"
      ;;
    openssl)
      grep -qi 'via openssl' "$log" \
        || fail "expected the openssl (85y.28) verifier path; log did not show it"
      ;;
    any) : ;;
    *) fail "unknown NXF_SMOKE_EXPECT_VERIFIER='${EXPECT_VERIFIER}' (want minisign|openssl|any)" ;;
  esac
}

# The canonical `nxs self-update` in a sandboxed HOME + XDG dirs (it persists channel/machine_id
# under the config dir, and the suite resolves XDG ahead of HOME — sandbox both so a manual run
# can't leak into ~). Swaps the whole suite (the single `nxs` binary + the `nxf`/`nxm` links).
# NXF_BASE_URL is pinned to the SAME origin we installed from: without it self-update would fall
# back to the binary's compiled-in DEFAULT_BASE_URL and silently probe a DIFFERENT chain than the
# one under test (invisible only while base == the default; the /nxs retarget exposed it).
self_update() {
  HOME="$SMOKE_HOME" \
    XDG_CONFIG_HOME="$SMOKE_HOME/.config" XDG_CACHE_HOME="$SMOKE_HOME/.cache" \
    NXF_BASE_URL="$BASE_URL" \
    "$NXS_BIN" self-update "$@"
}

HEAD="$(resolve_head_version)"
say "channel head: ${HEAD} (${OS}-${ARCH}, ${BASE_URL}, channel ${CHANNEL})"

if [ -n "$UPDATE_FROM" ]; then
  # Staging rehearsal: install an older build, then update it for real to the channel head.
  run_install "$UPDATE_FROM"
  got="$(installed_version)"
  [ "$got" = "$UPDATE_FROM" ] || fail "pinned install reports ${got}, expected ${UPDATE_FROM}"
  say "self-update: real upgrade ${UPDATE_FROM} -> ${HEAD}"
  out="$(self_update --channel "$CHANNEL" --json 2>&1)" \
    || { printf '%s\n' "$out" >&2; fail "self-update failed (fail-closed)"; }
  printf '%s\n' "$out"
  new="$(installed_version)"
  [ "$new" != "$UPDATE_FROM" ] || fail "self-update did not move off ${UPDATE_FROM} (no upgrade)"
  [ "$new" = "$HEAD" ] || fail "self-update landed on ${new}, expected channel head ${HEAD}"
  say "self-update upgraded ${UPDATE_FROM} -> ${new}"
else
  # Prod/verification mode: install the head, prove it runs, confirm self-update sees no downgrade.
  run_install ""
  got="$(installed_version)"
  [ "$got" = "$HEAD" ] || fail "installed --version ${got} != channel head ${HEAD}"
  say "binary runs: nxf ${got}"
  say "self-update --check: expect up-to-date on the freshly-installed head"
  out="$(self_update --check --channel "$CHANNEL" --json 2>&1)" \
    || { printf '%s\n' "$out" >&2; fail "self-update --check failed"; }
  printf '%s\n' "$out"
  printf '%s' "$out" | grep -q '"status":"up-to-date"' \
    || fail "expected status up-to-date on a head install, got: ${out}"
fi

say "SMOKE PASSED (${OS}-${ARCH}, ${BASE_URL}, channel ${CHANNEL})"
