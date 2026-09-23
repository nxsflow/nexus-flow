#!/bin/sh
# install.sh — install the nexus-flow suite without sudo. Multicall (nexus-flow-5jz.7): ONE real
# binary `nxs` is installed; `nxf`, `nxm`, and `nxc` are symlinks to it (each routes by argv[0]).
#
#   curl -fsSL https://nxsflow.com/nxs/install.sh | sh
#
# Spec: docs/specs/release-management.md §7.1 (nexus-flow-85y.16). This script is published
# versionless as an S3 object behind its own short-TTL CloudFront behavior (NOT served by the
# updater Lambda) — see §6.1 / publish-release.sh. It resolves the newest tarball for the host
# via `/latest` (302 → CloudFront), verifies it, and drops the binaries into ~/.local/bin.
#
# Verification (see §5.3):
#   - sha256 is MANDATORY and always runs — a mismatch aborts before anything is installed. It
#     proves INTEGRITY (the bytes weren't mangled in transit), but NOT authenticity: the sidecar
#     shares the tarball's origin, so it does not by itself defend against an origin/CDN compromise.
#   - the minisign signature is the AUTHENTICITY gate (verified against the embedded pubkey),
#     checked with the `minisign` binary if present, ELSE with a capable `openssl` (nexus-flow-
#     85y.28): the prehash is Ed25519 over BLAKE2b-512, which any OpenSSL 1.1+/3.x can verify, so
#     the friction is the minisign *binary*, not the crypto. (macOS's *system* openssl is
#     LibreSSL, which has neither primitive — spike, 85y.28 — so a Homebrew openssl or the
#     minisign binary is needed there.) A bad signature aborts; a transport error on the signature
#     fetch aborts (no silent downgrade); only a genuine 404 (signature not published) proceeds on
#     the verified sha256. On a host with NEITHER verifier, install.sh FAILS CLOSED by default
#     (nexus-flow-l83, f4y): it refuses rather than install bytes whose authenticity it cannot
#     establish. Set NXF_INSECURE=1 to explicitly bootstrap on the verified sha256 alone (with a
#     loud warning); either way the unconditional gate is `nxs self-update`, which always verifies
#     minisign fail-closed and which every install can run afterward.
#   - No sudo: installs under $HOME by default; idempotent (re-running upgrades in place).
#
# Configuration (all via env, no flags — the script is piped to `sh`):
#   NXF_CHANNEL=stable        promotion ring to pull from (stable|beta|alpha)
#   NXF_VERSION=0.2.0         pin an exact version (bypasses /latest, hits download/<ch>/<v>/…)
#   NXF_INSTALL_DIR=~/.local/bin   target directory (created if absent; never needs sudo)
#   NXF_INSTALL_RELAY=1       also install the `nxf-relay` binary
#   NXF_BASE_URL=https://nxsflow.com/nxs   public origin (point at staging to rehearse)
#   NXF_PLATFORM=linux-x86_64 override host detection (escape hatch / testing)
#   NXF_MINISIGN_PUBKEY=…     override the embedded minisign public key
#   NXF_INSECURE=1            on a host without minisign, bootstrap on the verified sha256 alone
#                            instead of failing closed (authenticity NOT verified — see above)

# Embedded minisign public key (PUBLIC — safe to ship in a world-readable script). The same
# key is compiled into `nxf` for self-update (release.yml bakes `vars.NXF_MINISIGN_PUBKEY`).
# Empty ⇒ the minisign step is skipped and sha256 is the sole gate. Rotation = update this line.
EMBEDDED_MINISIGN_PUBKEY="RWRnRFdblE2/DEmkBWpMPqXYele5JoNtQubF+N1Ryp7msJgq7HKn2Ola"

# ---------------------------------------------------------------------------------------------

