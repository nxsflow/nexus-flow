# Commands

The complete `nxm` surface: twelve commands, in the order a session tends to reach for them. Run
`nxm <command> --help` for the generated reference (flags, types, defaults); this guide adds the
narrative and the `--json` shapes.

Three things are global:

- **`--json`** on every command — the agent contract. Records carry a stable, declared field order,
  so the bytes are comparable and diffable.
- **`--db <path>`** (also `NXM_DB`) — point at a workspace database directly instead of discovering
  `.nxs/` upward from the working directory.
- **`NXM_ACTOR`** — the author recorded on every write (else `USER`, else `nxm`). `NXM_NOW` pins the
  timestamp a write stamps, which is what makes the examples in these guides byte-reproducible.

Errors are a structured envelope, not prose: `{"error":{"kind":…,"msg":…}}` with a non-zero exit.
The kinds you will actually branch on are `validation` (the call was wrong), `not_found` (no such
memory) and `no_workspace` (nothing to open here).

## Set up

### `nxm init`

Activate memory in the current directory: ensure a `.nxs/` workspace, register the `memory` module,
then hand the shared agent file and the SessionStart hooks to the `nxs`
assembler, which re-assembles them for *every* active module — so activating a second block never
overwrites the first one's block.

```bash
nxm init                 # human, with the banner
nxm init --json          # the machine-readable receipt
```

`--quiet` is silent success — it prints nothing at all, which is what makes it drivable by the
umbrella and by an agent:

```console
$ nxm init --quiet

```

`--from-beads` consents to the beads → nxs migration (tickets *and* memories) and is routed through
the umbrella, which owns it.

### `nxm agent-manifest`

What memory declares it contributes to the shared agent file — the prime command and the hook — as
data rather than as prose:

```console
$ nxm agent-manifest
nexus-memory agent manifest
  prime command:  nxm prime
  hook:           SessionStart → nxm prime

Run with --json for the machine contract the `nxs` umbrella assembles from.

```

`--json` is that contract (`{"prime_command":…,"hook":{"event":…,"command":…}}`); the umbrella reads
it from every active block and assembles one `AGENTS.md` and one hook.

## Write

### `nxm remember <TEXT>`

Add a fact, or update one in place. The whole write path is offline and deterministic — nothing here
asks a model.

```console
$ nxm remember "auth uses JWT, not sessions" --key auth-jwt --introduction "auth is JWT, not sessions"
remembered auth-jwt

```

Flags:

- **`--introduction <line>`** — **required.** One line, at most 200 characters. It is the only part
  of this memory a session start ever sees, so it says what the memory SAYS rather than what it is
  about — and for `--category rules` it SPEAKS the rule ("Never run X while Y is running", not
  "Rules about X"), because nobody looks a prohibition up before breaking it. Over the limit is
  refused at the write, with the measured length named. See
  [the introduction](nxm-core-concepts) for the whole argument.
- **`--key <key>`** — a stable, human-chosen key to upsert in place. Omit it and the key is a
  content hash: identical text dedups, reworded text mints a new fact
  ([core concepts](nxm-core-concepts)).
- **`--category <slug>`**, **`--scope item|project|global`**, **`--refs <id,id>`** — file the memory
  in the same write. An omitted flag leaves that register at its default on a new memory and
  *untouched* on an existing one.

An update is not exempt from `--introduction`: the body changed, so whether the line still describes
it is exactly the question.

`--json` returns the whole record, which is the same shape `recall` and `classify` return:

```console
$ nxm remember "auth uses JWT; the session table was dropped in v3" --key auth-jwt --introduction "auth is JWT; the session table went away in v3" --json
{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT; the session table went away in v3"}

```

An empty body is refused rather than stored:

```console
$ nxm remember "" --introduction "an empty body" --json
? 1
{"error":{"kind":"validation","msg":"a memory body must not be empty"}}

```

### `nxm classify <KEY>`

File an existing memory — category, reach, references and/or its introduction — **leaving its text
untouched**, so `updated` keeps dating the fact rather than the paperwork. `--introduction` here is
also how a memory written before that register existed gets its line.

```console
$ nxm classify auth-jwt --category architecture
classified auth-jwt
  category: architecture
  scope:    project
  refs:     none
  intro:    auth is JWT; the session table went away in v3

```

```console
$ nxm memories --category architecture
auth-jwt  auth uses JWT; the session table was dropped in v3

```

At least one of the four is required — silently doing nothing would be worse than saying so — and a
category must be a lower-case slug:

```console
$ nxm classify auth-jwt --json
? 1
{"error":{"kind":"validation","msg":"classify needs at least one of --category, --scope, --refs or --introduction"}}

```

```console
$ nxm classify auth-jwt --category "Not A Slug" --json
? 1
{"error":{"kind":"validation","msg":"invalid category 'Not A Slug': use a lower-case slug like 'introduction' (letters, digits, '-' and '_')"}}

```

