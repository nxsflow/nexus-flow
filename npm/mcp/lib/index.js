'use strict';

const fs = require('node:fs');
const path = require('node:path');

const { detectPlatform } = require('./platform');
const { resolveCacheDir, binaryPath, readCached, install } = require('./cache');
const { pinnedUrl, latestUrl, sidecarUrls, sameOrigin } = require('./resolve');
const { resolveRedirect, download } = require('./download');
const { verifySha256, verifyMinisign } = require('./verify');
const { extractMember } = require('./extract');
const { EMBEDDED_MINISIGN_PUBKEY, DEFAULT_BASE_URL, DEFAULT_CHANNEL } = require('./constants');

// Plain-SemVer shape a version must match (matches release/version's `X.Y.Z`, no suffixes).
const VERSION_RE = /^\d+\.\d+\.\d+$/;

// A security refusal that must NEVER be papered over by the offline cache fallback (an off-origin
// /latest redirect). A typed sentinel, so the catch below matches on the type — not on error text.
class OriginRefusedError extends Error {}

// Parse the version out of `nxf_<version>_<platform>.tar.gz`.
function versionFromTarballName(name, platform) {
  const m = new RegExp(`^nxf_(.+)_${platform.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.tar\\.gz$`).exec(name);
  if (!m) throw new Error(`unexpected tarball name '${name}' (cannot read version)`);
  return m[1];
}

// Compare two plain-SemVer strings (X.Y.Z) descending.
function semverDesc(a, b) {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    if ((pb[i] || 0) !== (pa[i] || 0)) return (pb[i] || 0) - (pa[i] || 0);
  }
  return 0;
}

// The newest already-cached, already-verified binary for this channel + platform, or null. Used as
// an offline fallback when the CDN is unreachable. Scoped to `channel` so a `stable` run never falls
// back to a `beta` binary cached earlier.
function newestCached(cacheDir, channel, platform) {
  const root = path.join(cacheDir, 'nxs', channel);
  let versions;
  try {
    versions = fs.readdirSync(root);
  } catch {
    return null;
  }
  const usable = versions
    .filter((v) => VERSION_RE.test(v) && fs.existsSync(binaryPath(cacheDir, channel, v, platform)))
    .sort(semverDesc);
  return usable.length
    ? { version: usable[0], path: binaryPath(cacheDir, channel, usable[0], platform) }
    : null;
}

// ensureNxs(opts): resolve → download → VERIFY (sha256 + minisign, fail-closed) → extract → cache
// the signed `nxs` binary, returning its path. All logging is on the caller-supplied `log` (the CLI
// routes it to stderr — stdout is the MCP channel). Options default to production values; tests
// inject a fixture base URL, platform, pubkey, and cache dir.
async function ensureNxs(opts = {}) {
  const env = opts.env || process.env;
  const log = opts.log || ((m) => process.stderr.write(`${m}\n`));
  const baseUrl = opts.baseUrl || env.NXF_BASE_URL || DEFAULT_BASE_URL;
  const channel = opts.channel || env.NXF_CHANNEL || DEFAULT_CHANNEL;
  const platform = opts.platform || detectPlatform(env);
  const pubkey = opts.pubkey || EMBEDDED_MINISIGN_PUBKEY;
  const cacheDir = opts.cacheDir || resolveCacheDir(env, platform);
  const pinnedVersion = opts.version || env.NXF_VERSION || null;

  // Validate a pinned version up front: it is interpolated into the cache path and the download URL,
  // so a malformed value (e.g. `../../..`) must be rejected outright rather than traversing the cache
  // dir. Same plain-SemVer shape enforced for a version discovered from `/latest`.
  if (pinnedVersion && !VERSION_RE.test(pinnedVersion)) {
    throw new Error(`invalid pinned version '${pinnedVersion}' (expected plain SemVer X.Y.Z)`);
  }

  // A pinned + cached version needs no network at all.
  if (pinnedVersion) {
    const hit = readCached(cacheDir, channel, pinnedVersion, platform);
    if (hit) {
      log(`nexus-flow: using cached nxs ${pinnedVersion} (${channel}, ${platform})`);
      return hit;
    }
  }

  // Resolve the tarball URL + concrete version. On any resolution failure, fall back to the newest
  // cached binary (offline resilience) before giving up.
  let tarUrl;
  let version;
  try {
    if (pinnedVersion) {
      version = pinnedVersion;
      tarUrl = pinnedUrl(baseUrl, channel, version, platform);
    } else {
      const resolved = await resolveRedirect(latestUrl(baseUrl, channel, platform));
      if (!resolved) throw new Error(`no release published for ${platform} on channel '${channel}'`);
      if (!sameOrigin(resolved, baseUrl)) {
        throw new OriginRefusedError(`refusing /latest redirect to a different origin: ${resolved}`);
      }
      tarUrl = resolved;
      version = versionFromTarballName(path.posix.basename(new URL(tarUrl).pathname), platform);
      const hit = readCached(cacheDir, channel, version, platform);
      if (hit) {
        log(`nexus-flow: using cached nxs ${version} (${channel}, ${platform})`);
        return hit;
      }
    }
  } catch (e) {
    // An off-origin redirect is a security refusal, never something to paper over with the cache.
    if (e instanceof OriginRefusedError) throw e;
    const fb = newestCached(cacheDir, channel, platform);
    if (fb) {
      log(
        `nexus-flow: warning: could not reach ${baseUrl} (${e.message}); using cached nxs ${fb.version} (${channel})`
      );
      return fb.path;
    }
    throw e;
  }

  // Download the tarball + both sidecars. A missing sidecar (download throws on non-2xx) aborts —
  // there is no downgrade to sha256-only and no unsigned path.
  log(`nexus-flow: downloading ${tarUrl}`);
  const { sha256: shaUrl, minisig: sigUrl } = sidecarUrls(tarUrl);
  const [tarball, shaText, sigText] = await Promise.all([
    download(tarUrl),
    download(shaUrl),
    download(sigUrl),
  ]);

  // Both gates are mandatory; either throws → nothing is extracted or cached.
  verifySha256(tarball, shaText.toString('utf8'));
  verifyMinisign(tarball, sigText.toString('utf8'), pubkey);
  log('nexus-flow: signature verified');

  const bin = extractMember(tarball, 'nxs');
  const p = install(cacheDir, channel, version, platform, bin);
  log(`nexus-flow: installed nxs ${version} → ${p}`);
  return p;
}

module.exports = { ensureNxs, versionFromTarballName, newestCached, OriginRefusedError };
