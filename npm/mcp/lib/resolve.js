'use strict';

// Pure URL construction for the artifact CDN. The network round-trips (fetch, follow the /latest
// 302) live in download.js/index.js; this module stays side-effect-free and unit-testable.

function stripSlash(base) {
  return base.replace(/\/+$/, '');
}

// `nxf_<version>_<platform>.tar.gz` — the release packaging name (release.yml).
function tarballName(version, platform) {
  return `nxf_${version}_${platform}.tar.gz`;
}

// The immutable per-version artifact path (used when NXF_VERSION pins a version).
function pinnedUrl(baseUrl, channel, version, platform) {
  return `${stripSlash(baseUrl)}/download/${channel}/${version}/${tarballName(version, platform)}`;
}

// /latest?channel=&target=&arch= → 302 to the newest channel tarball. `platform` is the "<os>-<arch>"
// key; target/arch are its two halves (os and arch each carry no '-').
function latestUrl(baseUrl, channel, platform) {
  const [os, arch] = platform.split('-');
  return `${stripSlash(baseUrl)}/latest?channel=${channel}&target=${os}&arch=${arch}`;
}

// The `.sha256` / `.minisig` sidecars sit next to the resolved tarball.
function sidecarUrls(tarUrl) {
  return { sha256: `${tarUrl}.sha256`, minisig: `${tarUrl}.minisig` };
}

// sameOrigin(url, base): true iff `url` has the same scheme://authority as `base`. Guards the
// server-controlled /latest redirect target against an open-redirect off our origin or an http
// downgrade before we fetch it.
function sameOrigin(url, base) {
  try {
    const a = new URL(url);
    const b = new URL(base);
    return a.protocol === b.protocol && a.host === b.host;
  } catch {
    return false;
  }
}

module.exports = { tarballName, pinnedUrl, latestUrl, sidecarUrls, sameOrigin };
