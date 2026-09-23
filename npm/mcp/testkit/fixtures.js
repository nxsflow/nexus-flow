'use strict';
// Test fixtures shared by the integration tests. Lives outside `test/` so node --test does not
// pick it up as a test file, and outside package.json "files" so it never ships.

const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

// A deterministic Ed25519 keypair packaged as a minisign public key (alg "Ed" || key_id || pk).
function makeKeypair(keyId = Buffer.from('0123456789abcdef', 'hex')) {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const rawPub = publicKey.export({ type: 'spki', format: 'der' }).subarray(12);
  const pubB64 = Buffer.concat([Buffer.from('Ed'), keyId, rawPub]).toString('base64');
  return { privateKey, pubB64, keyId };
}

// A real minisign `-H` (prehashed) signature over `buf`: Ed25519 signs the BLAKE2b-512 digest;
// line 2 is base64(alg "ED" || key_id || sig).
function signTarball(buf, kp) {
  const digest = crypto.createHash('blake2b512').update(buf).digest();
  const sig = crypto.sign(null, digest, kp.privateKey);
  const line2 = Buffer.concat([Buffer.from('ED'), kp.keyId, sig]).toString('base64');
  return `untrusted comment: signature\n${line2}\ntrusted comment: test\n${'A'.repeat(88)}\n`;
}

function sha256Sidecar(buf, name) {
  return `${crypto.createHash('sha256').update(buf).digest('hex')}  ${name}\n`;
}

// Build a `.tar.gz` with bare member names, exactly like release.yml packages it.
function makeTarGz(files) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nxs-fx-'));
  const names = Object.keys(files);
  for (const n of names) fs.writeFileSync(path.join(dir, n), files[n]);
  const tarball = path.join(dir, 'b.tar.gz');
  execFileSync('tar', ['-czf', tarball, '-C', dir, ...names]);
  const buf = fs.readFileSync(tarball);
  fs.rmSync(dir, { recursive: true, force: true });
  return buf;
}

// A complete signed release fixture for one platform/version. `nxsContent` becomes the `nxs`
// member; pass a `#!/bin/sh` script to get an executable stub the shim can exec.
function signedRelease({ version, platform, nxsContent, keypair = makeKeypair() }) {
  const name = `nxf_${version}_${platform}.tar.gz`;
  const tarball = makeTarGz({ nxs: nxsContent, 'README.md': 'readme' });
  return {
    version,
    platform,
    name,
    tarball,
    sha256: sha256Sidecar(tarball, name),
    minisig: signTarball(tarball, keypair),
    pubB64: keypair.pubB64,
    keypair,
  };
}

module.exports = { makeKeypair, signTarball, sha256Sidecar, makeTarGz, signedRelease };
