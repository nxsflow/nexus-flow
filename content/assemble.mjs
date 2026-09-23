#!/usr/bin/env node
// Assemble the federated content into an output dir the publish workflow uploads to the manufakt
// content prefix (nxsflow.com migration §4, 6j6v.0a6p):
//
//   <out>/content.json                 → s3://<bucket>/content.json      (/nxs/content.json)
//   <out>/docs.json                    → s3://<bucket>/docs.json         (/nxs/docs.json)
//   <out>/docs/<locale>/<slug>.md      → s3://<bucket>/docs/<locale>/…   (/nxs/docs/<locale>/…)
//   <out>/docs/assets/<name>           → s3://<bucket>/docs/assets/…     (/nxs/docs/assets/…)
//   <out>/docs-assets.json             → read by the workflow, NOT uploaded
//
// Inputs are the canonical, in-repo sources — the CI-drift-guarded terminal transcript, the
// release-notes feed, and the same guide markdown the CLIs serve — so the web surface can never
// drift stale-but-green from the CLI. Since 6j6v.9e3r there are THREE guide trees, one per building
// block (crates/{cli,memory,chat}/docs/guide), and `buildDocs` reads them off the repo root; the
// slug of a page is `<binary>-<topic>`, so the umbrella and the three blocks share one document
// without colliding. Since 6j6v.1pxs a block reads from a SECOND source as well —
// `content/docs-blocks/<block>/<locale>/` — for the pages the website carries and the CLI does not.
// Pure assembly here; validation is validate.mjs.
//
// Usage: node assemble.mjs [--out dir] [--channel stable|beta] [--base-path /nxs]

import fs from "node:fs";
import path from "node:path";
import url from "node:url";
import { buildContentManifest } from "./lib/content.mjs";
import { buildDocs, copyDocsAssets } from "./lib/docs.mjs";

const here = path.dirname(url.fileURLToPath(import.meta.url));
const repoRoot = path.join(here, "..");

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i !== -1 && i + 1 < process.argv.length ? process.argv[i + 1] : fallback;
}

const outDir = path.resolve(arg("--out", path.join(here, "dist")));
const channel = arg("--channel", "stable");
const basePath = arg("--base-path", "/nxs");

const demoPath = path.join(here, "data", "terminal-demo.json");
const releaseNotesPath = path.join(repoRoot, "release-notes.json");

function readJson(p, what) {
  if (!fs.existsSync(p)) {
    console.error(`[assemble] missing ${what}: ${p}`);
    process.exit(1);
  }
  try {
    return JSON.parse(fs.readFileSync(p, "utf8"));
  } catch (e) {
    console.error(`[assemble] ${what} is not valid JSON (${e.message}): ${p}`);
    process.exit(1);
  }
}

const demo = readJson(demoPath, "terminal-demo.json");
const releaseNotes = readJson(releaseNotesPath, "release-notes.json");

// docs.json + per-URL page objects (fail-closed on a missing guide / missing H1).
let docs;
try {
  docs = buildDocs({ repoRoot, basePath });
} catch (e) {
  console.error(`[assemble] docs assembly failed: ${e.message}`);
  process.exit(1);
}

// content.json — references docs.json via the top-level `docs` feed.
const content = buildContentManifest({
  demo,
  releaseNotes,
  channel,
  docsRef: `${basePath}/docs.json`,
});

// Write everything under a clean out dir.
fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(path.join(outDir, "content.json"), JSON.stringify(content, null, 2) + "\n");
fs.writeFileSync(path.join(outDir, "docs.json"), JSON.stringify(docs.manifest, null, 2) + "\n");
for (const file of docs.files) {
  const dest = path.join(outDir, file.path);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.writeFileSync(dest, file.markdown);
}

// Binary assets a page references. They are NOT docs pages — they never enter docs.json, so the
// landing build does not fetch them; the browser does, at runtime, from the same `/nxs/docs/…`
// prefix. The manifest is written BESIDE `docs/` rather than inside it, so the recursive
// markdown upload cannot sweep it into the bucket: it is an instruction to the workflow, not
// content. The workflow reads each file's content type from it instead of repeating the type as
// a literal of its own (an image uploaded as `text/markdown` is not an image, it is a download).
let assetManifest;
try {
  assetManifest = copyDocsAssets(repoRoot, outDir);
} catch (e) {
  console.error(`[assemble] ${e.message}`);
  process.exit(1);
}
fs.writeFileSync(
  path.join(outDir, "docs-assets.json"),
  JSON.stringify(assetManifest, null, 2) + "\n",
);

const blocks = (docs.manifest.groups ?? []).map((g) => g.id).join("+") || "flat";
console.log(
  `[assemble] content.json (channel=${channel}) + docs.json (${docs.manifest.nav.length} topics in ` +
    `${blocks}) + ${docs.files.length} docs page objects + ${assetManifest.length} asset(s) → ` +
    `${path.relative(process.cwd(), outDir)}`,
);
