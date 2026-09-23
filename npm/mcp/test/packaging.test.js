'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

// One `npm pack` for the whole file: it spawns a subprocess, and both tests below ask about the
// same report.
let report;
function packReport() {
  if (!report) {
    const root = path.join(__dirname, '..');
    const out = execFileSync('npm', ['pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' });
    report = JSON.parse(out)[0];
  }
  return report;
}

// Guard the published surface: the tarball must ship only the runtime (bin + lib + README +
// package.json) and NEVER the tests or testkit (which generate keypairs and fixtures). A drift here
// would leak test material into the published package.
test('npm pack ships only the runtime, never tests or fixtures', () => {
  const files = packReport().files.map((f) => f.path).sort();
  assert.deepEqual(files, [
    'README.md',
    'bin/cli.js',
    'lib/cache.js',
    'lib/constants.js',
    'lib/download.js',
    'lib/extract.js',
    'lib/index.js',
    'lib/platform.js',
    'lib/resolve.js',
    'lib/verify.js',
    'package.json',
  ]);
  assert.ok(!files.some((f) => f.startsWith('test/') || f.startsWith('testkit/')), 'no test material');
});

// `release.yml`'s staging rehearsal stamps the release version into package.json, then reads it
// back out of this same report (`jq -r '.[0].version'`) to prove the stamp landed in what would
// ship — and asserts the name the same way (6j6v.xk2t). That check runs only on
// `workflow_dispatch` / tag push, so a broken field path there would stay invisible until someone
// re-dispatched a rehearsal by hand. This pins the shape it reads, on every PR, exactly as the
// test above already pins `[0].files[].path`.
test('npm pack reports name and version where release.yml reads them', () => {
  const { name, version } = packReport();
  assert.equal(name, '@nexus-flow/mcp');
  assert.equal(version, require('../package.json').version);
});
