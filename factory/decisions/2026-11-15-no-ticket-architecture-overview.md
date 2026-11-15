# 2026-11-15 · no ticket · nexus-flow is an agent-native, offline-first suite — the board, the memory and the channel your agents work from — and nothing else.

- Decided by: recorded in the project memory (`nxm`, key `architecture-overview`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: architecture, v1

## The record

nexus-flow is an agent-native, offline-first suite: **the board (`nxf`), the memory (`nxm`) and the
channel (`nxc`) your agents work from** (and nothing else — not the agent, not brainstorming, not
quality gates). The public positioning is the README's (owner decision 2026-09-21, nxf 6j6v.psre):
an issue tracker for coding agents, with the memory and the channel beside it. The architecture is
`docs/specs/nxs-platform-foundation.md`; `docs/vision/nexus-flow.md` is the June vision, superseded
in parts (its own note says where).

**The `nxs` umbrella is the front door.** One **multicall binary** serves four personas, dispatched
by `argv[0]`:

- **`nxs`** — the umbrella: `nxs init` (pick which tools to set up + assemble the shared
  agent files), `nxs prime` (the fan-out you run by hand), `nxs guide`, `nxs
  migrate`/`doctor`/`status`, `nxs setup` (host integration), `nxs sync` (incl. the background
  daemon), `nxs mcp`, `nxs self-update`. `nxf`/`nxm`/`nxc` are personas of the *same* binary
  (`nxs flow …` = `nxf …`, `nxs memory …` = `nxm …`).
- **`nxf` (flow)** — the projects/tasks engine: items (the `type` is **plugin-declared**, not a
  core enum), dependencies + parent/contributes-to edges, due/defer, history + closing comment,
  and the derivation layer (ready/blocked/next as deterministic computation, not stored state).
- **`nxm` (memory)** — durable agent facts recalled across sessions (`remember`/`recall`/`forget`),
  its own `fact` op-log folded over the same substrate.
- **`nxc` (chat)** — the channel between you and your agents: `send --to`, `reply --thread`, escalation, channels
  and personas, with the agent sidecar compiled into the binary.

**THE CORE SUBSTRATE, one level down** (moved here on 2026-09-18 from [build-test](2026-11-15-no-ticket-build-test.md), where an
architecture description had no business sitting): the production engine core is `crates/core`
(`nexus-flow-core`), and it is an **op-log CRDT substrate**. An append-only `ops` table is the
single source of truth; a write appends an op and folds it into materialized views (`items`,
`edge_adds`/`edge_removes`, `note_tombstones`, `notes`) with keep-if-beats LWW plus an
observed-remove OR-set. Derivation (`ready`/`blocked`/`next`) and the invariants are pure SQL over
those views — which is why the work list is always consistent and never needs repairing. The
bake-off that chose this design is `docs/specs/E0-sync-substrate-spike.md`, the model itself
`docs/specs/E1-core-data-model.md`. `automerge` is a **dev-dependency only** (the differential
oracle) and is never in the binary.

Modules **self-register** (`inventory`) into the roster `nxs init`/`prime` read, so adding a
product needs no umbrella change. One shared **`.nxs/` workspace** holds a single op-log CRDT
substrate db that every active module folds its own views over. The host runs **one SessionStart hook
per active module** (`nxf prime`, `nxm prime`, `nxc prime` — since nxf n2m6 + a2a1), not one
`nxs prime`.

- **Plugins** (per consumer, under flow): vocabulary mapping, ranking policy, the type system, and
  presentation — `issue-tracker` and `personal-todo` are bundled.
- **Consumers**: `manufakt.io` (coding-agent platform), `nexflow.it` (knowledge-worker PM). Both
  bundle the suite invisibly ("powered by nexus-flow").
- **Sync**: full offline-first — structure via a **hand-rolled, narrow relational CRDT on plain
  SQLite** (LWW + OR-set + tombstones), chosen by the E0 bake-off; **Automerge is kept as a
  differential-test oracle** (not in the binary). The prose body rides the **same LWW-longtext
  substrate field** and converges through the normal op sync — the earlier char-level Yrs/Y.Doc
  body backend was cancelled (2026-06-16) and Yrs is not in the binary. The server is the durable
  truth.

**Crate map** (the workspace members, re-verified 2026-09-18): `foundation` (shared substrate:
workspace/schema/manifest) · `core` (`nexus-flow-core` engine) · `facade` (the embedding API +
SemVer contract) · `cli` (the `nxf` surface) · `memory` (`nxm`) · `chat` (`nxc`) · `nxs` (umbrella
binary + library) · `nxs-init` (shared self-registration registry + agent-file assembler) ·
`nxs-ui` (presentation) · `guide` (`nxs-guide`, the one guide mechanism all blocks are served
through) · `sync` · `service` (the background sync daemon) · `server` (the relay) ·
`plugin-probe` · `test-support` · plus `xtask`. `examples/tauri-board/src-tauri` is deliberately
OUT of the workspace (an `exclude` entry, so CI never builds Tauri + WebKit) and is checked by its
own workflow.

The roadmap went `E0` spike → `E1` core → `E2` CLI → `E3` plugin system → `E4` sync → the **`nxs`
platform** (umbrella + memory + unified onboarding) → **nexus-chat**, which has shipped: its own
crate, persona, guide tree and embedded sidecar. This repo dogfoods the engine for its own issue
tracking (migrated off beads on 2026-07-06); run `nxs prime` (or `nxf next`) for the current entry
point.
