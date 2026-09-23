#!/usr/bin/env node
// Validate an assembled content.json against the landing-published content-contract JSON schema,
// FAIL-CLOSED AT THE SOURCE (§4): a contract break stops in this repo's CI, not at the consumer.
//
// The authoritative schema is the one the landing publishes (until the apex go-live it is served
// from the prod distribution; after, https://nxsflow.com/schema/content-contract-v1.json). CI
// passes its URL via --schema (or CONTENT_CONTRACT_SCHEMA_URL) so a landing-side contract change
// turns THIS repo's CI red. A local path also works for offline/dev; the vendored copy under
// schema/ is a convenience mirror that the test-suite validates against.
//
// Usage:
//   node validate.mjs [--content dist/content.json] [--schema <url|path>]
//   CONTENT_CONTRACT_SCHEMA_URL=<url> node validate.mjs

import fs from "node:fs";
import path from "node:path";
import url from "node:url";
import { validateContent, formatErrors } from "./lib/validate.mjs";

const here = path.dirname(url.fileURLToPath(import.meta.url));

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i !== -1 && i + 1 < process.argv.length ? process.argv[i + 1] : fallback;
}

const contentPath = path.resolve(arg("--content", path.join(here, "dist", "content.json")));
const schemaRef =
  arg("--schema", process.env.CONTENT_CONTRACT_SCHEMA_URL) ||
  path.join(here, "schema", "content-contract-v1.json");

async function loadSchema(ref) {
  if (/^https?:\/\//.test(ref)) {
    const res = await fetch(ref);
    if (!res.ok) throw new Error(`fetch ${ref} → HTTP ${res.status}`);
    return { schema: await res.json(), from: ref };
  }
  const p = path.resolve(ref);
  if (!fs.existsSync(p)) throw new Error(`schema not found: ${p}`);
  return { schema: JSON.parse(fs.readFileSync(p, "utf8")), from: p };
}

async function main() {
  if (!fs.existsSync(contentPath)) {
    console.error(`[validate] content.json not found: ${contentPath} — run \`node assemble.mjs\` first`);
    process.exit(1);
  }
  const manifest = JSON.parse(fs.readFileSync(contentPath, "utf8"));

  let loaded;
  try {
    loaded = await loadSchema(schemaRef);
  } catch (e) {
    // Fail-closed: if we cannot obtain the authoritative schema, we do NOT publish.
    console.error(`[validate] could not load the content-contract schema (${e.message})`);
    process.exit(1);
  }

  const { ok, errors } = validateContent(manifest, loaded.schema);
  if (!ok) {
    console.error(`[validate] content.json does NOT satisfy the content-contract schema (${loaded.from}):`);
    console.error(formatErrors(errors));
    process.exit(1);
  }
  console.log(`[validate] content.json satisfies the content-contract schema (${loaded.from})`);
}

main().catch((e) => {
  console.error(`[validate] unexpected error: ${e.stack || e}`);
  process.exit(1);
});
