// The suite story, in federated-content form (nxsflow.com migration §4/§7, 6j6v.0a6p). It was
// the /open-source page until that page dissolved into nxsflow.com's homepage (2026-09), which
// now renders its suite-breakdown and feature-grid sections. This is the *what* (copy, data,
// order); the landing shell owns the *how* (layout, components) — so design stays uniform across
// federated products by construction.
//
// Source of truth for the narrative: this repo's README ("One binary, three tools") — one `nxs`
// binary carrying the board, the memory and the channel (flow, memory, chat) on one
// offline-first CRDT substrate. Keep this copy in step with the README; the terminal-demo
// transcript is NOT here — it is the CI-drift-guarded capture in data/terminal-demo.json.
//
// The assembler (lib/content.mjs) turns this into a schema-valid `content.json`: per-locale
// `sections` in a fixed order, discriminated on `type`. Adding a locale = add a key here and
// list it in LOCALES.required/optional; adding a section type is an additive contract change.

/** Suite identity. `product`/`slug` are the landing's handles for this feed (the `/open-source`
 *  page itself is gone, see above); the story represents the whole suite, not just flow (§4). */
export const PRODUCT = "nxs";
export const SLUG = "open-source";

/** Locale gates the landing enforces fail-closed: a missing `required` locale fails its build. */
export const LOCALES = { default: "en", required: ["en", "de"], optional: [] };

/** Machine endpoints (locale-free, §4.3) — display/link targets, never executed by the shell. */
export const INSTALL_CMD = "curl -fsSL https://nxsflow.com/nxs/install.sh | sh";
export const LATEST_ENDPOINT = "/nxs/latest";
export const RELEASE_FEED = "/nxs/release-notes.json";
export const REPO = { label: "nxsflow/nexus-flow", href: "https://github.com/nxsflow/nexus-flow" };

/** Per-locale copy. The assembler reads `hero`, `terminalDemo` (chrome only — transcript is
 *  injected), `featureGrid`, `changelog` (heading only — entries injected), `poweredBy`,
 *  `downloadCta`, plus `meta`. */