`--refs` replaces the whole set; `--refs ""` is the explicit "no references":

```console
$ nxm classify since-cutoff --refs "" --json
{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":[],"ordinal":null,"introduction":"the --since cutoff is exclusive"}

```

### `nxm reorder <KEY>...`

Store an explicit reading order: the first key becomes position 1, the second 2, and so on. Every
key must exist — a rejected sequence leaves the stored order exactly as it was. Deliberate and rare;
everyday writes never touch the order, and a memory nobody placed sorts after every placed one, in
the order it was written.

```console
$ nxm reorder auth-jwt since-cutoff
reordered 2 memories
  1  auth-jwt
  2  since-cutoff

```

`--json` returns the array of records in the order just written. The category still decides the
section — a position orders *within* one, it never lifts a memory out of it.

### `nxm forget <KEY>`

Forget a memory: a reversible tombstone. It drops out of `memories` and `recall`, and a later
`remember` under the same key revives it.

```console
$ nxm forget since-cutoff
forgot since-cutoff

```

```console
$ nxm recall since-cutoff --json
? 1
{"error":{"kind":"not_found","msg":"no memory 'since-cutoff'"}}

```

```console
$ nxm remember "the --since cutoff is exclusive" --key since-cutoff --introduction "the --since cutoff is exclusive"
remembered since-cutoff

```

The `--json` receipt is a plain acknowledgement, not the tombstone record — forgetting is an act,
not a state you read back:

```console
$ nxm forget f-bf8fe5f872591385 --json
{"key":"f-bf8fe5f872591385","ok":true}

```

Forgetting an unknown key is a `not_found`, so a script can tell "it is gone now" from "it was never
there".

## Read

### `nxm recall <KEY>`

One memory's full text; `--json` gives the record.

```console
$ nxm recall auth-jwt
auth uses JWT, not sessions

```

```console
$ nxm recall auth-jwt --json
{"key":"auth-jwt","body":"auth uses JWT, not sessions","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT, not sessions"}

```

A forgotten or never-written key is a `not_found`, never an empty answer:

```console
$ nxm recall ghost --json
? 1
{"error":{"kind":"not_found","msg":"no memory 'ghost'"}}

```

### `nxm memories [SEARCH]`

The active set, key-sorted, or the subset matching a **case-insensitive substring over key and
body**. There is no ranking and no relevance score: a substring either occurs or it does not.

```console
$ nxm memories
auth-jwt  auth uses JWT; the session table was dropped in v3
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too
since-cutoff  the --since cutoff is exclusive

```

Filters, which compose:

- **`--category <slug>`** — only memories filed under that section.
- **`--scope item|project|global`** — only memories with that reach.
- **`--ordered`** — the stored reading order (category, then position) instead of key order.

```console
$ nxm memories --ordered
auth-jwt  auth uses JWT; the session table was dropped in v3
since-cutoff  the --since cutoff is exclusive
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too

```

`--json` is the array an app lists from — the same record shape, one per memory:

```console
$ nxm memories --json
[{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"architecture","scope":"project","refs":[],"ordinal":1,"introduction":"auth is JWT; the session table went away in v3"},{"key":"f-bf8fe5f872591385","body":"the export endpoint pages at 500 rows","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"the export endpoint pages at 500 rows"},{"key":"f-f64a6f027ce71533","body":"the export endpoint pages at 500 rows, so --since must page too","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"the export endpoint pages at 500 rows, so --since must page too"},{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":[],"ordinal":2,"introduction":"the --since cutoff is exclusive"}]

```

### `nxm index`

The **whole** index: one written line per memory — its key and the introduction its author wrote —
in the reading order the session start replays. `nxm prime` renders as much of this index as its
byte budget allows and names this verb for the rest, so nothing a session start had to leave out is
out of reach.

Read the lines first, then open the one that matters with `nxm recall <key>`. That is the intended
order: the index is what decides which body is worth the tokens.

```console
$ nxm index
# nexus-memory — the whole index (3)

**One line per memory — this is an index, not the memories.** Each line is the introduction its author wrote for it. Read one in full with `nxm recall <key>`, or search their bodies with `nxm memories <text>`.

- **auth-jwt**: auth is JWT; the session table went away in v3
- **since-cutoff**: the --since cutoff is exclusive
- **f-f64a6f027ce71533**: the export endpoint pages at 500 rows, so --since must page too

```

`--json` mirrors `nxm prime --json` — the same records, plus their `count`:

```console
$ nxm index --json
{"count":3,"memories":[{"active":true,"author":"alice","body":"auth uses JWT; the session table was dropped in v3","category":"architecture","introduction":"auth is JWT; the session table went away in v3","key":"auth-jwt","ordinal":1,"refs":[],"scope":"project","updated":"2026-06-20T10:00:00Z"},{"active":true,"author":"alice","body":"the --since cutoff is exclusive","category":"unsorted","introduction":"the --since cutoff is exclusive","key":"since-cutoff","ordinal":2,"refs":[],"scope":"item","updated":"2026-06-20T10:00:00Z"},{"active":true,"author":"alice","body":"the export endpoint pages at 500 rows, so --since must page too","category":"unsorted","introduction":"the export endpoint pages at 500 rows, so --since must page too","key":"f-f64a6f027ce71533","ordinal":null,"refs":[],"scope":"project","updated":"2026-06-20T10:00:00Z"}]}

```

