'use strict';

const crypto = require('node:crypto');

// Two-gate download verification, ported faithfully from install.sh (§5.3) — but fail-closed
// UNCONDITIONALLY: this is the binary-exec path, so there is no NXF_INSECURE escape hatch and no
// 404-signature downgrade. sha256 proves INTEGRITY; minisign proves AUTHENTICITY. Any failure in
// either throws, and the caller must never exec an unverified binary.

// The fixed 12-byte SPKI DER header that wraps a raw Ed25519 public key so `crypto.createPublicKey`
// can import it (identical bytes to install.sh's `\060\052\060\005\006\003\053\145\160\003\041\000`).
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

// Strict base64 decode: reject anything outside the base64 alphabet BEFORE decoding (Node's decoder
// silently drops invalid chars, which would weaken the length checks below).
function strictBase64(s, what) {
  const trimmed = String(s).trim();
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(trimmed)) {
    throw new Error(`${what} is not valid base64 (refusing to verify)`);
  }
  return Buffer.from(trimmed, 'base64');
}

// verifySha256(buf, sidecarText): MANDATORY integrity gate. The sidecar is `<hex>  <name>`
// (publish-release.sh); compare only the hex so a name mismatch cannot weaken it.
function verifySha256(buf, sidecarText) {
  const expected = String(sidecarText).trim().split(/\s+/)[0];
  if (!expected) throw new Error('empty sha256 sidecar (refusing to install)');
  const actual = crypto.createHash('sha256').update(buf).digest('hex');
  if (expected.toLowerCase() !== actual) {
    throw new Error(`sha256 mismatch — expected ${expected}, got ${actual} (refusing to install)`);
  }
}

// Take the base64 body of a minisign public key: accept a bare base64 line OR a full minisign.pub
// (comment + base64), exactly as install.sh's openssl path does — the last non-comment, non-blank
// line.
function pubkeyBody(pubkeyText) {
  const lines = String(pubkeyText)
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('untrusted comment:'));
  if (lines.length === 0) throw new Error('minisign public key is empty (refusing to verify)');
  return lines[lines.length - 1];
}

// verifyMinisign(buf, minisigText, pubkeyText): MANDATORY authenticity gate. Verifies a prehashed
// (minisign `-H`) Ed25519/BLAKE2b-512 signature — the only form the release pipeline emits and the
// only form accepted here (a legacy un-prehashed "Ed" signature is refused, never verified on a
// different path). Throws on any failure.
function verifyMinisign(buf, minisigText, pubkeyText) {
  // Public key: 2-byte algorithm + 8-byte key id + 32-byte Ed25519 key = 42 bytes.
  const pub = strictBase64(pubkeyBody(pubkeyText), 'minisign public key');
  if (pub.length !== 42) {
    throw new Error('minisign public key has an unexpected length (refusing to verify)');
  }
  const pubKeyId = pub.subarray(2, 10);
  const rawKey = pub.subarray(10, 42);

  // Signature: line 2 of the .minisig — 2-byte algorithm + 8-byte key id + 64-byte signature.
  const sigLine = String(minisigText).split('\n')[1];
  if (!sigLine || !sigLine.trim()) {
    throw new Error('minisign .minisig has no signature line (refusing to install)');
  }
  const sig = strictBase64(sigLine, 'minisign signature');
  if (sig.length !== 74) {
    throw new Error('minisign signature has an unexpected length (refusing to install)');
  }
  // Algorithm bytes: "ED" = prehashed (what release.yml produces and what this shim requires).
  // Anything else (e.g. legacy "Ed") is refused here rather than verified on a different code path.
  const alg = sig.subarray(0, 2).toString('latin1');
  if (alg !== 'ED') {
    throw new Error(
      'minisign signature is not the prehashed (-H) form this shim verifies (refusing to downgrade)'
    );
  }
  // The signature's key id must match the configured public key's, exactly as minisign checks.
  if (!crypto.timingSafeEqual(sig.subarray(2, 10), pubKeyId)) {
    throw new Error('minisign signature key id does not match the configured public key (refusing to install)');
  }
  const signature = sig.subarray(10, 74);

  // Message = BLAKE2b-512 of the file (the minisign `-H` prehash), verified with pure Ed25519.
  const digest = crypto.createHash('blake2b512').update(buf).digest();
  let ok;
  try {
    const key = crypto.createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, rawKey]),
      format: 'der',
      type: 'spki',
    });
    ok = crypto.verify(null, digest, key, signature);
  } catch {
    ok = false;
  }
  if (!ok) {
    throw new Error('minisign signature verification FAILED (refusing to install)');
  }
}

module.exports = { verifySha256, verifyMinisign };