log()  { printf '%s\n' "$*" >&2; }
die()  { printf 'install.sh: error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# platform_for <uname-s> <uname-m> → prints "<os>-<arch>" (one of the four shipped platforms)
# and returns 0, or returns 1 for anything we don't ship. Pure: maps `uname` output to the
# canonical manifest/tarball key (spec §5.2). Forgiving on input (arm64≡aarch64, amd64≡x86_64).
# The os/arch tokens here are checked against release/platforms (the single source of truth for
# the shipped platforms) by `cargo xtask platforms check` in CI — nexus-flow-4sr.
platform_for() {
  _pf_os=""
  _pf_arch=""
  case "$1" in
    Darwin) _pf_os="darwin" ;;
    Linux)  _pf_os="linux" ;;
  esac
  case "$2" in
    arm64 | aarch64) _pf_arch="aarch64" ;;
    x86_64 | amd64)  _pf_arch="x86_64" ;;
  esac
  [ -n "$_pf_os" ] && [ -n "$_pf_arch" ] || return 1
  printf '%s-%s' "$_pf_os" "$_pf_arch"
}

# Resolve PLATFORM / OS / ARCH from `uname` (or the NXF_PLATFORM override). Aborts on an
# unsupported host so the failure is a clear message, not a 404 three steps later.
detect_platform() {
  if [ -n "${NXF_PLATFORM:-}" ]; then
    PLATFORM="$NXF_PLATFORM"
  else
    PLATFORM="$(platform_for "$(uname -s)" "$(uname -m)")" \
      || die "unsupported platform: $(uname -s)/$(uname -m). Supported: {darwin,linux}-{aarch64,x86_64}."
  fi
  OS="${PLATFORM%-*}"
  ARCH="${PLATFORM#*-}"
}

# fetch <url> <dest>: download to a file, failing on any HTTP error. curl or wget.
fetch() {
  if have curl; then
    curl -fsSL -o "$2" "$1"
  elif have wget; then
    wget -q -O "$2" "$1"
  else
    die "need curl or wget to download"
  fi
}

# resolve_redirect <url>: print the Location of a single 3xx response WITHOUT following it
# (used for /latest — we want the resolved tarball URL so the .sha256/.minisig sidecars sit
# next to it). A 404 (nothing published) prints nothing so the caller can die with a friendly
# message; a transport/5xx error aborts loudly here (don't mask a real failure as "not found").
resolve_redirect() {
  if have curl; then
    # No `-f`: a 404 must reach us as a readable status code, not a curl error under `set -e`.
    _rr="$(curl -sS -o /dev/null -w '%{http_code} %{redirect_url}' "$1")" \
      || die "request to $1 failed (network/TLS error)"
    _rr_code="${_rr%% *}"
    case "$_rr_code" in
      30[0-9]) printf '%s' "${_rr#* }" ;;
      404)     : ;; # nothing published on this channel/platform → empty (caller dies friendly)
      *)       die "unexpected HTTP $_rr_code from $1" ;;
    esac
  elif have wget; then
    # Strip a trailing CR off the header value — HTTP headers are CRLF-terminated and a CR
    # riding along would corrupt the URL we then fetch.
    wget -q -S --max-redirect=0 -O /dev/null "$1" 2>&1 \
      | awk 'tolower($1) == "location:" { sub(/\r$/, "", $2); print $2; exit }'
  else
    die "need curl or wget to download"
  fi
}

# same_origin <url> <base>: succeeds iff <url> has the SAME scheme://authority as <base>. The
# /latest redirect target is server-controlled; this bars an open-redirect to another host or a
# downgrade to http:// before we fetch it (the resolved URL flows into `fetch`, which follows
# redirects). Scheme and host are checked together (the authority captures host[:port]).
same_origin() {
  _so_a="$(printf '%s' "$1" | sed -n 's#^\([a-zA-Z][a-zA-Z0-9+.-]*://[^/]*\).*#\1#p')"
  _so_b="$(printf '%s' "$2" | sed -n 's#^\([a-zA-Z][a-zA-Z0-9+.-]*://[^/]*\).*#\1#p')"
  [ -n "$_so_a" ] && [ "$_so_a" = "$_so_b" ]
}

