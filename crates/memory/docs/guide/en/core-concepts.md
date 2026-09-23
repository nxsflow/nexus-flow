# Core Concepts

Six ideas carry the whole of `nxm`: a fact and its key, the introduction it is read by, the three
registers that file it, the retrieval rule that decides where it is read, the reading order, and the
tombstone. Everything else in [commands](nxm-commands) is a way of setting one of them.

## A fact and its key

A **memory** is one short piece of text under a **key**. The key is its address — the handle you
come back to when the fact turns out to be wrong or incomplete:

```console
$ nxm remember "auth uses JWT; the session table was dropped in v3" --key auth-jwt --introduction "auth is JWT; the session table went away in v3" --json
{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT; the session table went away in v3"}

```

```console
$ nxm memories
auth-jwt  auth uses JWT; the session table was dropped in v3

```

That is the *same* memory as before, evolved — not a second one. **Evolving a fact in place under a
stable key is the habit worth forming**, because the alternative is a store that holds three
half-true versions of the same rule and no way to tell which one is current.

`author` and `updated` date the **fact**: they move when the text moves, and only then.

## The introduction

`--introduction` is **required**, and it is the only part of a memory a session start ever sees.
`nxs prime` replays one line per memory — the key and this line — and nothing else; the body is
fetched on demand with `nxm recall <key>`. So the line is not a label for the memory, it is the
memory's one chance to be read at all. The block carries as many of those lines as its byte budget
allows and names `nxm index` for the rest, so a long line costs its own memory's place in the
session start before it costs anything else.

Three rules, all enforced at the write:

- **one line, at most 200 characters.** A refusal names the measured length, so you are never left
  counting characters by hand.
- **say what the memory SAYS, not what it is about.** "the export endpoint pages at 500 rows" is a
  line somebody can act on; "notes on the export endpoint" is a filing label.
- **for `--category rules`, the line SPEAKS the rule.** "Never run the lean build while the tests
  are running" — not "Rules about the lean build". This one carries real weight: a prohibition is
  only worth something when it is present *before* the mistake, and nobody looks a rule up before
  breaking one. Under an index, the line itself has to do the forbidding. A rule that cannot be said
  in 200 characters is not a rule; it is an essay with a rule inside it.

The limit is on the writer, not a truncation on the reader. That is deliberate: cutting an over-long
line at render time would produce the same bytes and teach nobody anything, while refusing it
reaches the one moment somebody is deciding what the memory says.

A memory written before this register existed has no line. It renders as
`_(no introduction written yet)_`, and the block says how many are in that state and what to type —
the gap is **named**, never filled with something derived. Close it with
`nxm classify <key> --introduction "<one line>"`, which leaves the body untouched.

### Auto-keys are a content hash

With no `--key`, the key is `f-` plus 16 hex characters of `sha256(body)` — a pure function of the
text. Two consequences follow, and they are the whole behaviour:

```console
$ nxm remember "the export endpoint pages at 500 rows" --introduction "the export endpoint pages at 500 rows"
remembered f-bf8fe5f872591385

$ nxm remember "the export endpoint pages at 500 rows" --introduction "the export endpoint pages at 500 rows"
remembered f-bf8fe5f872591385

$ nxm remember "the export endpoint pages at 500 rows, so --since must page too" --introduction "the export endpoint pages at 500 rows, so --since must page too"
remembered f-f64a6f027ce71533

```

A **byte-identical** body dedups to the one memory it already is; a **reworded** body is a new fact
under a new key, and the old one stays. That is right for capture — a machine writing what it just
learned has no name to give it and must not overwrite something else — and wrong for curation. When
you mean "this fact, corrected", give it a key.

## The three registers

Beyond its text and its introduction a memory carries three registers, and each is set
independently:

- **`category`** — which section of the project's memory it belongs to. A lower-case slug; the
  document leads with `introduction`, `architecture` and `rules`, and any other slug is equally
  valid. Nothing filed yet is `unsorted`.
- **`scope`** — how far it reaches: `item`, `project` (the default) or `global`.
- **`refs`** — the board items it is about, e.g. `ab12.0001`. The set is canonical: trimmed,
  deduplicated and sorted, so two writers naming the same items converge instead of fighting.

Set them while writing, or file an existing memory afterwards with `classify`. **Filing never
touches the text**, so `updated` keeps dating the fact rather than the paperwork:

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

An **omitted flag leaves that register alone** — it is not a request to reset it to the default.
`--refs ""` is the explicit way to say "no references"; leaving `--refs` off says nothing at all.
And `classify` with none of `--category`, `--scope`, `--refs` or `--introduction` is refused rather
than silently doing nothing.

## The retrieval rule

The reach decides **where and how deep** a memory surfaces. One deterministic rule, a total function
of the stored registers — no embedding, no similarity measure, no network:

