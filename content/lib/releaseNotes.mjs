// Select recent changelog entries for a ring, localized into the content-contract's
// changelog-entry shape ({version, date?, items:[{type, text}]}).
//
// The feed (release-notes.json) carries every ring with per-item {en, de}; the landing's
// changelog section renders the inline `entries` (live-fetching the feed is a later landing
// enhancement). Newest-first is the feed's own order (the release/promote pipeline prepends).

/** @param {any} releaseNotes  parsed release-notes.json ({channels: {alpha,beta,stable}})
 *  @param {{channel?:string, limit?:number, locale?:string}} opts */
export function recentEntries(releaseNotes, { channel = "stable", limit = 5, locale = "en" } = {}) {
  const channels = (releaseNotes && releaseNotes.channels) || {};
  const list = Array.isArray(channels[channel]) ? channels[channel] : [];
  return list.slice(0, Math.max(0, limit)).map((e) => {
    const items = Array.isArray(e.items)
      ? e.items
          .map((it) => ({ type: String(it.type ?? ""), text: String(it[locale] ?? it.en ?? "") }))
          .filter((it) => it.text !== "")
      : [];
    return {
      version: String(e.version),
      ...(e.date ? { date: String(e.date) } : {}),
      items,
    };
  });
}