# sig_fetch_status <url> <dest>: download <url> to <dest> and echo the HTTP status code. A
# transport/TLS error (curl exit != 0) is fatal — only a real HTTP status is a safe basis for
# the 404-vs-error decision in verify_minisign (a dropped connection must not read as "404").
sig_fetch_status() {
  if have curl; then
    curl -sSL -o "$2" -w '%{http_code}' "$1" || die "request to $1 failed (network/TLS error)"
  elif have wget; then
    if wget -q -O "$2" "$1"; then
      printf '200'
    elif wget -q --spider -S "$1" 2>&1 | grep -q ' 404'; then
      printf '404'
    else
      printf '000' # unknown failure ⇒ caller aborts (fail-closed), never a silent skip
    fi
  else
    die "need curl or wget to download"
  fi
}

# sha256_hex <file>: print the lowercase hex sha256 (sha256sum or shasum -a 256).
sha256_hex() {
  if have sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif have shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "need sha256sum or shasum to verify the download"
  fi
}

# verify_sha256 <tarball> <sidecar-file>: MANDATORY gate. The sidecar is `<hex>  <name>`
# (publish-release.sh); we compare only the hex so a name mismatch can't weaken it.
verify_sha256() {
  _vs_expected="$(awk 'NR==1 {print $1}' "$2")"
  [ -n "$_vs_expected" ] || die "empty sha256 sidecar"
  _vs_actual="$(sha256_hex "$1")"
  [ "$_vs_expected" = "$_vs_actual" ] \
    || die "sha256 mismatch — expected $_vs_expected, got $_vs_actual (refusing to install)"
  log "sha256 verified: $_vs_actual"
}

# ossl_can_verify <openssl-path> [parent-dir]: succeed iff this openssl can do EVERYTHING the
# openssl verification path needs — a BLAKE2b-512 digest AND pure-Ed25519 sign/verify with
# `-rawin`. It self-tests with a throwaway key rather than sniffing versions: macOS's system
# openssl is LibreSSL, which supports NEITHER primitive (spike, nexus-flow-85y.28), while OpenSSL
# 1.1+/3.x (every Linux, Homebrew openssl on macOS) supports both. The scratch dir is removed on
# every return path; when <parent-dir> is main's `$work` it is ALSO covered by main's EXIT/INT/
# TERM trap, so an interrupt mid-self-test can't leak it either.
ossl_can_verify() {
  _ocv_o="$1"
  _ocv_d="$(mktemp -d "${2:-${TMPDIR:-/tmp}}/nxf-ossl.XXXXXX")" || return 1
  _ocv_ok=1
  {
    printf '' | "$_ocv_o" dgst -blake2b512 >/dev/null 2>&1 &&
      "$_ocv_o" genpkey -algorithm ed25519 -out "$_ocv_d/k.pem" >/dev/null 2>&1 &&
      "$_ocv_o" pkey -in "$_ocv_d/k.pem" -pubout -out "$_ocv_d/p.pem" >/dev/null 2>&1 &&
      dd if=/dev/zero of="$_ocv_d/m.bin" bs=64 count=1 >/dev/null 2>&1 &&
      "$_ocv_o" pkeyutl -sign -inkey "$_ocv_d/k.pem" -rawin \
        -in "$_ocv_d/m.bin" -out "$_ocv_d/s.bin" >/dev/null 2>&1 &&
      "$_ocv_o" pkeyutl -verify -pubin -inkey "$_ocv_d/p.pem" -rawin \
        -in "$_ocv_d/m.bin" -sigfile "$_ocv_d/s.bin" >/dev/null 2>&1
  } || _ocv_ok=0
  rm -rf "$_ocv_d"
  [ "$_ocv_ok" = 1 ]
}

