# E9 — MCP server (`nxs mcp serve`)

The MCP seam beside the CLI (agents) and the embed API (apps): MCP-native hosts (Claude Desktop,
Amazon Q, …) can't drive the CLI, so `nxs mcp serve` exposes the active modules' read **and
write** ops as MCP tools over the **same** shared `.nxs/` store. Full architecture, defaults, and the
slice plan live in the epic. This doc covers the shipped
flow **read-write vertical** and how to verify it against a real host.

## What ships (flow read + write vertical)

A feature-gated server in `crates/nxs` (feature `mcp`, default-on; `--no-default-features` drops
rmcp + tokio). Built on the official `rmcp` SDK over stdio.

### Read tools

Three flow read tools, each a thin shell over the same Record-Facade the `nxf` CLI serializes under
`--json`, opened per call against the launch workspace:

| Tool | Mirrors | `structuredContent` |
|------|---------|---------------------|
| `flow_next`  | `nxf next --json`  | `{ items: [...] }` — ranked ready set under `items` (#76u.9) |
| `flow_show`  | `nxf show <id> --json` | composite `{item, deps, contributes_to, notes}` |
| `flow_list`  | `nxf list --json`  | `{ items: [...] }` — item array under `items` (#76u.9) |

There is no `flow_prime` tool (#76u.6): the prime function — workflow rules + orientation — is
delivered once as `initialize.instructions` (below), and the live snapshot via `flow_next`/`flow_list`.

### Write tools (#wpw / #yie / #zxj)

Nine flow mutation tools, each a thin shell over the **same** `nexus_flow_facade::write` layer the
`nxf` CLI uses — there is no second write path — so a tool's `structuredContent` receipt is
byte-identical to `nxf <command> --json` (the cross-seam parity tests pin this):

| Tool | Mirrors | Receipt |
|------|---------|---------|
| `flow_create` | `nxf create --json` | the new item's canonical record |
| `flow_update` | `nxf update <id> --set … --json` | the item's canonical record |
| `flow_claim`  | `nxf claim <id> --json` | the item's canonical record |
| `flow_close`  | `nxf close <id> --reason … --json` | the item's canonical record |
| `flow_dep_add` / `flow_dep_remove` | `nxf dep add\|remove <from> <to> --json` | `{ ok, msg }` |
| `flow_mention_add` / `flow_mention_remove` | `nxf mention add\|remove <from> <to> --json` | `{ ok, msg }` |
| `flow_note_add` | `nxf note add <id> <text> --json` | `{ id, body }` |

- **Explicit `now` + `actor` (epic invariant).** Both are explicit per-tool parameters, never ambient
  process state. `now` is the wall clock stamped on the emitted ops (defaults like the read tools'
  `now`); `actor` is the identity recorded on them. A long-lived, host-started server must not write
  every op under one identity, so the actor is a **hybrid** (#zxj): a per-tool `actor` argument wins,
  else the server-instance launch default `--actor`/`NXS_ACTOR`, else the `NXF_ACTOR`/`USER` fallback
  the CLI uses, else `nxs`.
- **Per-op `ToolAnnotations`.** Every mutation carries `readOnlyHint: false` and
  `openWorldHint: false` (the board is a closed local domain), plus accurate per-operation hints so a
  strict host can badge/guard each write rather than auto-approve it: `destructiveHint: false` for
  purely additive ops (`flow_create`, `flow_dep_add`, `flow_mention_add`, `flow_note_add`) and `true`
  for the field-overwriting / edge-removing ops; `idempotentHint: true` everywhere except the minting
  ops (`flow_create`, `flow_note_add`).
- **Errors** map the same way as reads — a domain failure is a tool result with `isError = true` and
  the `{ error: { kind, msg } }` envelope. The write path additionally exercises the `cycle` kind
  (a dependency cycle rejected at write time) the read surface can never emit.
- **Parity caveat for the minting ops.** `flow_create`'s item id is deterministic under
  `NXF_DETERMINISTIC_IDS` (so it parity-compares byte-for-byte against the CLI); `flow_note_add`'s
  note id is a fresh random ULID per call (that switch only seeds item ids), so its parity holds over
  the receipt shape + the `body`, with the id asserted present on both seams.

The `dep`/`mention` add and remove operations are **separate tools** (not one tool with an `op`
argument) precisely so each can carry its own honest `destructiveHint` (add is additive, remove is
destructive). Their multi-word CLI forms (`nxf dep add`, `nxf note add`, `nxf mention add`) stay
written as CLI references in the connect-time `instructions` — `seamify_commands` only rewrites a
single-verb command to a single tool, so it deliberately leaves these honest rather than invent a
fake `flow_dep`/`flow_note`/`flow_mention` tool name; the tools themselves are still discoverable
from the `## Tools` section of the instructions and from `tools/list`.

- **Cross-seam parity (invariant):** a tool's `structuredContent` carries the canonical record
  byte-identical to the matching `nxf <command> --json` for the same state (`now` pinned on both
  sides). MCP requires `structuredContent` to be a JSON **object**, so the array-returning tools
  (`flow_next`, `flow_list`) wrap their CLI-identical array under an `items` key (#76u.9): parity for
  those compares `structuredContent.items` to `nxf <command> --json`, while `flow_show` (already an
  object) compares directly. prime is no longer a tool, so it has no parity surface — the epic's
  prime exception now applies only to the seam-rendered `instructions` (tool names, not CLI syntax).
- **Errors:** domain failures map to a tool result with `isError = true` and
  `{ error: { kind, msg } }`, where `kind` is the foundation's closed `ErrorKind` verbatim; only
  protocol/start failures are JSON-RPC errors. The read surface emits a subset of the kinds —
  `no_workspace`, `not_found`, `validation`, `io` — the `cycle`/`conflict` write-path kinds and
  `verification` (signature checks) cannot arise on a read.
- **Connect-time instructions (#76u.1, #76u.5):** the `initialize` response carries STATIC usage
  guidance — a "call `flow_next` first" nudge, the tool reference, and each active module's workflow
  rules (the static part of `nxs prime`, command references seam-rendered to tool names) — so a host
  that honors `instructions` (Claude Desktop does) knows the platform without a tool call. No live
  ready/blocked snapshot: `instructions` are sent once at connect (= host start for stdio) and would
  go stale, so the live set comes from `flow_next`/`flow_list`. Byte-stable regardless of store
  state; absent a launch workspace the server starts cleanly with no instructions.

**Not in this vertical:** the memory tool line (`76u.2`/`.3`/`.4`, waiting on the memory facade
E5m), the `contributes`/`archive`/`unarchive` flow writes (facade ops not in the `yie` cut — a
follow-up), the rest of the read tools (`blocked`/`search`/`note_list`/`mention_list`),
install/distribution, and HTTP/remote transports — all later slices of the epic.

## Workspace discovery

MCP hosts have no useful cwd, so the launch workspace is pinned per server instance. The MCP persona
resolves it in this order (`crates/nxs/src/mcp/mod.rs::launch_workspace`), and — unlike the CLI — has
**no cwd walk-up** tier (a host's cwd is meaningless):

1. `--db`/`NXS_DB` — a direct store override (its parent dir is the workspace).
2. `--workspace`/`NXS_WORKSPACE` — an explicit launch root, honored verbatim.
3. **App-Data-Home** (the default, #76u.7) — a stable per-user directory, auto-initialized with a
   `.nxs/` on first start so the server "just works" under a host that passes no flags.

```
nxs mcp serve --workspace /path/to/project      # or NXS_WORKSPACE=/path/to/project
nxs mcp serve                                    # no flags → the App-Data-Home default (auto-init)
```

### App-Data-Home: a neutral, per-user default (#76u.7, 4cmg)

The default (tier 3) is a **neutral, nxs-owned** directory — deliberately **not** any consumer's
app-data home. It resolves `dirs::data_dir()/<identifier>` with the identifier `com.nxsflow.nxs`:

| OS      | Resolved App-Data-Home                                              |
|---------|--------------------------------------------------------------------|
| macOS   | `~/Library/Application Support/com.nxsflow.nxs/`                    |
| Linux   | `$XDG_DATA_HOME/com.nxsflow.nxs/` (else `~/.local/share/com.nxsflow.nxs/`) |
| Windows | `%APPDATA%\com.nxsflow.nxs\` (Roaming)                             |

- nexus-flow reaches it via `directories::BaseDirs::data_dir().join(APP_DATA_IDENTIFIER)` — `BaseDirs`
  (not `ProjectDirs`, which applies platform-specific naming and would diverge off macOS).
- the board is seeded with **flow active + the neutral bundled `issue-tracker` plugin** (`nxs init`'s
  non-interactive default), so the OSS server just works without assuming any consumer's plugin.

**Why neutral (4cmg).** Earlier this identifier was the nexflow.it app's `it.nexflow.app`, so the
standalone server and that app shared one board. That coupled the OSS default to a specific
proprietary consumer — the consumer's plugin would have to be known there — which is wrong for an
engine that ships bundled invisibly under *many* consumers. The default is now consumer-agnostic.

**Consumer override.** A consumer that wants its own board (e.g. the nexflow.it installer) installs
its **own** MCP entry pinned to its own `--workspace`/`--db`, which overrides this default; its
installer may also **import** this neutral board's data. No cross-repo identifier contract remains —
each side owns its own workspace root.

The identifier is pinned by a co-located tripwire test (`app_data_home_is_the_neutral_nxs_identifier`
in `crates/nxs/src/mcp/mod.rs`, plus the resolved-path assertion in
`default_workspace_auto_inits_app_data_home_not_cwd`, `crates/nxs/tests/mcp.rs`) so a rename is a
**deliberate** edit — it moves the on-disk path of every existing default board.

### Per-tool workspace override (S7/ppa)

The launch workspace above is the **default**, not a hard pin: every read AND write tool takes an
optional `workspace` argument that retargets that ONE call to a different board — so a single running
server serves many workspaces without a registry. Omitted (or empty) ⇒ the launch default. A
non-empty path is resolved on its own (dropping the launch `--db`) with the CLI's walk-up discovery,
so a path with no `.nxs/` in itself **or any ancestor** is a `no_workspace` **domain** error on the
tool result (run `nxs init` there) — distinct from a missing launch default, which fails at `serve()`
start. See `crates/nxs/src/mcp/mod.rs::effective`.

### Multi-workspace registry (0jq8)

The per-tool override lets one server reach many boards, but a host must still know each path. An
optional `~/.nexusflow/workspaces.toml` registry closes that gap: a small `[[workspace]] name = … path
= …` list a host reads via the umbrella **`list_workspaces`** tool — and the `nxs mcp workspaces` CLI,
whose `--json` is byte-identical to the tool's `structuredContent.items` (the parity invariant applied
to an umbrella op, via one shared builder) — to pick a board **by name**, then pass its `path` as the
per-tool `workspace` override above. `nxs mcp install --workspace <path>` upserts the pinned board
(keyed by absolute path, name = final component) idempotently; the file is also hand-editable. The
list is deterministic (name-then-path sorted). The registry is a convenience **index**, not an
access-control boundary — under the local-stdio trusted-host model `list_workspaces` discloses every
registered path regardless of launch scope, and a registered path is never treated as pre-validated
(resolving a stale entry still surfaces the normal `no_workspace` error). The registry itself lives
in `crates/service/src/registry.rs` since 6j6v.5zst — below every crate with a CLI, so an embedding
app can register and DEREGISTER a workspace without linking `nxs`; `crates/nxs/src/workspaces.rs`
keeps the rendering and re-exports the rest under its old names.

## Manual verification against a real host (Claude Desktop)

The automated acceptance test (`crates/nxs/tests/mcp.rs`) already drives a real `rmcp` client over
stdio end-to-end. To verify against Claude Desktop:

1. Build the binary: `cargo build --release` (the `mcp` feature is on by default). Note the absolute
   path to `target/release/nxs`.
2. Have a workspace: run `nxf init` (or `nxs init`) in a project directory so it has a `.nxs/` store
   with some items.
3. Add the server to Claude Desktop's MCP config (`claude_desktop_config.json`):

   ```json
   {
     "mcpServers": {
       "nexus-flow": {
         "command": "/absolute/path/to/target/release/nxs",
         "args": ["mcp", "serve", "--workspace", "/absolute/path/to/your/project"]
       }
     }
   }
   ```

   An absolute `command` path is required — if `nxs` is not on the host's `PATH`, a bare name fails
   to launch (install/distribution is a later slice).
4. Restart Claude Desktop. The `nexus-flow` server should connect; `flow_next`/`flow_show`/
   `flow_list` appear as tools (there is no `flow_prime` — #76u.6), and the static usage guidance
   arrives as the session's instructions. Ask it to list ready work, show an item, etc., and confirm
   the records match `nxf <command> --json` — for `flow_next`/`flow_list` the CLI array is under
   `structuredContent.items` (#76u.9).
