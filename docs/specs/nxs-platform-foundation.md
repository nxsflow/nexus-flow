# nxs — Platform Foundation: shared substrate, one DB, three products (design spec)

> Status: Design · As of: 2026-06-17 · Reference: `docs/specs/E1-core-data-model.md`
> (op-log substrate), `docs/specs/E4-sync-protocol-server.md` (sync), `crates/facade`
> (E5 / PR #71 — the embedding seam to be split), vision docs
> `../vision/nexus-memory.md` and `../vision/nexus-chat.md`.
> This spec is an **architecture + roadmap decision (ADR-style)**: it establishes that
> nexus-flow becomes the **platform `nxs`**, which carries flow/memory/chat as independent
> products on *one* shared CRDT substrate, *one* local DB and *one* sync. The actual
> implementation proceeds in phases (§9); the first plan is scoped to **P0** (§10).

## 1. Goal & scope

nexus-flow today is a single product (engine for projects & tasks, binary `nxf`,
v0.3.1, shipping to customers). Alongside it exist two **pure product visions without code**:
**nexus-memory** (persistent memory layer, binary `nxm`) and
**nexus-chat** (agent-to-agent messaging, binary `nxc`). All three are the same substrate in
different vocabulary: append-only op-log → materialized views → LWW + OR-set +
tombstones → CRDT sync on SQLite. flow calls it "tasks", memory "facts", chat "messages".

This spec decides: the three become **one composing suite under the platform umbrella
`nxs`**, with a shared foundation. Because two of the three are only visions, this is the
cheapest possible moment — we are not merging code, we are choosing the birthplace.

**In this spec:**

- The core decisions (§3) and their trade-offs (§8).
- The data architecture: *one* kind-tagged op-log, reducer registry, *one* sync (§4).
- The target crate topology (§5).
- The CLI surface & contracts: foundation-as-library, manifest contract, `nxs` verbs,
  `nxf init` refactor (§6).
- The `.nxs/` directory layout and the migration from `.nexusflow/` (§7).
- The decomposition into phases P0–P4 and the scope of the first plan (§9, §10).

**Not in this spec:**

- The detailed implementation of the foundation extraction → its own P0 plan (§10).
- The memory product data model (fact slots, views) → its own spec with P2.
- The chat product (daemon, event loop, agent-to-agent) → its own spec, later (P4).
- Concrete release/CI consolidation across the three products → follows P0/P1, builds on
  `docs/specs/release-management.md`.

## 2. Motivation & context

Three observations carry the decision:

1. **Shared sync is the real win.** Sync is the most expensive, most error-prone
   part of the system (that is what the E0 bake-off was for). Three products that share
   *one* op-log sync, *one* replica identity, *one* offline/online path instead of three is
   by far the largest operational simplification. Cross-domain references (msg → task → fact)
   come for free in the process: one consistent snapshot, one transaction — a *product*
   advantage for agent-native tools that collaborate, not mere plumbing.

2. **The timing is ideal.** Two of three products exist only as docs. No
   migration, no duplicate cores to merge. The cheapest moment.

3. **A shared, generic core protects flow's boundary.** nexus-flow's vision is deliberately
   narrow ("engine for projects and tasks — and nothing else"). If the core *must* serve
   flow + memory + chat from day one, it stays honestly generic instead of quietly accumulating
   flow domain knowledge.

## 3. Core decisions

| # | Decision | Essence |
|---|---|---|
| D1 | **Platform, not product repo** | The monorepo *is* the platform `nxs`; nexus-flow becomes *one* member, not the root identity. |
| D2 | **Shared substrate + shared DB + shared sync** | Refinement of "approach C": products are independent in **binary/version/release**, but coupled in **data/schema**. |
| D3 | **One kind-tagged op-log (granularity 1)** | *One* `ops` table; every op carries `kind` (`task`/`fact`/`message`). No op-log per module. |
| D4 | **Reducer registry** | The foundation supplies op-log + sync + OR-set/LWW/tombstone primitives; each module registers a **reducer** for its kinds, which folds into its own views. flow contributes *one* task reducer. (The reducer is the **data** axis — orthogonal to the flow `plugin`, which is **presentation**; see §4.4.) |
| D5 | **Foundation = library** | The idempotent foundation `setup()` operation lives as a crate function; every init path calls it. No binary shell-out, no `nxs` binary needed for flow-only. |
| D6 | **Manifest contract for agent files** | Every module CLI emits `<cli> agent-manifest --json`; `nxs` assembles AGENTS.md from these (CLAUDE.md left the set 2026-08-30, nxf `6j6v.q6e3` — see §6.2) and sets **one SessionStart hook per active module** → the manifest's own `hook.command` (`nxf prime`, `nxm prime`, `nxc prime`). *(Was one hook → `nxs prime` until nxf n2m6 + a2a1; see §6.2 for the per-hook-output measurement.)* |
| D7 | **Directory `.nxs/`** | Shared foundation directory; migration from `.nexusflow/` (only ~3 workspaces affected). |
| D8 | **`setup` is not a user verb** | `nxs` exposes `init`/`prime` (umbrella) + `sync`/`migrate`/`doctor` (foundation-only). "Create the foundation" is done idempotently by every module init via the library. |

## 4. Data architecture

### 4.1 One kind-tagged op-log

Today the op-log (`ops` table, E1) is single-kind (tasks). In future **every op carries a
`kind` discriminator field** (`task` | `fact` | `message` | …). The log remains what it is:
append-only, single source of truth, keep-if-beats LWW + observed-remove OR-set. Sync sends
*the whole* log as *one* stream — there are no three logs sitting side by side.

### 4.2 Reducer registry

The foundation knows **no** domain. It supplies:

- the op-log (append, iterate, sync),
- the CRDT primitives (LWW register, OR-set, tombstones),
- the materialized-view plumbing,
- replica identity, `config`/`replica` TOML, **migrations**, **foundation schema version**.

Each module registers a **reducer**: "for these kinds, fold into these views like this". flow:
`task` ops → `items`/`edge_adds`/`edge_removes`/`note_tombstones`/`notes` + derivation
(`ready`/`blocked`/`next`) + invariants. memory: `fact` ops → fact views. chat: `message` ops
→ message views. The reducer is the clean, independently testable boundary between foundation
and product.

### 4.3 One sync, one compatibility axis

- **One sync** over the entire log (no per-module sync; selective per-kind sync is YAGNI).
- **One replica identity / `site_id`** per workspace.
- **One ID prefix per kind** (`t-`/`f-`/`m-`) in the shared ID space.
- **The foundation schema version** is the compatibility axis: flow v0.4 + memory v0.1 in
  *one* DB are permitted as long as both speak the same foundation schema version. Product
  versions do **not** gate compatibility — the foundation schema version does.
- **Local consumer skew = sync skew (one axis).** Because the CLI (`nxf`) and the embedding app
  (E5) share *one* file, the same compatibility axis applies to consumers of different versions
  running side by side locally as to sync peers. Load-bearing invariant:
  migrations are **additive and old-binary-safe** (new columns with `DEFAULT`, no
  drops/renames/repurposing of read columns/views, new axes dormant), so that an older
  binary can **continue to read + write a forward-migrated DB in degraded mode**; unknown op shapes
  remain *store-don't-fold* per §7 (never lost, `refold` on upgrade). A
  **no-downgrade guard** prevents stamping `PRAGMA user_version` backwards; a
  **`MIN_COMPATIBLE_SCHEMA_VERSION`** separates "continue in degraded mode" from "fail-loud: workspace
  written by a newer nxs, please upgrade". Policy + tests: `aye.1.3` (requires the
  version spine `aye.1.1`). The normative "old-binary-safe" checklist that **every**
  foundation migration must satisfy (additive-only, new columns with `DEFAULT`, new axes
  dormant, raise the floor only on a real break) lives as a comment next to `migrate()` in
  `crates/core/src/schema.rs` — the floor is persisted raise-only via `PRAGMA application_id`,
  the schema-version stamp raise-only via `PRAGMA user_version`.