# capable_openssl [parent-dir]: print the path of an openssl that `ossl_can_verify`, or fail.
# Probes the PATH `openssl` first, then the usual Homebrew locations on macOS (where the PATH
# openssl is LibreSSL but a capable `openssl@3` may be installed). <parent-dir> (main's `$work`)
# is forwarded so the self-test scratch is cleaned by main's trap. Empty ⇒ no usable openssl.
capable_openssl() {
  for _co in openssl /opt/homebrew/bin/openssl /usr/local/bin/openssl \
    /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    _co_path="$(command -v "$_co" 2>/dev/null)" || continue
    [ -n "$_co_path" ] || continue
    if ossl_can_verify "$_co_path" "${1:-}"; then
      printf '%s' "$_co_path"
      return 0
    fi
  done
  return 1
}

# verify_minisign_openssl <tarball> <sig-file> <pubkey> <workdir> <openssl>: verify a prehashed
# (minisign `-H`) Ed25519/BLAKE2b-512 signature with openssl alone — for hosts that have a
# capable openssl but not the `minisign` binary (nexus-flow-85y.28). A minisign public key is
# base64 of: 2-byte algorithm + 8-byte key id + 32-byte Ed25519 key; a `.minisig` line 2 is
# base64 of: 2-byte algorithm + 8-byte key id + 64-byte signature. We SPKI-wrap the raw key so
# openssl can load it, then verify pure Ed25519 over the file's BLAKE2b-512 digest (the message
# minisign `-H` signs). Mirrors `nxf`'s prehashed-only posture: a legacy (un-prehashed, `Ed`)
# signature is refused rather than verified on a different path. Any failure aborts (no downgrade).
verify_minisign_openssl() {
  _vmo_tar="$1"
  _vmo_sig="$2"
  _vmo_pub="$3"
  _vmo_wd="$4"
  _vmo_ossl="$5"

  # Public key: accept a bare base64 line OR a full minisign.pub (comment + base64) — take the
  # last non-comment, non-blank line, exactly the base64 the minisign path feeds to `-P`.
  _vmo_pub_b64="$(printf '%s\n' "$_vmo_pub" \
    | sed -e '/^untrusted comment:/d' -e '/^[[:space:]]*$/d' | tail -n1)"
  printf '%s' "$_vmo_pub_b64" | "$_vmo_ossl" base64 -d -A >"$_vmo_wd/pub.raw" 2>/dev/null \
    || die "minisign public key is not valid base64 (refusing to verify)"
  [ "$(wc -c <"$_vmo_wd/pub.raw" | tr -d ' ')" = "42" ] \
    || die "minisign public key has an unexpected length (refusing to verify)"
  # The byte-slice `dd`s below are safe ONLY because the 42/74-byte length asserts above already
  # proved the ranges exist (dd never short-reads past EOF here); the `|| die` guards a genuine
  # I/O failure and attributes it clearly rather than letting a later step fail opaquely.
  dd if="$_vmo_wd/pub.raw" bs=1 skip=10 count=32 of="$_vmo_wd/pubkey32.raw" 2>/dev/null \
    || die "failed to parse signature fields (refusing to install)"
  # SPKI-wrap the raw Ed25519 key with the fixed 12-byte DER header so openssl can import it.
  printf '\060\052\060\005\006\003\053\145\160\003\041\000' >"$_vmo_wd/spki.der"
  cat "$_vmo_wd/pubkey32.raw" >>"$_vmo_wd/spki.der"
  "$_vmo_ossl" pkey -pubin -inform DER -in "$_vmo_wd/spki.der" \
    -pubout -out "$_vmo_wd/pub.pem" 2>/dev/null \
    || die "could not load the minisign Ed25519 public key via openssl (refusing to verify)"

  # Signature: line 2 of the .minisig.
  _vmo_sigline="$(sed -n '2p' "$_vmo_sig")"
  printf '%s' "$_vmo_sigline" | "$_vmo_ossl" base64 -d -A >"$_vmo_wd/sig.raw" 2>/dev/null \
    || die "minisign signature is not valid base64 (refusing to install)"
  [ "$(wc -c <"$_vmo_wd/sig.raw" | tr -d ' ')" = "74" ] \
    || die "minisign signature has an unexpected length (refusing to install)"
  # Algorithm bytes: "ED" = prehashed (what release.yml produces and `nxf` requires). Anything
  # else (e.g. legacy "Ed") is refused here rather than verified on a different code path.
  _vmo_alg="$(dd if="$_vmo_wd/sig.raw" bs=1 count=2 2>/dev/null)"
  [ "$_vmo_alg" = "ED" ] \
    || die "minisign signature is not the prehashed (-H) form this installer verifies via openssl (refusing to downgrade)"
  # The signature's key id (bytes 3-10) must match the public key's, exactly as minisign checks.
  dd if="$_vmo_wd/pub.raw" bs=1 skip=2 count=8 of="$_vmo_wd/pub.keyid" 2>/dev/null \
    || die "failed to parse signature fields (refusing to install)"
  dd if="$_vmo_wd/sig.raw" bs=1 skip=2 count=8 of="$_vmo_wd/sig.keyid" 2>/dev/null \
    || die "failed to parse signature fields (refusing to install)"
  cmp -s "$_vmo_wd/pub.keyid" "$_vmo_wd/sig.keyid" \
    || die "minisign signature key id does not match the configured public key (refusing to install)"
  dd if="$_vmo_wd/sig.raw" bs=1 skip=10 count=64 of="$_vmo_wd/sig64.raw" 2>/dev/null \
    || die "failed to parse signature fields (refusing to install)"

  # Message = BLAKE2b-512 of the file (the minisign `-H` prehash), verified with pure Ed25519.
  "$_vmo_ossl" dgst -blake2b512 -binary "$_vmo_tar" >"$_vmo_wd/digest.bin" 2>/dev/null \
    || die "openssl could not compute the BLAKE2b-512 digest (refusing to install)"
  "$_vmo_ossl" pkeyutl -verify -pubin -inkey "$_vmo_wd/pub.pem" -rawin \
    -in "$_vmo_wd/digest.bin" -sigfile "$_vmo_wd/sig64.raw" >/dev/null 2>&1 \
    || die "minisign signature verification (via openssl) FAILED (refusing to install)"
  # Explicit success: this is security-critical: a future appended command must NOT silently
  # become the return value of a function whose 0 means "authenticity verified".
  return 0
}

