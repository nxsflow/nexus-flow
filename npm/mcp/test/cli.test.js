'use strict';
const { test, afterEach } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { install } = require('../lib/cache');
const { signedRelease } = require('../testkit/fixtures');

const CLI = path.join(__dirname, '..', 'bin', 'cli.js');
const PLATFORM = 'linux-x86_64'; // cache-path label only; the stub is a portable /bin/sh script

function tmpCache() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'nxs-cli-'));
}

// Every spawned child — the shim, the nxs it execs, and any grandchild the stub forks — must be
// reapable as ONE unit. A descendant that outlives the normal exit path keeps the inherited
// stdout/stderr pipe (and thus the runner's event loop) alive, so `node --test` never exits: on CI
// this once hung `publish-npm` at the 6h job ceiling and left orphan processes behind (jnx2). We
// therefore spawn DETACHED (the child leads its own process group) and always reap the whole group.
const SPAWN_TIMEOUT_MS = 30_000; // generous vs the ~1s real runs; bounds a stuck child to seconds, not 6h

// Children whose process group may still hold a live member — i.e. those that have NOT yet fired
// 'close'. A child is added on spawn and removed only in its 'close' handler, so membership is
// exactly "the pipe is still open". Assumes sequential test execution (no test in this file opts into
// `{ concurrency: true }`); a concurrent sibling would otherwise share this Set and over-reap.
const live = new Set();

// Force-kill a child's WHOLE process group. Deliberately NOT gated on `child.exitCode`/`signalCode`:
// the immediate child (the shim) exits the instant its own child does (`bin/cli.js` calls
// `process.exit` on the nxs 'exit' event), but a *grandchild* can outlive it while still holding the
// inherited stdout/stderr pipe — and that lingering handle, not the immediate child, is what keeps
// `node --test`'s event loop alive. Gating on exitCode would skip the kill in exactly that case (the
// jnx2 hang mode). We gate only on 'close' NOT having fired yet (via `live` membership): until then a
// group member is alive, so the pgid is valid and safe to signal; once 'close' fires there is nothing
// left to reap (and skipping avoids signalling a since-recycled pgid).
function reap(child) {
  if (!live.has(child)) return; // 'close' already fired → group drained, nothing to kill
  try {
    process.kill(-child.pid, 'SIGKILL'); // negative pid → whole group: shim + nxs + any grandchild
  } catch {
    try {
      child.kill('SIGKILL'); // group signalling unavailable (non-POSIX) → best-effort single kill
    } catch {
      /* already gone */
    }
  }
}

// Backstop: no test may leave a live child behind (e.g. a detached grandchild still holding the pipe).
// `reap` re-checks `live` per child, so calling it here after an abort-path reap is a harmless no-op.
afterEach(() => {
  for (const child of [...live]) reap(child);
});

// Async spawn (NOT spawnSync): a fail-closed test serves the fixture CDN from THIS process, so the
// event loop must stay free to answer the child's requests while it runs. `onStdout` (optional) gets
// each chunk so a test can act (e.g. send a signal) once the child signals readiness. `abortSignal`
// (the per-test timeout, `t.signal`) force-reaps a stuck tree so the test fails fast in seconds
// instead of hanging the whole runner.
function runCli(args, env, onStdout, abortSignal) {
  return new Promise((resolve) => {
    const child = spawn('node', [CLI, ...args], {
      env: { PATH: process.env.PATH, ...env },
      detached: true, // own process group, so `reap` can kill the entire tree in one shot
    });
    live.add(child);
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => {
      stdout += d;
      if (onStdout) onStdout(String(d), child);
    });
    child.stderr.on('data', (d) => (stderr += d));
    const onAbort = () => reap(child);
    if (abortSignal) abortSignal.addEventListener('abort', onAbort, { once: true });
    child.on('close', (status, signal) => {
      if (abortSignal) abortSignal.removeEventListener('abort', onAbort);
      live.delete(child);
      resolve({ status, signal, stdout, stderr });
    });
  });
}

