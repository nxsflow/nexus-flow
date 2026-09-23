# Getting Started

`nxs` is the umbrella of the nexus-flow suite. It is **one binary** carrying three building
blocks — **flow** (`nxf`: projects, tasks, dependencies), **memory** (`nxm`: durable project
knowledge) and **chat** (`nxc`: the channel between you and your agents) — over one shared
store. You install it once and then decide, **per workspace**, which of the three to activate.

This guide takes you from nothing installed to a workspace whose agent restores its own context at
the start of every session. It is the one place that first setup is written down; the building
blocks' own guides start from here.

## Install

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

The script detects your platform, verifies the download — a sha256 **and** a signature — and
installs **one** program: `nxs`. The other names are symlinks to it: `nxf`, `nxm` and `nxc` are the
same binary, which decides which persona it is from the name you typed. That is why the four can
never drift apart in version — there is only one program to update.

Signature verification needs either `minisign` or an OpenSSL that can do Ed25519. A stock macOS has
neither (its `openssl` is LibreSSL), so either install minisign — `brew install minisign` — or
bootstrap with `NXF_INSECURE=1`, which checks the sha256 alone and proves integrity but not origin.
`nxs self-update` verifies signatures unconditionally from then on.

```bash
nxs --version
```

## Set up a workspace

A workspace is a `.nxs/` directory in your project: **one** store that every activated building
block writes into. Create it with:

```bash
nxs init
```

On a real terminal this asks which building blocks you want, with the arrow keys. A block's own
sub-configuration runs inside that frame — flow asks which plugin to use, because that choice
decides the vocabulary you will read and write for the life of the workspace.

On an agent, or in a script, name the blocks instead of being asked:

```bash
nxs init --module flow --module memory --json
```

Either way you land on the same workspace. To activate another block later, run `nxs init` again
and tick it — the command is idempotent and adds rather than replaces.

## What `nxs init` leaves behind

Four things, and it is worth knowing all four:

- **`.nxs/`** — the workspace itself: one SQLite store that every active block folds its own views
  over. It is git-ignored and local to this machine.
- **The shared agent file** — the `AGENTS.md` section each active block contributes, assembled
  into one document rather than three competing ones. Not `CLAUDE.md`: that belongs to the host
  running the hook below, which has already delivered the same thing.
- **One SessionStart hook per active block** — `nxf prime`, `nxm prime`, `nxc prime`.
- **An entry with the background service** — the workspace is added to the list that service
  attends, so a deferral or a window declared here has something that will actually look at it. It
  is a local index entry and nothing more: it does not sync anything anywhere, and nothing is sent
  off this machine. `nxs sync unregister` takes it back off the list.

[The workspace](nxs-the-workspace) goes through each of them in detail.

## The background service, and the question `init` asks about it

One process per machine keeps the deadlines of every workspace it attends, and syncs the ones that
are bound to a stream. Without it, a deferral that comes due in this project simply never fires —
which is why `nxs init` offers, on a real terminal, to set it up:

```
Set up the nexus-flow background service on this machine?
```

Say no and the answer is recorded in this workspace, so no later `nxs init` asks again; the frame
still tells you where the service stands, and `nxs sync daemon install` sets it up whenever you
change your mind. In a script or under `--json` the question is never asked and nothing is
installed: name it explicitly with `nxs init --service`, or settle it with `nxs init --no-service`.

The installer is macOS-only (it is a launchd agent). Everywhere else, run `nxs sync daemon` in the
foreground under your own supervisor — systemd, runit, whatever you already have — and it keeps
exactly the same deadlines.

## What restores context at the start of a session

The hooks are **one per active block**, not one for the umbrella — and that is a deliberate
reversal of the earlier shape. The host truncates each hook's output on its own, at 10,240 bytes,
so three hooks carry three budgets instead of one. That was measured, not assumed: three blocks'
hooks of roughly 8 KB each arrived whole, 24 KB between them, where one combined hook would have
been cut off at 10 KB.

`nxs prime` still exists, and it is still the fan-out:

```bash
nxs prime
```

It runs the `prime` verb of every **active** block and hands back one answer — flow's board,
memory's index, chat's channel — in a fixed order, with a single shared `now`, so a deferred date
means the same thing to every block in the same run. That is what you type when you want the whole
picture in one place. The session hooks simply no longer go through it.

Either way the effect is the one that matters: an agent starts a session already knowing what this
project is doing, what it has learned, and who else is working on it. A flow-only workspace yields
exactly flow's prime; memory appears the day you activate memory. Nothing is configured for that —
the hook set is derived from the active blocks.

## Which block do I reach for?

Short version: **flow** for what needs doing, **memory** for what the project knows, **chat** for
the messages between you and your agents. [Modules](nxs-modules) is the longer answer, and it is
worth reading before the three detail guides.

## Where to go next

- [modules](nxs-modules) — what each building block is for
- [the workspace](nxs-the-workspace) — what is on disk, and how to keep it healthy
- Then the block you actually need: [flow](nxf-getting-started), [memory](nxm-getting-started),
  [chat](nxc-getting-started)
