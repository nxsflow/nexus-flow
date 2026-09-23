import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import url from "node:url";
import {
  buildDocs,
  splitTitle,
  assertTopicParity,
  checkCrossLinks,
  slugFor,
  slugifyHeading,
  legacyAliases,
  BUILDING_BLOCKS,
  DOCS_BLOCKS_DIR,
  DOCS_ASSETS,
  copyDocsAssets,
} from "../lib/docs.mjs";

const here = path.dirname(url.fileURLToPath(import.meta.url));
const repoRoot = path.join(here, "..", "..");
const flow = BUILDING_BLOCKS.find((b) => b.id === "nxf");
const memory = BUILDING_BLOCKS.find((b) => b.id === "nxm");
const chat = BUILDING_BLOCKS.find((b) => b.id === "nxc");
const flowGuideDir = path.join(repoRoot, ...flow.dir);
/** Every page a block publishes, in nav order: its web documents first, then its guide topics. */
const blockPages = (b) => [...(b.documents ?? []), ...b.order];
/** Every block that ships pages today, in declared order — the blocks that contribute a group. */
const populated = BUILDING_BLOCKS.filter((b) => blockPages(b).length > 0);
/** Every slug the manifest should carry, in nav order. */
const allSlugs = populated.flatMap((b) => blockPages(b).map((t) => slugFor(b, t)));

/** A throwaway repo root with `<block>/docs/guide/{en,de}` trees, for the shapes the real tree
 *  cannot show today — three POPULATED blocks, a broken cross-link, a group with no entries.
 *
 *  `files` are GUIDE TOPICS, written under the block's own `dir`; `documentFiles` are WEB
 *  DOCUMENTS, written under `content/docs-blocks/<id>/<locale>/` where `buildDocs` reads the
 *  second source (6j6v.1pxs). Both are `{ name: body }`.
 *
 *  It also FILLS IN a placeholder `summaries` entry for every page the block lists, so that a
 *  fixture stays about the thing its test is about. A summary is mandatory (6j6v.5cxr); the test
 *  that says so deletes one back out again. */
function fixtureRoot(blocks) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "nxs-docs-"));
  for (const block of blocks) {
    block.summaries ??= {};
    for (const name of [...(block.documents ?? []), ...block.order]) {
      block.summaries[name] ??= { en: `${block.id} ${name}, in one line`, de: `${block.id} ${name}, in einer Zeile` };
    }
    for (const locale of ["en", "de"]) {
      const dir = path.join(root, ...block.dir, locale);
      fs.mkdirSync(dir, { recursive: true });
      for (const [topic, body] of Object.entries(block.files ?? {})) {
        fs.writeFileSync(path.join(dir, `${topic}.md`), `# ${topic} (${locale})\n\n${body}\n`);
      }
      if (!block.documentFiles) continue;
      const webDir = path.join(root, ...DOCS_BLOCKS_DIR, block.id, locale);
      fs.mkdirSync(webDir, { recursive: true });
      for (const [name, body] of Object.entries(block.documentFiles)) {
        fs.writeFileSync(path.join(webDir, `${name}.md`), `# ${name} (${locale})\n\n${body}\n`);
      }
    }
  }
  return root;
}

/** The umbrella, as small as the conventions allow it to be. EVERY fixture manifest needs one:
 *  `buildDocs` rejects a manifest with no `nxs` block carrying a `getting-started`, because that
 *  document IS the docs start page the landing composes from (6j6v.wvca). */
function umbrellaBlock() {
  return {
    id: "nxs",
    dir: ["crates", "nxs", "docs", "guide"],
    title: { en: "nxs (the umbrella)", de: "nxs (die Klammer)" },
    order: ["getting-started"],
    files: { "getting-started": "u1" },
  };
}

/** A fixture block carrying both sources: two guide topics and one web document. */
function twoSourceBlock() {
  return {
    id: "nxf",
    dir: ["crates", "cli", "docs", "guide"],
    title: { en: "flow (nxf)", de: "flow (nxf)" },
    order: ["getting-started", "commands"],
    files: { "getting-started": "f1", commands: "f2" },
    documents: ["index"],
    documentFiles: { index: "what this block is for" },
  };
}

test("splitTitle extracts the H1 and drops it from the body", () => {
  const { title, body } = splitTitle("# Getting Started\n\nsome body\n", "en/getting-started.md");
  assert.equal(title, "Getting Started");
  assert.equal(body, "some body");
});

test("splitTitle fails closed on a missing H1 or empty body", () => {
  assert.throws(() => splitTitle("no heading here", "x"), /no '# ' title heading/);
  assert.throws(() => splitTitle("# Title only\n\n", "x"), /empty body/);
});

test("buildDocs produces an ordered nav with per-locale titles + urls from the real guides", () => {
  const { manifest } = buildDocs({ repoRoot });
  assert.equal(manifest.contract, "1");
  assert.deepEqual(manifest.locales, { default: "en", required: ["en", "de"] });
  assert.deepEqual(manifest.nav.map((n) => n.slug), allSlugs);
  // The UMBRELLA heads the nav since 6j6v.0fvt: first setup and the map of the blocks are what a
  // reader needs before they know the blocks exist. Flow's own page is asserted right after, so
  // this still says "the head moved" rather than "flow's page changed" — it did not.
  const gs = manifest.nav[0];
  assert.equal(gs.slug, "nxs-getting-started");
  assert.equal(gs.title.en, "Getting Started");
  assert.equal(gs.title.de, "Erste Schritte");
  assert.equal(gs.url.en, "/nxs/docs/en/nxs-getting-started.md");
  assert.equal(gs.url.de, "/nxs/docs/de/nxs-getting-started.md");
  const flowGs = manifest.nav.find((n) => n.slug === "nxf-getting-started");
  assert.ok(flowGs, "flow keeps its own getting-started page");
  assert.equal(flowGs.url.en, "/nxs/docs/en/nxf-getting-started.md");
  assert.equal(flowGs.url.de, "/nxs/docs/de/nxf-getting-started.md");
});

