# MCP cold-start onboarding

The guided path from an NL wish in a host — *"I want to use nexus-flow in this project"* — to a live
server: **`nxs` available + a workspace created + the MCP server registered with the host**, working
even when `nxs` is **not installed**.

This is the *connective tissue* over the shipped distribution pieces — it **coordinates them, it does
not replace or duplicate them**:

- **S11 / `c3i`** — `nxs mcp install`: the idempotent host-config writer (`mcpServers.nxs` entry).
- **S12 / `65y`** — `nxs mcp install --setup`: register **and** materialize the board in one command.
- **S13 / `kz8`** — `@nexus-flow/mcp`: the `npx` runner-shim, the "not installed at all" path.
- **S14 / `ucp`** — the docs/website that publishes these steps for end users (copy-paste snippets).
- **S24 / `76u.7`** — the App-Data-Home default workspace + auto-init (the "no `--workspace`" tier).
- **S7 / `ppa`** — the per-tool `workspace` override (one launched server, many boards).

The E9 server itself is specified in [`E9-mcp-server.md`](./E9-mcp-server.md); this doc is only the
onboarding seam.

## Why this is the decisive first contact

The most common first contact is an agent in a host (Claude Desktop/Code, Cursor, Windsurf, Amazon
Quick) where the
user says, in effect, *"use nexus-flow in this project"* — and `nxs` is **not** on `PATH`. A host MCP
config only launches a command; if that command is a bare `nxs`, the launch **fails silently**.
Without a guided cold-start the MCP seam is unreachable for a newcomer, so this is adoption-critical
(hence P1, even though it is design- and docs-heavy and builds entirely on shipped pieces).

## Design decisions (the ticket's open questions, resolved)

### 1. Who runs the flow — layered by host capability, always reusing the shipped pieces

There is no single mechanism; the right one is a function of what the host can do. All three reuse the
shipped distribution — **never a bespoke new installer**.

| Host kind | Mechanism | Needs `nxs` first? |
|---|---|---|
| GUI hosts (Claude Desktop, Cursor, Windsurf, Amazon Quick) | copy-paste the **`npx` runner-shim** entry (`kz8`) | **No** — only Node |
| Shell/coding agents (Claude Code) with `nxs` on PATH | **`nxs mcp install --setup`** (`c3i`+`65y`) — one command registers **and** creates the board | Yes |
| Shell/coding agents without `nxs` | one-time signed `install.sh`, then `nxs mcp install --setup --runner npx …` | Bootstrapped, then yes |

**The `npx` runner-shim is the canonical answer to "runs without pre-installed `nxs`."** It is the
*only* entry that works before `nxs` exists: Node execs `npx -y @nexus-flow/mcp`, which fetches +
verifies + caches the signed `nxs` on first launch and then execs `nxs mcp serve` (`kz8`). GUI-host
users can't run install commands, so for them the guided path **is** "add this one config entry"
(`ucp` publishes it). `nxs mcp install --runner npx` writes exactly that entry once `nxs` is available,
so the same portable entry is reachable from the command line too.

### 2. "In this project" → `--workspace` — the host has no cwd, so the path is supplied explicitly

A host has no meaningful cwd, so the MCP persona **dropped cwd discovery** (`76u.7`). "In this
project" therefore cannot be inferred by the server — the project path must be supplied **explicitly**
by whoever writes the entry: a coding agent knows its own `$PWD`; a GUI user pastes an absolute path.
Three modes, straight off the shipped resolution tiers:

- **Omit `--workspace`** → the per-user **App-Data-Home** default, **auto-initialized** on first start
  (`76u.7`) — a neutral, nxs-owned board (`com.nxsflow.nxs`, 4cmg), not any consumer's; the host entry
  stays flag-less. A consumer that wants its own board installs its own `--workspace`-pinned entry. Note
  the plugin: the server's *own* auto-init seeds `DEFAULT_PLUGIN` = **`issue-tracker`** (the coding
  board), **not** personal-todo. Only `nxs mcp install --setup` (without `--plugin`) defaults to
  personal-todo — a deliberate first-writer-wins divergence: whichever path writes `.nxs/` first seats
  the plugin, and the server then reads it from `config.toml` and never re-seeds.
