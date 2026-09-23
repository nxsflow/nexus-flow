#!/usr/bin/env bash
# Hermetic test harness for install.sh (nexus-flow-85y.16).
#
# Two layers, no network and no AWS:
#   A. platform_for — the pure uname→platform mapping, sourced in library mode.
#   B. a full pinned install against a LOCAL static file server (python3 http.server), with a
#      throwaway minisign key, covering: happy path, idempotency, optional nxf-relay, and the
#      two fail-closed gates (sha256 mismatch, bad minisign signature).
#
# The `/latest` 302 path is deliberately NOT mocked here — it is verified for real against the
# live staging domain (see the PR / ticket notes). This harness stays hermetic so it can gate CI.
#
# Run: tests/install.sh.test.sh   (needs bash, python3, tar, sha256sum|shasum; minisign optional)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_SH="$ROOT/install.sh"

PASS=0
FAIL=0
ok()   { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }
check(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected [$3], got [$2])"; fi; }
assert_file(){   if [ -x "$1" ]; then ok "$2"; else bad "$2"; fi; }
refute_file(){   if [ ! -e "$1" ]; then ok "$2"; else bad "$2"; fi; }
# A persona is a symlink whose target is exactly `nxs` (multicall, nexus-flow-5jz.7).
assert_persona_link(){ if [ -L "$1" ] && [ "$(readlink "$1")" = "nxs" ] && [ -x "$1" ]; then ok "$2"; else bad "$2"; fi; }

# --- Layer A: pure platform_for ---------------------------------------------------------------
#
# `unset` on the NEXT line is load-bearing, and its absence made every install case below vacuous on
# macOS. `.` is a POSIX *special* built-in, so a variable assignment written in front of it PERSISTS
# after it returns — and bash-as-`/bin/sh` (which is what `/bin/sh` is on macOS) additionally marks
# it EXPORTED. `NXF_INSTALL_SH_LIB=1` therefore stayed in the environment of every later
# `sh "$INSTALL_SH"`, where it means "define the functions, run nothing": each install case then saw
# a script that printed nothing, exited 0, and installed nothing. Fourteen assertions failed for one
# reason that named itself nowhere. dash (Ubuntu's `/bin/sh`, i.e. CI) does not export it, which is
# why this was invisible to the gate and visible only to a developer running the suite locally.
# shellcheck source=../install.sh disable=SC1091
NXF_INSTALL_SH_LIB=1 . "$INSTALL_SH"
unset NXF_INSTALL_SH_LIB

platform_case() { # <uname-s> <uname-m> <expected|"">
  local got
  if got="$(platform_for "$1" "$2" 2>/dev/null)"; then :; else got=""; fi
  check "platform_for $1/$2" "$got" "$3"
}

echo "A. platform_for (pure mapping)"
platform_case Darwin arm64   darwin-aarch64
platform_case Darwin aarch64 darwin-aarch64
platform_case Darwin x86_64  darwin-x86_64
platform_case Linux  x86_64  linux-x86_64
platform_case Linux  amd64   linux-x86_64
platform_case Linux  aarch64 linux-aarch64
platform_case Linux  armv7l  ""
platform_case FreeBSD x86_64 ""
platform_case Windows x86_64 ""

# --- Layer B: pinned install against a local static server ------------------------------------
echo "B. pinned install (local static server)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-install-test.XXXXXX")"
SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true # swallow the job-control "Terminated" notice
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

VERSION="9.9.9"
CHANNEL="beta"
PLATFORM="linux-x86_64"
SRVROOT="$WORK/srv"
DLDIR="$SRVROOT/download/$CHANNEL/$VERSION"
mkdir -p "$DLDIR"