test("every slug is namespaced by its building block and unique across the manifest", () => {
  // The contract's own rule: `parseDocsManifest` throws on a duplicate slug, and `getting-started`
  // / `commands` exist in more than one group by design — so the namespace is what makes the
  // umbrella and the three blocks shareable in ONE document.
  const { manifest } = buildDocs({ repoRoot });
  const slugs = manifest.nav.map((n) => n.slug);
  assert.equal(new Set(slugs).size, slugs.length, "no duplicate slugs");
  // DERIVED from BUILDING_BLOCKS, not spelled out. This was `/^nx[sfmc]-/` until 6j6v.jepw, which
  // quietly asserted something narrower than the test's own name: that every group is a BINARY
  // called `nx?`. The develop group is a group without a binary — `nxs` serves its topics — so the
  // regex went red on a slug that is correctly namespaced. Deriving the set keeps the real claim
  // (a slug carries its group) and stops encoding today's roster as a rule.
  const groupIds = new Set(BUILDING_BLOCKS.map((b) => b.id));
  for (const slug of slugs) {
    const prefix = slug.slice(0, slug.indexOf("-"));
    assert.ok(
      groupIds.has(prefix),
      `${slug} is namespaced by "${prefix}", which is not a building-block id`,
    );
  }
});

test("nav entries carry their group and the groups registry matches nav order", () => {
  const { manifest } = buildDocs({ repoRoot });
  // Group level is all-or-nothing on the consumer side (gates G1/G2): every entry has one, and it
  // is the block the entry's own slug is namespaced with — asserted rather than assumed now that
  // more than one block is populated (6j6v.t6vd).
  for (const entry of manifest.nav) {
    assert.ok(entry.group, `${entry.slug} carries a group`);
    assert.ok(entry.slug.startsWith(`${entry.group}-`), `${entry.slug} sits in ${entry.group}`);
  }
  assert.deepEqual(
    manifest.groups.map((g) => g.id),
    populated.map((b) => b.id),
  );
  for (const group of manifest.groups) {
    const block = BUILDING_BLOCKS.find((b) => b.id === group.id);
    assert.equal(group.title.en, block.title.en);
    assert.equal(group.title.de, block.title.de);
  }
});

test("a block with no guides yet contributes no group and no nav entries", () => {
  // The state a block is in between shipping its guide MECHANISM (6j6v.9e3r) and writing its
  // CONTENT — chat until 6j6v.t6vd, memory until 6j6v.h4k0, and the next block that gets a guide
  // tree. Every registered block carries guides today, so this is proven on a FIXTURE rather than
  // deleted: the rule is live code in `buildDocs`, and it is what keeps the manifest legal. The
  // docs contract rejects a group with no nav entries outright ("it would render empty"), so
  // publishing a hollow block would break the manifest at the consumer — and would claim guides
  // that are not written. Silence is both the legal and the honest answer.
  const blocks = [
    umbrellaBlock(),
    { id: "nxz", dir: ["crates", "z", "docs", "guide"], title: { en: "z (nxz)", de: "z (nxz)" }, order: [] },
  ];
  const root = fixtureRoot(blocks);
  const { manifest } = buildDocs({ repoRoot: root, blocks });
  assert.deepEqual(manifest.groups.map((g) => g.id), ["nxs"], "the empty block declares no group");
  assert.ok(!manifest.nav.some((n) => n.group === "nxz"), "and contributes no nav entry");
  // Note what it is NOT asked for: the index every other block owes (6j6v.wvca). A block that
  // publishes nothing is not a block that publishes an introduction to nothing.
  // Its (empty) tree is still held by the parity guard — that is what catches the FIRST guide
  // added without listing it, which is how both content tickets were caught.
  assert.doesNotThrow(() =>
    assertTopicParity(path.join(root, "crates", "z", "docs", "guide"), []),
  );
});

test("a block's web documents are federated beside its guide topics (6j6v.1pxs)", () => {
  // The second source: `content/docs-blocks/<block>/<locale>/<name>.md`, read exactly like a guide
  // topic and published as a page of its own. A block's documents come FIRST — its index
  // introduces the block that its topics then detail — and both sources share the one slug scheme,
  // so the landing sees ONE list per block and needs no idea where an entry was stored.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  const root = fixtureRoot(blocks);
  const { manifest, files } = buildDocs({ repoRoot: root, blocks });

  assert.deepEqual(
    manifest.nav.filter((n) => n.group === "nxf").map((n) => n.slug),
    ["nxf-index", "nxf-getting-started", "nxf-commands"],
  );
  const index = manifest.nav.find((n) => n.slug === "nxf-index");
  assert.equal(index.group, "nxf");
  for (const locale of ["en", "de"]) {
    assert.equal(index.url[locale], `/nxs/docs/${locale}/nxf-index.md`);
    assert.equal(index.title[locale], `index (${locale})`);
  }
  // A document is a page object like any other: its own key, H1 stripped, both locales.
  assert.equal(files.find((f) => f.path === "docs/en/nxf-index.md").markdown, "what this block is for\n");
  assert.ok(manifest.pages.some((p) => p.key === "docs/de/nxf-index.md"));
});

