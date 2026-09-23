# Agents and MCP

Memory has three seams, and they serve the same store: the `nxm` CLI (what an agent with a shell
types), the **MCP tools** (what an MCP-native host calls), and `nxs prime` (what a session is handed
before it asks anything). This guide is about the second and third — the first is
[commands](nxm-commands).

The rule that ties them together: **an MCP tool's answer is byte-identical to the matching
`nxm … --json`** for the same state. Not "equivalent" — the same bytes, held by cross-seam parity
tests. So whatever you learned from the CLI record is what a tool returns, field for field.

## Connecting a host

The server is `nxs mcp serve`, and it is the suite's — one server for flow *and* memory over the one
shared `.nxs/` store. Registering it in Claude Desktop, Cursor or Windsurf, including the case where
nothing is installed yet, is covered once in flow's guide: [connect an MCP host](nxf-mcp). With
`nxs` on your `PATH` it is one command:

```bash
nxs mcp install
```

**The memory tools are gated on the memory module being active** in the workspace the server was
launched against. A flow-only workspace exposes no `memory_*` tools at all — the registered surface
is the fan-out over the active blocks, not a fixed list. If the tools are missing, the answer is
almost always `nxm init` (or `nxs init --module memory`) in that workspace.

Give a connected host the trust you would give a shell in that workspace: these are writes, not just
reads.

## The tool surface

Eight tools — three reads and five writes — each the exact counterpart of a CLI verb:

| tool | mirrors | notes |
| --- | --- | --- |
| `memory_list` | `nxm memories --json` | all active memories, key-sorted |
| `memory_search` | `nxm memories <query> --json` | case-insensitive substring over key + body |
| `memory_show` | `nxm recall <key> --json` | one record, or a `not_found` tool error |
| `memory_add` | `nxm remember <text> --introduction <line> --json` | the content-hash auto-key; no `key` parameter |
| `memory_update` | `nxm remember <text> --key <key> --introduction <line> --json` | upsert in place under a stable key |
| `memory_classify` | `nxm classify <key> --json` | file it, or rewrite its `introduction`; the text is untouched |
| `memory_reorder` | `nxm reorder <key>… --json` | the array of records in the order just written |
| `memory_close` | `nxm forget <key> --json` | the `{ok, key}` receipt, not a tombstone record |

`memory_add` and `memory_update` take the same optional classification trio the CLI flags set —
`category`, `scope`, `refs` — so a fact can be captured and filed in one call. Sending none of the
three writes the status-quo defaults.

**`introduction` is required on both**, exactly as `--introduction` is on the CLI: one line, at most
200 characters, the only part of the memory a session start ever sees. `memory_classify` takes it
too, optionally — that is how a memory written before the field existed gets its line.

**Why the write verbs are named that way**: `add` mints a content-hash key and `update` requires
one, so the split is visible in the tool name rather than hidden in whether a parameter was sent.
`close` follows the same house convention as flow's item verbs, and, like `forget`, is reversible.

### Parameters that are not flags

Three parameters exist only on this seam:

- **`now`** — the timestamp a write stamps as `updated`. Explicit rather than implicit, so a caller
  that needs reproducible output has one.
- **`actor`** — the author recorded on the write. Absent one, the umbrella server writes under its
  single `nxs` identity (`--actor` / `NXS_ACTOR` → `NXF_ACTOR`/`USER` → `nxs`), *not* the CLI's
  `NXM_ACTOR` default. That divergence is deliberate: one stdio server writes under one identity for
  every block it serves, rather than a per-module persona.
- **`workspace`** — a per-call override of which workspace this call opens. Omitted, it is the one
  the server was launched against.

### Errors

A refusal comes back as a **tool error**, not a protocol error, carrying the same kind and message
the CLI would print: an unknown key is `not_found`, an empty body or an unknown reach is
`validation`. A host stays connected and the model can read what went wrong and try again.

## What `nxs prime` contributes

There is **no `memory_prime` tool**, and that is a decision rather than a gap: the bootstrap is
delivered once at connect time, as the server's `initialize.instructions`, composed from the same
`nxs prime` fan-out you get by hand — the session hooks deliver the same blocks, one per active
module. Memory's share of it carries:

1. **The memory rule** — durable project knowledge goes in `nxm remember`, never in an ad-hoc
   `MEMORY.md`, because nothing reads such a file back.
2. **The correction rule** — a memory that contradicts the project is corrected in place under its
   key, or forgotten. The session reading it is the best-informed corrector it will ever have.
3. **How to write an introduction**, including the obligation that a `rules` line speaks its rule
   rather than announcing it.
4. **Why nothing imports `NEXUS_MEMORY.md` from `CLAUDE.md`** — the index is already in context, and
   the projection would repeat it and add every body underneath.
5. **The commands**, and **one line per memory the retrieval rule replays here**, in reading order
   — as many as the block's byte budget carries, with the heading and a note under the index saying
   so when there are more. Not the bodies: `memory_show` is what fetches one, and `memory_list`
   answers over the whole set whatever the block showed.

Those instructions are **static**: they are computed at connect and not refreshed while the session
runs. So the live set is what the tools answer — `memory_list` and `nxf`'s board reads — and the
instructions say so themselves rather than pretending to be a snapshot.

## Where an agent should write

The seam does not change the advice, so it is worth stating once:

- **A durable fact about the project** — a convention, a gotcha, a decision and its reason — goes in
  memory, under a key you can come back to. Not in a scratch file, not in a chat message.
- **A fact about one board item** goes in with `scope: "item"` and the item in `refs`. It then reads
  on that item instead of in every session start, which is what keeps the bootstrap readable.
- **Working state — what you are doing right now** — is not a memory. Flow tracks work
  ([nxf core concepts](nxf-core-concepts)); chat carries the conversation about it
  ([nxc core concepts](nxc-core-concepts)). Memory is for what is still true afterwards.

## Next

- [commands](nxm-commands) — the CLI verbs each tool mirrors.
- [core concepts](nxm-core-concepts) — the registers a tool call sets, and the retrieval rule.
- [connect an MCP host](nxf-mcp) — registering the server, for every host.
