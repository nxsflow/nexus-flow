'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { resolveCacheDir, binaryPath, readCached, install } = require('../lib/cache');

function tmp() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'nxs-cache-'));
}

test('resolveCacheDir honours the NXS_MCP_CACHE_DIR override', () => {
  assert.equal(
    resolveCacheDir({ NXS_MCP_CACHE_DIR: '/custom/cache' }, 'linux-x86_64', '/home/u'),
    '/custom/cache'
  );
});

test('resolveCacheDir uses ~/Library/Caches on darwin', () => {
  assert.equal(
    resolveCacheDir({}, 'darwin-aarch64', '/Users/u'),
    '/Users/u/Library/Caches/nexus-flow/mcp'
  );
});

test('resolveCacheDir honours XDG_CACHE_HOME on linux, else ~/.cache', () => {
  assert.equal(
    resolveCacheDir({ XDG_CACHE_HOME: '/xdg' }, 'linux-x86_64', '/home/u'),
    '/xdg/nexus-flow/mcp'
  );
  assert.equal(resolveCacheDir({}, 'linux-x86_64', '/home/u'), '/home/u/.cache/nexus-flow/mcp');
});

test('binaryPath is channel- + version- + platform-scoped', () => {
  assert.equal(
    binaryPath('/c', 'stable', '1.2.3', 'linux-x86_64'),
    path.join('/c', 'nxs', 'stable', '1.2.3', 'linux-x86_64', 'nxs')
  );
});

test('readCached returns null before install and the path after', () => {
  const dir = tmp();
  assert.equal(readCached(dir, 'stable', '1.2.3', 'linux-x86_64'), null);
  const written = install(dir, 'stable', '1.2.3', 'linux-x86_64', Buffer.from('BIN'));
  assert.equal(readCached(dir, 'stable', '1.2.3', 'linux-x86_64'), written);
});

test('the same version cached under a different channel is a separate entry', () => {
  const dir = tmp();
  install(dir, 'beta', '1.2.3', 'linux-x86_64', Buffer.from('BETA'));
  // A stable lookup must NOT find the beta-cached binary of the same version.
  assert.equal(readCached(dir, 'stable', '1.2.3', 'linux-x86_64'), null);
});

test('install writes the exact bytes as an executable file', () => {
  const dir = tmp();
  const bytes = Buffer.from('#!/executable\x00nxs', 'latin1');
  const p = install(dir, 'stable', '9.9.9', 'darwin-aarch64', bytes);
  assert.deepEqual(fs.readFileSync(p), bytes);
  // owner-executable bit set (0o100 in the mode).
  assert.equal(fs.statSync(p).mode & 0o100, 0o100);
});

test('install is idempotent (re-install overwrites cleanly, no temp files linger)', () => {
  const dir = tmp();
  install(dir, 'stable', '1.0.0', 'linux-aarch64', Buffer.from('v1'));
  const p = install(dir, 'stable', '1.0.0', 'linux-aarch64', Buffer.from('v2'));
  assert.deepEqual(fs.readFileSync(p), Buffer.from('v2'));
  const siblings = fs.readdirSync(path.dirname(p));
  assert.deepEqual(siblings, ['nxs'], 'only the final binary remains, no .tmp siblings');
});