test("every nav entry says which source it came from (6j6v.1pxs)", () => {
  // The one thing the two sources are NOT interchangeable in: a guide topic is also readable at
  // the terminal (`nxf guide commands`), a web document exists only on the website. The list is
  // source-blind; the mark is what lets a consumer say so if it wants to.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  const { manifest } = buildDocs({ repoRoot: fixtureRoot(blocks), blocks });
  assert.deepEqual(
    manifest.nav.map((n) => `${n.slug}:${n.source}`),
    [
      "nxs-getting-started:guide",
      "nxf-index:web",
      "nxf-getting-started:guide",
      "nxf-commands:guide",
    ],
  );
  // …and on the real registry: every entry declares one of the two, never absent, never invented.
  const real = buildDocs({ repoRoot }).manifest.nav;
  for (const entry of real) {
    assert.ok(["guide", "web"].includes(entry.source), `${entry.slug} declares its source`);
  }
  assert.ok(
    real.some((n) => n.source === "web") && real.some((n) => n.source === "guide"),
    "both sources are represented in the published feed",
  );
});

test("a web document is never published under a legacy alias key (6j6v.1pxs)", () => {
  // `legacyAliases` exists for the eight nxf topics that WERE published unprefixed before
  // 6j6v.9e3r. A web document has no pre-rename URL to keep alive, so writing `docs/en/index.md`
  // beside it would invent a public URL nobody ever linked — and `index.md` of all names.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  blocks[1].legacyUnprefixed = true;
  const { files } = buildDocs({ repoRoot: fixtureRoot(blocks), blocks });
  const paths = new Set(files.map((f) => f.path));
  assert.ok(paths.has("docs/en/commands.md"), "the guide topic keeps its alias");
  assert.ok(!paths.has("docs/en/index.md"), "the web document gets none");
});

test("a .md in a block's docs-blocks tree that nothing lists fails the build (6j6v.1pxs)", () => {
  // The parity guard of the FIRST source, applied to the second: a file added to the tree without
  // its line in `documents` would otherwise be a page that silently never publishes.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  blocks[1].documentFiles = { index: "listed", stray: "not listed" };
  const root = fixtureRoot(blocks);
  assert.throws(
    () => buildDocs({ repoRoot: root, blocks }),
    /nxf documents en\/ has topic\(s\) not listed for this block: stray/,
  );
});

test("a block whose ONLY pages are web documents still contributes a group (6j6v.1pxs)", () => {
  // The combination neither half of the pair above covers: `order` empty, `documents` not. The
  // empty-block branch keys on the COMBINED page count, so a block like this must contribute — and
  // it is not hypothetical: it is what a block looks like whose introduction is written before its
  // guides are (the mirror image of the `nxz` case, which has neither). Held here because the two
  // sources are read by different code paths and only their sum decides (PR #424 review, Test
  // Quality #1).
  const webOnly = {
    id: "nxc",
    dir: ["crates", "chat", "docs", "guide"],
    title: { en: "chat (nxc)", de: "chat (nxc)" },
    order: [],
    documents: ["index"],
    documentFiles: { index: "chat, introduced before its guides are written" },
  };
  const blocks = [umbrellaBlock(), webOnly];
  const { manifest } = buildDocs({ repoRoot: fixtureRoot(blocks), blocks });
  assert.deepEqual(manifest.groups.map((g) => g.id), ["nxs", "nxc"]);
  assert.deepEqual(
    manifest.nav.filter((n) => n.group === "nxc").map((n) => `${n.slug}:${n.source}`),
    ["nxc-index:web"],
  );
  // And it owes an index like every other non-nxs block — which it has, since that is all it has.
  assert.equal(manifest.nav.filter((n) => n.group === "nxc").length, 1);
});

test("a web document listed but missing in a locale fails the build (6j6v.1pxs)", () => {
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  const root = fixtureRoot(blocks);
  fs.rmSync(path.join(root, ...DOCS_BLOCKS_DIR, "nxf", "de", "index.md"));
  assert.throws(
    () => buildDocs({ repoRoot: root, blocks }),
    /nxf documents de\/ is missing listed topic\(s\): index/,
  );
});

/** Every `{module}:{topic}` → summary pair the golden `nxs guide --json` output holds.
 *
 *  This corpus is not a copy of anything: `crates/nxs/tests/golden/guide.trycmd` IS the command's
 *  output, kept honest by trycmd against the real binary. Reading it here costs no cargo — which
 *  matters, because the publish job is pure Node and could not run `nxs guide --json` if it wanted
 *  to (6j6v.5cxr). */
function goldenGuideSummaries() {
  const trycmd = fs.readFileSync(
    path.join(repoRoot, "crates", "nxs", "tests", "golden", "guide.trycmd"),
    "utf8",
  );
  const line = trycmd.split("\n").find((l) => l.startsWith('{"modules":'));
  assert.ok(line, "guide.trycmd no longer holds a `nxs guide --json` payload to hold the feed to");
  const out = new Map();
  for (const mod of JSON.parse(line).modules) {
    for (const topic of mod.topics) out.set(`${mod.module}:${topic.topic}`, topic.summary);
  }
  return out;
}