# A tarball with bare-name entries for the TWO real binaries — `nxs` + `nxf-relay` (matches
# release.yml packaging under multicall, nexus-flow-5jz.7). `nxf`/`nxm` are NOT shipped: install.sh
# creates them as symlinks to `nxs`. The fake `nxs` echoes its argv[0] so a persona-link invocation
# is observable (`nxf` → runs `nxs`, prints `nxf`).
STAGE="$WORK/stage"
mkdir -p "$STAGE"
# The `$(basename "$0")` is written LITERALLY into the fake nxs so it evaluates at runtime (a
# persona link invocation reports its own name) — the single quotes are deliberate.
# shellcheck disable=SC2016
printf '#!/bin/sh\necho "nxs-fake argv0=$(basename "$0")"\n' > "$STAGE/nxs"
printf '#!/bin/sh\necho nxf-relay-fake\n'                    > "$STAGE/nxf-relay"
chmod +x "$STAGE/nxs" "$STAGE/nxf-relay"
# The bundled `nxc` agent sidecar (nexus-flow-6j6v.81v5) — a THIRD tarball member, and the first
# one that is data rather than a program: `nxs` hands it to `node`, and finds it by looking beside
# its own binary, which is why installing it into the same dir is the contract and not a detail.
printf '// nxc-agent-sidecar-fake\n' > "$STAGE/nxc-agent-sidecar.mjs"
TARBALL="nxf_${VERSION}_${PLATFORM}.tar.gz"
tar -czf "$DLDIR/$TARBALL" -C "$STAGE" nxs nxf-relay nxc-agent-sidecar.mjs

sha_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}
printf '%s  %s\n' "$(sha_of "$DLDIR/$TARBALL")" "$TARBALL" > "$DLDIR/$TARBALL.sha256"

# minisign coverage. The signature-path cases (B5/B6) are security-critical, so under CI they
# MUST run: a CI runner without minisign would let them skip green and mask a regression.
HAVE_MINISIGN=0
TEST_PUBKEY=""
if command -v minisign >/dev/null 2>&1; then
  HAVE_MINISIGN=1
  minisign -G -W -p "$WORK/test.pub" -s "$WORK/test.key" >/dev/null 2>&1
  TEST_PUBKEY="$(sed -n '2p' "$WORK/test.pub")"
  minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1
elif [ "${CI:-}" = "true" ]; then
  bad "minisign must be installed in CI — the signature-path cases (B5/B6) would skip"
fi

# Serve $SRVROOT on a fixed-but-free port.
PORT=8731
( cd "$SRVROOT" && exec python3 -m http.server "$PORT" --bind 127.0.0.1 ) >/dev/null 2>&1 &
SERVER_PID=$!
# Wait for readiness.
for _ in $(seq 1 50); do
  if curl -fsS -o /dev/null "http://127.0.0.1:$PORT/download/$CHANNEL/$VERSION/$TARBALL" 2>/dev/null; then break; fi
  sleep 0.1
done

BIN="$WORK/bin"
# On a host WITHOUT minisign the embedded pubkey can't be verified, so the happy-path cases use
# the documented bootstrap opt-out (NXF_INSECURE=1) — exactly what a real user on a stock host
# would set. With minisign present (CI), it stays unset so the full signature path is exercised.
INSECURE=""
[ "$HAVE_MINISIGN" = "1" ] || INSECURE="1"
run_install() { # extra env...  → runs install.sh pinned at our local server
  env NXF_BASE_URL="http://127.0.0.1:$PORT" \
      NXF_CHANNEL="$CHANNEL" \
      NXF_VERSION="$VERSION" \
      NXF_PLATFORM="$PLATFORM" \
      NXF_INSTALL_DIR="$BIN" \
      NXF_MINISIGN_PUBKEY="${TEST_PUBKEY}" \
      NXF_INSECURE="$INSECURE" \
      "$@" \
      sh "$INSTALL_SH"
}

# B1 happy path (multicall): the ONE real `nxs` binary installed + executable; `nxf`/`nxm` are
# symlinks to it; nxf-relay NOT by default. The persona link runs nxs and reports its own argv[0].
rm -rf "$BIN"
if run_install >/dev/null 2>&1; then
  assert_file "$BIN/nxs" "B1 nxs installed + executable (the one real binary)"
  assert_persona_link "$BIN/nxf" "B1 nxf is a symlink -> nxs"
  assert_persona_link "$BIN/nxm" "B1 nxm is a symlink -> nxs"
  check "B1 nxf persona routes by argv0" "$("$BIN/nxf")" "nxs-fake argv0=nxf"
  refute_file "$BIN/nxf-relay" "B1 nxf-relay absent without NXF_INSTALL_RELAY"
  # The sidecar (6j6v.81v5): present beside the binary — where `nxs` looks for it, so that
  # `nxc send --to <persona>` works with no NXC_SIDECAR in the environment — and NOT executable,
  # because it is a Node module and `~/.local/bin` is on the PATH.
  if [ -f "$BIN/nxc-agent-sidecar.mjs" ] && [ ! -x "$BIN/nxc-agent-sidecar.mjs" ]; then
    ok "B1 nxc-agent-sidecar.mjs installed beside nxs as data (644, not executable)"
  else
    bad "B1 nxc-agent-sidecar.mjs missing or executable"
  fi
