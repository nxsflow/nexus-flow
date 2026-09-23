import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import url from "node:url";
import { buildContentManifest } from "../lib/content.mjs";
import { validateContent } from "../lib/validate.mjs";

const here = path.dirname(url.fileURLToPath(import.meta.url));
const schema = JSON.parse(fs.readFileSync(path.join(here, "..", "schema", "content-contract-v1.json"), "utf8"));

const DEMO = { session: [{ cmd: "nxf next", out: "0001 ...\n" }] };
const RN = { channels: { stable: [{ version: "0.27.0", date: "2026-07-14", items: [{ type: "added", en: "A", de: "A-de" }] }] } };

test("the assembled content manifest validates against the published content-contract v1 schema", () => {
  const manifest = buildContentManifest({ demo: DEMO, releaseNotes: RN, docsRef: "/nxs/docs.json" });
  const { ok, errors } = validateContent(manifest, schema);
  assert.ok(ok, "expected valid content.json, got errors:\n" + JSON.stringify(errors, null, 2));
});

test("validation is fail-closed: a manifest missing a required top-level field is rejected", () => {
  const manifest = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  delete manifest.contract;
  const { ok } = validateContent(manifest, schema);
  assert.equal(ok, false);
});

test("validation rejects a hero section missing its required headline", () => {
  const manifest = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const hero = manifest.content.en.sections.find((s) => s.type === "hero");
  delete hero.headline;
  const { ok } = validateContent(manifest, schema);
  assert.equal(ok, false);
});

test("validation rejects a suite-breakdown item missing its required body", () => {
  const manifest = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const suite = manifest.content.en.sections.find((s) => s.type === "suite-breakdown");
  delete suite.items[0].body;
  const { ok } = validateContent(manifest, schema);
  assert.equal(ok, false);
});

test("validation rejects a suite-breakdown section with no items at all", () => {
  const manifest = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const suite = manifest.content.en.sections.find((s) => s.type === "suite-breakdown");
  delete suite.items;
  const { ok } = validateContent(manifest, schema);
  assert.equal(ok, false);
});