test("every English guide summary in the feed is the string `nxs guide --json` prints (6j6v.5cxr)", () => {
  // THE DRIFT GATE, and it exists as a by-product: the summaries live in BUILDING_BLOCKS now (the
  // Rust catalogs are English-only and unreachable from a Node publish job), so the feed and the
  // CLI could say two different things about the same topic with nothing to notice. The golden
  // corpus is the second copy that makes the comparison possible.
  const golden = goldenGuideSummaries();
  const { manifest } = buildDocs({ repoRoot });
  let checked = 0;
  for (const entry of manifest.nav) {
    if (entry.source !== "guide") continue;
    const block = BUILDING_BLOCKS.find((b) => b.id === entry.group);
    const topic = entry.slug.slice(block.id.length + 1);
    const expected = golden.get(`${block.module}:${topic}`);
    assert.ok(expected, `${entry.slug} has no counterpart in guide.trycmd (${block.module}:${topic})`);
    assert.equal(
      entry.summary.en,
      expected,
      `${entry.slug}: the feed and \`nxs guide --json\` disagree about this topic's summary`,
    );
    checked++;
  }
  // Not a vacuous pass: every guide topic of every block was held against the golden output.
  assert.equal(checked, populated.flatMap((b) => b.order).length);
});

test("every nav entry carries its own summary, in both locales (6j6v.5cxr)", () => {
  // The two halves the golden output cannot hold: German has no counterpart anywhere (the Rust
  // catalogs embed `docs/guide/en` alone), and a web document has no Rust counterpart at all. Both
  // are new text either way, so they are held on PRESENCE — which is what the consumer needs, since
  // every one of these pages turns its summary into a `<meta description>`.
  const { manifest } = buildDocs({ repoRoot });
  for (const entry of manifest.nav) {
    for (const locale of ["en", "de"]) {
      assert.ok(entry.summary?.[locale]?.trim(), `${entry.slug} has a ${locale} summary`);
    }
    assert.notEqual(
      entry.summary.en,
      entry.summary.de,
      `${entry.slug}: the German summary is the English one — write the German page's own line`,
    );
  }
});

test("a page with no summary fails the build (6j6v.5cxr)", () => {
  // Fail-closed like everything else here: a page whose summary is missing would publish a nav
  // entry the consumer promised a description for, and the consumer's own gate turns THAT into a
  // failure. Better to stop it at the source, where the missing line is.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  const root = fixtureRoot(blocks);
  delete blocks[1].summaries.commands;
  assert.throws(() => buildDocs({ repoRoot: root, blocks }), /missing summary for nxf commands/);
});

test("a block that publishes pages without an index fails the build (6j6v.wvca)", () => {
  // Convention 1, refused rather than shrugged at: the landing DERIVES `/docs/<block>` from the
  // block's index. Without one there is no page to render at that url and nothing to stack onto
  // the docs start page — and the manifest would still be perfectly valid JSON, which is exactly
  // why the schema cannot catch it.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  delete blocks[1].documents;
  delete blocks[1].documentFiles;
  const root = fixtureRoot(blocks);
  assert.throws(
    () => buildDocs({ repoRoot: root, blocks }),
    /missing index for block nxf/,
  );
});

test("a manifest with no nxs getting-started fails the build (6j6v.wvca)", () => {
  // Convention 2. `nxs/getting-started` IS the docs start page — the one document the site opens
  // with, and the one every block index is written not to repeat. A feed without it composes
  // nothing.
  const blocks = [twoSourceBlock()];
  const root = fixtureRoot(blocks);
  assert.throws(
    () => buildDocs({ repoRoot: root, blocks }),
    /no nxs block with a getting-started/,
  );
});

test("a web document that shadows a guide topic of its block fails the build (6j6v.wvca)", () => {
  // Convention 3, and the one that only exists BECAUSE there are two sources now: both would
  // publish as the slug `nxf-commands`, so two documents would claim one url — the guide topic
  // and the document, one silently overwriting the other in the upload.
  const blocks = [umbrellaBlock(), twoSourceBlock()];
  blocks[1].documents = ["index", "commands"];
  blocks[1].documentFiles = { index: "i", commands: "a second commands page" };
  const root = fixtureRoot(blocks);
  assert.throws(
    () => buildDocs({ repoRoot: root, blocks }),
    /nxf web document\(s\) shadow a guide topic of the same name: 'commands' → 'nxf-commands'/,
  );

  // TWO collisions name TWO slugs. The message used to list both documents and only the first
  // slug, which reads as if the second one were fine (PR #424 review, Code Quality #2).
  const two = [umbrellaBlock(), twoSourceBlock()];
  two[1].documents = ["index", "commands", "getting-started"];
  two[1].documentFiles = { index: "i", commands: "c", "getting-started": "g" };
  assert.throws(
    () => buildDocs({ repoRoot: fixtureRoot(two), blocks: two }),
    /'commands' → 'nxf-commands', 'getting-started' → 'nxf-getting-started'/,
  );
});