else
  bad "B1 install failed unexpectedly"
fi

# B2 idempotency: a second run over an existing install succeeds and keeps the nxf persona link.
if run_install >/dev/null 2>&1 && [ -x "$BIN/nxf" ] && [ -L "$BIN/nxf" ]; then ok "B2 idempotent re-run"; else bad "B2 re-run failed"; fi

# B3 relay opt-in.
rm -rf "$BIN"
if run_install NXF_INSTALL_RELAY=1 >/dev/null 2>&1 && [ -x "$BIN/nxf-relay" ]; then
  ok "B3 NXF_INSTALL_RELAY=1 installs nxf-relay"
else
  bad "B3 nxf-relay not installed under opt-in"
fi

# B3b relay-less tarball (multicall): a tarball with `nxs` but no `nxf-relay` installs nxs + the
# persona links fine — the relay is opt-in and its absence is never fatal. Swap in an nxs-only
# tarball, then restore the full one.
rm -rf "$BIN"
cp "$DLDIR/$TARBALL" "$WORK/full.tar.gz"
cp "$DLDIR/$TARBALL.sha256" "$WORK/full.sha256"
[ "$HAVE_MINISIGN" = "1" ] && cp "$DLDIR/$TARBALL.minisig" "$WORK/full.minisig"
tar -czf "$DLDIR/$TARBALL" -C "$STAGE" nxs
printf '%s  %s\n' "$(sha_of "$DLDIR/$TARBALL")" "$TARBALL" > "$DLDIR/$TARBALL.sha256"
[ "$HAVE_MINISIGN" = "1" ] && minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1
# A leftover from a 0.52.0/0.53.0 install, so this case also covers the clearing half below.
mkdir -p "$BIN"
printf '// the 0.53.0 bundle\n' > "$BIN/nxc-agent-sidecar.mjs"
if run_install >/dev/null 2>&1; then
  assert_file "$BIN/nxs" "B3b nxs-only release installs nxs"
  assert_persona_link "$BIN/nxf" "B3b nxf persona still linked without a relay in the tarball"
  refute_file "$BIN/nxf-relay" "B3b no relay installed when the tarball lacks it (not requested)"
  # Same tolerance for the sidecar (6j6v.81v5): a release cut before it shipped has no such member,
  # and `NXF_VERSION`-pinning to one of those is legal. `tar -x <missing-member>` is an error, so
  # the install would abort on the LAST step if it were not extracted tolerantly.
  #
  # AND the clearing half (6j6v.smsz/6j6v.5vct): since the sidecar is compiled into `nxs`, no
  # current release carries the member either — so this same assertion now also pins that a copy
  # left beside the binary by <=0.53.0 is REMOVED rather than left to age there. It was seeded
  # above, so a `refute_file` that passed by never creating one would not pass here.
  refute_file "$BIN/nxc-agent-sidecar.mjs" "B3b no sidecar installed when the tarball lacks it, and a stale one is cleared"
else
  bad "B3b nxs-only install failed (a relay-less tarball must still install the CLI)"
fi
cp "$WORK/full.tar.gz" "$DLDIR/$TARBALL"
cp "$WORK/full.sha256" "$DLDIR/$TARBALL.sha256"
[ "$HAVE_MINISIGN" = "1" ] && cp "$WORK/full.minisig" "$DLDIR/$TARBALL.minisig"

# B4 fail-closed on sha256 mismatch: corrupt the tarball but keep the old sidecar → must abort,
# and must NOT install nxs.
rm -rf "$BIN"
cp "$DLDIR/$TARBALL" "$WORK/good.tar.gz"
cp "$DLDIR/$TARBALL.sha256" "$WORK/good.sha256"
printf 'corrupted-bytes' >> "$DLDIR/$TARBALL" # body no longer matches the sidecar hash
if run_install >/dev/null 2>&1; then
  bad "B4 install should have failed on sha256 mismatch"
else
  refute_file "$BIN/nxs" "B4 sha256 mismatch aborts, nxs not installed"
