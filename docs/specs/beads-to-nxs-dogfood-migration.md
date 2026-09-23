# beads → nxs dogfood migration (nexus-flow-88i.2)

On **2026-07-06** this repository's own issue tracking moved from **beads** (`.beads/`, `bd`) to
**nexus-flow** (`.nxs/`, `nxf`/`nxs`) — the engine now tracks its own work, the "first consumer
replaces beads" step the product vision calls for. Run with the installed `nxs` **v0.16.0** via
`nxs init --from-beads` (the consent-gated migration, nexus-flow-6ef).

This doc is the durable record of the migration and its verification, and it captures a
**recommendation** for what happens to `.beads/` once the repo goes public. The public-going
**decisions** themselves are out of scope here — they live on `88i.3` / `88i.4` (and the naming/
history decisions `9sk0` / `gax1`).

## What was migrated

`nxs init --from-beads` ran `bd export --all`, backed it up verbatim, set up flow (issue-tracker
plugin) + memory, imported the tickets and memories, and rolled back beads' *managed* config. The
`--json` report, checked against an independent `bd export --all` baseline captured beforehand:

| Metric | beads baseline | imported | notes |
|---|---:|---:|---|
| issues | 429 | 429 | 332 closed · 3 in progress · 94 open — all match |
| closed reasons | — | 7 defaulted | closed tickets with no recorded reason got a default closing comment |
| parent edges | 290 | 290 | |
| dependency (blocks) edges | 260 | 259 | 1 skipped — see *dropped edges* below |
| reference (mention) edges | 55 | 54 | 1 skipped — see *dropped edges* below |
| label applications | 125 | 125 | incl. `m:labels` on this very ticket (fresh from h89s.4) |
| memories | 24 | 24 | carried into `nxm` |
| defer / due dates | 0 | 0 | none in the source; nothing to carry |

**Derivation matches beads exactly.** After import the flow board reports **82 ready**, **12
blocked**, **0 deferred** — identical to `bd stats` (Ready 82, Blocked 12). The migration ticket
itself (`88i.2` → flow `6j6v.6byj`) kept its label (`m:labels`), its parent (the `88i` epic →
`fasc`), its dependency on the closed `h89s.1`, and its `in_progress` status.

Note: the importer mints **fresh flow ids** (a beads id → flow id translation table drives the edge
rewiring); it does not preserve the `nexus-flow-<x>` ids. The old ids survive as provenance in the
backup and in ticket bodies/notes.

### Dropped edges (2, expected)

Two edges could not be carried, and the migration surfaced both loudly (never silently):

- `76u.15 -[blocks]-> kz8`
- `951 -[discovered-from]-> aye.14`

Both are **pre-existing beads data quirks**: the `depends_on_id` was stored *without* the
`nexus-flow-` prefix (bare `kz8` / `aye.14`), so it matched no issue id. The intended targets do
exist in beads (`nexus-flow-kz8`, `nexus-flow-aye.14`), but the malformed edge is unresolvable, so
the importer skipped it rather than wiring a dangling edge.

Not re-created, on purpose: `76u.15` and `kz8` are both **closed** (a blocks edge between two closed
items has no effect on derivation), and `951 -[discovered-from]-> aye.14` is a non-blocking
historical *mention* to a closed item. Both are recoverable from the backup if ever wanted.

## Config rollback (agent files + hook)

- `.claude/settings.json`: the `bd prime` hooks were removed (SessionStart **and** PreCompact) and a
  single **`nxs prime`** SessionStart hook wired, plus the `Bash(nxs|nxf|nxm:*)` allowlist. This is
  the canonical nxs setup — SessionStart only. beads primed on PreCompact too; nxs deliberately does
  not, because the host re-runs the SessionStart prime on a new context window / after a compaction
  (the prime output says as much). No PreCompact hook was re-added, to keep the dogfood repo on the
  exact setup `nxs init` ships.
- `AGENTS.md` / `CLAUDE.md`: the managed `<!-- BEGIN BEADS INTEGRATION -->` blocks were stripped and
  the single managed `<!-- BEGIN NEXUS -->` pointer written. **Hand-written** bd references that lived
  *outside* the managed blocks (the AGENTS.md preamble, a stray `bd ready` in CLAUDE.md) were replaced
  by hand as part of this ticket — the migration only ever touches its own managed markers.

## Backup & safety

`bd export --all` was written verbatim to `.beads/migration-backups/beads-export-<timestamp>.jsonl`
before any mutation, so **the migration itself deleted nothing** — it backed up, imported, and rolled
back only beads' *managed config*. The migration step is non-destructive.

**Follow-up (2026-07-07): the `.beads/` tracker was then removed** from the repo (recommendation 1
below, decided + carried out by Carsten) — the repo tracks with nxf now and beads is retired. This is
a *separate* step after the migration, not part of it. Nothing is lost: every ticket is in `.nxs/`
(round-trip verified above), and the pre-migration beads database remains durably recoverable from
the git remote's **`refs/dolt/data`** ref (beads' Dolt sync — present on `origin`). The local
`.beads/migration-backups/` export was a redundant convenience copy and is deliberately not committed
(it was a ~1.1 MB blob).

## Recommendation for `.beads/` in the public repo (recommendation 1 now done)

The nxs workspace is **`.nxs/`, and it is git-ignored in its entirety** (`.nxs/.gitignore` is `*`):
the db (`db.sqlite`) and its config never enter git. By design the board is local + synced through a
relay ("the server is the durable truth"), not committed like beads' passive `.beads/issues.jsonl`
export. Consequence today: the migrated board lives only on this machine; no sync stream is bound.

Against that backdrop, the recommendation for going public:

1. **Remove the tracked `.beads/` artifacts from the public repo.** ✅ **Done (2026-07-07).** git
   tracked ~10 beads files (`config.yaml`, `metadata.json`, `interactions.jsonl`, the git hooks,
   `README.md`, …) that reference the old tracker + the internal Dolt remote and add nothing for an
   external reader; they were removed. Provenance is preserved off to the side (the pre-migration
   board is recoverable from the remote's `refs/dolt/data` ref), so no copy was committed.
2. **Do not ship the internal board in the public repo.** `.nxs/` is already git-ignored, which is
   the right default for OSS — the internal task board should not be public. External contributors
   use GitHub Issues; the internal nxs board stays local/relay.
3. **Bind a sync stream** (`nxf sync bind`) if the board must outlive this machine or be shared
   across the team — otherwise it is single-device. This is the natural companion to the
   public-going decisions and is where the "durable truth" actually lands.

Recommendation 1 (strip `.beads/`) has been carried out. Recommendations 2–3 — not shipping the
internal board and whether to bind a sync stream before or after flipping visibility — remain the
calls on `88i.3` / `88i.4`.