test("a block index does not repeat what nxs/getting-started already said (6j6v.cdmf)", () => {
  // THE AUTHORING RULE, as a gate. The five documents on the docs start page are read one after
  // another — `nxs`'s getting-started, then each block's index — so an index that installs the
  // suite a second time, or runs `nxs init` again, is not a small redundancy: it is the same
  // paragraph twice on one page. Nothing else would catch it; the build is green either way.
  //
  // Held on the SETUP the umbrella owns, not on wording, because that is the part that is
  // objectively somebody else's: installation, `nxs init`, and what it leaves on disk.
  const { manifest, files } = buildDocs({ repoRoot });
  const indexSlugs = BUILDING_BLOCKS.filter((b) => (b.documents ?? []).includes("index")).map((b) =>
    slugFor(b, "index"),
  );
  assert.deepEqual(
    indexSlugs,
    populated.filter((b) => b.id !== "nxs").map((b) => `${b.id}-index`),
    "every block but nxs carries an index",
  );
  const setup = [/`nxs init`/, /install\.sh/, /curl -fsSL/];
  for (const slug of indexSlugs) {
    assert.ok(manifest.nav.some((n) => n.slug === slug && n.source === "web"));
    for (const locale of ["en", "de"]) {
      const body = files.find((f) => f.path === `docs/${locale}/${slug}.md`).markdown;
      for (const pattern of setup) {
        assert.ok(
          !pattern.test(body),
          `${slug} (${locale}) repeats setup (${pattern}) — that is nxs-getting-started's page, ` +
            `and the two are read back to back. Introduce YOUR block instead.`,
        );
      }
    }
  }
});

test("chat's guides are federated under their own group (6j6v.t6vd)", () => {
  // The content ticket's own acceptance point, held where it is observable: the guides appear
  // under the chat block, in the order chat's `TOPICS` declares, with en+de urls each.
  const { manifest } = buildDocs({ repoRoot });
  const chatNav = manifest.nav.filter((n) => n.group === "nxc");
  assert.deepEqual(
    chatNav.map((n) => n.slug),
    // The block index leads (6j6v.cdmf); chat's own follow in the order `TOPICS` declares. The
    // COUNT is deliberately not in this test's name or in this comment any more (6j6v.9w08 made it
    // seven): the list is derived from `BUILDING_BLOCKS`, so a number here would only ever be a
    // second copy that rots.
    blockPages(chat).map((t) => `nxc-${t}`),
  );
  for (const entry of chatNav) {
    for (const locale of ["en", "de"]) {
      assert.equal(entry.url[locale], `/nxs/docs/${locale}/${entry.slug}.md`);
      assert.ok(entry.title[locale], `${entry.slug} has a ${locale} title`);
    }
  }
  // The collision the namespace exists for is now REAL, not hypothetical: two blocks carry
  // `getting-started` and two carry `commands`, and both pairs are distinct slugs.
  for (const shared of ["getting-started", "commands"]) {
    const carriers = populated.filter((b) => b.order.includes(shared));
    assert.ok(carriers.length > 1, `${shared} is carried by more than one block`);
    const slugs = carriers.map((b) => slugFor(b, shared));
    assert.equal(new Set(slugs).size, slugs.length);
  }
});

test("populated blocks assemble as contiguous groups in block order", () => {
  // The nav shape the registry produces, on a fixture small enough to read: one group per block,
  // in registry order, each block's pages consecutive and its index at their head. The real
  // manifest is asserted topic-for-topic in the per-block tests above; this one is about the SHAPE.
  const doc = { index: "i" };
  const blocks = [
    umbrellaBlock(),
    { id: "nxf", dir: ["crates", "cli", "docs", "guide"], title: { en: "flow (nxf)", de: "flow (nxf)" }, order: ["getting-started", "commands"], files: { "getting-started": "f1", commands: "f2" }, documents: ["index"], documentFiles: doc },
    { id: "nxm", dir: ["crates", "memory", "docs", "guide"], title: { en: "memory (nxm)", de: "memory (nxm)" }, order: ["getting-started"], files: { "getting-started": "m1" }, documents: ["index"], documentFiles: doc },
    { id: "nxc", dir: ["crates", "chat", "docs", "guide"], title: { en: "chat (nxc)", de: "chat (nxc)" }, order: ["getting-started", "commands"], files: { "getting-started": "c1", commands: "c2" }, documents: ["index"], documentFiles: doc },
  ];
  const root = fixtureRoot(blocks);
  const { manifest } = buildDocs({ repoRoot: root, blocks });
  assert.deepEqual(
    manifest.groups.map((g) => g.id),
    ["nxs", "nxf", "nxm", "nxc"],
  );
  assert.deepEqual(
    manifest.nav.map((n) => `${n.group}:${n.slug}`),
    [
      "nxs:nxs-getting-started",
      "nxf:nxf-index",
      "nxf:nxf-getting-started",
      "nxf:nxf-commands",
      "nxm:nxm-index",
      "nxm:nxm-getting-started",
      "nxc:nxc-index",
      "nxc:nxc-getting-started",
      "nxc:nxc-commands",
    ],
    "a block's pages are consecutive (contract gate G4a) and the registry order is the nav order",
  );
  // The collision the namespace exists for: `getting-started` FOUR times since 6j6v.0fvt (the
  // umbrella plus the three blocks), four distinct slugs.
  const slugs = manifest.nav.map((n) => n.slug);
  assert.equal(new Set(slugs).size, slugs.length);
});

