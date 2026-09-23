'use strict';

const zlib = require('node:zlib');

const BLOCK = 512;

// Cap on the INFLATED tar size. Well above a real release (nxs + nxf-relay + docs, tens of MB) yet
// bounded, so a decompression bomb (a tiny gzip that inflates to gigabytes) throws instead of
// exhausting memory. This runs before verification only for the tarball path already byte-capped in
// download.js — defense in depth on the two DoS surfaces the review flagged.
const DEFAULT_MAX_OUTPUT = 512 * 1024 * 1024;

// Read the NUL-terminated name from a ustar header block (bytes 0..100). Release tarballs ship bare
// member names (`nxs`, `nxf-relay`, …), all well under 100 chars, so the `prefix` field is never
// needed here.
function headerName(block) {
  const raw = block.subarray(0, 100);
  const nul = raw.indexOf(0);
  return raw.toString('latin1', 0, nul === -1 ? 100 : nul);
}

// Parse the octal size field (bytes 124..136).
function headerSize(block) {
  const raw = block.subarray(124, 136).toString('latin1').replace(/[\0 ]+$/g, '').trim();
  return raw ? parseInt(raw, 8) : 0;
}

// extractMember(tarGzBuffer, name): gunzip and walk the tar, returning the exact bytes of the
// regular-file member matching `name`. Extracting a single member by EXACT name (not a wildcard)
// also defeats any `../`/absolute-path entry a tampered tarball might carry — defense in depth
// behind the sha256 + minisign gates. Throws if the member is absent.
function extractMember(tarGzBuffer, name, { maxOutputLength = DEFAULT_MAX_OUTPUT } = {}) {
  // `maxOutputLength` makes gunzip throw (RangeError) rather than inflate an unbounded bomb.
  const tar = zlib.gunzipSync(tarGzBuffer, { maxOutputLength });
  let pos = 0;
  while (pos + BLOCK <= tar.length) {
    const header = tar.subarray(pos, pos + BLOCK);
    const entryName = headerName(header);
    if (entryName === '') break; // end-of-archive marker (zero block)
    const size = headerSize(header);
    const typeflag = header[156];
    const dataStart = pos + BLOCK;
    // Regular file: typeflag '0' (0x30) or NUL (legacy).
    if (entryName === name && (typeflag === 0x30 || typeflag === 0)) {
      return Buffer.from(tar.subarray(dataStart, dataStart + size));
    }
    pos = dataStart + Math.ceil(size / BLOCK) * BLOCK;
  }
  throw new Error(`member '${name}' not found in the downloaded tarball`);
}

module.exports = { extractMember };
