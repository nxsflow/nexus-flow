'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { extractMember } = require('../lib/extract');

// Build a real `.tar.gz` with bare member names the way release.yml packages it
// (`tar -czf <t> -C <stage> nxs nxf-relay README.md`), so we parse genuine tar output.
function makeTarGz(files /* {name: contents} */) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nxs-tar-'));
  const names = Object.keys(files);
  for (const name of names) fs.writeFileSync(path.join(dir, name), files[name]);
  const tarball = path.join(dir, 'bundle.tar.gz');
  execFileSync('tar', ['-czf', tarball, '-C', dir, ...names]);
  const buf = fs.readFileSync(tarball);
  fs.rmSync(dir, { recursive: true, force: true });
  return buf;
}

test('extracts a named member by exact name', () => {
  const nxs = Buffer.from('#!/fake nxs binary contents\x00\x01\x02', 'latin1');
  const targz = makeTarGz({ nxs, 'README.md': 'hello', 'nxf-relay': 'relay' });
  assert.deepEqual(extractMember(targz, 'nxs'), nxs);
});

test('returns the exact bytes for a larger member spanning many blocks', () => {
  // > 512 bytes forces multi-block reads and correct padding math.
  const big = Buffer.alloc(5000);
  for (let i = 0; i < big.length; i++) big[i] = i % 256;
  const targz = makeTarGz({ nxs: big, other: 'x' });
  assert.deepEqual(extractMember(targz, 'nxs'), big);
});

test('throws when the member is absent', () => {
  const targz = makeTarGz({ 'nxf-relay': 'relay', 'README.md': 'hello' });
  assert.throws(() => extractMember(targz, 'nxs'), /nxs.*not found/i);
});

test('does not confuse a different member with the target name', () => {
  const targz = makeTarGz({ 'nxs-notes.txt': 'decoy', 'README.md': 'hello' });
  assert.throws(() => extractMember(targz, 'nxs'), /not found/i);
});

test('aborts on a gzip whose inflated size exceeds the cap (decompression-bomb guard)', () => {
  // A highly-compressible 200 KB member inflates well past a tiny cap.
  const targz = makeTarGz({ nxs: Buffer.alloc(200_000) });
  assert.throws(() => extractMember(targz, 'nxs', { maxOutputLength: 4096 }));
});
