'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { EMBEDDED_MINISIGN_PUBKEY } = require('../lib/constants');

// The shim's embedded minisign pubkey IS its authenticity anchor. It must stay byte-identical to
// install.sh's EMBEDDED_MINISIGN_PUBKEY (and, in CI, to vars.NXF_MINISIGN_PUBKEY — checked in
// npm-mcp-ci.yml). This local guard catches drift between the two embedded copies with no network
// or CI variable — a rotation that misses one copy fails here.
test('embedded pubkey matches install.sh', () => {
  const installSh = fs.readFileSync(path.join(__dirname, '..', '..', '..', 'install.sh'), 'utf8');
  const m = /^EMBEDDED_MINISIGN_PUBKEY="(.*)"$/m.exec(installSh);
  assert.ok(m, 'could not read EMBEDDED_MINISIGN_PUBKEY from install.sh');
  assert.equal(
    EMBEDDED_MINISIGN_PUBKEY,
    m[1],
    'shim pubkey must equal install.sh EMBEDDED_MINISIGN_PUBKEY (rotation/typo?)'
  );
});