fi
cp "$WORK/good.tar.gz" "$DLDIR/$TARBALL" # restore for the next case

# B5 fail-closed on bad minisign signature: valid sha256, but the .minisig is for other bytes.
if [ "$HAVE_MINISIGN" = "1" ]; then
  rm -rf "$BIN"
  printf 'other content' > "$WORK/other"
  minisign -S -H -s "$WORK/test.key" -m "$WORK/other" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1
  if run_install >/dev/null 2>&1; then
    bad "B5 install should have failed on bad minisign signature"
  else
    refute_file "$BIN/nxs" "B5 bad minisign aborts, nxs not installed"
  fi
  # restore the correct signature
  minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1

  # B6 minisign installed but NO .minisig published (genuine 404): warn + install on sha256.
  # This is the deliberate degradation path — distinct from a transport error, which must abort.
  rm -rf "$BIN"
  mv "$DLDIR/$TARBALL.minisig" "$WORK/saved.minisig"
  if run_install >/dev/null 2>&1; then
    assert_file "$BIN/nxs" "B6 missing .minisig (404) → warn + install on sha256"
  else
    bad "B6 a genuine 404 on .minisig should NOT abort (sha256 verified)"
  fi
  mv "$WORK/saved.minisig" "$DLDIR/$TARBALL.minisig" # restore
else
  echo "  skip B5/B6 (minisign not installed)"
fi

# B7 fail-closed on a cross-origin /latest redirect: a 302 pointing off our origin must be
# refused before the tarball is fetched. We can't easily mint a 302 with the static server, so
# assert the guard directly via the sourced helper (same_origin is pure).
if same_origin "https://evil.example/download/beta/9.9.9/x.tar.gz" "http://127.0.0.1:$PORT"; then
  bad "B7 same_origin accepted a cross-origin redirect"
else
  ok "B7 same_origin rejects a cross-origin / scheme-mismatched redirect"
fi
if same_origin "http://127.0.0.1:$PORT/download/beta/9.9.9/x.tar.gz" "http://127.0.0.1:$PORT"; then
  ok "B7 same_origin accepts the same origin"
else
  bad "B7 same_origin rejected a legitimate same-origin redirect"
fi

# B7b origin-pinning is scheme+host+port ONLY (NOT string-prefix matching) and must hold when the
# base carries a URL PATH — the shape the brand-domain cutover shipped (the default is now
# https://nxsflow.com/nxs, spec §6 step 8). The bare-origin B7 cases can't tell a correct
# origin-stripping guard from a naive `url starts_with base`, because their accepted URL literally
# prefixes its base; these do.
NXS_BASE="http://127.0.0.1:$PORT/nxs"
if same_origin "http://127.0.0.1:$PORT/nxs/download/beta/9.9.9/x.tar.gz" "$NXS_BASE"; then
  ok "B7b same_origin accepts a same-origin redirect under a path-bearing base"
else
  bad "B7b same_origin rejected a legitimate same-origin redirect (path-bearing base)"
fi
# THE discriminator: same scheme+host+port but a path that does NOT start with the base's path. A
# correct origin-stripping guard accepts it; a naive prefix match (`url starts_with base`) rejects it.
if same_origin "http://127.0.0.1:$PORT/elsewhere/x.tar.gz" "$NXS_BASE"; then
  ok "B7b same_origin accepts a same origin with a different top-level path (origin-stripping, not prefix-matching)"
else
  bad "B7b same_origin wrongly rejected a same-origin URL whose path differs from the base (naive prefix match?)"
fi
if same_origin "https://evil.example/nxs/download/beta/9.9.9/x.tar.gz" "$NXS_BASE"; then
  bad "B7b same_origin accepted a cross-origin redirect against a path-bearing base"
else
  ok "B7b same_origin rejects a cross-origin redirect against a path-bearing base"
fi