test("memory's five guides are federated under their own group (6j6v.h4k0)", () => {
  // The content ticket's own acceptance point, held where it is observable: the guides appear
  // under the memory block, in the order memory's `TOPICS` declares, with en+de urls each — the
  // THIRD group beside flow and chat, which is the gap the published docs showed until now.
  const { manifest } = buildDocs({ repoRoot });
  assert.ok(manifest.groups.some((g) => g.id === "nxm"), "memory declares a group");
  const memoryNav = manifest.nav.filter((n) => n.group === "nxm");
  assert.deepEqual(
    memoryNav.map((n) => n.slug),
    // The block index leads (6j6v.cdmf); memory's own five follow in the order `TOPICS` declares.
    blockPages(memory).map((t) => `nxm-${t}`),
  );
  for (const entry of memoryNav) {
    for (const locale of ["en", "de"]) {
      assert.equal(entry.url[locale], `/nxs/docs/${locale}/${entry.slug}.md`);
      assert.ok(entry.title[locale], `${entry.slug} has a ${locale} title`);
    }
  }
  // With memory populated the shared topics collide across blocks — which is the whole reason the
  // `<binary>-<topic>` namespace exists. The counts are spelled out PER TOPIC rather than shared,
  // because the exact collision is the claim: since 6j6v.0fvt `getting-started` is carried by FOUR
  // groups (the umbrella's suite-wide one plus each block's own), while `core-concepts` and
  // `commands` stay with the three blocks — the umbrella owns no domain and carries neither.
  for (const [shared, expected] of [
    // FIVE since 6j6v.cdmf: the umbrella's suite-wide one, each block's own — and develop's, which
    // is a WEB document precisely because a fifth guide topic of that name could not exist
    // (`nxs` serves the develop catalog and already carries `getting-started`).
    ["getting-started", 5],
    ["core-concepts", 3],
    ["commands", 3],
  ]) {
    const carriers = populated.filter((b) => blockPages(b).includes(shared));
    assert.equal(carriers.length, expected, `${shared} is carried by ${expected} groups`);
    const slugs = carriers.map((b) => slugFor(b, shared));
    assert.equal(new Set(slugs).size, slugs.length);
  }
});

