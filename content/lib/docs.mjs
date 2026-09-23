// Assemble `docs.json` (navigation tree + per-URL page refs) plus the page bodies as SEPARATE
// markdown objects — no monolith (§4). Built from the canonical guides the CLIs themselves serve,
// so the web docs can never drift stale-but-green from the command line. The landing fetches
// docs.json, renders the nav, and pulls each page markdown from its own
// /nxs/docs/<locale>/<slug>.md object behind its sanitizer.
//
// THREE BUILDING BLOCKS, NOT ONE PRODUCT (nexus-flow 6j6v.9e3r). Until now every guide came from
// crates/cli/docs/guide — so the "suite documentation" documented nxf and nothing else, while the
// hero promised "one binary, three products". Each block now brings its own guide tree, and the
// manifest carries the `groups` level the landing published for exactly this (cyb7.93q6,
// https://nxsflow.com/schema/docs-contract-v1.json). Two consequences run through this whole file:
//
//   1. SLUGS ARE NAMESPACED. `getting-started` and `commands` exist in more than one block, and
//      docs.json slugs are the anchors of a single page — `parseDocsManifest` throws on a duplicate.
//      The scheme is `<binary>-<topic>` (`nxf-getting-started`), which is also how a reader would
//      say it out loud. The CLI keeps the bare topic name: there the binary already namespaces it.
//   2. CROSS-LINKS INSIDE THE GUIDES MOVE WITH THE SLUGS. A guide links a sibling by its bare
//      target — `[plugins](nxf-plugins)` — and the landing's `docsHref` resolves it ONLY if it is a
//      known slug; anything else silently degrades to plain text. No error, no red check, just a
//      link that is no longer one. `checkCrossLinks` below turns that class into a build failure.

import fs from "node:fs";
import path from "node:path";

export const DEFAULT_LOCALES = ["en", "de"];

/**
 * The SECOND source a block reads from (6j6v.1pxs): `content/docs-blocks/<block>/<locale>/<name>.md`.
 *
 * A block has two sources. Its guide tree serves BOTH `<binary> guide` and the website; this tree
 * serves the website alone — the block indexes, and develop's `getting-started`. The layout mirrors
 * the guide one (`<dir>/<locale>/<name>.md`), so there is nothing new to learn, and it sits under
 * `content/**`, which the `paths:` filter of publish-content.yml already triggers on.
 *
 * WEB-ONLY IS A PLACE, NOT A SWITCH. Two gates make every `.md` under a GUIDE tree a topic the CLI
 * serves — `assertTopicParity` here and `the_*_embedded_dir_and_topics_list_are_in_sync` on the Rust
 * side — so a file dropped in a guide tree that the binary does not know is a failing build, not a
 * hidden page. A document that must not reach `nxs guide` therefore needs a tree of its own rather
 * than a flag, and `develop/getting-started` is the case that forces it: `nxs` already carries a
 * `getting-started`, and two topics of one name in one binary's catalogs is a failing build too
 * (`topic_names_do_not_collide_across_the_two_nxs_catalogs`).
 *
 * The landing sees ONE list of documents per block: same slug scheme, same page objects, ordered by
 * the registry rather than by source. THE ORDER AND THE LIST ARE SOURCE-BLIND, which is what the
 * epic's design means by "the split is a filing decision in this repo, not a term of the contract".
 *
 * THE `source` MARK ON EACH NAV ENTRY IS A KNOWN EXCEPTION TO THAT SENTENCE, not an oversight, and
 * the two board items say different things about it: 6j6v.1pxs's own acceptance criterion requires
 * a web document to be MARKED so the landing can tell it from a guide topic, while the epic's
 * design says the split is not a contract term. Both are implemented — a source-blind list plus an
 * additive field a consumer may ignore — because there is exactly one respect in which the two are
 * not interchangeable: a guide topic can also be read at the terminal (`nxf guide commands`), a web
 * document cannot. The supersession is recorded on the epic (6j6v.wwrh) and on 6j6v.1pxs rather
 * than left as a contradiction between the design text and this file. If a consumer never needs the
 * distinction, deleting this field is a one-line change that breaks no convention.
 */
export const DOCS_BLOCKS_DIR = ["content", "docs-blocks"];