### 4.4 Reducer ≠ plugin — two orthogonal seams

PR 71 (E5 `nexus-flow-facade`) makes it concrete that "the opinion-free seam above the core" exists
on *two* levels, which this spec must cleanly separate:

- **Reducer (data axis):** folds kind ops into views. One per kind/product. Lives at the
  foundation boundary.
- **Plugin (presentation axis, E3):** vocabulary, `priority` labels, `next` ranking,
  presentation *over* a product's views (`facade::plugin::PluginConfig`). **Multiple** per
  product — flow today already ships `issue-tracker` *and* `personal-todo` over the *same*
  task data.

The two are orthogonal: a product has *one* reducer (how it folds data) and *n*
plugins (how it presents them). The generalized foundation `Engine` must accordingly
**not** hardwire a single product plugin (today's `Engine` still does this,
`engine.rs:29`) — it holds the shared store; products layer a read layer + plugins on top.

## 5. Crate topology (target — refined by P0)

Today: `core`, `facade`, `cli`, `sync`, `server`, `xtask`. The `facade` introduced in PR 71 (E5)
is a deliberately dependency-light **shared seam** (engine handle + watch + workspace +
read/record/plugin) for CLI, MCP and embedding apps — it **splits** along the
foundation boundary (paragraph below the table).