# B8 full install against a PATH-BEARING base URL: the real prod/staging default is now a base with
# a URL path (https://nxsflow.com/nxs), not the bare origin the B1-B6 cases use. Serve the SAME
# artifacts under a /nxs/ prefix and drive the whole curl install pipeline against it, proving
# install.sh joins ${base}/download/… and verifies (sha256[+minisign]) when the base carries a path
# — the primary install path, exercised here in the PR-gating hermetic suite (not just the weekly
# live smoke). NXF_VERSION is pinned, so this hits /download directly; the same_origin guard on the
# /latest redirect is covered by B7b above.
mkdir -p "$SRVROOT/nxs"
ln -s ../download "$SRVROOT/nxs/download"
rm -rf "$BIN"
if env NXF_BASE_URL="$NXS_BASE" \
       NXF_CHANNEL="$CHANNEL" NXF_VERSION="$VERSION" NXF_PLATFORM="$PLATFORM" \
       NXF_INSTALL_DIR="$BIN" NXF_MINISIGN_PUBKEY="${TEST_PUBKEY}" NXF_INSECURE="$INSECURE" \
       sh "$INSTALL_SH" >/dev/null 2>&1; then
  assert_file "$BIN/nxs" "B8 install from a path-bearing base (…/nxs) installs nxs"
  assert_persona_link "$BIN/nxf" "B8 nxf persona linked under a path-bearing base"
else
  bad "B8 install from a path-bearing base (…/nxs) failed"
fi

# --- Layer C: verify_minisign authenticity gate (nexus-flow-l83, f4y) -------------------------
# install.sh keeps sha256 as the mandatory bootstrap and does NOT hard-require the external
# `minisign` binary in general. But authenticity is the real guarantee, so when a pubkey IS
# configured and minisign is missing it now FAILS CLOSED by default (f4y) — refusing rather than
# installing something it cannot prove authentic. A user on a stock host can still bootstrap by
# EXPLICITLY accepting the risk with NXF_INSECURE=1, which downgrades to verified-sha256 with a
# loud warning. (No pubkey at all ⇒ minisign is off by design ⇒ silent sha256-only, unchanged.)
echo "C. verify_minisign authenticity gate (l83, fail-closed f4y)"

# Discover a genuinely capable openssl (real probe) for the Layer-D openssl-path cases BEFORE we
# stub it out below. Empty on stock macOS (LibreSSL has neither Ed25519 nor BLAKE2b — 85y.28).
OPENSSL_BIN="$(capable_openssl 2>/dev/null || true)"

# Simulate a host WITHOUT the minisign binary while keeping every other tool present.
have() { if [ "$1" = minisign ]; then return 1; fi; command -v "$1" >/dev/null 2>&1; }

# Control whether a capable openssl is "present" per case. The fail-closed cases (C1/C1b/C2)
# require NEITHER verifier, so they force this empty; Layer D sets it to the discovered binary.
CAPABLE_OSSL_OVERRIDE=""
capable_openssl() {
  [ -n "$CAPABLE_OSSL_OVERRIDE" ] && {
    printf '%s' "$CAPABLE_OSSL_OVERRIDE"
    return 0
  }
  return 1
}

# C1 pubkey configured, minisign absent, NO override → FAIL CLOSED (rc != 0); the refusal names
# the NXF_INSECURE escape hatch so the user knows how to bootstrap on a stock host.
if c1_err="$(NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$EMBEDDED_MINISIGN_PUBKEY" \
              verify_minisign "$WORK/none" "http://x/none.minisig" "$WORK" 2>&1 1>/dev/null)"; then
  c1_rc=0
else
  c1_rc=$?
fi
if [ "$c1_rc" != "0" ]; then ok "C1 missing minisign fails closed by default (rc $c1_rc)"; else bad "C1 expected fail-closed (non-zero) when minisign absent and NXF_INSECURE unset"; fi
c1_lc="$(printf '%s' "$c1_err" | tr '[:upper:]' '[:lower:]')"
case "$c1_lc" in
  *nxf_insecure*) ok "C1 refusal names the NXF_INSECURE opt-out" ;;
  *) bad "C1 expected the refusal to name NXF_INSECURE, got [$c1_err]" ;;
esac
# The no-verifier refusal must also point a macOS user at the fix (nexus-flow-85y.28/h1h): the
# real-runner smoke can't reproduce a host without ANY verifier (CI always ships a capable
# openssl), so this is the deterministic home for the "refusal carries the brew hint" guarantee.
case "$c1_lc" in
  *"brew install minisign"*) ok "C1 refusal includes the macOS 'brew install minisign' hint" ;;
  *) bad "C1 expected the refusal to suggest 'brew install minisign', got [$c1_err]" ;;
esac