/**
 * The suite's building blocks, in the order they are read — the ONE list this module is driven by.
 *
 * `order` MUST match the `TOPICS` list in each block's own guide module (the agent contract for
 * `<binary> guide --json`): `crates/cli/src/commands/guide.rs`, `crates/memory/src/guide.rs`,
 * `crates/chat/src/guide.rs`. `assertTopicParity` fails the build if a list and the guides on disk
 * drift apart — a guide added without listing it here would otherwise be silently dropped from the
 * federated docs, the exact bug F3 caught, and it mirrors the Rust-side drift guard.
 *
 * All three blocks carry guides now: 6j6v.t6vd wrote chat's first six (6j6v.9w08 added a seventh,
 * `writing-declarations`), 6j6v.h4k0 memory's five, and
 * 6j6v.0fvt added the umbrella's own three above them as a fourth group. Each
 * time the whole wiring was the `<topic>.md` files under `docs/guide/{en,de}` plus their lines in
 * `order` — the group then appeared on its own. The zero-topics case is still live code, not
 * history: a block listed with an empty `order` contributes no group and no nav entries at all (see
 * `buildDocs` for why the contract makes that mandatory rather than optional), which is what a
 * fourth block would look like between shipping its mechanism and writing its content.
 *
 * `documents` is the block's SECOND source (6j6v.1pxs): the web-only pages under
 * {@link DOCS_BLOCKS_DIR}, in reading order, published beside the guide topics and BEFORE them. It
 * carries the block index every non-`nxs` block owes the interpreter, and — for `develop` alone —
 * the `getting-started` that cannot be a guide topic because `nxs` already serves one.
 * `assertTopicParity` holds it against the tree exactly as it holds `order` against the guides.
 *
 * `summaries` is one line per page, per locale — the `<meta description>` of the page it becomes
 * (6j6v.5cxr), and mandatory: `buildDocs` throws on a page without one. It lives HERE, beside the
 * other per-locale display text, because the two obvious alternatives are not available. The
 * publish job is pure Node (setup-node, npm ci, node assemble.mjs), so `nxs guide --json` cannot be
 * called at publish time; and the Rust catalogs embed `docs/guide/en` alone, so the German half
 * exists nowhere else — as do the web documents' summaries, in both languages. The drift that
 * introduces is closed by a gate that already existed as a by-product: `guide.trycmd` holds the
 * exact `nxs guide --json` output, and a content test asserts the English summaries here are those
 * same strings.
 *
 * The titles are the product names as the landing's own suite breakdown says them
 * (`content/data/story.mjs`), with the binary in parentheses because the slugs and every command in
 * the guides are spelled that way. They are deliberately the SAME string in both locales: a product
 * name is not translated, and inventing German marketing copy is not this ticket's business.
 */
