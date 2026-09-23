# nexus-memory — Product Vision

> Status: Vision · As of: 2026-06-22
>
> **Superseded in parts (2026-09-21).** Kept as written in June. The suite it calls "the nxs
> platform" with "three products" is, in the current wording of the [README](../../README.md), one
> binary with three tools: the board, the memory and the channel your agents work from.
>
> This vision describes the **purpose, scope, and positioning** of nexus-memory. It is
> deliberately narrative and conceptual — not a technical spec. The detailed modeling
> lives in the [Memory data-model spec](../specs/nxs-memory-datamodel.md) and in the
> [nxs platform foundation](../specs/nxs-platform-foundation.md).
>
> Origin: This file rescues the original seed vision (which emerged from comparing
> `bd prime` with `nxf prime`) into the nexus-flow monorepo and **corrects its
> framing**: nexus-memory is not a standalone sibling repo, but the
> **memory module of the nxs platform** — on the same substrate, in the same DB. The
> bulk of what is described here is already built (Phase 2, Epic `aye.3`).

## In one sentence

**nexus-memory is an agent-native, offline-first memory layer: keyed, durable
insights that survive compaction, `/clear`, and new sessions, primed automatically at
session start and synced across machines.** It is the standalone home for the
persistent-knowledge capability that deliberately does *not* belong in a task engine.

## The problem (the gap we found)

Two observations from comparing beads' `bd prime` with nexus-flow's `nxf prime` —
the **prime-memory asymmetry**, empirically motivated by the nxf-vs-bd benchmark:

1. **nexus-flow is deliberately narrow.** Its vision is explicit: an "engine for projects
   and tasks — and nothing else (not the agent, not brainstorming)". Persistent
   agent knowledge that is *not* a task has no home there; forcing it in would erode
   exactly that focus.
2. **beads already proved the pattern with `bd remember`.** Keyed, persistent
   memories are surfaced automatically at session start via `bd prime`:
   each memory is a durable insight, updatable in-place by key,
   searchable (`bd memories <kw>`), and removable (`bd forget`). That is real value —
   but it is *agent memory*, not a task, so in a clean design it belongs in
   its own product.

nexus-memory is that product: the memory capability, extracted from the task engine
and treated as first-class and standalone.

## What nexus-memory is — and is not

The sharpest design decision is a **boundary**:

**Insights are memory. Tasks are work.**

nexus-memory is the agent's durable knowledge layer. It is explicitly **not**:

- the task tracker — tasks, dependencies, and ready/blocked derivation are the concern of
  [nexus-flow](nexus-flow.md),
- an agent/chat runtime — nexus-memory stores distilled, durable insight,
  not a raw conversation log; agent-to-agent communication is [nexus-chat](nexus-chat.md),
- a replacement for scattered `MEMORY.md` files — this is precisely the anti-pattern
  (fragmentation across accounts/machines) it aims to retire, not reproduce.

## The core model

Three properties, carried over from what we learned with `bd remember` — and already
implemented as verbs in nxm (`remember` / `recall` / `memories` / `forget`):

- **Keyed entries.** Every memory has a stable key; writing the same key
  updates in-place instead of fragmenting (the `bd remember --key` lesson). The
  anti-pattern to avoid is scattered `MEMORY.md` files.
- **Primed at session start.** `nxm prime` surfaces the relevant memories at the top
  of a session (hook-wired in the host — and, via the nxs platform, captured in *one*
  shared `nxs prime` hook together with `nxf prime`). That is the whole
  point: memory you don't have to ask for.
- **Searchable + forgetful.** Find by keyword; remove deliberately. Persistence you
  don't need beats lost context — but stale memory must be prunable.

## Architecture: a module of the nxs platform

This is where the architecture as built diverges most from the original seed vision —
and that is the most important correction. The seed vision conceived nexus-memory as a
**sibling product in its own repo**, "from the same design lineage" as nexus-flow,
sharing the *same family* of sync substrate. The nxs platform decision (D1–D8,
Epic `aye`) discarded that framing and replaced it with something sharper:

**nexus-flow becomes the platform `nxs`, which carries flow/memory/chat as independent
products (their own binaries, versions, releases) on *one* shared CRDT substrate, *one*
local DB (`.nxs/`), and *one* sync.**

Concretely, for memory this means:

- **No substrate of its own, but the same one.** No more "same family of sync" —
  literally the same kind-tagged op-log. Memory is a **`fact` reducer**
  (`domain=fact` → `memories` view), orthogonal to flow's task reducer. Reducer = the
  data axis; a product adds an axis, not a second repo.
- **One DB, one sync.** memory facts and flow tasks live in the same `.nxs/` DB and
  travel over the same sync (LWW + OR-set + tombstones); the server is the durable truth.
- **Its own binary, its own contract.** The crate `nexus-memory` (`crates/memory`) ships
  the `nxm` binary with the memory verbs plus contract verbs (`init` / `agent-manifest` /
  `prime`) at parity with `nxf` — and plugs into the *one* `nxs prime` fan-out via the
  manifest contract.

The remaining principles of the seed vision hold unchanged and are only *more strongly*
fulfilled by the platform: an agent-ergonomic CLI (`--json` everywhere, deterministic
output), offline-first, narrow scope (memory and nothing else).

## Implementation status

Phase 2 of the `aye` epic (memory product) is **complete**:

| Vision element | Ticket | Status |
|---|---|---|
| Data model | `aye.19` Memory data-model spec | ✓ |
| keyed entries / `fact` reducer | `aye.20` nexus-memory crate + fact reducer | ✓ |
| remember / recall / memories / forget | `aye.21` nxm memory verbs | ✓ |
| primed at session start | `aye.22` nxm contract verbs (parity with nxf) | ✓ |
| multi-module (one DB, fact + task, sync round-trip) | `aye.23` P2 acceptance gate | ✓ |
| MEMORY.md import (on-ramp) | `aye.24` nxm init: import suggestion | ○ open |

The open on-ramp (`aye.24`) closes the loop back to the core model: existing
Claude-native memories are imported into nexus-memory non-destructively and idempotently,
after which the "no MEMORY.md" rule is cleanly enforceable rather than merely
prohibitive.

## Relationship to nexus-flow and nexus-chat

Three products, one platform, sharp boundaries. nexus-flow tells the agent *what* to
work on; [nexus-chat](nexus-chat.md) tells it *with whom*; nexus-memory tells it *what we
have already learned*. A host can wire multiple primes at session start — via the
platform, even as *one* `nxs prime` fan-out: flow surfaces the work graph,
memory surfaces the durable knowledge. Neither absorbs the other — that separation is
exactly the design.
