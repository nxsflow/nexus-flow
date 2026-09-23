'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { tarballName, pinnedUrl, latestUrl, sidecarUrls, sameOrigin } = require('../lib/resolve');

// Names + paths must match what release.yml/publish-release.sh produce and install.sh consumes:
// `nxf_<version>_<platform>.tar.gz` under `download/<channel>/<version>/`.
test('tarballName follows the release naming', () => {
  assert.equal(tarballName('1.2.3', 'linux-x86_64'), 'nxf_1.2.3_linux-x86_64.tar.gz');
});

test('pinnedUrl addresses the immutable per-version path', () => {
  assert.equal(
    pinnedUrl('https://nxsflow.com/nxs', 'stable', '1.2.3', 'darwin-aarch64'),
    'https://nxsflow.com/nxs/download/stable/1.2.3/nxf_1.2.3_darwin-aarch64.tar.gz'
  );
});

test('pinnedUrl tolerates a trailing slash on the base', () => {
  assert.equal(
    pinnedUrl('https://nxsflow.com/nxs/', 'beta', '0.9.0', 'linux-aarch64'),
    'https://nxsflow.com/nxs/download/beta/0.9.0/nxf_0.9.0_linux-aarch64.tar.gz'
  );
});

test('latestUrl queries /latest with channel/target/arch (from a platform key)', () => {
  assert.equal(
    latestUrl('https://nxsflow.com/nxs', 'stable', 'darwin-aarch64'),
    'https://nxsflow.com/nxs/latest?channel=stable&target=darwin&arch=aarch64'
  );
});

test('sidecarUrls sit next to the tarball', () => {
  const url = 'https://nxsflow.com/nxs/download/stable/1.2.3/nxf_1.2.3_linux-x86_64.tar.gz';
  assert.deepEqual(sidecarUrls(url), { sha256: `${url}.sha256`, minisig: `${url}.minisig` });
});

// The /latest redirect target is server-controlled — bar an open-redirect off our own origin or a
// scheme downgrade before we fetch it (install.sh's same_origin guard).
test('sameOrigin accepts the same scheme://authority', () => {
  assert.equal(
    sameOrigin('https://nxsflow.com/nxs/download/stable/1.2.3/x.tar.gz', 'https://nxsflow.com/nxs'),
    true
  );
});

test('sameOrigin rejects a different host', () => {
  assert.equal(sameOrigin('https://evil.example/x.tar.gz', 'https://nxsflow.com/nxs'), false);
});

test('sameOrigin rejects an http downgrade', () => {
  assert.equal(sameOrigin('http://nxsflow.com/nxs/x.tar.gz', 'https://nxsflow.com/nxs'), false);
});

// Discriminator: sameOrigin is scheme+host+port, NOT string-prefix. The accepted URL's path
// (/elsewhere) does NOT start with the base's path (/nxs), so a naive `url.startsWith(base)` guard
// would wrongly reject it — this fixture fails against a prefix matcher and passes against real
// origin-stripping. Matters now the base carries a /nxs path (cutover) and nxsflow.com is shared.
test('sameOrigin accepts the same origin with a different top-level path', () => {
  assert.equal(sameOrigin('https://nxsflow.com/elsewhere/x.tar.gz', 'https://nxsflow.com/nxs'), true);
});

test('sameOrigin rejects a host that merely suffixes ours', () => {
  assert.equal(sameOrigin('https://nxsflow.com.evil.example/nxs/x.tar.gz', 'https://nxsflow.com/nxs'), false);
});