export const BUILDING_BLOCKS = [
  {
    // The umbrella goes FIRST (6j6v.0fvt). It is not a module and owns no domain — what it owns is
    // the reader's first ten minutes: install, `nxs init`, what lands on disk, and the map of which
    // block does what. Those questions come BEFORE knowing that nxf/nxm/nxc exist, so the group
    // that answers them heads the nav rather than trailing the three products.
    id: "nxs",
    module: "nxs",
    dir: ["crates", "nxs", "docs", "guide"],
    // Deliberately NOT the "<name> (<binary>)" shape of the three blocks: the umbrella is not a
    // building block among them, and a title that pretended otherwise would put a fourth product
    // on the page. The parenthetical says what it is instead.
    title: { en: "nxs (the umbrella)", de: "nxs (die Klammer)" },
    // The three topics `nxs guide` serves, in its declared order — kept in lockstep with `TOPICS`
    // in crates/nxs/src/guide.rs, which `assertTopicParity` below holds against the files on disk.
    order: ["getting-started", "modules", "the-workspace"],
    summaries: {
      "getting-started": {
        en: "Install the suite, set up a workspace, and see what restores an agent's context.",
        de: "Die Suite installieren, einen Arbeitsbereich einrichten und sehen, was den Kontext eines Agenten wiederherstellt.",
      },
      modules: {
        en: "What flow, memory and chat are each for, and which one to reach for.",
        de: "Wofür flow, memory und chat jeweils da sind — und zu welchem du greifst.",
      },
      "the-workspace": {
        en: "What `nxs init` leaves on disk, what is committed, and how to check it is healthy.",
        de: "Was `nxs init` auf der Platte hinterlässt, was eingecheckt wird, und wie du prüfst, dass es gesund ist.",
      },
    },
  },
  {
    id: "nxf",
    module: "flow",
    dir: ["crates", "cli", "docs", "guide"],
    title: { en: "flow (nxf)", de: "flow (nxf)" },
    // The eight topics `nxf guide` serves, in its declared order.
    order: [
      "getting-started",
      "mcp",
      "core-concepts",
      "commands",
      "plugins",
      "migration",
      "deferring-and-waiting",
      "running-a-relay",
    ],
    // These eight were published UNPREFIXED until 6j6v.9e3r, when they became `nxf-<topic>`. Their
    // old markdown object URLs stay resolvable — see `legacyAliases`.
    legacyUnprefixed: true,
    documents: ["index"],
    summaries: {
      index: {
        en: "The block that tracks what needs doing — and why its work list is derived, not stored.",
        de: "Der Baustein für das, was zu tun ist — und warum seine Arbeitsliste abgeleitet statt gespeichert wird.",
      },
      "getting-started": {
        en: "Install, init a workspace, and create your first task.",
        de: "Installieren, einen Arbeitsbereich anlegen und die erste Aufgabe erstellen.",
      },
      mcp: {
        en: "Connect an MCP host (Claude Desktop, Cursor, …) — register the server and bootstrap.",
        de: "Einen MCP-Host anbinden (Claude Desktop, Cursor, …) — den Server registrieren und den Einstieg laden.",
      },
      "core-concepts": {
        en: "Items, dependencies, and how `next` and `blocked` are derived rather than stored.",
        de: "Items, Abhängigkeiten, und wie `next` und `blocked` abgeleitet statt gespeichert werden.",
      },
      commands: {
        en: "A tour of the most-used nxf commands.",
        de: "Eine Führung durch die meistgenutzten nxf-Befehle.",
      },
      plugins: {
        en: "How vocabulary, ranking, and presentation are configured.",
        de: "Wie Vokabular, Rangfolge und Darstellung konfiguriert werden.",
      },
      migration: {
        en: "Bring an existing tracker in and bind a sync stream.",
        de: "Einen bestehenden Tracker übernehmen und einen Sync-Strom binden.",
      },
      "deferring-and-waiting": {
        en: "Defer only for real calendar dates; model waiting on a delivery as a WAIT chore.",
        de: "Aufschieben nur für echte Kalenderdaten; das Warten auf eine Lieferung als WARTE-Chore modellieren.",
      },
      "running-a-relay": {
        en: "Run your own sync server for backup or collaboration — SQLite, or a Postgres such as Supabase.",
        de: "Einen eigenen Sync-Server betreiben, für Sicherung oder Zusammenarbeit — SQLite oder ein Postgres wie Supabase.",
      },
    },
  },
  {
    id: "nxm",
    module: "memory",
    dir: ["crates", "memory", "docs", "guide"],
    title: { en: "memory (nxm)", de: "memory (nxm)" },
    // The five topics `nxm guide` serves (6j6v.h4k0), in its declared order — kept in lockstep with
    // `TOPICS` in crates/memory/src/guide.rs, which `assertTopicParity` below enforces against the
    // guides on disk from the other side.
    order: [
      "getting-started",
      "core-concepts",
      "commands",
      "agents-and-mcp",
      "import-and-migration",
    ],
    documents: ["index"],
    summaries: {
      index: {
        en: "Durable facts that outlive the session which learned them, and the one channel for them.",
        de: "Durable Tatsachen, die ihre Sitzung überleben — und der eine Kanal, in den sie gehören.",
      },
      "getting-started": {
        en: "What memory is for, what it needs, and your first remembered fact.",
        de: "Wofür memory da ist, was es braucht, und deine erste gemerkte Tatsache.",
      },
      "core-concepts": {
        en: "Keys and auto-keys, the three registers, the retrieval rule, reading order, and the tombstone.",
        de: "Schlüssel und Auto-Schlüssel, die drei Register, die Abrufregel, die Lesereihenfolge und der Grabstein.",
      },
      commands: {
        en: "Every shipped verb, with its `--json` shape and the errors it can answer with.",
        de: "Jedes ausgelieferte Verb, mit seiner `--json`-Form und den Fehlern, mit denen es antworten kann.",
      },
      "agents-and-mcp": {
        en: "The `memory_*` MCP tools, what `nxs prime` contributes, and where an agent writes.",
        de: "Die `memory_*`-MCP-Werkzeuge, was `nxs prime` beisteuert, und wohin ein Agent schreibt.",
      },
      "import-and-migration": {
        en: "Take a Claude-host memory store in, and file what a workspace already accumulated.",
        de: "Einen Memory-Speicher aus einem Claude-Host übernehmen und einsortieren, was ein Arbeitsbereich schon angesammelt hat.",
      },
    },
  },
  {
    id: "nxc",
    module: "chat",
    dir: ["crates", "chat", "docs", "guide"],
    title: { en: "chat (nxc)", de: "chat (nxc)" },
    // The seven topics `nxc guide` serves (6j6v.t6vd, plus `writing-declarations` from 6j6v.9w08),
    // in its declared order — kept in lockstep with `TOPICS` in crates/chat/src/guide.rs, which
    // `assertTopicParity` below enforces against the guides on disk from the other side.
    order: [
      "getting-started",
      "core-concepts",
      "commands",
      "personas",
      "channels",
      "writing-declarations",
      "limits-and-safety",
    ],
    documents: ["index"],
    summaries: {
      index: {
        en: "The channel between you and your agents, and the declarations that decide who may.",
        de: "Der Kanal zwischen dir und deinen Agenten — und die Deklarationen, die entscheiden, wer darf.",
      },
      "getting-started": {
        en: "Activate chat, declare a persona, and get your first answer back.",
        de: "chat aktivieren, eine Persona deklarieren und die erste Antwort bekommen.",
      },
      "core-concepts": {
        en: "The shared op-log, qualified handles, threads and their derived quorum, and how a message is delivered.",
        de: "Das geteilte Op-Log, qualifizierte Handles, Fäden und ihr abgeleitetes Quorum, und wie eine Nachricht zugestellt wird.",
      },
      commands: {
        en: "Every shipped verb, with its `--json` shape — and what was removed, with what to do instead.",
        de: "Jedes ausgelieferte Verb, mit seiner `--json`-Form — und was entfernt wurde, samt dem, was stattdessen zu tun ist.",
      },
      personas: {
        en: "Declaring one agent: identity, prompt layers, model band, tools, and who may address it.",
        de: "Einen Agenten deklarieren: Identität, Prompt-Schichten, Modellband, Werkzeuge, und wer ihn ansprechen darf.",
      },
      channels: {
        en: "Declaring a group — and the ordered channel (`flow: sequential`) that IS the workflow.",
        de: "Eine Gruppe deklarieren — und der geordnete Kanal (`flow: sequential`), der DER Arbeitsablauf ist.",
      },
      "writing-declarations": {
        en: "From an intention to a declaration that answers usefully: seven measured error classes, and a checklist.",
        de: "Von einer Absicht zu einer Deklaration, die brauchbar antwortet: sieben gemessene Fehlerklassen und eine Checkliste.",
      },
      "limits-and-safety": {
        en: "The hop cap, the human gate, the working-copy lease, and what an agent does NOT decide.",
        de: "Die Hop-Obergrenze, das menschliche Tor, die Reservierung der Arbeitskopie, und was ein Agent NICHT entscheidet.",
      },
    },
  },
  {
    // The fifth group is NOT a building block and not a binary — it is an AUDIENCE (6j6v.jepw).
    // Everything above answers "how do I use this"; this answers "how is it built, and what may I
    // build on". It goes LAST because that is the reading order: you meet the umbrella, then the
    // three products, and only then the seams underneath them.
    //
    // `module` is a registry key with no binary of its own: `nxs` serves these topics, so a reader
    // still types `nxs guide architecture`. That is why the slug prefix (`develop-`) and the
    // command (`nxs guide …`) disagree here and nowhere else — the group names the audience, the
    // command names the binary that carries it. crates/nxs/src/guide.rs holds the second catalog
    // and a test that no topic name appears in both, because `nxs guide <topic>` has no way to
    // ask "which one did you mean" when both answers are the same command.
    id: "develop",
    module: "develop",
    dir: ["crates", "nxs", "docs", "develop"],
    title: { en: "Develop with nexus-flow", de: "Mit nexus-flow entwickeln" },
    // Kept in lockstep with `DEVELOP_TOPICS` in crates/nxs/src/guide.rs.
    order: [
      "architecture",
      "the-journey-of-one-op",
      "a-reply-across-two-machines",
    ],
    // develop is the one block with TWO web documents (6j6v.cdmf). Its two audiences — someone who
    // wants to contribute, and someone who wants to build an application on the engine — both need
    // a first step, and `getting-started` is the one they share. It cannot be a guide topic:
    // `nxs` serves this catalog and already carries a `getting-started`, and two topics of one name
    // across one binary's catalogs is a failing build
    // (`topic_names_do_not_collide_across_the_two_nxs_catalogs`).
    documents: ["index", "getting-started"],
    summaries: {
      index: {
        en: "Two doors: build your own application on the engine, or contribute to the project itself.",
        de: "Zwei Türen: eine eigene Anwendung auf der Engine bauen, oder am Projekt selbst mitbauen.",
      },
      "getting-started": {
        en: "The source on your machine, the four gates, and the example application to copy from.",
        de: "Die Quellen auf deiner Maschine, die vier Tore, und die Beispiel-Anwendung, von der man abschaut.",
      },
      architecture: {
        en: "The map: five layers, which way the arrows point, and which seams are contracts.",
        de: "Die Landkarte: fünf Schichten, wohin die Pfeile zeigen, und welche Nähte Verträge sind.",
      },
      "the-journey-of-one-op": {
        en: "One `nxf close`, followed from the verb to a derived answer nobody wrote.",
        de: "Ein `nxf close`, verfolgt vom Verb bis zu einer abgeleiteten Antwort, die niemand geschrieben hat.",
      },
      "a-reply-across-two-machines": {
        en: "The same trip over two replicas: what an arriving op carries, and what stays behind.",
        de: "Dieselbe Fahrt über zwei Repliken: was eine ankommende Op mitbringt, und was zurückbleibt.",
      },
    },
  },
];

