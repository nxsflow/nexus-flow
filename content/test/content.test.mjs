import { test } from "node:test";
import assert from "node:assert/strict";
import { buildContentManifest, terminalTranscript } from "../lib/content.mjs";
import { LOCALES } from "../data/story.mjs";

const DEMO = { session: [{ cmd: "nxf next", out: "0001 ...\n" }, { cmd: "nxf blocked", out: "0003 ...\n" }] };
const RN = {
  channels: {
    stable: [{ version: "0.27.0", date: "2026-07-14", items: [{ type: "added", en: "A", de: "A-de" }] }],
    beta: [{ version: "0.28.0", date: "2026-07-20", items: [{ type: "added", en: "B", de: "B-de" }] }],
  },
};

test("terminalTranscript flattens {cmd,out} pairs into kind-tagged lines", () => {
  assert.deepEqual(terminalTranscript(DEMO), [
    { kind: "cmd", text: "nxf next" },
    { kind: "out", text: "0001 ...\n" },
    { kind: "cmd", text: "nxf blocked" },
    { kind: "out", text: "0003 ...\n" },
  ]);
  assert.deepEqual(terminalTranscript(null), []);
});

test("manifest carries the contract identity + every required locale", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  assert.equal(m.contract, "1");
  assert.equal(m.product, "nxs");
  assert.equal(m.slug, "open-source");
  assert.deepEqual(m.locales, LOCALES);
  for (const loc of ["en", "de"]) {
    assert.ok(m.content[loc], `content.${loc} present`);
    assert.equal(typeof m.content[loc].meta.title, "string");
    assert.equal(typeof m.content[loc].meta.description, "string");
  }
});

test("sections appear in the fixed story order with the expected discriminants", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const types = m.content.en.sections.map((s) => s.type);
  // suite-breakdown sits directly after the hero: the hero's sub names the three tools,
  // the breakdown is what they are (landing spec 2026-07-30, 6j6v.bced).
  assert.deepEqual(types, [
    "hero",
    "suite-breakdown",
    "terminal-demo",
    "feature-grid",
    "changelog",
    "powered-by",
    "download-cta",
  ]);
});

test("suite-breakdown names the three tools with their commands, in both locales", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  for (const locale of ["en", "de"]) {
    const s = m.content[locale].sections.find((x) => x.type === "suite-breakdown");
    assert.ok(s, `${locale} carries the section`);
    // Contract-required fields (content-contract v1, $defs.suiteBreakdownSection).
    assert.ok(Array.isArray(s.items) && s.items.length === 3, `${locale} has three items`);
    assert.deepEqual(
      s.items.map((i) => i.command),
      ["nxf", "nxm", "nxc"],
      `${locale} lists the commands in product order`,
    );
    // `command` and `tagline` are OPTIONAL in the contract (the landing owns it, and other
    // providers may omit them), but this provider always emits both — so the guard against
    // silently dropping one lives here rather than in the vendored schema mirror.
    for (const item of s.items) {
      for (const field of ["name", "command", "tagline", "body"]) {
        assert.equal(typeof item[field], "string", `${locale} item ${item.name} has a ${field}`);
        assert.ok(item[field].length > 0, `${locale} item ${item.name} has a non-empty ${field}`);
      }
    }
    // NONE of the three claims anything extra any more. chat carried a note —
    // "in the binary today, its guides follow with 1.0" — and it stopped being
    // true: the guides exist (they still want work, but they are there), and a
    // note that defers them tells a reader not to go and look. It was removed
    // rather than reworded because the note's whole job was to excuse an absence
    // that is over.
    //
    // Asserted as "none of them" rather than by deleting the case: a note is a
    // claim about readiness, it is the kind of line that gets pasted back in
    // during a release, and this section is on the LANDING PAGE now (cyb7.y7v4)
    // rather than on a subpage somebody had to find.
    for (const item of s.items) {
      assert.equal(item.note, undefined, `${locale} item ${item.name} claims nothing extra`);
    }
  }
});

test("suite-breakdown copy is written per locale, not one language twice", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const en = m.content.en.sections.find((x) => x.type === "suite-breakdown");
  const de = m.content.de.sections.find((x) => x.type === "suite-breakdown");
  assert.notEqual(en.title, de.title);
  assert.notEqual(en.items[0].tagline, de.items[0].tagline);
  // The product names are the products' names — identical across locales by design.
  assert.deepEqual(
    en.items.map((i) => i.name),
    de.items.map((i) => i.name),
  );
});

test("hero carries the install one-liner + repo; download-cta points at /nxs/latest", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN });
  const hero = m.content.en.sections.find((s) => s.type === "hero");
  assert.match(hero.install, /install\.sh \| sh$/);
  assert.equal(hero.repo.href, "https://github.com/nxsflow/nexus-flow");
  const cta = m.content.en.sections.find((s) => s.type === "download-cta");
  assert.equal(cta.endpoint, "/nxs/latest");
});

test("changelog carries the feed + inline entries, localized per section locale", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN, channel: "stable" });
  const en = m.content.en.sections.find((s) => s.type === "changelog");
  const de = m.content.de.sections.find((s) => s.type === "changelog");
  assert.equal(en.feed, "/nxs/release-notes.json");
  assert.equal(en.entries[0].items[0].text, "A");
  assert.equal(de.entries[0].items[0].text, "A-de");
});

test("changelog falls back to the beta ring when the chosen ring is empty", () => {
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN, channel: "stable", fallbackChannel: "beta" });
  // stable has entries → no fallback needed
  assert.equal(m.content.en.sections.find((s) => s.type === "changelog").entries[0].version, "0.27.0");
  // stable empty → fall back to beta
  const empty = { channels: { stable: [], beta: RN.channels.beta } };
  const m2 = buildContentManifest({ demo: DEMO, releaseNotes: empty, channel: "stable", fallbackChannel: "beta" });
  assert.equal(m2.content.en.sections.find((s) => s.type === "changelog").entries[0].version, "0.28.0");
});

test("docs ref is included only when provided", () => {
  assert.equal(buildContentManifest({ demo: DEMO, releaseNotes: RN }).docs, undefined);
  const m = buildContentManifest({ demo: DEMO, releaseNotes: RN, docsRef: "/nxs/docs.json" });
  assert.equal(m.docs, "/nxs/docs.json");
});
