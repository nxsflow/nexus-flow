import { test } from "node:test";
import assert from "node:assert/strict";
import { recentEntries } from "../lib/releaseNotes.mjs";

const RN = {
  channels: {
    alpha: [],
    beta: [
      { version: "0.28.0", date: "2026-07-20", items: [{ type: "added", en: "beta thing", de: "Beta-Ding" }] },
    ],
    stable: [
      { version: "0.27.0", date: "2026-07-14", items: [
        { type: "changed", en: "changed A", de: "geändert A" },
        { type: "added", en: "added B", de: "hinzugefügt B" },
      ] },
      { version: "0.26.0", date: "2026-07-01", items: [{ type: "fixed", en: "fix C", de: "Fix C" }] },
    ],
  },
};

test("selects the requested ring, newest-first, capped by limit", () => {
  const out = recentEntries(RN, { channel: "stable", limit: 1, locale: "en" });
  assert.equal(out.length, 1);
  assert.equal(out[0].version, "0.27.0");
  assert.equal(out[0].date, "2026-07-14");
});

test("localizes item text to the requested locale, falling back to en", () => {
  const de = recentEntries(RN, { channel: "stable", limit: 1, locale: "de" });
  assert.deepEqual(de[0].items, [
    { type: "changed", text: "geändert A" },
    { type: "added", text: "hinzugefügt B" },
  ]);
  // Missing locale key falls back to en.
  const partial = { channels: { stable: [{ version: "1.0.0", items: [{ type: "added", en: "only en" }] }] } };
  const out = recentEntries(partial, { channel: "stable", locale: "de" });
  assert.equal(out[0].items[0].text, "only en");
});

test("shape matches the contract's changelog entry (type + text only)", () => {
  const out = recentEntries(RN, { channel: "stable", limit: 5, locale: "en" });
  for (const e of out) {
    assert.equal(typeof e.version, "string");
    for (const it of e.items) assert.deepEqual(Object.keys(it).sort(), ["text", "type"]);
  }
});

test("empty / unknown ring yields an empty list (not a throw)", () => {
  assert.deepEqual(recentEntries(RN, { channel: "alpha" }), []);
  assert.deepEqual(recentEntries(RN, { channel: "nope" }), []);
  assert.deepEqual(recentEntries({}, { channel: "stable" }), []);
  assert.deepEqual(recentEntries(null, { channel: "stable" }), []);
});
