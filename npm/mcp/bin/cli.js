#!/usr/bin/env node
'use strict';

// @nexus-flow/mcp — the runner shim for hosts where `nxs` is not installed. A host MCP config
// launches `npx -y @nexus-flow/mcp -- <args>`; this fetches the signed `nxs` prebuilt for the
// platform, VERIFIES it (sha256 + minisign, fail-closed), caches it, and execs `nxs mcp serve`
// with the host args passed through. stdout is the MCP stdio channel — every diagnostic goes to
// stderr so it never corrupts the protocol stream.

const { spawn } = require('node:child_process');
const { ensureNxs } = require('../lib/index');

async function main() {
  const passthrough = process.argv.slice(2); // e.g. ["--workspace", "/proj"]
  // npx forwards the `--` option terminator into the child's argv, so the DOCUMENTED host form
  // `npx -y @nexus-flow/mcp -- --workspace <p>` reaches this shim as ["--", "--workspace", <p>].
  // Drop a single leading `--` so nxs receives `mcp serve --workspace <p>` (clap flags) — not
  // `mcp serve -- --workspace <p>`, where clap treats everything after `--` as a positional and
  // aborts with "unexpected argument '--workspace'". `nxs mcp serve` takes only flags, so a leading
  // `--` is never meaningful to pass on.
  if (passthrough[0] === '--') passthrough.shift();

  let nxs;
  try {
    nxs = await ensureNxs({ env: process.env });
  } catch (e) {
    process.stderr.write(`nexus-flow: could not obtain a verified nxs binary: ${e.message}\n`);
    process.exit(1);
  }

  // Hand off to the real server. `stdio: 'inherit'` wires the host's stdin/stdout/stderr straight
  // through to nxs (MCP speaks over stdio). Async spawn (not spawnSync) keeps the event loop free so
  // a SIGINT/SIGTERM sent to THIS process (a host that signals the node PID directly, not just via
  // stdin-EOF) is forwarded to nxs instead of leaving it orphaned. Propagate the child's exit code,
  // or re-raise the terminating signal so callers observe the real cause.
  const child = spawn(nxs, ['mcp', 'serve', ...passthrough], { stdio: 'inherit' });

  const forward = (sig) => {
    if (!child.killed) child.kill(sig);
  };
  process.on('SIGINT', () => forward('SIGINT'));
  process.on('SIGTERM', () => forward('SIGTERM'));
  process.on('SIGHUP', () => forward('SIGHUP'));

  child.on('error', (e) => {
    process.stderr.write(`nexus-flow: failed to launch nxs: ${e.message}\n`);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      // Re-raise so the parent dies of the same signal (128+n exit), the conventional shell contract.
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });
}

main();
