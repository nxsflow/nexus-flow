# Getting Started

`nxm` is the **memory** building block of the nexus suite: the durable project knowledge an agent
still has at the start of the next session. One binary ships three tools — flow (`nxf`) tracks the
work, chat (`nxc`) carries the messages about it, and memory (`nxm`) is what your agents still know
tomorrow.

This is memory's own introduction: what the module is for, what it needs before it can run, and the
first command that is yours to type. Installing the suite and setting up a workspace is the same
for all three blocks and is documented once, with `nxs`.

## What it is for

A coding session accumulates facts that are worth more than the session: a convention, a gotcha, the
reason a decision went the way it did. Left in a chat transcript they are gone at the next
compaction; written into a scratch file nothing reads them again.

`nxm` is the one durable channel for exactly that class. A **memory** is a short piece of text under
a **key**, stored in the workspace, and replayed into every new session by `nxs prime`. There is no
model on the write path, no embedding, and no similarity search: what comes back is decided by
registers you set, so the same store answers the same way today and in six months.

**The rule that follows from it is a hard one, and `nxs prime` states it in every session:** durable
project knowledge goes in `nxm remember` and nowhere else. Never create a `MEMORY.md` or any other
ad-hoc memory file — nothing reads it back, so it is never replayed and that knowledge is silently
lost. If your project already has one, it is not a dead end: see
[import and migration](nxm-import-and-migration).

## What it needs

A `.nxs/` **workspace** — the one shared store all three blocks write into. If your project has none
yet, `nxs init` creates it and asks which blocks to activate. If it already has one (because flow or
chat is running there), memory is added to it, never beside it:

```bash
nxs init --module memory   # add memory to the workspace, non-interactively
```

`nxm init` does the same thing from memory's side, and is the entry to use when memory is the only
block you want:

```bash
nxm init
```

Either way you end up with the same three things: the `.nxs/` store, an `AGENTS.md` carrying a short
managed block that tells any agent landing in this repo to run `nxs prime`, and one `SessionStart`
hook per active block — memory's runs `nxm prime` for you. `nxm init --quiet` is the same setup with no banner — the
non-interactive entry for an agent or a script.

## Your first memory

Write a fact under a key you choose, with the one line a session start will read it by:

```console
$ nxm remember "auth uses JWT, not sessions" --key auth-jwt --introduction "auth is JWT, not sessions"
remembered auth-jwt

```

```console
$ nxm memories
auth-jwt  auth uses JWT, not sessions

```

The key is the address of the fact. Reuse it and the fact is **updated in place** — one memory that
got better, not two memories that disagree. Omit `--key` and the key is a hash of the text, which is
the right choice when a machine is capturing something and has no name for it.

`--introduction` is required, and it is not a label: it is the *only* part of this memory a session
start will ever see. One line, at most 200 characters, saying what the memory says. The body is a
`nxm recall <key>` away, and this line is what decides whether anyone fetches it. See
[core concepts](nxm-core-concepts) for the rules it follows — including the one that matters most,
that a memory filed under `rules` must SPEAK its rule rather than announce it.

Reading is `recall` for one memory and `memories` for the set, and `--json` is on every command:

```console
$ nxm recall auth-jwt
auth uses JWT, not sessions

```

```console
$ nxm recall auth-jwt --json
{"key":"auth-jwt","body":"auth uses JWT, not sessions","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT, not sessions"}

```

That record is the shape everything else returns too, field for field: the CLI under `--json`, the
MCP tools, and the generated document all project the same stored memory.

## What `nxs prime` contributes

At the start of a session every active block's own `prime` runs, each from its own hook; `nxs prime`
is the same fan-out when you ask for it by hand. Memory's share carries five things:

1. **The memory rule** — durable knowledge goes in `nxm remember`, never in an ad-hoc file.
2. **The correction rule** — if you apply a memory and the project says otherwise, correct it in
   place (`nxm remember "<the corrected fact>" --key <key>`) or `nxm forget <key>` it. The session
   reading a memory is the best-informed corrector it will ever have.
3. **How to write an introduction**, including the obligation on a `rules` line — read before you
   write, which is the only time it can help.
4. **The commands**, spelled with their flags, so a session does not have to guess them.
5. **One line per memory that holds for this workspace**, in its stored reading order: the key and
   the introduction its author wrote. Not the bodies — a session start that carried those measured
   90 KB on this project's own workspaces, which is past the point where the host stops delivering
   the block at all. The index itself stops at the same limit: once a workspace has more lines than
   the block can carry, the heading reads `## Memories (showing 34 of 80)` and a line under it names
   `nxm index`, which prints the whole of it.

You never type it yourself — the hooks do, one per active block. Re-run `nxs prime` by hand after a
context compaction; memory's block says so at the top for exactly that reason.

Item-scoped memories are deliberately *not* in that dump: they read on the board items they name
instead. That is the retrieval rule, and it is the one thing worth understanding before you file
anything — see [core concepts](nxm-core-concepts).

## Where the memories also live

Every write regenerates `NEXUS_MEMORY.md` at the workspace root: the project's memory as a **build
product**, so the context survives for a reader with no suite installed and for anyone browsing the
repository on a forge. It is a projection, not a source — the store is the truth, and an edit made
in the file is overwritten by the next write. `nxm doc` prints it; `nxm doc --check` reports drift.

## Next

- [core concepts](nxm-core-concepts) — keys, the three registers, the retrieval rule, reading order.
- [commands](nxm-commands) — the full reference, with `--json`.
- [agents and MCP](nxm-agents-and-mcp) — the memory tools an MCP host gets, and the prime seam.
- [import and migration](nxm-import-and-migration) — bringing an existing memory store in.
