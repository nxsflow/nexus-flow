# The Workspace

`nxs init` leaves state in three places. This page says what each one is, so you can tell what is
yours to edit, what is generated, and what never leaves this machine.

## `.nxs/` — the workspace itself

```
.nxs/
  config.toml    which blocks are active, plus each block's own settings
  db.sqlite      the one store every active block writes into
  replica.toml   this machine's identity: the id prefix and site id
  signing.key    the key every op this workspace writes is signed with — readable by you alone
  .gitignore     a single `*` — the directory ignores itself
```

`config.toml` is the only file here meant for human eyes:

```toml
active_modules = ["flow", "memory", "chat"]

[flow]
plugin = "issue-tracker"
```

`active_modules` is what `nxs prime` and `nxs guide` fan out over, and each block contributes its
own `[<block>]` table. You can edit it, but `nxs init` is the safer route — it validates the names
and runs each block's own setup.

**The store is device-local and git-ignored.** `.nxs/.gitignore` is a single `*`, so the workspace
never lands in a commit: your board does not travel with your code. That is a deliberate choice, not
an oversight — the way to share state is a sync stream (`nxs sync`), and the way to share a decision
is the pull request. If you came expecting the tracker in git, this is the thing to know first.

**`signing.key` is a secret, and it is this workspace's identity.** Every op the workspace writes is
signed with it, and another machine that trusts its key (`nxs sync key`, then `nxs sync trust add`
there) lets its agents act on what this workspace starts. On macOS and Linux it is readable by
you alone; on Windows it is as private as the folder the workspace sits in. Do not copy it to another machine — a
second workspace is a second replica with a key of its own — and do not delete it: a new key would
make this workspace's earlier ops look foreign to machines that trusted the old one.

A pre-existing `.nexusflow/` directory from an older version is renamed to `.nxs/` when the
workspace is opened — idempotently, and only where the workspace actually resolves, so nothing is
stranded and opening an old workspace repeatedly is harmless.

## The shared agent file

Each active block contributes a managed section to `AGENTS.md`, assembled into one document rather
than three competing ones. That file **is** committed: it is how a contributor — or an agent on
another machine — learns the conventions of this project.

`CLAUDE.md` is deliberately not a second copy. It belongs to the host that *runs* the session hooks
below, so a block there would be a second delivery of what the hook has already handed the session
— and the memory binding in it would arrive as every memory's full body instead of the budgeted
index. `nxs init` therefore only ever takes a retired block back *out* of `CLAUDE.md`; it never
writes one.

## The session hooks

`nxs init` wires **one SessionStart hook per active block** into `.claude/settings.json`, each
running that block's own `prime`. **Memory's** entry — and only memory's — carries a
`|| cat NEXUS_MEMORY.md` tail, so that a contributor who has *not* installed the suite still gets
the project's memory: the settings file is committed, and on their machine the block binaries are
simply not commands. It hangs on one entry rather than all of them because three entries carrying
it would deliver the same file into the same session three times.

A failing hook is left to fail loudly rather than being papered over with an `echo`. A hook that
exits zero while delivering nothing is worse than one that says the tool is missing.

## Checking it is healthy

```bash
nxs doctor
```

`doctor` — `nxs status` is the same command — reports the active modules, the schema version and
its standing, the replica identity, sync state, the op count, and a database integrity check. It
adds one warning line if a half-finished migration from beads is still wired, and one if two ops
share a single `(lamport, site)` coordinate — something only a log written before nxs could refuse
it carries, and on which a concurrent edit is decided by arrival order rather than the CRDT order.
A clean workspace prints neither. It is
foundation-only: it works whatever blocks are active, and it opens no product view, so it still
answers when a block is unhappy.

```bash
nxs migrate
```

The schema is raised on open by default; `migrate` is the explicit lever for CI and repair. It says
which of the two things happened rather than succeeding silently — `schema v3 → v4` when it really
migrated, and `workspace db already current (schema v4)` when there was nothing to do, which is the
usual answer in CI.

## Going further

- Sharing a workspace between machines or people: `nxs sync`, and
  [running a relay](nxf-running-a-relay) for the server half.
- What each block does with the store: [modules](nxs-modules).