/**
 * Binary files a docs page references, copied verbatim into `<out>/docs/assets/` and served at
 * `/nxs/docs/assets/<to>` — a path the landing distribution ALREADY routes to the artifacts
 * bucket (`"/nxs/docs/*"` in nxsflow-landing-page `src/infra/nxs-behaviors.ts`), so an image
 * needs no change in that repo. The markdown side is `![alt](/nxs/docs/assets/<to>)`, which the
 * landing sanitizer passes because `img` is on its tag allowlist and a root-relative url clears
 * `safeSameOriginUrl`.
 *
 * The list is EXPLICIT rather than a directory sweep, and that is the point: `docs/architecture/`
 * also holds the `.drawio` sources and a detailed variant that is deliberately NOT published
 * (6j6v.jepw — only the simplified view goes to the docs). A sweep would ship all of them.
 *
 * `contentType` is LIVE DATA, not documentation: `copyDocsAssets` returns it and `assemble.mjs`
 * writes it into `<out>/docs-assets.json`, which is the ONE thing the publish workflow reads to
 * decide the header it uploads each file with. It used to be a second, independent hardcoded
 * string in that workflow — a shape where editing this line looked like it changed the published
 * header and did not (6j6v.jepw review, Code Quality #1).
 *
 * What nothing here guarantees: that an export still matches the `.drawio` it came from. There is
 * no gate for that; the convention that stands in its place — both files travel in one commit —
 * is written down in `xtask/export-architecture-diagram.sh`, which is also what produces them.
 */