test('execs the cached nxs as `mcp serve` and passes host args through', { timeout: SPAWN_TIMEOUT_MS }, async (t) => {
  const cacheDir = tmpCache();
  const stub = '#!/bin/sh\necho "MCP-STUB argv: $*"\n';
  install(cacheDir, 'stable', '1.0.0', PLATFORM, Buffer.from(stub));
  const r = await runCli(['--workspace', '/proj', '--actor', 'bob'], {
    NXF_VERSION: '1.0.0',
    NXF_PLATFORM: PLATFORM,
    NXS_MCP_CACHE_DIR: cacheDir,
  }, null, t.signal);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /MCP-STUB argv: mcp serve --workspace \/proj --actor bob/);
});

test('strips a leading `--` that npx forwards, so `-- --workspace <p>` reaches nxs as flags', { timeout: SPAWN_TIMEOUT_MS }, async (t) => {
  // Regression (live cold-start, nexus-flow-76u.15): the documented host form is
  // `npx -y @nexus-flow/mcp -- --workspace <p>`, and npx forwards the `--` separator into this
  // shim's argv. Without dropping it, nxs got `mcp serve -- --workspace <p>` and clap rejected
  // `--workspace` as an unexpected positional (everything after `--` is positional).
  const cacheDir = tmpCache();
  const stub = '#!/bin/sh\necho "MCP-STUB argv: $*"\n';
  install(cacheDir, 'stable', '1.0.0', PLATFORM, Buffer.from(stub));
  const r = await runCli(['--', '--workspace', '/proj'], {
    NXF_VERSION: '1.0.0',
    NXF_PLATFORM: PLATFORM,
    NXS_MCP_CACHE_DIR: cacheDir,
  }, null, t.signal);
  assert.equal(r.status, 0, r.stderr);
  // The leading `--` is gone: nxs sees `mcp serve --workspace /proj`, not `mcp serve -- --workspace`.
  assert.match(r.stdout, /MCP-STUB argv: mcp serve --workspace \/proj\s*$/m);
  assert.doesNotMatch(r.stdout, /mcp serve -- /);
});

test('propagates the exit code of the exec\'d nxs', { timeout: SPAWN_TIMEOUT_MS }, async (t) => {
  const cacheDir = tmpCache();
  install(cacheDir, 'stable', '1.0.0', PLATFORM, Buffer.from('#!/bin/sh\nexit 42\n'));
  const r = await runCli([], { NXF_VERSION: '1.0.0', NXF_PLATFORM: PLATFORM, NXS_MCP_CACHE_DIR: cacheDir }, null, t.signal);
  assert.equal(r.status, 42);
});

test('forwards SIGTERM to the exec\'d nxs (no orphaned child)', { timeout: SPAWN_TIMEOUT_MS }, async (t) => {
  // A stub that traps SIGTERM, records it, and exits — if the signal never reached it, it would run
  // forever and the test would time out. Proves the shim forwards a signal sent to the node PID.
  const cacheDir = tmpCache();
  const stub = "#!/bin/sh\ntrap 'echo GOT-TERM; exit 7' TERM\necho READY\nwhile true; do sleep 0.05; done\n";
  install(cacheDir, 'stable', '1.0.0', PLATFORM, Buffer.from(stub));
  let sent = false;
  const r = await runCli(
    [],
    { NXF_VERSION: '1.0.0', NXF_PLATFORM: PLATFORM, NXS_MCP_CACHE_DIR: cacheDir },
    (chunk, child) => {
      if (!sent && chunk.includes('READY')) {
        sent = true;
        child.kill('SIGTERM'); // signal the node (shim) process; it must forward to nxs
      }
    },
    t.signal
  );
  assert.match(r.stdout, /GOT-TERM/, 'the exec\'d nxs received the forwarded SIGTERM');
  assert.equal(r.status, 7, 'the shim propagates the child exit code after clean signal handling');
});

// Poll until `pid` no longer exists (signal 0 throws ESRCH), bounded — a SIGKILL'd orphan is reaped by
// init a beat after the signal, so allow a short settle before asserting it's gone.
async function waitPidGone(pid, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      process.kill(pid, 0);
    } catch {
      return true; // ESRCH → gone
    }
    if (Date.now() >= deadline) return false;
    await new Promise((r) => setTimeout(r, 20));
  }
}

