'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { platformFor, detectPlatform } = require('../lib/platform');

// Mirrors install.sh's `platform_for`: map (uname -s, uname -m) → the canonical "<os>-<arch>"
// tarball/manifest key, forgiving on synonyms (arm64≡aarch64, amd64≡x86_64). The four shipped
// keys are the single source of truth in release/platforms.
test('maps the four shipped platforms', () => {
  assert.equal(platformFor('Darwin', 'arm64'), 'darwin-aarch64');
  assert.equal(platformFor('Darwin', 'x86_64'), 'darwin-x86_64');
  assert.equal(platformFor('Linux', 'x86_64'), 'linux-x86_64');
  assert.equal(platformFor('Linux', 'aarch64'), 'linux-aarch64');
});

test('accepts arch synonyms (amd64, aarch64/arm64)', () => {
  assert.equal(platformFor('Linux', 'amd64'), 'linux-x86_64');
  assert.equal(platformFor('Darwin', 'aarch64'), 'darwin-aarch64');
  assert.equal(platformFor('Linux', 'arm64'), 'linux-aarch64');
});

test('returns null for an unsupported os or arch', () => {
  assert.equal(platformFor('Windows_NT', 'x86_64'), null);
  assert.equal(platformFor('Linux', 'riscv64'), null);
  assert.equal(platformFor('', ''), null);
});

// detectPlatform honours the NXF_PLATFORM override (escape hatch / testing), else derives from
// the process os/arch, else throws a clear message rather than 404ing three steps later.
test('detectPlatform honours the NXF_PLATFORM override verbatim', () => {
  assert.equal(detectPlatform({ NXF_PLATFORM: 'linux-x86_64' }), 'linux-x86_64');
});

test('detectPlatform derives from node os/arch tokens', () => {
  // node process.platform/arch tokens: 'darwin'/'linux' and 'arm64'/'x64'.
  assert.equal(detectPlatform({}, 'darwin', 'arm64'), 'darwin-aarch64');
  assert.equal(detectPlatform({}, 'linux', 'x64'), 'linux-x86_64');
});

test('detectPlatform throws on an unsupported host', () => {
  assert.throws(() => detectPlatform({}, 'win32', 'x64'), /unsupported platform/i);
});
