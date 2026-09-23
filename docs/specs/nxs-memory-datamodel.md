# nexus-memory — product data model (`fact` domain, binary `nxm`)

> Status: Design · As of: 2026-06-20 · Phase: **P2** (nxs roadmap)
> Reference: `docs/specs/nxs-platform-foundation.md` §4 (data architecture), §5 (crate topology),
> §6 (contracts), §7 (`.nxs/` layout); the bd prior art (see [`../../README.md`](../../README.md#prior-art--acknowledgements)).
>
> This spec defines the **memory product data model** that the platform spec deliberately left
> open (§9-P2: "a dedicated product data-model spec follows with this phase"). It is the source of
> truth for the implementation tickets `aye.20`–`aye.23`.

## 1. Goal & scope

nexus-memory is the **first new product on the shared nxs foundation** — a persistent,
agent-native store for **durable project knowledge** (conventions, pitfalls, architecture facts,
tooling quirks). It is the nxs equivalent of beads' `bd remember`/`bd prime` store: a **small,
high-quality, hand-curated** body of knowledge that is automatically replayed into context at the
start of every session — a UX proven by beads.

memory is the same substrate as flow in a different vocabulary (platform §1): append-only op-log →
materialized views → keep-if-beats LWW + tombstones on SQLite. flow calls its unit "task" (domain
`task`); memory calls it **"fact"** (domain `fact`). The **fact reducer** is the clean data
boundary (platform §4.2/§4.4): it folds `fact` ops into the `memories` view and knows **no** flow
vocabulary; the foundation knows **no** memory vocabulary.

**Not in this spec:** the `nxs` umbrella (P3), the chat product (P4), semantic search / embeddings
(not a v1 topic), release/CI consolidation.

## 2. Data model — the keyed text fact

A memory is a **text fact addressed by `key`**:

| Field     | Origin              | Role |
|-----------|---------------------|-------|
| `key`     | entity ID (`target_id` of the op) | stable addressing key; equivalent to bd's `--key` |
| `body`    | LWW register (`value`) | the full text of the memory |
| `author`  | op metadata (`author`) | who wrote it last |
| `updated` | op metadata (`wall_clock`) | display timestamp of the last mutation (display-only, never order-bearing — platform §4.1) |
| `active`  | derived from the op kind | `true` = remembered, `false` = forgotten (tombstone) |
| `category`| LWW register (`value`) | which **section** the memory belongs to; `unsorted` until filed (§2.4) |
| `scope`   | LWW register (`value`) | how far the memory **reaches**: `item` · `project` · `global`; `project` by default (§2.4) |
| `refs`    | LWW register (`value`) | which **board items** the memory is about; canonical (sorted, deduplicated), possibly empty (§2.4) |
| `ordinal` | LWW register (`value`) | an explicit **reading position**; `NULL` until `reorder` writes one (§2.5) |

`key` is the identity (like bd's stable key): knowledge under the **same** key evolves in place
instead of duplicating — the most important lever against uncontrolled growth, and a pattern
proven by bd.

`author`/`updated` date the **fact**: they move with the body register only. Filing or reordering a
memory does not change what it says, so it does not touch the stamp a reader uses to judge how
current the knowledge is.

### 2.1 Op form (`fact` domain)

All memory ops carry `domain = "fact"` and `target_kind = "fact"`; the `field` selects which
register the op writes:

| Verb                    | `field`    | `op_type` | `target_id` | `value`         | Meaning |
|-------------------------|------------|-----------|-------------|-----------------|-----------|
| `remember`              | `body`     | `set`     | `key`       | `Some(<text>)`  | upsert: `body=<text>`, `active=1` |
| `forget`                | `body`     | `forget`  | `key`       | `None`          | tombstone: `body=NULL`, `active=0` |
| `remember`/`classify`   | `category` | `set`     | `key`       | `Some(<slug>)`  | file the memory under a section |
| `remember`/`classify`   | `scope`    | `set`     | `key`       | `Some(<reach>)` | set how far it reaches |
| `remember`/`classify`   | `refs`     | `set`     | `key`       | `Some("a,b")`   | set the board items it is about |
| `reorder`               | `ordinal`  | `set`     | `key`       | `Some("<n>")`   | set the reading position |

`is_foldable` ⇔ `target_kind == "fact"` **and** either `field == "body" && op_type ∈ {set, forget}`
or `field ∈ {category, scope, refs, ordinal} && op_type == set && value.is_some()`. Any other form
is *store-don't-fold* (platform §7) — never folded, never discarded, revivable via `refold`. This is
exactly what a **pre-v5 binary** does with the classification ops a v5 binary writes: it keeps them
in the log and folds them the moment it is upgraded.

### 2.2 ID prefix `f-` and auto-key

The **fact-domain ID prefix is `f-`** (platform §4.3, "one ID prefix per kind"). It applies to
**system-minted** keys, analogous to how flow namespaces its minted task IDs with the replica
prefix (flow mints `ab12.7x3k`, not `t-…` — the kind prefix is a convention for minted IDs, not a
hard namespace on every domain ID):

- **Explicit `--key K`:** `key = K` **verbatim** (parity with bd — the human chooses speaking,
  stable handles like `auth-jwt`). This is the primary growth lever.
- **Missing `--key` (auto-key):** `key = "f-" + hex(sha256(body))[..16]`. Consequence:
  **byte-identical** content yields the **same** key → deduplicates in place; **reworded** content
  yields a **new** key → accumulates. The auto-key is a **pure function of the body** →
  deterministic (unlike flow's random note-op IDs → golden-test stable without wildcards).

### 2.3 Where the fields came from (`6j6v.e0z6`)

Recurring in practice: a memory store grows into a heap of fragments with no structure, and about
half of what is in it turns out to be **mechanics** — conventions and workarounds that exist only
because some tool cannot express something — rather than knowledge. Categories and reach are what
make that distinction addressable at all. They are also what a generated `NEXUS_MEMORY.md`
(epic `6j6v.5gvj`, E5) needs to be better than the hand-written `CLAUDE.md` it replaces: that file
is not a pile of facts but an **argument** — first what the workspace is, then why the design is
what it is, then the hard rules.

The IP boundary runs finer than it looks: **storage and addressing are the engine's** (`nxm` learns
reference and reach); **what gets written and how it is judged is the product's**. Deciding *which*
reach a memory deserves is therefore not part of this data model.

### 2.4 Classification: category, reach, references

- **`category`** — a lower-case slug (`[a-z0-9][a-z0-9_-]*`). `introduction`, `architecture` and
  `rules` are the sections a generated document leads with, but the set is **open**: any slug is a
  category. The slug shape is what keeps a document's section anchors deterministic.
  **`unsorted` is reserved** and is what every memory carries until something files it — chosen
  because it is **countable**, which is the signal the later judging migration (`6j6v.9yaj`) works
  from.
- **`scope`** — `item` (reaches only the board items in `refs`) · `project` (this workspace) ·
  `global` (beyond it). **`project` is the default because it is exactly what every memory already
  did**: `nxs prime` replays it for this workspace. A reach a newer peer writes is kept as an opaque
  string on read and only parsed on the write path, so an unknown value survives a round-trip.
- **`refs`** — the board items the memory is about, stored as one canonical comma-joined list
  (trimmed, deduplicated, **sorted**). Sorting is what makes the single LWW register converge: two
  writers naming the same items in different orders land on the same bytes. memory validates the
  **shape** of a reference and never its existence — the fact reducer knows no flow vocabulary (§7).

### 2.5 Order: a stored property, never a model call

Order is a **stored** property with insertion order as its default, and it is changed **only** by
the explicit `nxm reorder <key>…` verb, which assigns positions 1, 2, … to the given sequence.

**Deliberately NOT: asking a model for an ordering at write time.** It would make an everyday write
non-deterministic (two runs, two orderings → noise in exactly the git history a generated memory
document is meant to win), it would run at the wrong moment (order matters when **reading**, writing
happens constantly), and it would break the engine's **offline promise** — `nxm remember` would
require a network and a provider. The rare, deliberate `reorder` may be fed by whatever decided the
sequence, including a model; the engine only records the decision. Nothing in the `nexus-memory`
library links an HTTP client, and a test asserts it.

Why order is tractable at all: **a memory may be as large as a section**. The need for ordering only
arises once forty fragments lie around loose; a coherent introduction stays *one* memory and one
thought. That shrinks the ordering question to a few entries per category.

Reading order is **the category first, the position within it second** (6j6v.643z): the conventional
categories lead in their declared order (`introduction`, `architecture`, `rules` — what the workspace
is, then the idea that carries it, then the hard rules), then every other category by name, and
`unsorted` last, whatever its slug would sort as. Inside a category the order is
`ordinal IS NULL, ordinal, created_v, created_site, key` — placed memories first in their positions,
then everything never reordered in **insertion order**, with `key` as the final tiebreak so the order
is total and stable.

An ordinal is therefore a position *within* a category and never lifts a memory out of its section.
Both halves are stored facts, so the whole order converges across replicas and is byte-stable.

It is served on request (`nxm memories --ordered`) **and** wherever the memories are read as a
document rather than looked up: `prime` — and with it the `NEXUS_MEMORY.md` projection, which renders
`prime`'s own set with `prime`'s own renderer. `recall` and the plain `memories` listing keep serving
**key order**: a directory is not a document.

## 3. CRDT semantics — one LWW register per key

Each `key` is **one** keep-if-beats LWW register, versioned by the `(lamport, site)` pair of the
winning op (`v`, `site` in the view). `body`, `author`, `updated` and `active` move **together**
with the winning op:

- if a `remember` op wins → `body = value`, `active = 1`, `author/updated` = its metadata.
- if a `forget` op wins → `body = NULL`, `active = 0`, `author/updated` = its metadata.

The fold is **one** upsert (keep-if-beats), exactly following the pattern of flow's
`fold_item_lww`:

```sql
INSERT INTO memories(key, body, author, updated, active, v, site)
VALUES (?key, ?body, ?author, ?wall_clock, ?active, ?lamport, ?site)
ON CONFLICT(key) DO UPDATE SET
    body=excluded.body, author=excluded.author, updated=excluded.updated,
    active=excluded.active, v=excluded.v, site=excluded.site
WHERE (excluded.v, excluded.site) > (v, site);
```

### 3.1 forget + revive

`forget` is a **reversible** tombstone, **not** a grow-only grave: a later `remember` with a higher
`(lamport, site)` **wins the register back** and revives the memory (re-remember allowed). This is
**exactly why** it is an LWW `active` register and **not** a grow-only OR-set like flow's note
redaction (which is deliberately permanent): memory-forget must be reversible.

### 3.2 Convergence — why forget sets the body to NULL

For the register to converge **order-independently** (the central CRDT invariant, hard-tested via
the differential oracle + merge-order tests), every op must carry the **complete** winning state. A
`forget` op that "freezes" the *existing* body would be merge-order dependent:

```
A: remember(K,"v1")@1 ; B (concurrent): remember(K,"v2")@2 ; A: forget(K)@3
Order [r1,r2,f]: f wins, body = (at that point) "v2"
Order [r1,f,r2]: f wins, r2@2 < f@3 loses → body = "v1"   ← DIVERGENCE
```

Solution: **`forget` sets `body = NULL`** (instead of keeping the current body). Then `body` is a
**pure** function of the winning op (remember → `value`, forget → `NULL`), independent of merge
order → convergent. The "lost" body is invisible anyway (forgotten = `active=0`, filtered out of
`memories`/`recall`); a revive `remember` provides fresh body.

## 4. View `memories`

A materialized table per `.nxs/` DB, folded by the fact reducer:

```
memories(
    key     TEXT PRIMARY KEY,   -- the entity ID (= target_id)
    body    TEXT,               -- NULL when last forgotten
    author  TEXT,               -- author of the winning op
    updated TEXT,               -- wall_clock of the winning op (display-only, may be '')
    active  INTEGER NOT NULL,   -- 1 = active, 0 = forgotten
    v       INTEGER NOT NULL DEFAULT 0,   -- lamport of the winning op (LWW version)
    site    INTEGER NOT NULL DEFAULT 0    -- site of the winning op (LWW tiebreak)
)
```

- `recall(key)` → the **active** memory for `key` (`active=1`), otherwise nothing.
- `memories(search?)` → all **active** memories, **deterministically sorted by `key`**.
  `search` = **substring** (case-insensitive) over `key` **and** `body` (parity with bd).
- `active=0` rows stay in the table (tombstone carriers for revive) but are filtered out of every
  read surface.

## 5. Verb surface (parity with bd + nxs contract)

Binary **`nxm`**, `--json` deterministic everywhere (stable field order) + human output.

### 5.1 Memory verbs (aye.21 — parity with bd)

| Verb | bd prior art | Behavior |
|------|------------|-----------|
| `nxm remember "<text>" [--key <key>] [--category <c>] [--scope <s>] [--refs <a,b>]` | `bd remember` | upsert by key; auto-key when `--key` is missing (§2.2); the classification flags file it in the same step (§2.4) |
| `nxm recall <key>` | `bd recall` | full text of an active memory |
| `nxm memories [<search>] [--category <c>] [--scope <s>] [--ordered]` | `bd memories` | list/search active memories, key-sorted; the filters narrow by classification, `--ordered` serves the stored reading order instead (§2.5) |
| `nxm forget <key>` | `bd forget` | tombstone (§3.1); `recall`/`memories` hide it |
| `nxm classify <key> [--category <c>] [--scope <s>] [--refs <a,b>]` | — (6j6v.e0z6) | file an existing memory **without touching its text**; an omitted flag leaves that register alone, and at least one is required (§2.4) |
| `nxm reorder <key>…` | — (6j6v.e0z6) | store the given sequence as positions 1, 2, …; the ONE verb that changes order, deliberate and rare (§2.5) |

`forget` of an unknown/inactive key is a `not_found` error (agent-ergonomic — no silent tombstoning
of a typo); the store layer stays pure (it emits the op), the CLI checks existence for the friendly
message (pattern: flow's `require_item`). `classify` and `reorder` are `not_found` on an unknown key
for the same reason, and `reorder` checks **every** key before writing any position, so a rejected
sequence leaves the order exactly as it was.

Both new verbs have their counterpart on the library seam (`Engine::classify`/`Engine::reorder` over
`facade::{classify, reorder}`), as the verb-seam gate requires — an app files and orders memories
through the same core the CLI does, proven byte-for-byte by the `parity.rs` differential.

### 5.2 Contract verbs (aye.22 — parity with nxf, platform §6)

| Verb | Role |
|------|-------|
| `nxm init` | delegates the foundation to `foundation::setup()`, registers `memory` as an active module, renders its AGENTS.md contribution from its **own** manifest (self-assembly like nxf, aye.12), and sets the SessionStart hook → `nxm prime` |
| `nxm agent-manifest --json` | declares nxm's agent-file contribution as data (manifest contract, platform §6.2 / aye.10) |
| `nxm prime` | replays the memories the **retrieval rule** sends here (§5.3) deterministically as a context block **and pins the memory rule** — the `bd prime` equivalent, the empirically motivated core |

`nxm prime` is the actual trick: not the storing, but the automatic **loading** at the start of
every session. Deterministic output, in the reading order of §2.5.

**The prime block is normative, not just display** (after the bd prior art — this project's own
`CLAUDE.md` today states: "Use `bd remember` for persistent knowledge — do NOT use MEMORY.md
files"). `nxm prime` must therefore, in every session:

1. **pin the memory rule:** durable project knowledge is stored **exclusively** via
   `nxm remember` — **no** `MEMORY.md`, **no** other ad-hoc file. This is the only way, because only
   `nxm remember` is automatically replayed at the next session start (a `MEMORY.md` would require
   the agent to actively read it).
2. **explain the most important memory commands:** `remember` (create/update, stable `--key`),
   `recall` (full text), `memories [<search>]` (review/search), `forget` (remove) — concisely, so
   the agent masters curation without looking things up.

The same rule is carried by the managed AGENTS.md block from `nxm init` (§5.2/§9), so the
directive is present both statically (file) and deterministically (hook → prime). **In AGENTS.md
and there only** since nxf `6j6v.q6e3`: a `CLAUDE.md` belongs to a host that runs the hook, so the
block's `@NEXUS_MEMORY.md` line delivered every memory's BODY there beside the budgeted index —
95-120 KB per session in the product repos. The static copy is for the reader who has no hook, and
that reader reads AGENTS.md.

### 5.3 The retrieval rule (`6j6v.srpg`)

The reach (§2.4) is not decoration: it decides **where and how deep** a memory surfaces.

| reach     | `nxf next`                 | `nxf show <id>` | `nxs prime` |
|-----------|----------------------------|-----------------|-------------|
| `item`    | a hint that there are some | **full text**   | —           |
| `project` | —                          | —               | **full text** |
| `global`  | —                          | —               | **full text** |

The list stays a list, the detail sits where somebody is already looking, and the session start gets
exactly what holds for the whole workspace. **This is the deterministic answer that makes semantic
search unnecessary in the first cut** — reach plus references, no embedding, no similarity measure,
no network. The purpose behind it: a reader (coding agent or orchestrator) orients on the
**distillates** first and descends into the thread only when it must — the antidote to a ticket that
swallowed a full transcript.

Three consequences the implementation follows:

- **The cells partition.** A memory reads in full on exactly ONE surface — never both (noise), never
  neither (silent data loss).
- **The bootstrap withholds a memory only when it has somewhere else to appear.** That is why `refs`
  is part of the rule and not an afterthought: an `item`-scoped memory naming **no** board item
  appears on no board item either, so it is replayed at session start instead of vanishing. Same for
  a reach a **newer peer** wrote, which this build cannot place. Rejecting the write instead would
  break the incremental classification the seam promises — `--scope item` before `--refs …` is one
  natural two-step of registers that deliberately move one at a time — and would leave any memory
  already written that way stranded.
- **Membership is exact and shallow.** A board item carries the `item`-scoped memories whose `refs`
  name **it**. Nothing walks the graph on the reader's behalf: a memory about an epic does not leak
  onto that epic's children, because the engine addresses reach and traversal would be a judgement
  about what a reader deserves (the IP boundary, §2.3). The one case the engine cannot see is a
  `refs` entry naming an item that does not exist — memory validates the shape of a reference, never
  its existence (§7).

The rule lives in `model::{replayed_at_session_start, reaches_item}` — two pure predicates over the
stored register — and has exactly two callers: `facade::prime` (the bootstrap column) and
`facade::memories_about` (the board-item column, also on the library seam as
`Engine::memories_about`). flow reads the latter over the ONE shared `.nxs/` store, gated on memory
being an **active module**; a flow-only workspace never opens memory's store, and its output is
byte-identical to a build without any of this.

Two placement decisions worth stating, because both could otherwise read as oversights:

- **The join is attached once and used by both flow seams.** `nxf show`/`nxf next --json` and the
  MCP `flow_show`/`flow_next` tools are pinned byte-identical, so they share ONE join
  (`nexus_flow_cli::memories`) rather than each applying the rule themselves. In `--json` the item's
  memories ride a sparse `memories` key (full records on `show`, a count on `next`), so an item
  without any is byte-identical to the pre-rule output.
- **`nxs prime`'s `next` section carries no hint.** The table gives item reach a `—` in the prime
  column; a marker there would be a third place one memory is signalled. The hint belongs to the
  live work list, which is where a reader acts on it.

### 5.4 The judging migration (`6j6v.9yaj`)

The structural migration (§2.3) set standards; it cannot decide that a memory is a **rule** and not
an architecture note, or that it holds **everywhere** and not just here. `nxm migrate` is the
explicitly invoked command that carries that judgement, and it finishes the handover the generated
document (`6j6v.8q88`) began.

| Verb | Does |
|------|------|
| `nxm migrate plan [--with-judge [<assistant>]] [--judge-timeout <duration>] [--again]` | Reads. Emits one plan document: every memory still `unsorted`, plus every section of the hand-written context documents. |
| `nxm migrate apply --plan <file\|-> [--keep-sources] [--dry-run] [--again]` | Writes. Files the memories, writes the sections as memories, stores the plan's sequence as the reading order, moves the migrated sections out of their documents, leaves the mark. |
| `nxm migrate status` | What is still unfiled here, and whether a run already happened on this stream. |

Both are on the library seam as `Engine::migrate_plan` / `migrate_apply` / `migrate_status`, so an
embedding app puts its own review UI exactly where the CLI puts a file.

Five properties carry the design:

- **The human decides, the machine proposes.** Two verbs with a readable, editable document in
  between — never a silent bulk filing. There is deliberately **no heuristic guess**: an unjudged
  plan carries the status quo, and `apply` *refuses* an entry still filed as `unsorted` rather than
  laundering "not filed" into a decision. `action: skip` is how a plan says "leave this one alone".
- **The command may ask a model, and that does not break the offline promise.** What is forbidden is
  a model call in the **write path** — it would make every `nxm remember` indeterminate and
  network-bound (§2.5), and `crates/memory/tests/classification.rs` pins that structurally: the
  library links **no HTTP client at all**, by name. A one-off, human-invoked command is the
  sanctioned exception, so the judge is an **external process** the plan is piped through. The
  assistant is a *name* in one table (`migration::Judge`, today `claude` → `claude -p`), not a
  free-form command — the same discipline `nexus_chat::role::Model` follows for model aliases — so a
  second assistant is one entry, and an unknown name fails by name instead of spawning what it was
  handed. `NXM_JUDGE_CMD` overrides the argv as the documented test seam.

  **The wait for that process is bounded, generously** (`6j6v.wbwe`): `JudgeOptions::timeout`,
  `--judge-timeout <n>[smh]` on the command line, 30 minutes by default and `0` for no limit at all.
  Generous is the requirement, not a detail — a judge reading a plan this size legitimately thinks
  for minutes (this repository's own run: a 77 KB request, about three of them), and a limit tight
  enough to interrupt that would throw away a whole judgement, which is worse than the hang it
  prevents. What it does end is the failure a human cannot distinguish from work: a judge that is
  alive and simply never answers, which used to leave the command standing until somebody pressed
  Ctrl-C. The deadline is announced before the wait and named again if it expires, and the plan is
  rebuilt from the store, so re-running costs only the wait.
- **Only hand-written text is imported, and the rule hangs on the CONTENT.** `AGENTS.md` is a
  generated document in this house and importing it back would declare a projection to be the
  source; elsewhere it is hand-maintained. So a file whose first non-empty line is a generated-header
  comment is skipped whole, a section overlapping an assembler-managed block is skipped, and
  everything else is offered. Every skip is *stated* in the plan (`skipped_sources`), never silent.
- **One section is one memory** — the size rule of §2.4 made concrete: the cut is at depth ≤ 2, a
  `###` subsection stays inside the argument it belongs to, and a `#` inside a fenced block is not a
  heading.
- **Once per stream, not once per machine.** The sync daemon carries everything written here, so a
  second device running the judgement again would produce a second independent filing that
  last-writer-wins blends into something neither device decided. The run therefore leaves a **mark**:
  a `fact` op on the reserved target `nxm:migration` carrying the `migration` field, which no
  register column answers to — so the reducer stores it and never folds it (§7). It is durable, it
  syncs, and it materializes nothing, which is why it can be seen by a device without being seen by a
  reader. A marked stream reports the earlier run and stops; `--again` is the explicit answer.

  **The guard is enforced on `apply`, at the seam** (`ApplyOptions::again`), not in the CLI verb that
  happens to run first. It sat in `nxm migrate plan` alone at first — the one path that writes
  nothing — which left both an embedding app on the Engine seam and `apply --plan <a file saved
  earlier>` completely unguarded, and re-applying a stale plan after a mark exists **is** the
  two-independent-filings case (PR #289 review, Test Quality #1). `plan --again` survives as a
  courtesy stop: it saves building and judging a plan nobody would be allowed to apply.

- **`validate_plan` is the whole refusal set, before a single op.** `--plan <file>` is the one entry
  point that never passes through `reconcile`, so everything the write path would reject has to be
  rejected there too: a malformed `refs` (which would otherwise fail mid-loop with earlier entries
  already committed), a `source.path` outside `CONTEXT_DOCUMENTS` (it decides which file gets
  rewritten), and a `remember` key a *different* memory already holds (last-writer-wins would
  overwrite it in silence). Identical text under the same key is deliberately **not** a collision, so
  an interrupted run stays resumable.

- **A section is identified by position, not by its heading text.** Two `##` blocks can carry the
  same words; the keys differ (`notes`, `notes-2`) but the heading does not, and pruning by heading
  membership deleted **both** when only one was remembered — destroying the skipped one's text, which
  existed nowhere else. `EntrySource.index` disambiguates, and the heading is re-checked at that
  position so a document that moved on between `plan` and `apply` is left alone rather than mis-cut.

**How the user hears about it.** Not through `nxs self-update`: that swaps a binary, the workspaces
are many and elsewhere, and it also runs unattended — something that touches N databases and asks
questions does not belong there. What it does is mention the new command **once**, beside the result
line it prints anyway (persisted as `migration_hint_shown` in the per-install user config). The real
carrier is **`nxs prime`**: it runs per workspace and is the only place that knows whether *this* one
still has unfiled memories, so it names an open migration and stays silent when none is open. No
coercion — whoever never migrates keeps working memories with reach `project`.

**Keeping memories true** is the same section's second half, and deliberately the cheapest thing that
could work: `prime` heads every memory with its key and carries the instruction to **correct** a
memory found to be wrong rather than read past it (`PRIME_CORRECTION_RULE`). The agent that just
applied a rule and found its opposite in the code is the best-informed corrector this fact will ever
have; what it lacked was the permission and the handle. Decay, contradiction checks and
confirm-on-read are earned only once this demonstrably falls short.

### 5.5 MCP tool surface (E9 `#76u`, `6j6v.9a1r`)

The `nxs mcp serve` memory tools are thin shells over the SAME facade the `nxm` CLI serializes, so
each tool's `structuredContent` is byte-identical to the matching `nxm <cmd> --json`. They are
registered only when the memory module is active in the launch workspace.

| Tool | Mirrors |
|------|---------|
| `memory_list` / `memory_search` | `nxm memories [<query>] --json` |
| `memory_show` | `nxm recall <key> --json` |
| `memory_add` / `memory_update` | `nxm remember <text> [--key <key>] [--category …] [--scope …] [--refs …] --json` |
| `memory_classify` | `nxm classify <key> [--category …] [--scope …] [--refs …] --json` |
| `memory_reorder` | `nxm reorder <key>… --json` |
| `memory_close` | `nxm forget <key> --json` |

The classification half arrived one ticket after the CLI's (`6j6v.9a1r` after `6j6v.e0z6`) because
the **tool-parameter schema is its own declared contract** and belonged in its own change. Leaving
it open was not survivable, though: MCP is the path our own apps write over, and a memory written
without a reach is exactly the entry §5.3 can show **nowhere**. The trio is optional on every write
and an omitted field leaves that register untouched, so a caller that sends none produces the same
bytes it did before the parameters existed.

## 6. Sync — the nxs advantage over bd

fact ops travel over the **shared op-log** and the **shared sync** (platform §4.1/§4.3): the
`domain` has been **on the wire** since `aye.1.4` (`WireOp.domain`, default `task` for old peers,
foreign domains always serialize). A `fact` op survives the relay round-trip **with its domain
preserved** and folds into the peer's `memories` view there — **synchronized** across machines
rather than fragmented per machine. This is the concrete advantage over bd's **per-repo Dolt**
model: one replica identity, one offline/online path, one consistent snapshot for all three
products.

## 7. Multi-module coexistence (aye.23)

`nxf` and `nxm` share **one** `.nxs/` workspace: `fact` and `task` ops coexist in **one** op-log;
each reducer folds **only** its domain (foreign domain: *store-don't-fold*, no cross-contamination,
platform §4.2/§7). Each product opens **its** store over the same `db.sqlite`, registers **its**
reducer, and materializes **its** views (`items` vs. `memories`) — the views are durable, so no
refold is needed on reopen.

**Behavioral invariant:** flow stays **byte-identical** — differential oracle + flow trycmd goldens
remain unchanged and green. fact does not change flow, because flow's task reducer never folds fact
ops and the foundation runs additive, old-binary-safe migrations (platform §4.3).

## 8. Deliberate non-goals (v1)

- **No auto-decay / no auto-compression** (bd philosophy): the store is **curatorial**, not
  automatic. It grows until **the human** intervenes (stable keys, targeted `forget`, occasional
  consolidation).
- **Consolidation is manual, not a dedicated verb:** multiple fragments are condensed by hand —
  `recall a; recall b` → `remember "<condensed>" --key topic` → `forget a; forget b`.
- **No global/user scope.** memory is **project-/workspace-bound** (`.nxs/`, shared DB). A
  global/user-wide scope is a **later option**, not a v1 topic.

## 9. Known interim consequence: two SessionStart hooks

Because `nxm init` — like `nxf` — sets its **own** SessionStart hook via self-assembly, P2 has
**two** hooks (`nxf prime` **and** `nxm prime`) and two managed blocks (with disjoint markers, so
they do not clobber each other). This is the known self-assembly consequence from P1. The **nxs
umbrella (P3, `aye.4`)** later unifies this into **one** hook → `nxs prime` with multi-module
assembly (platform §6.2/§6.3). Until then: documented, not "solved".

## 10. Mapping to the implementation tickets

| Ticket | Delivers | This spec |
|--------|---------|-----------|
| `aye.20` | `nexus-memory` crate + fact reducer (`memories` view) | §2, §3, §4, §7 |
| `aye.21` | `nxm` memory verbs (remember/recall/memories/forget) | §2.2, §5.1 |
| `aye.22` | `nxm` contract verbs (init/agent-manifest/prime) | §5.2, §9 |
| `aye.23` | multi-module integration + sync round-trip + P2 gate | §6, §7 |
| `6j6v.e0z6` | classification + order (`category`/`scope`/`refs`/`ordinal`) + the structural migration | §2.3–§2.5, §5.1 |
| `6j6v.srpg` | the retrieval rule — the reach decides where a memory surfaces | §5.3 |
| `6j6v.9a1r` | the MCP memory tools can SET the classification (+ `memory_classify`/`memory_reorder`) | §5.5 |
| `6j6v.8q88` | `NEXUS_MEMORY.md` as a build product — the projection, its drift guard, generation in the sync daemon | §5.4 |
| `6j6v.9yaj` | the judging migration (`nxm migrate`), the `prime` carrier, the correction rule | §5.4 |
