'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

// resolveCacheDir(env, platform, homedir): the per-user cache root for downloaded binaries. The
// NXS_MCP_CACHE_DIR override wins (testing / locked-down homes); otherwise the OS convention — macOS
// ~/Library/Caches, Linux $XDG_CACHE_HOME or ~/.cache.
function resolveCacheDir(env = process.env, platform = '', homedir = os.homedir()) {
  if (env.NXS_MCP_CACHE_DIR) return env.NXS_MCP_CACHE_DIR;
  const os_ = platform.split('-')[0];
  if (os_ === 'darwin') return path.join(homedir, 'Library', 'Caches', 'nexus-flow', 'mcp');
  const xdg = env.XDG_CACHE_HOME || path.join(homedir, '.cache');
  return path.join(xdg, 'nexus-flow', 'mcp');
}

// The immutable, channel- + version- + platform-scoped path a verified `nxs` lands at. Versions are
// immutable, so a binary present here was already sha256+minisign-verified before it was moved into
// place. The `channel` segment keeps the offline fallback from ever serving a binary from a
// different promotion ring than the one requested (e.g. a cached `beta` for a `stable` run).
function binaryPath(cacheDir, channel, version, platform) {
  return path.join(cacheDir, 'nxs', channel, version, platform, 'nxs');
}

// readCached: the binary path if a verified copy is already cached, else null.
function readCached(cacheDir, channel, version, platform) {
  const p = binaryPath(cacheDir, channel, version, platform);
  return fs.existsSync(p) ? p : null;
}

// install: write verified bytes to a temp sibling, chmod 0755, then atomically rename into the
// cache path — a concurrent reader never sees a half-written binary. Returns the final path.
// Caller MUST have verified the bytes first (only verified bytes are ever passed here).
function install(cacheDir, channel, version, platform, bytes) {
  const dest = binaryPath(cacheDir, channel, version, platform);
  const dir = path.dirname(dest);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = path.join(dir, `.nxs.tmp.${process.pid}`);
  try {
    fs.writeFileSync(tmp, bytes, { mode: 0o755 });
    fs.chmodSync(tmp, 0o755); // writeFileSync mode is subject to umask; force it
    fs.renameSync(tmp, dest);
  } catch (e) {
    try {
      fs.rmSync(tmp, { force: true });
    } catch {
      /* best-effort cleanup */
    }
    throw e;
  }
  return dest;
}

module.exports = { resolveCacheDir, binaryPath, readCached, install };