# verify_minisign <tarball> <sig-url> <workdir>: the AUTHENTICITY gate, run when minisign is
# installed and a pubkey is configured. sha256 proves integrity but not authenticity — the
# sidecar shares the tarball's origin, so an attacker who controls the bytes controls the hash;
# minisign (verified against the embedded pubkey) is the only gate that survives an origin/CDN
# compromise. Therefore: a genuine 404 (signature not published) warns and proceeds on sha256,
# but a transport/5xx error on the signature fetch ABORTS — an attacker who can drop or block
# the .minisig must NOT be able to silently downgrade us to sha256-only.
#
# Authenticity posture without the minisign binary (nexus-flow-l83, fail-closed f4y): install.sh
# does NOT hard-require minisign in general (no key configured ⇒ sha256-only by design). But when
# a pubkey IS configured and the binary is missing, authenticity cannot be established, so it
# FAILS CLOSED by default — refusing rather than installing unverifiable bytes (sha256 shares the
# tarball's origin and does not survive a compromised mirror/CDN). A user on a stock host can
# still bootstrap by EXPLICITLY accepting the risk with NXF_INSECURE=1, which downgrades to the
# verified-sha256 path with a loud warning. Either way `nxs self-update` afterward verifies
# minisign unconditionally (fail-closed), so the gap is bounded to this first install.
verify_minisign() {
  _vm_pubkey="${NXF_MINISIGN_PUBKEY:-$EMBEDDED_MINISIGN_PUBKEY}"
  # No key configured at all (bootstrap/rotation, EMBEDDED_… deliberately empty): minisign is
  # off by design and sha256 is the sole gate — stay quiet, this is the documented posture.
  if [ -z "$_vm_pubkey" ]; then
    return 0
  fi
  # Pick a verifier. The `minisign` binary is the primary path; a capable openssl is the
  # fallback (nexus-flow-85y.28) so the friction is the *binary*, not the crypto — Ed25519 over
  # a BLAKE2b-512 prehash is just openssl, present on every Linux and on macOS via Homebrew. The
  # macOS *system* openssl is LibreSSL and can do neither, so `capable_openssl` returns nothing
  # there and the host stays on the binary requirement below.
  _vm_verifier=""
  _vm_ossl=""
  if have minisign; then
    _vm_verifier="minisign"
  elif _vm_ossl="$(capable_openssl "$3")"; then
    _vm_verifier="openssl"
  fi
  # A key IS configured but neither verifier is available (typically stock macOS without the
  # minisign binary): fail closed unless the user explicitly opts out.
  if [ -z "$_vm_verifier" ]; then
    if [ "${NXF_INSECURE:-}" = "1" ]; then
      log ""
      log "WARNING: no minisign binary or capable openssl found and NXF_INSECURE=1 — installing on the verified sha256 ALONE."
      log "  The AUTHENTICITY of this download was NOT verified; sha256 over TLS proves integrity but"
      log "  not origin and does not defend against a compromised mirror/CDN. After install run"
      log "  \`nxs self-update\` (it ALWAYS verifies minisign, fail-closed)."
      log ""
      return 0
    fi
    die "neither the minisign binary nor a capable openssl is available, so the AUTHENTICITY of this download cannot be verified (refusing to install). Install minisign and re-run for the full guarantee (on macOS: \`brew install minisign\`), or set NXF_INSECURE=1 to bootstrap on the verified sha256 alone (it proves integrity but not origin; \`nxs self-update\` then verifies signatures unconditionally afterward)."
  fi
  _vm_sig="$3/sig.minisig"
  _vm_code="$(sig_fetch_status "$2" "$_vm_sig")"
  case "$_vm_code" in
    2??) ;; # signature fetched — verify it below
    404)
      log "note: no minisign signature published (404) — proceeding on the verified sha256"
      return 0
      ;;
    *)
      die "minisign signature fetch failed (HTTP ${_vm_code}) — refusing to downgrade to sha256-only"
      ;;
  esac
  if [ "$_vm_verifier" = "minisign" ]; then
    minisign -V -P "$_vm_pubkey" -m "$1" -x "$_vm_sig" >/dev/null 2>&1 \
      || die "minisign signature verification FAILED (refusing to install)"
    log "minisign signature verified"
  else
    verify_minisign_openssl "$1" "$_vm_sig" "$_vm_pubkey" "$3" "$_vm_ossl"
    log "minisign signature verified (via openssl)"
  fi
}