test('force-reaps a descendant that outlives the immediate child — the jnx2 hang mode (regression)', { timeout: SPAWN_TIMEOUT_MS }, async () => {
  // The exact scenario the fix exists for, which a plain `node --test` run does NOT otherwise
  // exercise (so a green suite alone is not evidence the hang is fixed): the immediate child
  // (shim → sh) exits 0 at once, but it backgrounds a long-lived `sleep` that inherits — and holds
  // open — the stdout pipe. 'close' therefore cannot fire (the runner's event loop stays alive)
  // until the WHOLE process group is reaped, even though the immediate child's exitCode is already
  // set. Gating the group-kill on exitCode (the original bug) would skip it here and hang to the
  // timeout; gating on 'close' reaps the descendant and lets the runner exit promptly.
  const cacheDir = tmpCache();
  const stub = '#!/bin/sh\nsleep 30 &\necho "GCHILD $!"\nexit 0\n';
  install(cacheDir, 'stable', '1.0.0', PLATFORM, Buffer.from(stub));
  const ac = new AbortController();
  let gchildPid = 0;
  await runCli(
    [],
    { NXF_VERSION: '1.0.0', NXF_PLATFORM: PLATFORM, NXS_MCP_CACHE_DIR: cacheDir },
    (chunk, child) => {
      const m = chunk.match(/GCHILD (\d+)/);
      if (!m || gchildPid) return;
      gchildPid = Number(m[1]);
      // Trigger the reap only AFTER the immediate child has exited, so `child.exitCode` is already
      // non-null when `reap` runs — that is the precise condition under which the original
      // exitCode-guard skipped the group-kill. Aborting on the echo alone races the exit and would
      // let the buggy guard pass by luck (exitCode still null). The descendant keeps the pipe open, so
      // 'close' stays pending until the group is actually reaped.
      const fire = () => ac.abort();
      if (child.exitCode !== null || child.signalCode !== null) fire();
      else child.once('exit', fire);
    },
    ac.signal
  );
  // Reaching here at all proves 'close' fired: with the exitCode-guard bug it never would (the
  // backgrounded `sleep` keeps the pipe open), and this test would hang to its 30s timeout and fail.
  assert.ok(gchildPid, 'the stub reported its backgrounded descendant pid');
  assert.ok(
    await waitPidGone(gchildPid),
    `the lingering descendant (pid ${gchildPid}) was force-reaped, not left orphaned`
  );
});

test('fail-closed end-to-end: a signature that does not verify against the EMBEDDED key aborts, nothing exec\'d, nothing cached', { timeout: SPAWN_TIMEOUT_MS }, async (t) => {
  // Signed with a throwaway test key → sha256 passes, minisign fails against the shim's real
  // embedded pubkey. Proves the authenticity gate holds through the actual CLI entry point.
  const release = signedRelease({ version: '5.0.0', platform: PLATFORM, nxsContent: Buffer.from('BIN') });
  const server = http.createServer((req, res) => {
    const tarPath = `/download/stable/${release.version}/${release.name}`;
    const p = new URL(req.url, 'http://x').pathname;
    if (p === '/latest') {
      res.writeHead(302, { Location: `http://127.0.0.1:${server.address().port}${tarPath}` });
      return res.end();
    }
    if (p === tarPath) { res.writeHead(200); return res.end(release.tarball); }
    if (p === tarPath + '.sha256') { res.writeHead(200); return res.end(release.sha256); }
    if (p === tarPath + '.minisig') { res.writeHead(200); return res.end(release.minisig); }
    res.writeHead(404); res.end();
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  server.unref(); // the fixture server alone must never keep the runner's event loop alive
  const cacheDir = tmpCache();
  try {
    const r = await runCli([], {
      NXF_BASE_URL: `http://127.0.0.1:${server.address().port}`,
      NXF_PLATFORM: PLATFORM,
      NXS_MCP_CACHE_DIR: cacheDir,
    }, null, t.signal);
    assert.notEqual(r.status, 0, 'must exit non-zero on a verification failure');
    assert.match(r.stderr, /verification FAILED|refusing/i);
    assert.equal(
      fs.existsSync(path.join(cacheDir, 'nxs', 'stable', '5.0.0', PLATFORM, 'nxs')),
      false,
      'nothing cached'
    );
  } finally {
    server.closeAllConnections?.(); // destroy any lingering keep-alive socket before closing (Node 18.2+)
    server.close();
  }
});
