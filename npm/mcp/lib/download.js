'use strict';

const http = require('node:http');
const https = require('node:https');
const { sameOrigin } = require('./resolve');

function client(url) {
  return url.startsWith('https:') ? https : http;
}

// Idle-socket timeout for a single GET: fires when no bytes arrive for this long (it RESETS on
// activity, so a large-but-flowing download never trips it — only a stall / hung TLS does). Bounds
// the slow-loris case where a CDN accepts then goes silent.
const DEFAULT_TIMEOUT_MS = 30_000;

// Hard cap on a single response body. The largest legitimate artifact is one tarball (nxs +
// nxf-relay + docs, tens of MB); this ceiling is comfortably above that but stops a malicious /
// compromised origin from OOM-ing startup by streaming unbounded bytes BEFORE verification.
const DEFAULT_MAX_BYTES = 256 * 1024 * 1024;

// One GET, no redirect following, resolving to { statusCode, location, body }. Rejects on a
// transport/TLS error, on an idle timeout, or when the body exceeds `maxBytes` — every real HTTP
// status is a resolved value the caller decides on.
function request(url, { timeoutMs = DEFAULT_TIMEOUT_MS, maxBytes = DEFAULT_MAX_BYTES } = {}) {
  return new Promise((resolve, reject) => {
    const req = client(url).get(url, (res) => {
      const chunks = [];
      let total = 0;
      res.on('data', (c) => {
        total += c.length;
        if (total > maxBytes) {
          // Refuse before buffering more: this runs pre-verification, so an unbounded body must
          // never be allowed to exhaust memory.
          req.destroy();
          reject(new Error(`response from ${url} is too large (exceeds ${maxBytes} bytes) — refusing`));
          return;
        }
        chunks.push(c);
      });
      res.on('end', () =>
        resolve({
          statusCode: res.statusCode,
          location: res.headers.location,
          body: Buffer.concat(chunks),
        })
      );
    });
    // Idle timeout on the underlying socket (covers connect-then-stall and never-respond alike).
    req.setTimeout(timeoutMs, () => {
      req.destroy();
      reject(new Error(`request to ${url} timed out after ${timeoutMs}ms`));
    });
    req.on('error', reject);
  });
}

// resolveRedirect(url): return the Location of a single 3xx WITHOUT following it (so the caller can
// same-origin-check the server-controlled target before fetching). null on a genuine 404 (nothing
// published); throw on any other status, an idle timeout, or a transport error — a dropped/blocked
// request must NOT read as "not found" and silently downgrade.
async function resolveRedirect(url, opts = {}) {
  const { statusCode, location } = await request(url, opts);
  if (statusCode >= 300 && statusCode < 400 && location) return location;
  if (statusCode === 404) return null;
  throw new Error(`unexpected HTTP ${statusCode} resolving ${url}`);
}

// download(url): GET the bytes, following redirects (bounded), rejecting on a non-2xx final status,
// an idle timeout, or an oversized body. Redirect hops are same-origin-guarded against the ORIGINAL
// url (rejecting a cross-host jump or an https→http downgrade): minisign is the real integrity
// backstop, but there is no reason to follow a server-directed fetch off our own origin.
async function download(url, { maxRedirects = 5, timeoutMs, maxBytes } = {}) {
  const reqOpts = {};
  if (timeoutMs !== undefined) reqOpts.timeoutMs = timeoutMs;
  if (maxBytes !== undefined) reqOpts.maxBytes = maxBytes;
  let current = url;
  for (let i = 0; i <= maxRedirects; i++) {
    const { statusCode, location, body } = await request(current, reqOpts);
    if (statusCode >= 200 && statusCode < 300) return body;
    if (statusCode >= 300 && statusCode < 400 && location) {
      const next = new URL(location, current).toString();
      if (!sameOrigin(next, url)) {
        throw new Error(`refusing cross-origin/downgraded redirect from ${url} to ${next}`);
      }
      current = next;
      continue;
    }
    throw new Error(`download failed: HTTP ${statusCode} for ${current}`);
  }
  throw new Error(`download failed: too many redirects for ${url}`);
}

module.exports = { resolveRedirect, download, DEFAULT_TIMEOUT_MS, DEFAULT_MAX_BYTES };