Target:

| Crate | Role | Origin |
|---|---|---|
| `nxs-foundation` | substrate: op-log, kinds, reducer registry, CRDT primitives, view plumbing, replica/TOML, migrations, schema version, `setup()`, **embedding `Engine` handle + watch** | extracted from `core` + the **generic part** of `facade` (engine/watch/workspace) |
| `nxs-sync` | sync over the foundation log | today's `sync` |
| `nxs-server` | durable server | today's `server` (consolidation later) |
| `nexus-flow` | task reducer + derivation + flow read layer (records) + plugins (issue-tracker/personal-todo) + `nxf` | flow part of `core`/`facade` + `cli` |
| `nexus-memory` | fact reducer + `nxm` | **new (P2)** |
| `nexus-chat` | message reducer + `nxc` | **new (P4)** |
| `nxs` | umbrella CLI: `init`/`prime` + `sync`/`migrate`/`doctor`, agent-file assembler | **new (P3)** |
| `xtask` | build tasks | unchanged |

**The facade split is the heart of P0:** the generic part of today's `facade` —
`Engine` lifecycle (`Arc<Mutex<Store>>`, Clone+Send+Sync), the `data_version` watch, the
`.nxs/` workspace resolution — moves into `nxs-foundation` and becomes the product-agnostic
**embedding API for all products**. The flow-specific read compute (`ready`/`blocked`/
`next`/`show`/`prime`), the record shapes and the plugin configs stay with `nexus-flow`. The
foundation boundary thus runs *through* today's facade, not only through `core`.

## 6. CLI surface & contracts

### 6.1 Foundation as a library

`foundation::setup(workspace)` is the idempotent "make sure `.nxs/` exists —
DB, schema version, `replica.toml`, `config`" operation. **Every** init path calls it:
`nxf init`, `nxm init`, `nxs init`. A flow-only user therefore needs **no** `nxs` binary;
`nexus-flow` links `nxs-foundation` directly. In practice, `setup()` is the generalization of the
already dependency-light extracted `facade::workspace::init` to `.nxs/` + multi-module
config — a starting point that de-risks P0.

### 6.2 Manifest contract (agent-file integration)

The shared agent files (AGENTS.md, `.claude/settings.json`) are a
**project-/platform-wide** concern, not a module concern — they belong to `nxs`. Mechanics:

- Each module CLI implements `<cli> agent-manifest --json` and **declares** its contribution
  as data: AGENTS.md section, prime command, hook need.
- `nxs` collects the manifests of the active modules, **assembles** AGENTS.md and sets
  **one SessionStart hook per active module** → `nxf prime`, `nxm prime`, `nxc prime`. The
  settings.json integration still belongs up at `nxs`, for the reason below.

