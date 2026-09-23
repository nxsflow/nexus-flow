// Assemble the federated `content.json` for /open-source from the story copy + live inputs
// (the drift-guarded terminal transcript and the release-notes feed). Pure: no fs, no network —
// so the fail-closed shape unit-tests in plain node. The wire shape is the landing's
// content-contract v1 (validated against the published JSON schema in CI, fail-closed at source).

import { STORY, PRODUCT, SLUG, LOCALES, RELEASE_FEED, REPO } from "../data/story.mjs";
import { recentEntries } from "./releaseNotes.mjs";

/** All locales to emit content for: default ∪ required ∪ optional, de-duplicated, default first. */
function allLocales() {
  return [...new Set([LOCALES.default, ...LOCALES.required, ...(LOCALES.optional ?? [])])];
}

/** Flatten the drift-guarded demo ({session:[{cmd,out}]}) into the contract's transcript
 *  ([{kind:"cmd",text},{kind:"out",text}] per shown command). */
export function terminalTranscript(demo) {
  const session = Array.isArray(demo && demo.session) ? demo.session : [];
  const lines = [];
  for (const b of session) {
    lines.push({ kind: "cmd", text: String(b.cmd ?? "") });
    lines.push({ kind: "out", text: String(b.out ?? "") });
  }
  return lines;
}

function sectionsFor(locale, { transcript, entries }) {
  const s = STORY[locale];
  return [
    {
      type: "hero",
      kicker: s.hero.kicker,
      headline: s.hero.headline,
      sub: s.hero.sub,
      install: s.hero.install,
      platforms: s.hero.platforms,
      repo: REPO,
    },
    {
      // Directly after the hero, whose sub names the three tools — this is what they are
      // (landing spec 2026-07-30 `cyb7.0n93`, returned here as 6j6v.bced). `note` is optional
      // per item and only emitted when the copy carries one.
      type: "suite-breakdown",
      kicker: s.suiteBreakdown.kicker,
      title: s.suiteBreakdown.title,
      body: s.suiteBreakdown.body,
      items: s.suiteBreakdown.items.map((i) => ({
        name: i.name,
        command: i.command,
        tagline: i.tagline,
        body: i.body,
        ...(i.note ? { note: i.note } : {}),
      })),
    },
    {
      type: "terminal-demo",
      kicker: s.terminalDemo.kicker,
      title: s.terminalDemo.title,
      label: s.terminalDemo.label,
      caption: s.terminalDemo.caption,
      transcript,
    },
    {
      type: "feature-grid",
      kicker: s.featureGrid.kicker,
      title: s.featureGrid.title,
      items: s.featureGrid.items.map((i) => ({ index: i.index, title: i.title, body: i.body })),
    },
    {
      type: "changelog",
      kicker: s.changelog.kicker,
      title: s.changelog.title,
      feed: RELEASE_FEED,
      entries,
    },
    {
      type: "powered-by",
      kicker: s.poweredBy.kicker,
      title: s.poweredBy.title,
      body: s.poweredBy.body,
      items: s.poweredBy.items.map((i) => ({ name: i.name, href: i.href, description: i.description })),
    },
    {
      type: "download-cta",
      kicker: s.downloadCta.kicker,
      title: s.downloadCta.title,
      body: s.downloadCta.body,
      label: s.downloadCta.label,
      endpoint: s.downloadCta.endpoint,
      platforms: s.downloadCta.platforms,
    },
  ];
}

/**
 * Build the content manifest.
 * @param {{demo:any, releaseNotes:any, channel?:string, fallbackChannel?:string|null,
 *          limit?:number, docsRef?:string}} opts
 */
export function buildContentManifest({
  demo,
  releaseNotes,
  channel = "stable",
  fallbackChannel = "beta",
  limit = 5,
  docsRef,
} = {}) {
  const transcript = terminalTranscript(demo);

  const content = {};
  for (const locale of allLocales()) {
    let entries = recentEntries(releaseNotes, { channel, limit, locale });
    // Pre-first-stable: fall back to the beta ring so the changelog section is never empty.
    if (entries.length === 0 && fallbackChannel) {
      entries = recentEntries(releaseNotes, { channel: fallbackChannel, limit, locale });
    }
    content[locale] = {
      meta: { ...STORY[locale].meta },
      sections: sectionsFor(locale, { transcript, entries }),
    };
  }

  return {
    contract: "1",
    product: PRODUCT,
    slug: SLUG,
    locales: LOCALES,
    content,
    // `docs` is a top-level feed ref (§4.4); absent ⇒ the landing renders no /open-source/docs.
    ...(typeof docsRef === "string" && docsRef ? { docs: docsRef } : {}),
  };
}