export const DOCS_ASSETS = [
  {
    from: ["docs", "architecture", "nexus-flow-simplified.webp"],
    to: "nexus-flow-simplified.webp",
    contentType: "image/webp",
  },
  {
    from: ["docs", "architecture", "the-journey-of-one-op.webp"],
    to: "the-journey-of-one-op.webp",
    contentType: "image/webp",
  },
  {
    from: ["docs", "architecture", "a-reply-across-two-machines.webp"],
    to: "a-reply-across-two-machines.webp",
    contentType: "image/webp",
  },
];

/**
 * Copy every {@link DOCS_ASSETS} entry into `<outDir>/docs/assets/` and return the manifest the
 * publish workflow consumes — `[{ file, contentType }]`, one entry per copied file.
 *
 * A missing source THROWS rather than exiting: this is library code, and `assemble.mjs` owns the
 * process. It is also the only failure mode here worth a test, which is why the copying lives in
 * this module at all rather than inline in the script (6j6v.jepw review, Test Quality #2).
 */
export function copyDocsAssets(repoRoot, outDir, assets = DOCS_ASSETS) {
  return assets.map((asset) => {
    const src = path.join(repoRoot, ...asset.from);
    if (!fs.existsSync(src)) {
      throw new Error(`missing docs asset: ${path.join(...asset.from)}`);
    }
    const dest = path.join(outDir, "docs", "assets", asset.to);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(src, dest);
    return { file: asset.to, contentType: asset.contentType };
  });
}

/** The published slug of one block's topic. The single place the scheme is spelled. */
export function slugFor(block, topic) {
  return `${block.id}-${topic}`;
}

/** Split a guide file into its H1 title and the remaining body (the nav carries the title, so it
 *  is dropped from the body to avoid a duplicate heading). Fail-closed: every guide must open with
 *  an H1 and carry a non-empty body. */
export function splitTitle(md, where) {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const i = lines.findIndex((l) => /^#\s+\S/.test(l));
  if (i === -1) throw new Error(`${where}: no '# ' title heading`);
  const title = lines[i].replace(/^#\s+/, "").trim();
  const body = lines.slice(i + 1).join("\n").trim();
  if (!body) throw new Error(`${where}: empty body`);
  return { title, body };
}

/** Fail-closed drift guard (mirrors the `TOPICS`↔dir parity test each block's guide module runs):
 *  every `<name>.md` under each locale MUST be listed, and vice-versa. Without this, a page added
 *  to the repo but not to its list would be silently dropped from the federated docs nav (F3).
 *  Throws with a legible message on any mismatch so CI (content-ci.yml) goes red at the source.
 *  `label` names the tree in the message: the pre-6j6v.9e3r wording ("guide en/ has topic(s) not in
 *  TOPIC_ORDER") named a single flat list and, with four guide trees and a second SOURCE beside
 *  them (6j6v.1pxs), would no longer say WHICH tree drifted. Runs over a block's guide tree
 *  (`order`) and over its web documents (`documents`) alike — the drift is the same class. */
export function assertTopicParity(treeDir, order, locales = DEFAULT_LOCALES, label = "guide") {
  const listed = new Set(order);
  for (const locale of locales) {
    const dir = path.join(treeDir, locale);
    if (!fs.existsSync(dir)) {
      throw new Error(
        `${label}: ${dir} does not exist — every block needs ${locales.join(" + ")} ` +
          `directories here (empty ones carry a .gitkeep; see the tree's README)`,
      );
    }
    const onDisk = fs
      .readdirSync(dir)
      .filter((f) => f.endsWith(".md"))
      .map((f) => f.replace(/\.md$/, ""));
    const unlisted = onDisk.filter((slug) => !listed.has(slug));
    if (unlisted.length > 0) {
      throw new Error(
        `${label} ${locale}/ has topic(s) not listed for this block: ${unlisted.join(", ")} — add ` +
          `them to the block's list in BUILDING_BLOCKS (content/lib/docs.mjs); a guide tree's list ` +
          `is kept in lockstep with the block's own TOPICS`,
      );
    }
    const missing = order.filter((slug) => !onDisk.includes(slug));
    if (missing.length > 0) {
      throw new Error(`${label} ${locale}/ is missing listed topic(s): ${missing.join(", ")}`);
    }
  }
}

/**
 * Anchor id for a heading — a byte-for-byte mirror of `slugifyHeading` in the landing's
 * `src/lib/markdown-sanitize.ts`. Duplicated rather than imported because the two repos ship
 * independently; `checkCrossLinks` is only as truthful as this stays, so it is the one function
 * here that must be changed in lockstep with the landing.
 */
export function slugifyHeading(text) {
  return text
    .toLowerCase()
    .replace(/ß/g, "ss") // NFKD leaves ß alone; the German guides need it folded
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "") // drop combining marks left by NFKD
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

/** Every `[text](target)` target in a markdown body, outside fenced code blocks (a fence may hold
 *  a literal `](…)` in sample output, which is text, not a link). */
function linkTargets(md) {
  const out = [];
  let inFence = false;
  for (const line of md.replace(/\r\n/g, "\n").split("\n")) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    for (const m of line.matchAll(/\]\(([^)\s]*)\)/g)) out.push(m[1]);
  }
  return out;
}

