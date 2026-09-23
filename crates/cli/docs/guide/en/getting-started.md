# Getting Started

`nxf` is the nexus-flow agent CLI: a thin, deterministic, offline-first interface over the
engine core. This guide takes you from an empty directory to a planned, tracked piece of work.
Every command supports `--json` for machine-readable, byte-stable output — that is the contract
agents build on.

## Install

Download and install the latest release with the install script:

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

It detects your platform, verifies the download (sha256 + signature), and puts one binary on your
`PATH`, **`nxs`** (the umbrella that ties the tools together), with **`nxf`** (the issue tracker),
**`nxm`** (durable agent memory) and **`nxc`** (the channel) linked to it. You install once and
choose which tools to **activate per workspace**. Check it:

```bash
nxf --version
```

## Initialize a workspace

A workspace is a `.nxs/` directory in your project — one shared store that every tool you activate
writes into. The quickest way to set one up is the umbrella, which asks which tools to use and
wires everything together:

```bash
nxs init
```

It assembles the shared agent file and wires one SessionStart hook per active tool (`nxf prime`,
`nxm prime`, `nxc prime`), so from then on your agent is handed the context of every active tool at
the start of a session (see
[the nxs umbrella](#the-nxs-umbrella)). On an agent or in a script it is non-interactive —
`nxs init --module flow --module memory` (or `--json`) sets up the same state with no prompt.

If you only want the issue tracker, initialize it directly. This is a **deliberate plugin choice**
— there is no default, because the plugin decides the vocabulary you read and write (see
[plugins](nxf-plugins)). For software work, pick `issue-tracker`:

```bash
nxf init --plugin issue-tracker
```

Either path lands on the same `.nxs/` workspace, the same per-module SessionStart hooks, and the
same entry with the background service — the list it attends is where a deferral or a window
declared here gets something that will actually look at it. Every id in this
workspace is minted under a short, stable namespace called the `prefix` (the examples below use
`ab12`). Run `nxf init --help` to see the available plugins and their descriptions.

## Create your first items

Items are **projects** and **tasks**. Create a project to group the work, then the tasks under
it. The `--json` record is the canonical shape — note the engine-level field names (`type`,
`belongs_to`, `status`), which never change regardless of the active plugin:

```console
$ nxf create --type epic --title "Ship v1" --description "Cut the first release" --priority P1 --json
{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Cut the first release","design":null,"due":null,"id":"ab12.0001","priority":"1","status":"open","title":"Ship v1","type":"epic"}
```

```console
$ nxf create --type feature --title "Write the CLI" --description "Build the command-line tool" --priority P1 --due 2026-12-31 --parent ab12.0001 --json
{"archived":null,"assignee":null,"belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":null,"due":"2026-12-31","id":"ab12.0002","priority":"1","status":"open","title":"Write the CLI","type":"feature"}
```

The new task `belongs_to` the project (via `--parent`), is due by a date, and carries priority
`1`.

Long fields need no shell-escaping: pass `-` to read one from STDIN
(`cat design.md | nxf create … --design -`), use `--description-file ./desc.md`, or pipe the
whole item as JSON with `nxf create --json -`. See [commands](nxf-commands) for the full set.

## See what to work on

State like *ready* is **derived**, never stored (see [core-concepts](nxf-core-concepts)).
With nothing blocking it, the work is ready, and `next` ranks it by the plugin's policy. The
human view speaks the issue-tracker vocabulary — `P1`, `open`:

```console
$ nxf next

0001  P1  open  [epic]  Ship v1
0002  P1  open  [feature]  Write the CLI
    ↳ 0001 · Ship v1
```

That is the whole loop: `init` once, `create` work, then let `next` tell you where to
go. From here, read [core-concepts](nxf-core-concepts) for the model, or [commands](nxf-commands) for a
full working session.

## The nxs umbrella

`nxs` is the one binary under the suite; `nxf`, `nxm` and `nxc` are links to it. `nxs`
ties together whichever you activate in a workspace, over the one shared `.nxs/` store:

- `nxs init` — set up the suite: pick which tools to use (interactively, or `--module …`/`--json`
  for an agent), assemble the shared agent file, and wire one SessionStart hook per active module
  (`nxf prime`, `nxm prime`, `nxc prime`).
- `nxs prime` — the session bootstrap you run by hand: it fans out to the `prime` of each active
  tool with one shared clock and concatenates the results. A flow-only workspace yields exactly
  flow's prime; memory's joins after you add it.
- `nxs sync bind` / `nxs sync run` — sync the shared store with a relay (one op-log, so syncing is
  a suite operation, not a per-tool one). Binding a git repo needs no flags — the stream id is
  derived from `origin`. On macOS this also installs `nxs sync daemon` (a background launchd
  agent, `--no-daemon` to skip) so the workspace stays synced on its own; elsewhere the daemon
  must be run explicitly, in the foreground.
- `nxs migrate` — raise the workspace to the current schema, and bring an out-of-date SessionStart
  wiring up to the current one: one entry per active module. It converges both shapes that shipped
  before — the pre-v0.6.0 `nxf prime` hook and the single `nxs prime` umbrella hook — and adds the
  entry of a module that has joined since. A workspace with no hook of ours is left alone; `migrate`
  repairs a wiring, it does not decide you want one. Idempotent.
- `nxs doctor` (alias `nxs status`) — a cross-tool diagnosis: active modules, schema version,
  replica identity, sync state, and store integrity.

Each tool keeps its own branded init when you type it directly (`nxf init`, `nxm init`); they all
land on the same `.nxs/` store and the same per-module SessionStart hooks.