- **`--workspace <project-abs-path>`** → pin that project's board in the entry. A path with no `.nxs/`
  in it **or any parent** is an error (it is **not** auto-init'd — only the App-Data-Home is;
  resolution walks up as the CLI does), so create it in the same step with `--setup`. This is the
  coding-agent reading of "in this project": `nxs mcp install --setup --workspace "$PWD" --plugin issue-tracker`.
- **Per-tool `workspace` override (`ppa`)** → one launched server can read/write a *different* board on
  a single tool call, so a host pinned to the shared default can still act "in this project" per call
  without re-registering.

The install writer and the server resolve the workspace through the **same** rule (`65y`'s design
trap: setup target == server launch-resolution), so setup and the server never point at two different
boards.

### 3. Security — every fetch path is signature-verified, fail-closed

Bootstrapping fetches an executable, a supply-chain vector, so no path ever execs unverified bytes:

- **`npx` shim** — sha256 (integrity) **and** minisign (Ed25519 over BLAKE2b-512, authenticity)
  against the pubkey embedded in the package, verify-**before**-cache, no insecure escape hatch
  (`kz8`).
- **`install.sh`** — sha256 mandatory + minisign authenticity; on a host with **no** verifier it
  **fails closed** (refuses to install), `NXF_INSECURE=1` being the only, loud, opt-out. `nxs
  self-update` afterward always re-verifies minisign fail-closed.
- The **same** embedded minisign pubkey is used by `nxs`, `install.sh`, and the `npx` shim.

## The guided path (runbook)

From the NL wish to ready work:

1. **Wish** — the user tells the host agent *"I want to use nexus-flow in this project."*
2. **Pick the entry by host capability** (decision 1):
   - GUI host → add the `npx` runner-shim entry to the host's MCP config (canonical snippet in the
     [`@nexus-flow/mcp` README](../../npm/mcp/README.md); `ucp` publishes a copy-paste version):

     ```json
     {
       "mcpServers": {
         "nxs": {
           "command": "npx",
           "args": ["-y", "@nexus-flow/mcp", "--", "--workspace", "/absolute/path/to/project"]
         }
       }
     }
     ```

     Drop the trailing `--workspace …` to use the neutral App-Data-Home default instead.
   - Coding agent with `nxs` → `nxs mcp install --setup [--workspace "$PWD"] [--plugin issue-tracker]`
     (registers every installed host and creates the board in one idempotent command).
   - Coding agent without `nxs` → `curl -fsSL https://nxsflow.com/nxs/install.sh | sh`, then
     `nxs mcp install --setup --runner npx [--workspace "$PWD"] [--plugin issue-tracker]` (writes the
     portable, machine-independent entry).
3. **Choose the workspace mode** (decision 2): shared default (omit `--workspace`) · pinned project
   (`--workspace "$PWD"` + `--setup`) · per-tool override later (`ppa`).
4. **Restart / reconnect the host.** The server connects; the connect-time `instructions` (`76u.1`)
   arrive, and `flow_next` returns the ready set. First `npx` launch downloads + verifies `nxs` (a few
   seconds); later launches use the cache and start instantly, offline.

The outcome matches the acceptance: a documented, guided path to **`nxs` available + workspace created
+ MCP entry registered**, working **without** pre-installed `nxs` (the signature-verified `npx` shim),
with the project→`--workspace` mapping made explicit.

## What is intentionally NOT here

- The polished, localized end-user pages + copy-paste snippets live on the site (`ucp`).
- The mechanics of the install writer (`c3i`/`65y`), the shim (`kz8`), and the App-Data-Home
  (`76u.7`) live with those tickets; this doc references them.
- Live npm publishing of `@nexus-flow/mcp` (so the real `npx` path resolves against the CDN) is the
  ops step in `76u.15`; until then the shim path is exercised by its own tests, not the live registry.