/** Every heading anchor a rendered page offers, i.e. the ids the landing's sanitizer assigns to the
 *  headings of the BODY it is handed (the H1 is stripped by `splitTitle`, so it offers none). */
function headingAnchors(body) {
  const out = new Set();
  let inFence = false;
  for (const line of body.replace(/\r\n/g, "\n").split("\n")) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const m = /^(#{1,6})\s+(.*\S)\s*$/.exec(line);
    if (m) out.add(slugifyHeading(m[2]));
  }
  return out;
}

/**
 * The guard the slug rename needs (6j6v.9e3r, and the landing coordinator's warning of 2026-08-16).
 *
 * The landing resolves a link target with `docsHref` (`src/lib/docs-links.ts`): a bare
 * `plugins` / `plugins.md` / `plugins#x` becomes `#<slug>` **only if it is a known nav slug**, and
 * a `#anchor` becomes that topic's namespaced heading id. Anything it cannot resolve returns
 * `undefined`, and the renderer keeps the link TEXT with no anchor. That is the damage this
 * function exists for: renaming `core-concepts` to `nxf-core-concepts` without carrying the
 * cross-references along produces no error, no failing check, and a page full of links that are no
 * longer links — "kein Fehler, keine rote Pruefung, nur ein Link, der keiner mehr ist".
 *
 * So every intra-docs target is resolved here, at assembly, exactly as the landing would:
 *  - a bare slug-shaped target MUST be a slug in this manifest;
 *  - a `#anchor` MUST be a slug, or a heading that exists in the page it is written in;
 *  - anything with a scheme, a slash or a query is the sanitizer's business (`safeUrl`) and is
 *    left alone here.
 *
 * `pages` is `[{ where, slug, body }]` — one entry per locale per topic, so a link that was carried
 * in EN and forgotten in DE fails too.
 */
