# Connect an MCP host

MCP-native hosts — Claude Desktop, Claude Code, Cursor, Windsurf — can't drive the `nxf` CLI, so
nexus-flow ships an **MCP server** (`nxs mcp serve`) that exposes your board's read **and** write ops
as MCP tools over the same shared `.nxs/` store. This guide connects a host to it, including the case
where **nothing is installed yet**.

A host config only launches a *command*. So connecting a host is two decisions: **which command**
launches the server, and **which workspace** it opens.

## The quickest path: the `npx` runner (no install)

If you have Node (most machines do) you don't need to install
anything first. Point the host at the `@nexus-flow/mcp` runner — it fetches the signed `nxs`, verifies
it, caches it, and starts the server on first launch.

For **Claude Desktop**, add this to `claude_desktop_config.json` and restart:

```json
{
  "mcpServers": {
    "nxs": {
      "command": "npx",
      "args": ["-y", "@nexus-flow/mcp"]
    }
  }
}
```

That serves your **default board** — a per-user workspace nexus-flow creates automatically on first
start (the same one the nexflow.it desktop app uses, so you see one board in both). To point it at a
specific project instead, append the workspace after `--`:

```json
{
  "mcpServers": {
    "nxs": {
      "command": "npx",
      "args": ["-y", "@nexus-flow/mcp", "--", "--workspace", "/absolute/path/to/your/project"]
    }
  }
}
```

Everything after `--` is passed straight to `nxs mcp serve`. The first launch downloads and verifies
`nxs` (a few seconds); later launches use the cache and start instantly, offline.

## If you already have `nxs`: `nxs mcp install`

With `nxs` on your `PATH`, one command registers the server in every installed host for you — no
hand-editing JSON:

```bash
nxs mcp install
```

It writes an idempotent entry naming the **absolute path** of your `nxs` binary plus `mcp serve`, and
patches Claude Desktop, Cursor, and Windsurf if it finds them (re-running changes nothing). Add
`--setup` to also **create the board** in the same step — one command that both registers the server
**and** materializes the workspace it opens:

```bash
# Register + create a personal-todo board at the per-user default (knowledge-worker default):
nxs mcp install --setup

# Register + create a coding board pinned to this project:
nxs mcp install --setup --workspace "$PWD" --plugin issue-tracker
```

To write the portable, machine-independent `npx` entry from the command line instead of the direct
binary path, add `--runner npx`.

## Choosing the workspace

Because a host has no working directory, the server can't guess "this project" — you say which board
it opens:

- **Omit `--workspace`** → the per-user **default workspace**, created automatically on first start.
  Best for a single personal board shared across your hosts and the desktop app.
- **`--workspace <absolute-path>`** → that project's board. Create it first (`nxs mcp install --setup
  --workspace <path>`, or `nxs init` there) — an explicit path with no board is an error, not an
  auto-create.
- A single running server can also be pointed at a **different** board per tool call by passing a
  `workspace` argument to any tool — one server, many projects, no re-registration.

## Bootstrapping `nxs` when it isn't installed

Two signed ways to get `nxs`, both fail-closed:

- **The one-line installer** (installs `nxf`/`nxm`/`nxs` under `~/.local/bin`, no sudo):

  ```bash
  curl -fsSL https://nxsflow.com/nxs/install.sh | sh
  ```

- **The `npx` runner** (above) — it *is* the bootstrap for GUI hosts: it downloads and verifies `nxs`
  on demand, so the host config is the only thing you add.

## Security

Every fetch path verifies before it runs, and there is **no** insecure escape hatch on the `npx`
route:

- **sha256** proves the bytes weren't mangled in transit.
- **minisign** (an Ed25519 signature over the tarball, checked against a public key baked into the
  installer and the runner) proves the bytes are genuinely ours. A tampered artifact, a wrong key, or
  a missing signature aborts before anything runs.

Only bytes that pass **both** gates are cached and executed. `install.sh` fails closed on a host with
no verifier available (rather than install unverified bytes), and `nxs self-update` always re-verifies.

## Other hosts

The same entries work for **Cursor** and **Windsurf** (and any MCP host that launches a stdio
command). `nxs mcp install` detects and patches all three; to target one explicitly, pass
`--host claude-desktop | cursor | windsurf`. Once connected, the server sends usage instructions at
connect time, and `flow_next` returns your ready work — call it first in each session.