It answers over the same memories the session start does, which is what makes the block's "N more
not listed" arithmetic true: an `item`-scoped memory that names board items reads on those items
(`nxf show`) and appears in neither. For everything the workspace holds, including those, use
`nxm memories`.

### `nxm doc`

Print the generated project-memory document — the `NEXUS_MEMORY.md` projection every write
regenerates. Nothing here writes.

```console
$ nxm doc
<!-- generated by nexus-flow — do not edit; run `nxm remember` instead -->
# Project memory

> **Generated file — do not edit.** Every entry below is a memory in this project's `nxm` store, projected here so the context survives without nexus-flow installed. Change it with `nxm remember` / `nxm forget`; an edit made here is overwritten by the next write. `nxm doc --check` reports drift.

> **You have already been given the index below** — all of it, or as much of it as a session start could carry. It is the same index `nxm prime` replays, which stops at the byte budget the host delivers and says so when it does (`nxm index` prints the whole of it), so there is nothing to gain by reading it again here. Underneath it stands the FULL TEXT of each memory — that is what `nxm recall <key>` serves, and it is meant to be read one memory at a time, when the index tells you a particular one matters. Reading this file end to end is the expensive way to obtain what you already have.

## Index (3)

- **auth-jwt**: auth is JWT; the session table went away in v3
- **since-cutoff**: the --since cutoff is exclusive
- **f-f64a6f027ce71533**: the export endpoint pages at 500 rows, so --since must page too

## Full text

### `auth-jwt`

auth uses JWT; the session table was dropped in v3

---

### `since-cutoff`

the --since cutoff is exclusive

---

### `f-f64a6f027ce71533`

the export endpoint pages at 500 rows, so --since must page too

```

`--check` compares the file on disk with the store instead of printing it, and **exits non-zero**
when they have drifted apart — the guard for a hand-edited or never-regenerated file. Under `--json`
it reports `{"path":…,"status":"in_sync"}`, so it drops straight into a pre-commit hook or CI.

### `nxm guide [TOPIC]`

Print an embedded, offline guide, or list the topics with no argument. It needs no workspace — the
content is compiled into the binary — which is why it answers before anything is set up.

```console
$ nxm guide --json
[{"summary":"What memory is for, what it needs, and your first remembered fact.","topic":"getting-started"},{"summary":"Keys and auto-keys, the three registers, the retrieval rule, reading order, and the tombstone.","topic":"core-concepts"},{"summary":"Every shipped verb, with its `--json` shape and the errors it can answer with.","topic":"commands"},{"summary":"The `memory_*` MCP tools, what `nxs prime` contributes, and where an agent writes.","topic":"agents-and-mcp"},{"summary":"Take a Claude-host memory store in, and file what a workspace already accumulated.","topic":"import-and-migration"}]

```

`nxs guide` fans out over every active block and lists all three. A topic that several blocks carry
— `getting-started`, `core-concepts` and `commands` each exist three times — is never picked for
you: the umbrella names `nxf guide …` / `nxm guide …` / `nxc guide …` and lets you choose.

## Take work in

### `nxm import`

Import an existing Claude-host memory store. Each fact comes in under its frontmatter `name:` as a
stable key, so a re-run upserts in place and the source is never modified. With no `--from`, the
Claude memory directory for this project is auto-detected.

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

See [import and migration](nxm-import-and-migration) for the whole path, including what to do with
what you already have.

### `nxm migrate <plan|apply|status>`

The judging migration: file this workspace's `unsorted` memories and move its hand-written context
documents into them, section by section. `status` reports where it stands:

```console
$ nxm migrate status
3 unfiled memories — run `nxm migrate plan --with-judge`

```

```console
$ nxm migrate status --json
{"unsorted":3}

```

`plan` proposes and writes nothing; `apply` executes a decided plan. Both are covered in
[import and migration](nxm-import-and-migration), which is also where the one command in this
product that may ask a model is explained.

## What is deliberately absent

- **No delete.** `forget` is a tombstone, and the log keeps the history. Nothing here erases.
- **No search ranking, no embeddings, no network.** `memories <search>` is a substring match. What
  comes back is decided by registers you set, so the same store answers the same way in six months.
- **No `nxm sync`.** There is one shared op-log for the whole suite, so syncing is a suite
  operation: `nxs sync`.
- **No `nxm prime` in your hands.** It exists; memory's own `SessionStart` hook runs it, and the
  `nxs prime` fan-out calls it too. The user-facing entry is the umbrella's, so one command by hand
  gets you every active block.
