'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const { verifySha256, verifyMinisign } = require('../lib/verify');

// --- helpers: forge a REAL minisign `-H` (prehashed) signature from a Node Ed25519 keypair, so
// the tests exercise the actual crypto path (no mocks). This mirrors what `minisign -S -H` and
// the release pipeline produce: line 2 is base64(alg[2] || key_id[8] || ed25519_sig[64]) where
// the signed message is the file's BLAKE2b-512 digest.

function makeKeypair(keyId /* 8-byte Buffer */) {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const rawPub = publicKey.export({ type: 'spki', format: 'der' }).subarray(12); // 32 bytes
  const pubB64 = Buffer.concat([Buffer.from('Ed'), keyId, rawPub]).toString('base64');
  return { privateKey, rawPub, pubB64, keyId };
}

// alg defaults to 'ED' (prehashed, what the pipeline emits); pass 'Ed' to forge a legacy sig.
function makeMinisig(buf, kp, { alg = 'ED', sigKeyId = kp.keyId, signKey = kp.privateKey } = {}) {
  const digest = crypto.createHash('blake2b512').update(buf).digest();
  const sig = crypto.sign(null, digest, signKey); // pure Ed25519 over the 64-byte prehash
  const line2 = Buffer.concat([Buffer.from(alg), sigKeyId, sig]).toString('base64');
  // The trusted-comment global signature is not part of the authenticity gate (install.sh's
  // openssl path does not verify it either), so a dummy 4th line is fine.
  return `untrusted comment: signature from test\n${line2}\ntrusted comment: test\n${'A'.repeat(88)}\n`;
}

const KEYID = Buffer.from('0123456789abcdef', 'hex');

// --- sha256 (integrity) ---

test('verifySha256 accepts a matching sidecar', () => {
  const buf = Buffer.from('the tarball bytes');
  const hex = crypto.createHash('sha256').update(buf).digest('hex');
  assert.doesNotThrow(() => verifySha256(buf, `${hex}  nxf_1.2.3_linux-x86_64.tar.gz`));
});

test('verifySha256 compares only the hex, not the filename', () => {
  const buf = Buffer.from('the tarball bytes');
  const hex = crypto.createHash('sha256').update(buf).digest('hex');
  assert.doesNotThrow(() => verifySha256(buf, `${hex}  a-totally-different-name`));
});

test('verifySha256 aborts on a hash mismatch', () => {
  const buf = Buffer.from('the tarball bytes');
  const wrong = 'f'.repeat(64);
  assert.throws(() => verifySha256(buf, `${wrong}  nxf.tar.gz`), /sha256 mismatch/i);
});

test('verifySha256 aborts on an empty sidecar', () => {
  assert.throws(() => verifySha256(Buffer.from('x'), '  \n'), /sha256/i);
});

// --- minisign (authenticity) ---

test('verifyMinisign accepts a valid prehashed signature', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  assert.doesNotThrow(() => verifyMinisign(buf, makeMinisig(buf, kp), kp.pubB64));
});

test('verifyMinisign accepts a full minisign.pub (comment + base64) as the key', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  const fullPub = `untrusted comment: minisign public key\n${kp.pubB64}\n`;
  assert.doesNotThrow(() => verifyMinisign(buf, makeMinisig(buf, kp), fullPub));
});

test('verifyMinisign aborts when the bytes were tampered', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  const sig = makeMinisig(buf, kp);
  const tampered = Buffer.from(buf);
  tampered[0] ^= 0xff;
  assert.throws(() => verifyMinisign(tampered, sig, kp.pubB64), /verification.*failed/i);
});

test('verifyMinisign aborts on a signature from a different key', () => {
  const kp = makeKeypair(KEYID);
  const attacker = makeKeypair(KEYID); // same key id, different key material
  const buf = Buffer.from('a signed release tarball');
  const forged = makeMinisig(buf, attacker);
  assert.throws(() => verifyMinisign(buf, forged, kp.pubB64), /verification.*failed/i);
});

test('verifyMinisign aborts on a key-id mismatch', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  const otherId = Buffer.from('fedcba9876543210', 'hex');
  const sig = makeMinisig(buf, kp, { sigKeyId: otherId });
  assert.throws(() => verifyMinisign(buf, sig, kp.pubB64), /key id/i);
});

test('verifyMinisign refuses a legacy (non-prehashed "Ed") signature', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  const legacy = makeMinisig(buf, kp, { alg: 'Ed' });
  assert.throws(() => verifyMinisign(buf, legacy, kp.pubB64), /prehash/i);
});

test('verifyMinisign aborts on a malformed public key', () => {
  const buf = Buffer.from('a signed release tarball');
  const kp = makeKeypair(KEYID);
  assert.throws(() => verifyMinisign(buf, makeMinisig(buf, kp), 'not-valid-base64!!'), /public key/i);
});

test('verifyMinisign aborts when the .minisig has no signature line', () => {
  const kp = makeKeypair(KEYID);
  const buf = Buffer.from('a signed release tarball');
  assert.throws(() => verifyMinisign(buf, 'untrusted comment: only\n', kp.pubB64), /signature/i);
});