test("pages are separate markdown objects (no monolith) — one per topic per locale", () => {
  const { manifest, files } = buildDocs({ repoRoot });
  assert.equal(manifest.pages.length, allSlugs.length * 2);
  // The manifest indexes pages by URL but carries NO markdown bodies.
  for (const p of manifest.pages) {
    assert.equal(typeof p.url, "string");
    assert.ok(!("markdown" in p), "manifest page ref must not inline markdown");
  }
  // Each file body has had its H1 stripped and is non-empty.
  for (const f of files) {
    assert.ok(f.path.startsWith("docs/"));
    assert.ok(f.markdown.length > 0);
    assert.ok(!/^#\s+/.test(f.markdown), "page body should not start with the H1 title");
  }
});

test("legacy deep links to the pre-rename nxf topics stay resolvable as alias objects", () => {
  // 6j6v.9e3r DoD: the eight nxf topics were published at /nxs/docs/<locale>/<topic>.md before the
  // slug rename. They are written a SECOND time under those keys, byte-identical, so an existing
  // link keeps answering — and the alias is a file only, never a nav/page entry, so it cannot
  // become a second (duplicate) slug.
  const { manifest, files } = buildDocs({ repoRoot });
  const paths = new Set(files.map((f) => f.path));
  for (const topic of flow.order) {
    for (const locale of ["en", "de"]) {
      assert.ok(paths.has(`docs/${locale}/${topic}.md`), `legacy alias for ${locale}/${topic}`);
      assert.ok(paths.has(`docs/${locale}/nxf-${topic}.md`), `canonical ${locale}/${topic}`);
    }
  }
  const canonical = files.find((f) => f.path === "docs/en/nxf-commands.md");
  const alias = files.find((f) => f.path === "docs/en/commands.md");
  assert.equal(alias.markdown, canonical.markdown, "the alias is byte-identical");
  const slugs = new Set(manifest.nav.map((n) => n.slug));
  for (const topic of flow.order) assert.ok(!slugs.has(topic), `${topic} is not a slug any more`);
  // Only the block that WAS published unprefixed gets aliases.
  assert.deepEqual(legacyAliases({ id: "nxm", order: [] }, "getting-started", "en"), []);
});

test("buildDocs fails closed when a topic is missing a locale", () => {
  const blocks = [{ ...flow, order: [...flow.order, "does-not-exist"] }];
  assert.throws(() => buildDocs({ repoRoot, blocks }), /missing listed topic/);
});

test("the deferring-and-waiting topic is federated (F3 regression)", () => {
  const { manifest } = buildDocs({ repoRoot });
  const slugs = manifest.nav.map((n) => n.slug);
  assert.ok(slugs.includes("nxf-deferring-and-waiting"), "must appear in the docs nav");
  // Counted against every populated block's own order rather than a literal: the guard exists to
  // catch a topic being silently DROPPED (F3), and `assertTopicParity` already fails closed if the
  // order and the guides on disk disagree.
  assert.equal(slugs.length, allSlugs.length, "every guide topic is federated");
});

test("assertTopicParity passes for the real guides and fails closed on drift", () => {
  assert.doesNotThrow(() => assertTopicParity(flowGuideDir, flow.order));
  // A list that omits an on-disk guide fails closed (the exact silent-drop F3 caught) — and names
  // the block, which matters now that there are three trees.
  assert.throws(
    () => assertTopicParity(flowGuideDir, ["getting-started"], undefined, "nxf guide"),
    /nxf guide en\/ has topic\(s\) not listed/,
  );
});

test("assertTopicParity holds EVERY block's tree, not just flow's", () => {
  // One guard per block, run over the real trees: a `.md` added to any of the three without its
  // line in `order` goes red instead of vanishing from the federated nav (F3).
  for (const block of BUILDING_BLOCKS) {
    assert.doesNotThrow(() =>
      assertTopicParity(path.join(repoRoot, ...block.dir), block.order, undefined, `${block.id} guide`),
    );
  }
});

test("slugifyHeading mirrors the landing's heading anchors, umlauts and ß included", () => {
  // Held byte-for-byte against src/lib/markdown-sanitize.ts in the landing repo: `checkCrossLinks`
  // is only as truthful as this mirror.
  assert.equal(slugifyHeading("The nxs umbrella"), "the-nxs-umbrella");
  assert.equal(slugifyHeading("Das nxs-Umbrella"), "das-nxs-umbrella");
  assert.equal(slugifyHeading("Abhängigkeiten & Größe"), "abhangigkeiten-grosse");
});

test("every cross-link in the real guides resolves against the published slugs", () => {
  // THE guard the rename needed (the landing coordinator's 2026-08-16 note): an un-carried
  // cross-reference degrades to plain text with no error anywhere. buildDocs runs this on every
  // assemble; asserting it here states the guarantee where a reader looks for it.
  assert.doesNotThrow(() => buildDocs({ repoRoot }));
});

test("no published guide body prints `ready` as a lane you could ask for (6j6v.4dvc)", () => {
  // The decision of 2026-09-02, as a gate. There is no `nxf ready`; the lanes you can query are
  // `next` and `blocked`. *ready* stays as the state an item is IN — but in code font, beside those
  // two, it taught a reader a lane that does not exist, and nothing caught it: the docs and the CLI
  // were each consistent with themselves.
  //
  // WHY HERE and not in a Rust test beside one crate's guide (the reviewer's suggestion for PR
  // #420): the rule is about everything PUBLISHED, and `buildDocs` is the one place that already
  // walks every block in both locales. A test in `crates/cli` would have held flow's eight English
  // pages and left the umbrella's, chat's, memory's, the develop tree's, and every German page
  // unguarded — four fifths of the surface the decision is about.
  //
  // Backticks are the whole rule: they mean "you can type this". Prose that says an item is ready,
  // or names the ready set, is untouched and must stay that way.
  const { files } = buildDocs({ repoRoot });
  const offenders = files
    .filter((f) => f.markdown.includes("`ready`"))
    .map((f) => f.path)
    .sort();
  assert.deepEqual(
    offenders,
    [],
    `these published pages print \`ready\` in code font, beside lanes that are real:\n` +
      `${offenders.join("\n")}\n` +
      `Say what you mean instead: \`nxf next\` / \`nxf blocked\` for the lanes, or plain *ready* ` +
      `for the state an item is in. See crates/core/src/derive.rs for the rule and its reason.`,
  );
});

test("checkCrossLinks rejects a stale bare target and a dangling in-page anchor", () => {
  const slugs = new Set(["nxf-plugins", "nxf-commands"]);
  // The exact failure the rename would have caused: the pre-rename target, now nobody's slug.
  assert.throws(
    () => checkCrossLinks([{ where: "en/x.md", slug: "nxf-commands", body: "see [p](plugins)" }], slugs),
    /cross-link 'plugins' is not a docs slug/,
  );
  // The other half of the same damage: an anchor that names no heading on its page.
  assert.throws(
    () =>
      checkCrossLinks(
        [{ where: "en/x.md", slug: "nxf-commands", body: "## Real\n\nsee [a](#nope)" }],
        slugs,
      ),
    /matches no heading/,
  );
  // What must NOT trip: a carried slug, an anchor that IS a heading, a fenced sample, a real URL.
  assert.doesNotThrow(() =>
    checkCrossLinks(
      [
        {
          where: "en/x.md",
          slug: "nxf-commands",
          body: "## The nxs umbrella\n\n[a](nxf-plugins) [b](#the-nxs-umbrella) [c](https://x.dev/y)\n\n```\n[not](a-link)\n```",
        },
      ],
      slugs,
    ),
  );
});

test("a cross-link that was carried in EN but forgotten in DE still fails", () => {
  // Both locales are checked independently — the half-done rename is the realistic mistake.
  const blocks = [
    {
      id: "nxf",
      dir: ["crates", "cli", "docs", "guide"],
      title: { en: "flow (nxf)", de: "flow (nxf)" },
      order: ["a", "b"],
      files: { a: "x", b: "y" },
    },
  ];
  const root = fixtureRoot(blocks);
  // Break DE only: EN links the new slug, DE still links the old bare topic.
  fs.writeFileSync(path.join(root, "crates/cli/docs/guide/en/a.md"), "# A\n\nsee [b](nxf-b)\n");
  fs.writeFileSync(path.join(root, "crates/cli/docs/guide/de/a.md"), "# A\n\nsiehe [b](b)\n");
  assert.throws(() => buildDocs({ repoRoot: root, blocks }), /nxf de\/a\.md: cross-link 'b'/);
});

/** A throwaway tree holding just the source files a DOCS_ASSETS list points at. */
function assetRoot(assets) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "nxs-assets-"));
  for (const asset of assets) {
    const src = path.join(root, ...asset.from);
    fs.mkdirSync(path.dirname(src), { recursive: true });
    fs.writeFileSync(src, `bytes of ${asset.to}`);
  }
  return root;
}