# install_one <name> <extract-dir>: place one binary into NXF_INSTALL_DIR atomically
# (write a temp sibling, chmod, rename) so a concurrent `nxf` is never a half-written file.
install_one() {
  [ -f "$2/$1" ] || die "$1 not found in the downloaded tarball"
  _io_tmp="$NXF_INSTALL_DIR/.$1.tmp.$$"
  # The temp sibling lives outside the scratch dir's trap; clean it up on any failed step so a
  # crashed install never strands a `.nxf.tmp.<pid>` next to the real binary.
  if cp "$2/$1" "$_io_tmp" && chmod 755 "$_io_tmp" && mv -f "$_io_tmp" "$NXF_INSTALL_DIR/$1"; then
    log "installed $NXF_INSTALL_DIR/$1"
  else
    rm -f "$_io_tmp"
    die "failed to install $1 into $NXF_INSTALL_DIR"
  fi
}

# link_persona <name>: point NXF_INSTALL_DIR/<name> at the real `nxs` binary as a RELATIVE symlink
# (multicall, nexus-flow-5jz.7 — typing `nxf`/`nxm` runs `nxs`, which routes by argv[0]). Atomic:
# create a temp sibling link, then rename over any prior file/link. `nxs` must already be installed.
link_persona() {
  _lp_tmp="$NXF_INSTALL_DIR/.$1.tmp.$$"
  rm -f "$_lp_tmp"
  if ln -s nxs "$_lp_tmp" && mv -f "$_lp_tmp" "$NXF_INSTALL_DIR/$1"; then
    log "linked $NXF_INSTALL_DIR/$1 -> nxs"
  else
    rm -f "$_lp_tmp"
    die "failed to link $1 -> nxs in $NXF_INSTALL_DIR"
  fi
}