| reach | `nxs prime` | `nxf show <id>` | `nxf next` |
| --- | --- | --- | --- |
| `item` | — | **full text** | a hint that there are some |
| `project` | **one index line** | — | — |
| `global` | **one index line** | — | — |

So an item-scoped memory naming board items is deliberately **absent** from the session bootstrap:
it reads where a reader is already looking. Without that, every session start would grow every
ticket-local note ever written until the bootstrap drowned in detail about one item.

```console
$ nxm remember "the --since cutoff is exclusive" --key since-cutoff --scope item --refs ab12.0001 --introduction "the --since cutoff is exclusive" --json
{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":["ab12.0001"],"ordinal":null,"introduction":"the --since cutoff is exclusive"}

```

```console
$ nxm memories --scope item
since-cutoff  the --since cutoff is exclusive

```

Two edges are worth knowing, because both are the rule protecting you rather than surprising you:

- **An `item` memory that names nothing is replayed anyway.** It has no board item to read on, so
  withholding it from the bootstrap would make it invisible everywhere — and an invisible memory is
  silent data loss. `--scope item` before `--refs …` is the natural two-step of registers that move
  one at a time, and the read stays total across it.
- **Membership is exact and shallow.** A memory reaches an item only if it is `item`-scoped *and*
  names that id. It does not walk the board: a memory about an epic does not leak onto the epic's
  children, and a `project` memory that happens to mention an id stays out — its reach says it
  belongs to the whole workspace, and the bootstrap is where the whole workspace reads.

Memory validates the **shape** of a reference, never its existence — it knows no flow vocabulary —
so a mistyped id is a memory filed against nothing. That boundary is deliberate.

## Reading order

A memory document is an argument: what the workspace *is*, then the idea that carries it, then the
hard rules. That argument is the stored order, and it has two levels.

**Between categories** the rank is fixed: `introduction`, `architecture`, `rules`, then every other
category alphabetically, and `unsorted` last — what nobody has filed belongs at the end, not
wherever its slug happens to sort. **Within a category**, explicitly placed memories come first in
the position `reorder` gave them, then everything never reordered, in the order it was written.

`reorder` is the one verb that writes a position, and it is meant to run rarely and deliberately —
everyday writes never touch the order:

```console
$ nxm reorder auth-jwt since-cutoff
reordered 2 memories
  1  auth-jwt
  2  since-cutoff

```

```console
$ nxm memories --ordered
auth-jwt  auth uses JWT; the session table was dropped in v3
since-cutoff  the --since cutoff is exclusive
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too

```

A position therefore never lifts a memory out of its section: `auth-jwt` leads because it is filed
under `architecture`, and `since-cutoff` leads the `unsorted` tail because it was placed. This is
the order `nxs prime` replays and `NEXUS_MEMORY.md` is generated in.

## Forgetting is reversible

`forget` writes a **tombstone**. The memory drops out of `memories` and `recall` — asking for it is a
`not_found`, not an empty answer — and a later `remember` under the same key brings it back:

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

Nothing is deleted, because nothing here is stored as state to delete. Which brings us to the last
idea.

## One store, one log

A memory is not a row somebody overwrites; it is a fold over **operations** in the same shared
`.nxs/` op-log flow and chat write into. Three things follow, and they are why the model above holds
under collaboration:

- **The registers are versioned independently.** Writing a category cannot roll back a concurrent
  edit of the text, and a rare `reorder` cannot touch anyone's wording.
- **Conflicts resolve, they do not corrupt.** Each register is a keep-if-beats last-writer-wins
  register; the winning write decides, and `author`/`updated` travel with the body.
- **One `nxs sync` moves all of it.** There is no per-module sync, because there is no per-module
  log.

And the whole write path is offline and deterministic: no command in this guide asks a model.
The one command that may is `nxm migrate plan --with-judge`, which you invoke on purpose — see
[import and migration](nxm-import-and-migration).

## The generated document

Every write regenerates `NEXUS_MEMORY.md` at the workspace root, in exactly the reading order above:

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

It is a **projection, not a source**: the store is the truth and git supplies the history. It exists
so the context survives for a reader with no suite installed, and so the `SessionStart` hook has
something to fall back on. An edit made in the file is overwritten by the next write, and
`nxm doc --check` is the guard that goes red once file and store disagree.

Because the document is a projection of what `nxs prime` already replays, **nothing needs to import
it from `CLAUDE.md`** — doing so would place every memory in your context twice.

## Next

- [commands](nxm-commands) — every verb, with its `--json` shape.
- [agents and MCP](nxm-agents-and-mcp) — the same model over the MCP seam.
- [import and migration](nxm-import-and-migration) — filing what you already have.