# C1b explicit opt-out: NXF_INSECURE=1 with a pubkey configured but minisign absent → proceed
# (rc 0) AND still warn loudly that authenticity was NOT verified.
if c1b_err="$(NXF_INSECURE=1 NXF_MINISIGN_PUBKEY="$EMBEDDED_MINISIGN_PUBKEY" \
               verify_minisign "$WORK/none" "http://x/none.minisig" "$WORK" 2>&1 1>/dev/null)"; then
  c1b_rc=0
else
  c1b_rc=$?
fi
check "C1b NXF_INSECURE=1 proceeds (explicit opt-out)" "$c1b_rc" "0"
c1b_lc="$(printf '%s' "$c1b_err" | tr '[:upper:]' '[:lower:]')"
case "$c1b_lc" in
  *warning*authenticity*) ok "C1b still warns loudly under the override" ;;
  *) bad "C1b expected a loud authenticity WARNING under NXF_INSECURE=1, got [$c1b_err]" ;;
esac

# C2 boundary: no pubkey configured at all (bootstrap/rotation) → silent skip, NO warning. We
# only warn/refuse when a key IS configured but unusable, never when minisign is off by design.
if c2_err="$(EMBEDDED_MINISIGN_PUBKEY="" NXF_MINISIGN_PUBKEY="" \
              verify_minisign "$WORK/none" "http://x/none.minisig" "$WORK" 2>&1 1>/dev/null)"; then
  c2_rc=0
else
  c2_rc=$?
fi
check "C2 no key configured still proceeds" "$c2_rc" "0"
case "$c2_err" in
  *WARNING*) bad "C2 must not warn when no key is configured (got [$c2_err])" ;;
  *) ok "C2 stays quiet when minisign is off by design" ;;
esac

# --- Layer D: openssl verification fallback (nexus-flow-85y.28) -------------------------------
# When the minisign BINARY is absent but a capable openssl IS present (every Linux, macOS with
# Homebrew openssl), install.sh verifies the prehashed Ed25519/BLAKE2b-512 signature with openssl
# alone: a valid signature is a FULL authenticity check (no NXF_INSECURE needed), an invalid one
# aborts (no downgrade to sha256). D1-D3 cover happy-path / crypto-fail / 404; D4-D5 cover the
# structural refusal boundaries (wrong-length signature; legacy un-prehashed `Ed` rejection).
# Needs minisign to MINT the test signatures + a capable openssl to verify — both present in CI
# (ubuntu) and on a Homebrew macOS; in CI a missing capable openssl FAILS rather than skips.
echo "D. openssl verification fallback (minisign binary absent, 85y.28)"