# sync_sidecar <extract-dir>: make the `nxc` agent sidecar beside the binary agree with what the
# release being installed actually carries (nexus-flow-6j6v.smsz).
#
# THE NORMAL PATH PLACES NOTHING. Since 0.54.0 the sidecar is compiled INTO `nxs`, so no release
# carries it as a tarball member and there is no second file to install — which is the whole point:
# the code that places a second file always lives in the version being installed, so the update
# that INTRODUCES it is performed by the previous version, which knows nothing about it. What this
# does instead is REMOVE a file left behind by <=0.53.0: a bundle belonging to a version that is no
# longer installed, i.e. exactly the binary/sidecar drift 6j6v.5vct described. The running `nxs`
# prefers its own embedded copy either way, so the removal reclaims 1.4 MB and ends the pairing
# question rather than changing which sidecar runs.
#
# The placing half stays for ONE case: `NXF_VERSION` can pin a release from 0.52.0/0.53.0, whose
# binary has no embedded sidecar and can only find one beside itself. Placing it is the same atomic
# temp+rename `install_one` uses, but mode 644 — it is a Node module `nxs` hands to `node`, not a
# program on the PATH.
sync_sidecar() {
  _is_name="nxc-agent-sidecar.mjs"
  if [ ! -f "$1/$_is_name" ]; then
    if [ -e "$NXF_INSTALL_DIR/$_is_name" ]; then
      # Housekeeping, so a failure is SAID rather than fatal — the install itself succeeded, and the
      # leftover is inert either way (`nxs` prefers its own embedded copy). Said out loud all the
      # same, matching `selfupdate.rs`'s warning on the same step: a removal that quietly does
      # nothing is how 1.4 MB of superseded code sits next to a binary for a year unnoticed.
      if rm -f "$NXF_INSTALL_DIR/$_is_name"; then
        log "removed stale $NXF_INSTALL_DIR/$_is_name (this nxs carries its own)"
      else
        log "note: could not remove the superseded $NXF_INSTALL_DIR/$_is_name — it is inert (the sidecar is built into nxs now), so this is housekeeping, not a failure"
      fi
    fi
    return 0
  fi
  _is_tmp="$NXF_INSTALL_DIR/.$_is_name.tmp.$$"
  if cp "$1/$_is_name" "$_is_tmp" && chmod 644 "$_is_tmp" && mv -f "$_is_tmp" "$NXF_INSTALL_DIR/$_is_name"; then
    log "installed $NXF_INSTALL_DIR/$_is_name"
  else
    rm -f "$_is_tmp"
    die "failed to install $_is_name into $NXF_INSTALL_DIR"
  fi
}

# Warn (don't fail) when the install dir is not on PATH, printing the exact line to add.
check_path() {
  case ":${PATH:-}:" in
    *":$NXF_INSTALL_DIR:"*) : ;;
    *)
      log ""
      log "note: $NXF_INSTALL_DIR is not on your PATH. Add this to your shell profile:"
      log "  export PATH=\"$NXF_INSTALL_DIR:\$PATH\""
      ;;
  esac
}