test("copyDocsAssets lands each asset under docs/assets and reports its declared type", () => {
  const assets = [
    { from: ["docs", "architecture", "a.webp"], to: "a.webp", contentType: "image/webp" },
    { from: ["docs", "b.png"], to: "b.png", contentType: "image/png" },
  ];
  const root = assetRoot(assets);
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "nxs-out-"));

  const manifest = copyDocsAssets(root, out, assets);

  // The bytes land flat under docs/assets/<to>, whatever nesting the source had.
  assert.equal(fs.readFileSync(path.join(out, "docs", "assets", "a.webp"), "utf8"), "bytes of a.webp");
  assert.equal(fs.readFileSync(path.join(out, "docs", "assets", "b.png"), "utf8"), "bytes of b.png");
  // The manifest is what the publish workflow reads to choose each upload's content type. It
  // carries the type from the LIST, which is the whole point of the field existing (the workflow
  // used to repeat it as a literal of its own).
  assert.deepEqual(manifest, [
    { file: "a.webp", contentType: "image/webp" },
    { file: "b.png", contentType: "image/png" },
  ]);
});

test("copyDocsAssets throws when a declared asset is missing, naming it", () => {
  const assets = [{ from: ["docs", "gone.webp"], to: "gone.webp", contentType: "image/webp" }];
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "nxs-out-"));

  // The failure path the publish strecke depends on: a DOCS_ASSETS entry whose source was moved
  // or renamed must stop the build, not publish a page whose <img> 404s. It THROWS rather than
  // exiting so it can be tested at all — assemble.mjs owns the process and turns this into a
  // non-zero exit (6j6v.jepw review, Test Quality #2).
  assert.throws(
    () => copyDocsAssets(assetRoot([]), out, assets),
    /missing docs asset: docs.gone\.webp/,
  );
  assert.equal(fs.existsSync(path.join(out, "docs", "assets")), false, "nothing half-copied");
});

test("every real DOCS_ASSETS entry resolves to a file that exists", () => {
  // The list is the one place a published image is declared; a typo here is a 404 on the docs
  // site and nothing else would catch it before the bucket.
  for (const asset of DOCS_ASSETS) {
    assert.ok(
      fs.existsSync(path.join(repoRoot, ...asset.from)),
      `DOCS_ASSETS names ${path.join(...asset.from)}, which is not in the repo`,
    );
    assert.match(asset.contentType, /^[a-z]+\/[a-z0-9.+-]+$/, "contentType is a media type");
  }
});

/** The `paths:` patterns that gate the publish workflow's push trigger, in file order.
 *
 *  Parsed rather than YAML-loaded on purpose: `content/` carries no yaml dependency, and the
 *  shape being read is a flat list of quoted globs directly under `paths:`. If that ever stops
 *  being true this throws on an empty list rather than passing vacuously. */
function publishTriggerPaths() {
  const yml = fs.readFileSync(
    path.join(repoRoot, ".github", "workflows", "publish-content.yml"),
    "utf8",
  );
  const lines = yml.split("\n");
  const start = lines.findIndex((l) => /^\s{4}paths:\s*$/.test(l));
  assert.ok(start !== -1, "publish-content.yml has no `paths:` block under its push trigger");
  const patterns = [];
  for (const line of lines.slice(start + 1)) {
    const entry = line.match(/^\s+- "(.+)"\s*$/);
    if (entry) {
      patterns.push(entry[1]);
      continue;
    }
    if (/^\s*#/.test(line)) continue; // comments interleave the list
    break;
  }
  assert.ok(patterns.length > 0, "parsed no patterns out of the `paths:` block");
  return patterns;
}

test("publish trigger covers every published source", () => {
  // WHY THIS EXISTS: a `paths:` filter that fails to match is not an error — the workflow simply
  // does not run, the site keeps serving the previous text, and nothing anywhere goes red. That
  // is what happened to the umbrella's guide tree: it arrived with 6j6v.0fvt and was never added
  // here, so an edit to an `nxs` page alone published nothing at all (found 6j6v.jepw). Every
  // OTHER drift in this area has a gate; this one had none, which is exactly why it survived.
  const patterns = publishTriggerPaths();
  const covers = (file) =>
    patterns.some((p) => (p.endsWith("/**") ? file.startsWith(p.slice(0, -2)) : p === file));

  // Every block that ships guides must gate the workflow — derived from BUILDING_BLOCKS, so a
  // fifth block cannot be added without this failing.
  for (const block of populated) {
    const dir = block.dir.join("/");
    assert.ok(
      patterns.includes(`${dir}/**`),
      `publish-content.yml does not trigger on ${dir}/** — an edit there would publish nothing`,
    );
  }

  // The second source (6j6v.1pxs) publishes from `content/**`, which the filter already carries.
  // Asserted rather than assumed: it is the same silent class — a `paths:` filter that fails to
  // match is not an error, it is a page that never publishes.
  assert.ok(
    covers(`${DOCS_BLOCKS_DIR.join("/")}/nxf/en/index.md`),
    `publish-content.yml does not trigger on ${DOCS_BLOCKS_DIR.join("/")}/** — a web document ` +
      `would publish nothing`,
  );

  // …and so must every binary asset a page embeds: re-exporting a diagram changes what the site
  // serves exactly as re-wording a page does.
  for (const asset of DOCS_ASSETS) {
    const file = asset.from.join("/");
    assert.ok(
      covers(file),
      `publish-content.yml does not trigger on ${file} — a re-export would publish nothing`,
    );
  }
});