if [ "$HAVE_MINISIGN" = "1" ] && [ -n "$OPENSSL_BIN" ]; then
  CAPABLE_OSSL_OVERRIDE="$OPENSSL_BIN"
  SIG_URL="http://127.0.0.1:$PORT/download/$CHANNEL/$VERSION/$TARBALL.minisig"

  # D1 valid prehashed signature, verified via openssl → rc 0, NO NXF_INSECURE required.
  if d1_err="$(NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$TEST_PUBKEY" \
                verify_minisign "$DLDIR/$TARBALL" "$SIG_URL" "$WORK" 2>&1 1>/dev/null)"; then
    ok "D1 openssl verifies a valid prehashed signature (no minisign binary, no opt-out)"
  else
    bad "D1 openssl path should verify a valid signature (got [$d1_err])"
  fi

  # D2 invalid signature (signed over other bytes, same key) → ABORT, no downgrade.
  printf 'other content for D2' >"$WORK/d2other"
  minisign -S -H -s "$WORK/test.key" -m "$WORK/d2other" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1
  # Explicit subshell so the script's `die` (which `exit`s) is contained and its rc captured.
  if (NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$TEST_PUBKEY" \
    verify_minisign "$DLDIR/$TARBALL" "$SIG_URL" "$WORK") >/dev/null 2>&1; then
    d2_rc=0
  else
    d2_rc=$?
  fi
  if [ "$d2_rc" != "0" ]; then ok "D2 openssl path aborts on an invalid signature (no downgrade)"; else bad "D2 openssl path should ABORT on a bad signature"; fi
  minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1 # restore

  # D3 genuine 404 on the signature → warn + proceed on the verified sha256 (parity w/ minisign).
  if d3_err="$(NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$TEST_PUBKEY" \
                verify_minisign "$DLDIR/$TARBALL" \
                "http://127.0.0.1:$PORT/download/$CHANNEL/$VERSION/nope.minisig" "$WORK" 2>&1 1>/dev/null)"; then
    ok "D3 missing signature (404) on the openssl path proceeds on sha256"
  else
    bad "D3 a genuine 404 should not abort on the openssl path (got [$d3_err])"
  fi

  # D4 structural refusal: a signature whose line-2 decodes to the wrong length (truncated /
  # malformed) must abort at the length gate, never reach the verify. Valid base64, 9 bytes ≠ 74.
  {
    printf 'untrusted comment: bad\n'
    printf 'AAAAAAAAAAAA\n'
    printf 'trusted comment: bad\n'
    printf 'AAAA\n'
  } >"$DLDIR/$TARBALL.minisig"
  if (NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$TEST_PUBKEY" \
    verify_minisign "$DLDIR/$TARBALL" "$SIG_URL" "$WORK") >/dev/null 2>&1; then
    d4_rc=0
  else
    d4_rc=$?
  fi
  if [ "$d4_rc" != "0" ]; then ok "D4 wrong-length signature aborts at the length gate"; else bad "D4 a malformed-length signature should ABORT"; fi

  # D5 legacy-rejection (a security boundary): a LEGACY (un-prehashed, alg "Ed") signature must
  # abort with the downgrade-refusal message, mirroring `nxf`'s allow_legacy=false. minisign 0.12
  # prehashes by default, so we SYNTHESIZE a legacy sig: take a valid prehashed one and flip the
  # algorithm's 2nd byte 'D'(0x44)→'d'(0x64) so alg reads "Ed", keeping length (74) + key id
  # intact — so only the algorithm-byte check stands between it and a verify on the wrong path.
  minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$WORK/valid.minisig" >/dev/null 2>&1
  sed -n '2p' "$WORK/valid.minisig" | "$OPENSSL_BIN" base64 -d -A >"$WORK/legacy.raw"
  printf '\144' | dd of="$WORK/legacy.raw" bs=1 seek=1 count=1 conv=notrunc 2>/dev/null # 'D'->'d'
  legacy_b64="$("$OPENSSL_BIN" base64 -A <"$WORK/legacy.raw")"
  {
    printf 'untrusted comment: legacy\n'
    printf '%s\n' "$legacy_b64"
    printf 'trusted comment: legacy\n'
    printf 'AAAA\n'
  } >"$DLDIR/$TARBALL.minisig"
  if d5_err="$(NXF_INSECURE="" NXF_MINISIGN_PUBKEY="$TEST_PUBKEY" \
                verify_minisign "$DLDIR/$TARBALL" "$SIG_URL" "$WORK" 2>&1 1>/dev/null)"; then
    bad "D5 a legacy (un-prehashed) signature should ABORT"
  else
    d5_lc="$(printf '%s' "$d5_err" | tr '[:upper:]' '[:lower:]')"
    case "$d5_lc" in
      *downgrade*) ok "D5 legacy (Ed) signature aborts with the downgrade-refusal message" ;;
      *) bad "D5 legacy signature aborted but without the downgrade message (got [$d5_err])" ;;
    esac
  fi
  minisign -S -H -s "$WORK/test.key" -m "$DLDIR/$TARBALL" -x "$DLDIR/$TARBALL.minisig" >/dev/null 2>&1 # restore valid

  CAPABLE_OSSL_OVERRIDE=""
elif [ "${CI:-}" = "true" ] && [ "$HAVE_MINISIGN" = "1" ]; then
  # In CI the openssl security path (D1-D5) MUST run, never skip green: ubuntu-latest ships a
  # capable OpenSSL, so an empty OPENSSL_BIN means the probe regressed, not that openssl is absent.
  bad "a capable openssl must be present in CI — the openssl security path (D1-D5) would skip (OPENSSL_BIN=${OPENSSL_BIN:-none})"
else
  echo "  skip D1-D5 (need minisign to mint a sig + a capable openssl; HAVE_MINISIGN=$HAVE_MINISIGN, OPENSSL_BIN=${OPENSSL_BIN:-none})"
fi

# --- summary ----------------------------------------------------------------------------------
echo ""
echo "install.sh tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