export function checkCrossLinks(pages, slugs) {
  const bare = /^([a-z0-9][a-z0-9-]*)(?:\.md)?(?:#[^/?#]*)?$/i;
  const problems = [];
  for (const page of pages) {
    const anchors = headingAnchors(page.body);
    for (const target of linkTargets(page.body)) {
      const trimmed = target.trim();
      if (trimmed === "") continue;
      if (trimmed.startsWith("#")) {
        const anchor = trimmed.slice(1).toLowerCase();
        if (anchor === "" || (!slugs.has(anchor) && !anchors.has(anchor))) {
          problems.push(
            `${page.where}: in-page link '${target}' matches no heading in that page ` +
              `(and no topic slug) — it would render as plain text`,
          );
        }
        continue;
      }
      const m = bare.exec(trimmed);
      if (!m) continue; // a URL / path: `safeUrl`'s business, not ours.
      const slug = m[1].toLowerCase();
      if (!slugs.has(slug)) {
        problems.push(
          `${page.where}: cross-link '${target}' is not a docs slug — it would silently render ` +
            `as plain text. Slugs are '<binary>-<topic>' (e.g. nxf-core-concepts).`,
        );
      }
    }
  }
  if (problems.length > 0) {
    throw new Error(`docs cross-links do not resolve:\n  - ${problems.join("\n  - ")}`);
  }
}

/**
 * The two conventions the landing's interpreter STANDS ON, held against the finished manifest
 * (6j6v.wvca; the third, a document shadowing a guide topic, is caught in `buildDocs` where the
 * second source is read).
 *
 *   1. Every block but `nxs` carries an `index` — the landing derives `/open-source/docs/<block>`
 *      from it, and the docs start page is composed of those indexes.
 *   2. There is an `nxs` block with a `getting-started` — that document IS the start page.
 *
 * WHY NOT THE JSON SCHEMA. Both are SEMANTIC rules; a schema cannot express "a block named nxs
 * exists" or "every other block resolves an index". It is the same distinction docs-contract.ts
 * already draws for the group level (its G1-G4 gates are code, not schema). nexus-flow's CI
 * validates docs.json against the schema, so a throw here is what actually stops a missing
 * convention before it is published — claiming the schema covers it would be a false map.
 *
 * Only blocks that CONTRIBUTE are held: a block with no pages at all declares no group and owes no
 * introduction to nothing.
 */
function assertConventions(groups, nav) {
  const slugs = new Set(nav.map((n) => n.slug));
  for (const group of groups) {
    if (group.id === "nxs") continue;
    if (!slugs.has(`${group.id}-index`)) {
      throw new Error(
        `missing index for block ${group.id} — every block but nxs owes the docs site its own ` +
          `introduction at content/docs-blocks/${group.id}/<locale>/index.md, and the landing ` +
          `derives /open-source/docs/${group.id} from it (list it in the block's \`documents\`)`,
      );
    }
  }
  if (!slugs.has("nxs-getting-started")) {
    throw new Error(
      `no nxs block with a getting-started — that document is the docs start page the landing ` +
        `composes the block indexes onto; without it there is nothing to open the site with`,
    );
  }
}

/**
 * @param {{repoRoot:string, docsPrefix?:string, basePath?:string, blocks?:object[], locales?:string[]}} opts
 * @returns {{manifest:object, files:{path:string, markdown:string}[]}}
 *   `manifest` is docs.json (groups + nav + per-URL page index, NO bodies); `files` are the page
 *   markdown objects to upload under the content prefix.
 */
export function buildDocs({
  repoRoot,
  docsPrefix = "docs",
  basePath = "/nxs",
  blocks = BUILDING_BLOCKS,
  locales = DEFAULT_LOCALES,
} = {}) {
  if (!repoRoot) throw new Error("buildDocs: repoRoot is required");
  const groups = [];
  const nav = [];
  const pages = [];
  const files = [];
  const bodies = [];

  for (const block of blocks) {
    const guideDir = path.join(repoRoot, ...block.dir);
    // Drift guard first: a guide on disk but not in `order` must fail the build, not vanish
    // silently. Runs for an EMPTY block too — that is what caught `6j6v.t6vd`'s and `6j6v.h4k0`'s
    // first topics, before they were listed here.
    assertTopicParity(guideDir, block.order, locales, `${block.id} guide`);

    // …and the same guard over the block's SECOND source (6j6v.1pxs). Skipped only when the block
    // declares no documents AND has no tree — a block that has never had one is not missing
    // anything; the moment either exists, both halves are held against each other.
    const documents = block.documents ?? [];
    const webDir = path.join(repoRoot, ...DOCS_BLOCKS_DIR, block.id);
    if (documents.length > 0 || fs.existsSync(webDir)) {
      assertTopicParity(webDir, documents, locales, `${block.id} documents`);
    }

    // A document that SHADOWS a guide topic of the same block would publish under the one slug
    // both compute — two pages claiming one url, the second silently overwriting the first in the
    // upload. It is the one failure the second source invents, so it is caught where the second
    // source is read (convention 3 of 6j6v.wvca; the other two are held on the finished manifest).
    const shadowed = documents.filter((name) => block.order.includes(name));
    if (shadowed.length > 0) {
      // One clause per collision, not one name list and a single slug: with two shadowed names the
      // shorter form named both documents and only the first slug, which reads as if the second
      // were fine (review of PR #424, Code Quality #2).
      const collisions = shadowed.map((name) => `'${name}' → '${slugFor(block, name)}'`);
      throw new Error(
        `${block.id} web document(s) shadow a guide topic of the same name: ` +
          `${collisions.join(", ")} — each pair publishes under one slug, so one page would ` +
          `overwrite the other. Rename the web document.`,
      );
    }

    // ONE list per block, documents first: a block's index introduces the block its topics then
    // detail, and `develop`'s `getting-started` is the first step into it. The consumer reads the
    // list in this order and needs no idea which tree an entry was filed in.
    const entries = [
      ...documents.map((name) => ({ name, source: "web", dir: webDir })),
      ...block.order.map((name) => ({ name, source: "guide", dir: guideDir })),
    ];

    if (entries.length === 0) {
      // A block with no pages AT ALL — neither guides nor documents — contributes NOTHING, not an
      // empty group. (Either source alone is enough to make it contribute.) The docs contract is
      // explicit: `docs.groups[i] ("x") has no nav entries — it would render empty` is a throw, so
      // publishing a hollow block would break the whole manifest at the consumer rather than show
      // a placeholder. Silence is also the honest answer: a block that ships its guide MECHANISM
      // before its content must not have a docs page claiming otherwise. No registered block is in
      // that state today (6j6v.h4k0 was the last one), which is why `docs.test.mjs` proves this on
      // a fixture rather than on the real registry.
      continue;
    }
    groups.push({ id: block.id, title: { ...block.title } });

    for (const entry of entries) {
      const slug = slugFor(block, entry.name);
      // Every page turns its summary into a `<meta description>` on the consumer side, so a
      // missing one is a page that ships without a description — fail-closed here, where the
      // missing line is, rather than at the consumer's own gate (6j6v.5cxr).
      const summary = block.summaries?.[entry.name];
      if (!summary || !locales.every((l) => summary[l]?.trim())) {
        throw new Error(
          `missing summary for ${block.id} ${entry.name} — every page needs one per locale ` +
            `(${locales.join(" + ")}); it becomes the page's meta description. Add it to ` +
            `\`summaries\` in BUILDING_BLOCKS (content/lib/docs.mjs).`,
        );
      }
      const title = {};
      const url = {};
      for (const locale of locales) {
        const where = `${block.id} ${locale}/${entry.name}.md`;
        const file = path.join(entry.dir, locale, `${entry.name}.md`);
        // Belt and braces: the parity guards above already proved every listed name exists in
        // every locale, so this cannot fire today. It is kept because it is the ONE check that
        // does not depend on them having run — if a future caller ever reads a tree without a
        // parity pass, the failure is still a named page, not a stack trace from readFileSync.
        if (!fs.existsSync(file)) {
          throw new Error(
            `missing ${entry.source === "web" ? "document" : "guide"} ${where} ` +
              `(every page needs ${locales.join(" + ")})`,
          );
        }
        const parsed = splitTitle(fs.readFileSync(file, "utf8"), where);
        const key = `${docsPrefix}/${locale}/${slug}.md`;
        const pageUrl = `${basePath}/${key}`;
        title[locale] = parsed.title;
        url[locale] = pageUrl;
        pages.push({ key, locale, slug, url: pageUrl });
        files.push({ path: key, markdown: parsed.body + "\n" });
        bodies.push({ where, slug, body: parsed.body });
        // Aliases are the guide tree's business only: they keep PUBLISHED urls answering, and a
        // web document has none to keep (see `legacyAliases`).
        if (entry.source === "guide") {
          for (const alias of legacyAliases(block, entry.name, locale, docsPrefix)) {
            files.push({ path: alias, markdown: parsed.body + "\n" });
          }
        }
      }
      // `group` on EVERY entry: the contract's group level is all-or-nothing, and an entry without
      // one while `groups` is declared is a throw on the consumer side (gate G2). `source` is the
      // one thing the two trees are not interchangeable in — see DOCS_BLOCKS_DIR.
      nav.push({
        slug,
        title,
        summary: Object.fromEntries(locales.map((l) => [l, summary[l]])),
        url,
        group: block.id,
        source: entry.source,
      });
    }
  }

  // With the nav built, hold every intra-guide cross-reference against it (see `checkCrossLinks`).
  checkCrossLinks(
    bodies,
    new Set(nav.map((n) => n.slug)),
  );

  // …and last, the conventions the landing's interpreter stands on: they are claims about the
  // FINISHED nav, so they can only be asked once it exists.
  assertConventions(groups, nav);

  const manifest = {
    contract: "1",
    locales: { default: locales[0], required: locales },
    // Additive (cyb7.93q6): `contract` stays "1" and a renderer without group support renders flat.
    // Omitted entirely when nothing is grouped, so a flat feed stays byte-identical to before.
    ...(groups.length > 0 ? { groups } : {}),
    nav,
    pages,
  };
  return { manifest, files };
}

/**
 * The legacy object keys one topic must ALSO be published under, so deep links that predate the
 * slug rename keep resolving (6j6v.9e3r DoD).
 *
 * flow's eight topics were published as `/nxs/docs/<locale>/<topic>.md` for the life of the docs
 * feed; the rename to `nxf-<topic>` would 404 every one of them. They are therefore written a
 * SECOND time under their old key, byte-identical. The alias is a file, not a manifest entry: it
 * never appears in `nav` or `pages`, so it cannot collide with a slug, cannot be rendered twice,
 * and costs the landing nothing — it exists purely so an already-published URL keeps answering.
 *
 * Scope, stated plainly: this repo owns the OBJECT urls and fixes them here. It does not own the
 * page ANCHOR (`/open-source/docs#getting-started`) — a fragment never reaches a server, so only
 * the landing shell can map an old one, and that is recorded for cyb7 rather than faked here.
 */
export function legacyAliases(block, topic, locale, docsPrefix = "docs") {
  if (!block.legacyUnprefixed) return [];
  return [`${docsPrefix}/${locale}/${topic}.md`];
}