main() {
  set -eu

  channel="${NXF_CHANNEL:-stable}"
  install_dir="${NXF_INSTALL_DIR:-$HOME/.local/bin}"
  base_url="${NXF_BASE_URL:-https://nxsflow.com/nxs}"
  base_url="${base_url%/}" # tolerate a trailing slash
  NXF_INSTALL_DIR="$install_dir"

  detect_platform
  log "nexus-flow installer — platform $PLATFORM, channel $channel"

  if [ -n "${NXF_VERSION:-}" ]; then
    # Pinned: address the immutable per-version path directly, no /latest round-trip.
    version="$NXF_VERSION"
    tarball_name="nxf_${version}_${PLATFORM}.tar.gz"
    tar_url="${base_url}/download/${channel}/${version}/${tarball_name}"
  else
    # Newest in the channel: /latest answers 302 with the CloudFront tarball URL.
    tar_url="$(resolve_redirect "${base_url}/latest?channel=${channel}&target=${OS}&arch=${ARCH}")"
    [ -n "$tar_url" ] \
      || die "no release published for $PLATFORM on channel '$channel' (${base_url}/latest returned no redirect)"
    # The redirect target is server-controlled — refuse to follow it off our own origin.
    same_origin "$tar_url" "$base_url" \
      || die "refusing /latest redirect to a different origin: $tar_url (expected origin $base_url)"
    tarball_name="$(basename "$tar_url")"
  fi

  work="$(mktemp -d "${TMPDIR:-/tmp}/nxf-install.XXXXXX")"
  # Clean the scratch dir on any exit (success, failure, signal) — never leave downloads behind.
  trap 'rm -rf "$work"' EXIT INT TERM

  tarball="$work/$tarball_name"
  log "downloading $tar_url"
  fetch "$tar_url" "$tarball" || die "download failed: $tar_url"
  fetch "${tar_url}.sha256" "$work/sha256" || die "missing sha256 sidecar: ${tar_url}.sha256"

  verify_sha256 "$tarball" "$work/sha256"
  verify_minisign "$tarball" "${tar_url}.minisig" "$work"

  mkdir -p "$NXF_INSTALL_DIR"

  # Extract ONLY the binaries we install, by exact member name — naming the members defeats any
  # `../`/absolute-path entry a tampered tarball might carry (defense-in-depth behind sha256 +
  # minisign; the archive ships bare names `nxs`/`nxf-relay`, see release.yml packaging).
  #
  # Multicall (nexus-flow-5jz.7): the tarball ships ONE real binary, `nxs`, which contains the whole
  # suite and serves the `nxf`/`nxm`/`nxc` surfaces by argv[0]. Install `nxs`, then create
  # `nxf`/`nxm`/`nxc` as symlinks to it — so typing them keeps working with no extra copy on disk.
  # Every persona must be linked here (must match crates/nxs/build.rs + PERSONA_LINKS in
  # selfupdate.rs); a missing one breaks that install path — `nxc` was missing, so `nxs init` for
  # chat failed to spawn `nxc agent-manifest` (6j6v.kew6).
  tar -xzf "$tarball" -C "$work" nxs
  install_one nxs "$work"
  link_persona nxf
  link_persona nxm
  link_persona nxc
  # The `nxc` agent sidecar. Since 6j6v.smsz it is compiled into `nxs` and NO current release
  # carries it as a member; 0.52.0/0.53.0 do, and `NXF_VERSION` can still pin one. `tar -x
  # <missing-member>` is an error — so ASK whether the archive carries it, and leave the extraction
  # itself hard. `2>/dev/null || true` on the extract would have reported a full disk, an unwritable
  # scratch dir, or a truncated member as a property of the release, right after sha256 + minisign
  # passed: the one place where a discrepancy is actually interesting.
  if tar -tzf "$tarball" nxc-agent-sidecar.mjs >/dev/null 2>&1; then
    tar -xzf "$tarball" -C "$work" nxc-agent-sidecar.mjs \
      || die "failed to extract nxc-agent-sidecar.mjs from the verified tarball"
  fi
  sync_sidecar "$work"
  if [ "${NXF_INSTALL_RELAY:-0}" = "1" ]; then
    tar -xzf "$tarball" -C "$work" nxf-relay
    install_one nxf-relay "$work"
  fi

  check_path
  log ""
  log "nexus-flow suite installed (nxf, nxm, nxc, nxs). Run: nxs init"
}

# Library mode (NXF_INSTALL_SH_LIB=1): define the functions but do not run, so the test
# harness can source this file and exercise the helpers directly.
if [ "${NXF_INSTALL_SH_LIB:-0}" != "1" ]; then
  main "$@"
fi