**CLAUDE.md left this list on 2026-08-30 (owner's decision, nxf `6j6v.q6e3`).** It used to be
assembled alongside AGENTS.md — the same managed block, or an `@AGENTS.md` import that pulled it
in. It is the file of the host that RUNS the hook wired in the same operation, so everything the
block carries has already arrived by the time it is read; and one of its three lines,
`@NEXUS_MEMORY.md`, is a Claude Code import directive naming the projection of the memory store's
whole body — measured 2026-08-30 at 95.359 B (nexus-flow), 110.508 B (app-foundations) and
119.625 B (manufakt-io) per session, beside a hook block of ~6 KB carrying the budgeted index. The
assembler now only ever CLEANS that file: it removes a block an earlier version wrote, and writes
nothing. AGENTS.md keeps the block unchanged — that is where the hookless reader was always meant
to find it.
- `nxs` hardcodes **no** module knowledge — it only consumes manifests. The *only* hard-wired
  module information is the **roster** (which products exist, what their binary is called:
  `flow`→`nxf`, `memory`→`nxm`, `chat`→`nxc`) — platform composition, not vocabulary.

**P3 refinement (scoping 2026-06-22) — SUPERSEDED 2026-08-28 by nxf n2m6 + a2a1, kept because a
reader has to see that it was right when it was written:**

- ~~**The hook is ALWAYS `nxs prime`** — even a flow-only workspace. One stable single hook that
  stays the same as each additional module is added (modules grow, the hook does not). The
  per-module `prime_command` / `hook.command` from the manifest (`nxf prime`, `nxm prime`) is
  **only the fan-out target** that `nxs prime` (§6.3) calls — **not** what the host hook calls
  directly.~~
- **The assembler is a shared library unit** in the `nxs` crate that **every** init path calls
  (`nxs init`, `nxf init`, `nxm init`) — whatever triggers it, the same logic, the same hooks.
  *(Unchanged. This half was never about how many hooks there are.)*

**The measurement that superseded the one-hook rule (2026-08-28, nxf q3fh):**

The host truncates SessionStart output **per hook output**, not per session. Measured over 1.114
SessionStart hook events drawn from 947 transcripts:

    largest hook output ever passed through : 10.164 B
    smallest hook output ever truncated     : 10.203 B
    ever passed through above 10.240 B      : never

The edge is 10.240 B = 10 KiB **per hook output**, and three SessionStart hooks of ~8 KB each were
observed arriving whole — 24.127 B between them. The JSON channel
`hookSpecificOutput.additionalContext` is capped identically and is therefore no way out.

One hook was strictly simpler while the budget was per session; it is strictly *worse* once the
budget is per hook, because three modules then share one 10 KiB slot instead of holding one each.
So:

- **The host hook is the per-module `hook.command`** (`nxf prime`, `nxm prime`, `nxc prime`) — one
  entry per ACTIVE module, so a module that joins brings its entry and a module that falls away
  takes it with it. TB-9's question is re-answered accordingly.
- **`nxs prime` is unchanged and stays** (§6.3) as the composed fan-out: the verb a person runs by
  hand, and what a persona's prompt is composed from. What moved is only what the HOST is wired
  to run.
- **The `|| cat NEXUS_MEMORY.md 2>/dev/null` fallback hangs on exactly one entry, memory's**
  (nxf 6j6v.1k6y). Still exactly one, for Ruling R1's own second reason: three entries each
  carrying it would deliver the same file three times into one session. But *which* one is decided
  by ownership, not position — `NEXUS_MEMORY.md` is the projection of exactly the memories
  `nxm prime` replays, so it is the one command it can stand in for. A workspace without memory
  carries it nowhere: there is no document to read.
  **Superseded 2026-08-29:** Ruling R1 said "the first wired — not 'the flow entry', because
  nothing guarantees flow is active, while 'the first' is well-defined for every module
  combination". That is a sound test for a rule's *definedness*, and it was applied to the wrong
  kind of rule: a position rule fits a tail belonging to the suite, and this one belongs to a
  module. Measured before the change: in a flow-only workspace the tail sat where the file cannot
  exist, and adding memory later did not move it.
- **The other modules' failure is left loud** (owner, 2026-08-29). A companion `|| echo "<the suite
  is not installed>"` was considered and declined: `echo` succeeds, so the hook would exit 0 and a
  host that surfaces failing hooks would fall silent. A contributor without `nxs` gets the shell's
  `command not found` and a failing hook — the honest report that nothing was delivered.
- **The hooks are written in ROSTER order, the same sequence `nxs prime` fans out in**
  (nxf 6j6v.shwz) — `nxf`, `nxc`, `nxm`, and only for modules that are active. It used to be
  `active_modules` order, which is the order `init` happened to append keys to `config.toml`: an
  accident of set-up history, measured in the field on 2026-08-29 as 15 workspaces with flow first
  and one (`manufakt-io`, config `["memory", "flow"]`) with memory first. Memory going last is the
  same reason the fan-out has it last — it is the one block a session can fetch back afterwards.
  The sort sits in `assemble_with`, the one place all four assembly paths (`nxs init`,
  `nxs setup claude`, `nxm init`, `nxc init`) reach, because this item exists because two paths
  disagreed and a rule applied per call site is one the fifth can miss. The AGENTS.md pointer's
  tool list is sorted with them, so one sequence describes the suite everywhere.
- **The wiring is rewritten in full each run, and "nothing changed" is MEASURED** (same item): the
  managed set is removed — the wanted entries included — and written back in order, then the
  document is compared with the one on disk. Excluding the wanted entries from the removal, which
  is what kept a re-run a no-op before, left a surviving entry in its old position while a
  rewritten one was appended; the order was then a residue of history rather than a decision. A
  steady-state re-run still writes nothing. A group somebody wrote by hand without `"matcher"` is
  normalized once and then stable.
- **No block may refer to another.** Hook output order is not guaranteed — a foreign hook was
  observed landing between two of ours — so each module's block has to read on its own.
  "Activating nxs" is not an extra step: it happens at the first `<mod> init` (foundation + agent
  files + hook are delegated to the shared seam).
- **TB-5 = shell-out for `prime`/`agent-manifest`** (resolved P3): collecting manifests and the
  prime fan-out call the *standalone* module binaries (`<cli> agent-manifest --json`, `<cli> prime`).
  The assembler resolves sibling binaries next to its own executable (an installed suite), otherwise
  via `PATH`. **Reversed for the init PATH by the Unified-init epic (5jz, see §6.5):** `init` now
  composes the modules as a LIBRARY (each self-registers an init entry the umbrella calls
  in-process); only `prime`/`agent-manifest` stay shell-out.
- **`nxs` owns the FRAME, not a captured sub-process.** (P3 had: modules run in a silent driven/
  `--quiet` mode whose stdout `nxs` captured + discarded.) **Superseded by 5jz.3 (§6.5):** the
  umbrella drives each chosen module's self-registered init IN-PROCESS, so the module's OWN
  interactive sub-config (flow's plugin chooser) runs VISIBLY inside the one frame instead of being
  swallowed. The module prints no aggregate summary of its own; `nxs` frames the result. The
  brand-conformant flourish (manufakt "Forge", ember red) stays **TTY-gated**: only when
  `!json && is_terminal()` — `--json`/pipes/CI/trycmd stay byte-stable plain ASCII.
- **TB-8 stays out of scope for P3** (kind-aware watch is not on the init/prime path; `nxs prime`
  is a one-shot bootstrap call, not a live watcher).

### 6.3 `nxs` verbs

| Verb | Role |
|---|---|
| `nxs init` | interactive module selection → drives each chosen module's self-registered init **in-process** (§6.5) → collects manifests → assembles agent files → sets the hook. Recognises what's already active and frames a welcome + first-command success moment (5jz.4/5jz.6) |
| `nxs prime` | fan-out: calls `prime` of the active modules in turn — with *one* consistent `now` (an explicit parameter since PR 71, `engine.rs:104`) |
| `nxs sync` | **foundation-only**: one sync over the shared log (with one log, sync is no longer a module operation) |
| `nxs migrate` | **foundation-only**: raise the shared DB to the current foundation schema version (default: auto-migrate-on-open; `migrate` as an explicit/CI lever) |
| `nxs doctor` / `status` | **foundation-only**: cross-module diagnosis (active modules, schema version, replica, sync state, DB integrity) |

`setup` does **not** exist as a user verb (D8).

### 6.4 `nxf init` refactor (touches shipping code)

Today `nxf init` writes AGENTS.md itself (managed header). In future it:

- only sets up its own state and **delegates the foundation** to
  `foundation::setup()`,
- **declares** its agent contribution via `nxf agent-manifest --json`, instead of writing the shared
  files.

If `nxf` exposes a `sync` verb today, it migrates to `nxs sync` (check in P0/P1).

### 6.5 Unified init + multicall (epic 5jz, 2026-06-24)

The Unified-init epic made `init` ONE experience regardless of which binary the user typed, and
collapsed the suite to a single shipped binary. Two structural decisions, recorded here because they
reverse/refine the P3 model above (see §6.2, TB-5, TB-10):

- **Self-registration registry (5jz.1).** The umbrella hardcodes no roster. Each module
  `inventory::submit!`s ONE `ModuleInit` descriptor (`nxs-init`) carrying everything `nxs` needs:
  roster identity (`key`/`binary`/`now_env`), chooser `blurb`, `recommended` flag, `order`,
  `accepts_plugin`, its in-process `init_fn`, and (5jz.6) its `welcome` copy + `first_command`
  success moment. A new product appears in the chooser, the prime fan-out, the welcome frame, and
  the success moment **purely by registering** — no `nxs` source change.

- **Library composition for init; shell-out stays for prime/manifest (TB-5 reversed for init,
  5jz.1/5jz.3).** `nxs` LINKS the module crates so their registrations populate the roster, and
  `nxs init` drives each chosen module's `init_fn` **in-process**. This dissolves the original #5jz
  problem (a module's interactive sub-config — flow's plugin chooser — was swallowed by the captured
  `--quiet` sub-process): it now runs VISIBLY inside the one frame. `prime`/`agent-manifest` still
  shell out to the standalone binaries (TB-5 holds there).

- **Single Front Door + entry-point preselection (5jz.2).** On a terminal, `nxf init`/`nxm init`
  re-exec the shared `nxs init` frame with the entered tool **pre-selected** (`--preselect`), so
  every entry point lands on the same suite chooser. Non-interactive (`--json`/piped/CI) stays
  module-native, flag-driven (`--module`/`--plugin`), and byte-stable — no prompt.

- **Multicall: one binary, `nxf`/`nxm` as `argv[0]` symlinks (5jz.7/5jz.8).** Only `nxs` is
  compiled; `nxf`/`nxm` are symlinks to it. The binary routes by the **`argv[0]` basename** (not
  `current_exe()`, which resolves the symlink back to `nxs`): invoked as `nxf` → the flow surface,
  `nxm` → memory, `nxs` → the umbrella PLUS hidden `nxs flow …` ≡ `nxf …` / `nxs memory …` ≡ `nxm …`
  routes intercepted before clap (so they never appear in `nxs --help`). One artifact to build /
  sign / notarize / release ⇒ **no version drift between the tools by construction** (this dissolves
  the "release coupling" the epic had otherwise had to guard). `nxf-relay` (the E4 sync relay) stays
  a separate server binary. Design: `docs/specs/plans/2026-06-24-5jz7-multicall-design.md`.

- **Installed-detection collapses to active-in-workspace (5jz.4).** With one binary every module is
  always installed, so the only meaningful dimension is whether a module is already in
  `config.active_modules`. `nxs init` surfaces that in the chooser (pre-checked + "already set up"),
  the human summary, and an additive `--json` `already_active` field, so a re-run is informative.

## 7. `.nxs/` layout & migration

```
.nxs/
  config.toml      # active modules + platform config
  replica.toml     # site_id, replica_uuid, ID prefixes per kind
  db.sqlite        # the one op-log + all module views
  .gitignore       # local state ignored, tracked files explicit
```

**Migration `.nexusflow/` → `.nxs/`:** a one-time script; additionally, the foundation reads a
legacy `.nexusflow/` on open and migrates idempotently. Only ~3 existing
workspaces are affected (nexus-flow itself, nexus-memory's tracking workspace, possibly one more) — cheap.

## 8. Trade-offs / accepted costs

- **Data coupling, not just crate coupling.** A shared DB couples more tightly than a
  shared crate. "Independent" now means precisely **release/version-independent**, *not*
  data/schema-independent. flow migration and memory migration touch the same file. —
  Verdict: here it is **correct**, not a smell. These are not three foreign products, but one
  composing suite in the same workspace; tight data coupling is intended.
- **A fourth version axis.** The foundation schema version must be explicitly designed and gated
  (§4.3), not hoped for.
- **The reducer registry is real work.** The foundation needs a
  registration mechanism — a clean, testable boundary, but effort.
- **Naming correction.** `.nexusflow` was semantically wrong (the foundation belongs to the platform,
  not flow); `.nxs/` corrects this, at the cost of the (cheap) migration from §7.
- **P0 touches shipping code.** The foundation extraction changes the production
  flow core. Safeguard: behavior must stay unchanged, the differential oracle (`automerge`,
  dev-dep) must stay green, all quality gates likewise.

## 9. Decomposition / roadmap

Too big for *one* plan. Important: **"memory is next" as a product — but the next *code*
is the foundation.** Granularity (1) requires that the kind-tagged log + the
reducer registry exist *before* memory can exist. Hence the foundation extraction is
technically upstream of the memory product.

| Phase | Content | Risk |
|---|---|---|
| **P0 — foundation extraction** | kind-tagged op-log, reducer registry, `nxs-foundation` crate (incl. generic facade part: engine/watch/workspace), `.nxs/` migration, foundation schema version. flow becomes its **own first reducer**; behavior unchanged. | **high** (shipping code) |
| **P1 — contracts** | manifest contract, `nxf init` refactor (delegation + manifest emit), `foundation::setup()` as a library seam. | medium |
| **P2 — memory product** | `nexus-memory` crate: fact reducer + views, `nxm` with `init`/`prime`/`agent-manifest`. First real test of the multi-module thesis. | medium |
| **P3 — `nxs` umbrella** | `nxs init`/`prime` (selection + fan-out + agent-file assembler) + `sync`/`migrate`/`doctor`. Only worthwhile at ≥2 modules. | low |
| **P4 — chat** | `nexus-chat` crate: message reducer, `nxc`, daemon/event loop. Own spec. | open |

## 10. Scope of the first plan

The first implementation plan following this spec is scoped to **P0 (foundation extraction)**:

- Introduce the `kind` discriminator into the op-log without changing flow behavior.
- Reducer registry mechanism; extract flow's current fold logic as the first registered reducer.
- Carve `nxs-foundation` crate out of `core` **and the generic facade part** (engine/watch/
  workspace); `nexus-flow` keeps the flow read layer + derivation + plugins
  (issue-tracker/personal-todo) + `nxf`. `Engine` may no longer hardwire a single plugin.
- `.nexusflow/` → `.nxs/` migration (script + idempotent legacy read).
- Introduce the foundation schema version.
- **Gate:** all quality gates green (`cargo test`, `cargo test --release`, `clippy`, `fmt`),
  differential oracle green, flow CLI behavior unchanged (golden tests).

## 11. Open points / tie-breaker log

| ID | Question | Provisional resolution |
|---|---|---|
| TB-1 | Exact reducer trait signature & registration | P0 plan; orient on E1 view fold |
| TB-2 | Schema version: one global counter vs. foundation + per-reducer version | P0; start with one global foundation counter, per-reducer only when needed |
| TB-3 | Auto-migrate-on-open vs. explicit `nxs migrate` only | default auto-on-open (robust), `migrate` as a CI/repair lever |
| TB-4 | `nxf sync` → `nxs sync`: does a flow sync verb exist today? | check in P0/P1; if so, move to `nxs` |
| TB-5 | `nxs` as a multi-binary shell-out vs. library composition of the products | **RESOLVED (P3, 2026-06-22): shell-out** for `prime`/`agent-manifest`. **Refined (5jz, 2026-06-24, §6.5): the init PATH is library composition** — modules self-register an in-process init the umbrella drives; `prime`/`agent-manifest` stay shell-out. The roster is no longer hardcoded: modules self-register it (5jz.1). With multicall (5jz.7) all three CLIs are one binary, so the shell-out targets `nxf`/`nxm` are `argv[0]` symlinks to `nxs`. |
| TB-6 | Release/CI consolidation of the 15 flow workflows across the platform | own follow-up spec, builds on `release-management.md` |
| TB-7 | Create beads epics/issues for P0–P4 | after spec approval (session-close protocol) |
| TB-8 | Watch with *one* DB: `PRAGMA data_version` is DB-global, does not distinguish products | **P3: stays out of scope** — `nxs prime` is a one-shot bootstrap call, not a live watcher; kind-aware watch as its own follow-up if needed. |
| TB-9 | What does the host hook call directly — `nxs prime` or the per-module `prime_command`? | ~~**RESOLVED (P3, 2026-06-22): ALWAYS `nxs prime`**, even flow-only. The manifest `prime_command` is only the fan-out target. A stable single hook across all module combinations.~~ **RE-ANSWERED 2026-08-28 (nxf n2m6 + a2a1): the per-module `hook.command`** — one SessionStart entry per ACTIVE module (`nxf prime`, `nxm prime`, `nxc prime`), with memory's carrying the `\|\| cat NEXUS_MEMORY.md` fallback (nxf 6j6v.1k6y; it was the FIRST entry's until then). Reason: the host truncates per hook OUTPUT at 10.240 B (1.114 events / 947 transcripts), so a single hook made three modules share one 10 KiB budget instead of holding one each; three ~8 KB hooks were measured arriving whole. `nxs prime` remains the composed fan-out for manual use. See §6.2. |
| TB-10 | Who owns the init presentation, and how does `nxs init` drive the modules? | **RESOLVED (P3, 2026-06-22): `nxs` owns the visible presentation.** **Superseded by 5jz.3 (2026-06-24, §6.5):** `nxs` owns the FRAME, but drives each module's init IN-PROCESS (not a captured `--quiet` sub-process), so a module's own interactive sub-config runs VISIBLY in the frame. Flourish stays TTY-gated (manufakt "Forge"). |
