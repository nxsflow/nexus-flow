'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { ensureNxs } = require('../lib/index');
const { signedRelease } = require('../testkit/fixtures');

const PLATFORM = 'linux-x86_64';

function tmpCache() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'nxs-idx-'));
}

// A fixture CDN: /latest 302 → the versioned tarball path; the tarball + its .sha256/.minisig.
// Options let a test tamper the bytes, drop the signature, or redirect off-origin.
async function startCdn(release, { tamper = false, dropSig = false, offOrigin = false } = {}) {
  let hits = 0;
  const server = http.createServer((req, res) => {
    hits++;
    const url = new URL(req.url, 'http://x');
    const base = `http://127.0.0.1:${server.address().port}`;
    const tarPath = `/download/stable/${release.version}/${release.name}`;
    if (url.pathname === '/latest') {
      const loc = offOrigin ? 'https://evil.example' + tarPath : base + tarPath;
      res.writeHead(302, { Location: loc });
      return res.end();
    }
    if (url.pathname === tarPath) {
      const body = tamper ? Buffer.concat([release.tarball.subarray(0, 1), Buffer.from([0xff]), release.tarball.subarray(2)]) : release.tarball;
      res.writeHead(200);
      return res.end(body);
    }
    if (url.pathname === tarPath + '.sha256') {
      res.writeHead(200);
      return res.end(release.sha256);
    }
    if (url.pathname === tarPath + '.minisig') {
      if (dropSig) {
        res.writeHead(404);
        return res.end();
      }
      res.writeHead(200);
      return res.end(release.minisig);
    }
    res.writeHead(404);
    res.end();
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  return { base, close: () => server.close(), hits: () => hits };
}

function opts(cdn, release, cacheDir, extra = {}) {
  return {
    env: {},
    baseUrl: cdn.base,
    channel: 'stable',
    platform: PLATFORM,
    pubkey: release.pubB64,
    cacheDir,
    log: () => {},
    ...extra,
  };
}

test('ensureNxs downloads, verifies, extracts and caches the nxs binary', async () => {
  const release = signedRelease({ version: '1.2.3', platform: PLATFORM, nxsContent: Buffer.from('REAL NXS BINARY') });
  const cdn = await startCdn(release);
  const cacheDir = tmpCache();
  try {
    const p = await ensureNxs(opts(cdn, release, cacheDir));
    assert.deepEqual(fs.readFileSync(p), Buffer.from('REAL NXS BINARY'));
    assert.equal(fs.statSync(p).mode & 0o100, 0o100, 'cached binary is executable');
  } finally {
    cdn.close();
  }
});

test('the full pipeline yields an executable nxs that runs `mcp serve` with the args', async () => {
  // Positive end-to-end across the composition seam: real download → real sha256+minisign verify →
  // extract → cache → EXEC. The cached member is an executable stub; running it exactly as the CLI
  // does proves the whole chain produces a launchable binary, with real crypto (no mocks).
  const stub = '#!/bin/sh\necho "MCP-EXEC argv: $*"\n';
  const release = signedRelease({ version: '1.4.0', platform: PLATFORM, nxsContent: Buffer.from(stub) });
  const cdn = await startCdn(release);
  const cacheDir = tmpCache();
  try {
    const nxs = await ensureNxs(opts(cdn, release, cacheDir));
    const run = spawnSync(nxs, ['mcp', 'serve', '--workspace', '/x'], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /MCP-EXEC argv: mcp serve --workspace \/x/);
  } finally {
    cdn.close();
  }
});

test('a warm cache (pinned version) is used with no network', async () => {
  const release = signedRelease({ version: '2.0.0', platform: PLATFORM, nxsContent: Buffer.from('BIN2') });
  const cdn = await startCdn(release);
  const cacheDir = tmpCache();
  await ensureNxs(opts(cdn, release, cacheDir)); // prime the cache via /latest
  cdn.close();
  // Pin the version and point at a dead base URL: a cache hit must not touch the network.
  const p = await ensureNxs(opts({ base: 'http://127.0.0.1:1' }, release, cacheDir, { version: '2.0.0' }));
  assert.deepEqual(fs.readFileSync(p), Buffer.from('BIN2'));
});

test('tampered bytes abort before anything is cached (fail-closed)', async () => {
  const release = signedRelease({ version: '3.1.0', platform: PLATFORM, nxsContent: Buffer.from('BIN3') });
  const cdn = await startCdn(release, { tamper: true });
  const cacheDir = tmpCache();
  try {
    await assert.rejects(() => ensureNxs(opts(cdn, release, cacheDir)), /sha256|verification/i);
    assert.equal(
      fs.existsSync(path.join(cacheDir, 'nxs', 'stable', '3.1.0', PLATFORM, 'nxs')),
      false,
      'nothing cached'
    );
  } finally {
    cdn.close();
  }
});

test('a missing signature aborts (no downgrade to sha256-only)', async () => {
  const release = signedRelease({ version: '3.2.0', platform: PLATFORM, nxsContent: Buffer.from('BIN') });
  const cdn = await startCdn(release, { dropSig: true });
  const cacheDir = tmpCache();
  try {
    await assert.rejects(() => ensureNxs(opts(cdn, release, cacheDir)), /404|signature|download failed/i);
  } finally {
    cdn.close();
  }
});

test('an off-origin /latest redirect is refused', async () => {
  const release = signedRelease({ version: '3.3.0', platform: PLATFORM, nxsContent: Buffer.from('BIN') });
  const cdn = await startCdn(release, { offOrigin: true });
  const cacheDir = tmpCache();
  try {
    await assert.rejects(() => ensureNxs(opts(cdn, release, cacheDir)), /origin/i);
  } finally {
    cdn.close();
  }
});

test('a malformed pinned version is rejected before any cache-path use (traversal guard)', async () => {
  const cacheDir = tmpCache();
  await assert.rejects(
    () => ensureNxs(opts({ base: 'http://127.0.0.1:1' }, { pubB64: 'x' }, cacheDir, { version: '../../etc' })),
    /invalid pinned version/i
  );
});

test('offline with a warm cache falls back to the newest cached binary', async () => {
  const release = signedRelease({ version: '4.5.6', platform: PLATFORM, nxsContent: Buffer.from('CACHED') });
  const cdn = await startCdn(release);
  const cacheDir = tmpCache();
  await ensureNxs(opts(cdn, release, cacheDir)); // prime
  cdn.close();
  // /latest now unreachable; no pinned version → fall back to newest cached.
  const warnings = [];
  const p = await ensureNxs(opts({ base: 'http://127.0.0.1:1' }, release, cacheDir, { log: (m) => warnings.push(m) }));
  assert.deepEqual(fs.readFileSync(p), Buffer.from('CACHED'));
  assert.match(warnings.join('\n'), /cache/i);
});
