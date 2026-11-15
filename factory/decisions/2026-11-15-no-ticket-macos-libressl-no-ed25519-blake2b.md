# 2026-11-15 · no ticket · macOS system openssl is LibreSSL: no blake2b512, no Ed25519 — minisign-via-openssl cannot cover stock macOS; nxs self-update is the backstop.

- Decided by: recorded in the project memory (`nxm`, key `macos-libressl-no-ed25519-blake2b`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: install, signing, macos

## The record

SPIKE (2026-06-13): the macOS system `/usr/bin/openssl` is LibreSSL (3.3.6 tested) and supports
NEITHER `dgst -blake2b512` NOR Ed25519 ("Algorithm ed25519 not found", no ed25519 pkey algo). So
minisign-via-openssl verification CANNOT cover stock macOS — it works only where openssl is real
OpenSSL 1.1+/3.x (every Linux, and macOS with a Homebrew openssl).

This is an ALGORITHM-AVAILABILITY gap in LibreSSL, NOT a key-format issue — re-creating the signing
key does not help.

`install.sh` therefore probes CAPABILITY by self-test (`ossl_can_verify`: a blake2b512 plus
Ed25519 `-rawin` sign/verify round trip), never by version. Stock macOS is consequently left with:
install minisign (`brew install minisign`), OR set `NXF_INSECURE=1` to bootstrap on the verified
sha256 alone with a loud warning. Either way the UNCONDITIONAL gate is `nxs self-update`, which
always verifies minisign fail-closed and which every install can run afterwards.

An out-of-band public key (defence against an origin/CDN compromise) is a separate concern and not
solved by this. The strongest macOS first-install hardening is Developer-ID notarization (the Apple
membership is active) plus a Homebrew cask — notarization is the channel-independent trust anchor,
Homebrew is the channel.