export const STORY = {
  en: {
    meta: {
      title: "nexus-flow — the open-source nxs suite",
      description:
        "The board, the memory and the channel your agents work from — offline-first, fast locally, convergent when connected.",
    },
    hero: {
      kicker: "The nxs open-source suite",
      headline: "The record your agents work from.",
      sub: "One binary, three tools — the board, the memory and the channel — offline-first, fast locally, convergent when connected.",
      install: INSTALL_CMD,
      platforms: "macOS & Linux · Apple Silicon and x86-64 · signed, same-origin updates",
    },
    suiteBreakdown: {
      kicker: "One binary, three tools",
      title: "What is inside the suite",
      // Deliberately NOT the "one database, one sync, one change-log" line — feature-grid 06
      // two sections down already carries it. This one says the part that is only true here:
      // install once, activate per workspace.
      body: "One binary carries all three. Install it once; each workspace then activates only the products it actually needs.",
      items: [
        {
          name: "flow",
          command: "nxf",
          tagline: "the board",
          body: "Issues, epics and the dependencies between them. ready, blocked and next are derived from the graph on every query — never stored, so the work list cannot go stale.",
        },
        {
          name: "memory",
          command: "nxm",
          tagline: "the memory",
          body: "Durable project facts under stable keys — a fact is evolved in place instead of duplicated, and replayed into every session that needs it.",
        },
        {
          name: "chat",
          command: "nxc",
          tagline: "the channel",
          body: "Messages between you and your agents, on the record in the same change-log: a message hands the work on, and the answer comes back to whoever asked — no scraping each other's output.",
        },
      ],
    },
    terminalDemo: {
      kicker: "See it work",
      title: "next and blocked — derived, not stored",
      label: "nxf — agent session",
      caption:
        "Real output captured from nxf and drift-guarded in CI: the ready work, the work the graph holds back, and the same query as --json.",
    },
    featureGrid: {
      kicker: "Why it's built this way",
      title: "An engine agents can trust",
      items: [
        {
          index: "01",
          title: "Offline-first by default",
          body: "Everything lives in a local database, so reads and writes are instant and keep working when the network is gone.",
        },
        {
          index: "02",
          title: "Convergent CRDT sync",
          body: "Each change is appended to a change-log, never overwritten. When devices reconnect, every one folds to the same result: edits to different fields both survive, and two edits to the same field go to the same winner everywhere.",
        },
        {
          index: "03",
          title: "Derived, never stored",
          body: "ready, blocked, and next are computed deterministically from the dependency graph — so the work list is always consistent, never a stale cache.",
        },
        {
          index: "04",
          title: "Plugin-declared vocabulary",
          body: "Item types, ranking, and presentation live in plugins — an issue tracker and a personal to-do ship in the box; you can declare your own.",
        },
        {
          index: "05",
          title: "--json everywhere",
          body: "A deterministic, byte-stable machine surface on every command — the contract agents build on, and the measuring stick for the whole CLI.",
        },
        {
          index: "06",
          title: "One binary, one substrate",
          body: "flow, memory, and chat share one local database, one sync, and one change-log — so an agent's plan, its memory, and its messages converge as one truth.",
        },
      ],
    },
    changelog: {
      kicker: "Changelog",
      title: "What's new",
    },
    poweredBy: {
      kicker: "In production",
      title: "Powered by nexus-flow",
      body: "Two products bundle the suite invisibly and put their own surface on top.",
      items: [
        {
          name: "manufakt.io",
          href: "https://manufakt.io",
          description: "Coding-agent platform — you lead, the agents ship.",
        },
        {
          name: "nexflow.it",
          href: "https://nexflow.it",
          description: "Project management for knowledge work.",
        },
      ],
    },
    downloadCta: {
      kicker: "Get started",
      title: "Install the suite",
      body: "One artifact to install, verify, and update — signed and same-origin. Detects your platform and drops nxs (with nxf/nxm/nxc) on your PATH.",
      label: "Download the latest release",
      endpoint: LATEST_ENDPOINT,
      platforms: ["macOS (Apple Silicon)", "macOS (Intel)", "Linux (x86-64)", "Linux (aarch64)"],
    },
  },

  de: {
    meta: {
      title: "nexus-flow — die Open-Source-Suite nxs",
      description:
        "Board, Gedächtnis und Kanal, aus denen deine Agenten arbeiten — offline-first, lokal schnell, konvergent sobald verbunden.",
    },
    hero: {
      kicker: "Die nxs Open-Source-Suite",
      headline: "Der Datenstand, aus dem deine Agenten arbeiten.",
      sub: "Ein Binary, drei Werkzeuge — Board, Gedächtnis und Kanal — offline-first, lokal schnell, konvergent sobald verbunden.",
      install: INSTALL_CMD,
      platforms: "macOS & Linux · Apple Silicon und x86-64 · signierte, same-origin Updates",
    },
    suiteBreakdown: {
      kicker: "Ein Binary, drei Werkzeuge",
      title: "Was in der Suite steckt",
      body: "Alle drei stecken in einem Binary. Einmal installieren — jeder Workspace aktiviert dann nur, was er wirklich braucht.",
      items: [
        {
          name: "flow",
          command: "nxf",
          tagline: "das Board",
          body: "Issues, Epics und die Abhängigkeiten dazwischen. ready, blocked und next entstehen bei jeder Abfrage neu aus dem Graphen — nie gespeichert, deshalb kann die Arbeitsliste nicht veralten.",
        },
        {
          name: "memory",
          command: "nxm",
          tagline: "das Gedächtnis",
          body: "Dauerhafte Projekt-Fakten unter festen Schlüsseln — ein Fakt wächst an seiner Stelle weiter, statt sich zu vervielfachen, und liegt in jeder Session wieder vor, die ihn braucht.",
        },
        {
          name: "chat",
          command: "nxc",
          tagline: "der Kanal",
          body: "Nachrichten zwischen dir und deinen Agenten, festgehalten im selben Change-Log: Eine Nachricht übergibt die Arbeit, und die Antwort kommt bei dem an, der gefragt hat — ohne sich gegenseitig die Ausgabe abzulesen.",
        },
      ],
    },
    terminalDemo: {
      kicker: "In Aktion",
      title: "next und blocked — abgeleitet, nicht gespeichert",
      label: "nxf — Agent-Session",
      caption:
        "Echte, in CI drift-gesicherte nxf-Ausgabe: die bereite Arbeit, die vom Graphen zurückgehaltene Arbeit und dieselbe Abfrage als --json.",
    },
    featureGrid: {
      kicker: "Warum so gebaut",
      title: "Eine Engine, der Agenten vertrauen",
      items: [
        {
          index: "01",
          title: "Offline-first von Haus aus",
          body: "Alles liegt in einer lokalen Datenbank — Lesen und Schreiben sind sofort und funktionieren weiter, wenn das Netz weg ist.",
        },
        {
          index: "02",
          title: "Konvergente CRDT-Sync",
          body: "Jede Änderung wird an ein Change-Log angehängt, nie überschrieben. Verbinden sich Geräte wieder, kommt jedes zum selben Ergebnis: Edits an verschiedenen Feldern bleiben beide erhalten, und von zwei Edits am selben Feld gewinnt überall derselbe.",
        },
        {
          index: "03",
          title: "Abgeleitet, nie gespeichert",
          body: "ready, blocked und next werden deterministisch aus dem Abhängigkeitsgraphen berechnet — die Arbeitsliste ist immer konsistent, nie ein veralteter Cache.",
        },
        {
          index: "04",
          title: "Plugin-deklariertes Vokabular",
          body: "Item-Typen, Ranking und Präsentation leben in Plugins — ein Issue-Tracker und eine persönliche To-do-Liste sind dabei; eigene lassen sich deklarieren.",
        },
        {
          index: "05",
          title: "Überall --json",
          body: "Eine deterministische, byte-stabile Maschinen-Schnittstelle auf jedem Befehl — der Vertrag, auf dem Agenten bauen, und der Maßstab der ganzen CLI.",
        },
        {
          index: "06",
          title: "Ein Binary, ein Substrat",
          body: "flow, memory und chat teilen eine lokale Datenbank, eine Sync und ein Change-Log — Plan, Gedächtnis und Nachrichten eines Agenten konvergieren zu einer Wahrheit.",
        },
      ],
    },
    changelog: {
      kicker: "Changelog",
      title: "Neu",
    },
    poweredBy: {
      kicker: "Im Einsatz",
      title: "Powered by nexus-flow",
      body: "Zwei Produkte bündeln die Suite unsichtbar und setzen ihre eigene Oberfläche darauf.",
      items: [
        {
          name: "manufakt.io",
          href: "https://manufakt.io",
          description: "Coding-Agent-Plattform — du führst, die Agenten liefern.",
        },
        {
          name: "nexflow.it",
          href: "https://nexflow.it",
          description: "Projektmanagement für Wissensarbeit.",
        },
      ],
    },
    downloadCta: {
      kicker: "Loslegen",
      title: "Suite installieren",
      body: "Ein Artefakt zum Installieren, Verifizieren und Aktualisieren — signiert und same-origin. Erkennt deine Plattform und legt nxs (mit nxf/nxm/nxc) in deinen PATH.",
      label: "Neuestes Release herunterladen",
      endpoint: LATEST_ENDPOINT,
      platforms: ["macOS (Apple Silicon)", "macOS (Intel)", "Linux (x86-64)", "Linux (aarch64)"],
    },
  },
};
