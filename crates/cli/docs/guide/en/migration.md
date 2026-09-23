# Migration

This guide covers two kinds of migration: bringing **existing work** into nexus-flow, and moving
between **major versions** of nexus-flow itself.

## Bringing an existing tracker in

nexus-flow has no bulk importer — and does not need one. Items are created through the same
`nxf create` you use day to day, so a migration is a short script that walks your old tracker and
calls `nxf` once per item, mapping each record onto the [core model](nxf-core-concepts): a `--type`
(project or task), a `--title`, optionally a `--parent`, `--priority`, `--due`, and `--description`.
Re-create dependencies afterward with `nxf dep add`. Because every command takes `--json` and is
deterministic, the script is easy to write and verify:

```bash
# sketch: one create per legacy ticket, then wire dependencies
nxf create --type project --title "Imported backlog" --json
nxf create --type task --title "Legacy #1234" --parent ab12.0001 --priority P2 --json
nxf dep add ab12.0002 ab12.0003
```

Pick the [plugin](nxf-plugins) whose vocabulary matches the source tracker at `nxf init` — that is
the only up-front decision the import depends on.

## Binding a sync stream

nexus-flow is offline-first: you work locally against `.nxs/`, and a **sync stream**
converges every replica on the durable server truth. In a git repo, binding needs no flags at
all — the stream id is derived from the repo's `origin` remote, so every clone lands on the same
stream automatically, with nothing to copy between machines:

```bash
nxs sync endpoint https://your-relay.example.com   # set once, machine-wide
nxs sync bind
```

Use `nxs sync bind --create` to mint a fresh stream when there's no remote to derive from (or you
deliberately want an independent stream), or `nxs sync bind --join <stream_id>` to attach to a
specific shared id instead. `nxs sync bind --endpoint <url>` pins just this workspace to a
different relay than the global default. Re-running `bind` with the same id is a no-op; switching
to a different stream needs the explicit `--rebind` flag.

The relay has no authentication yet, so a derived stream id is only as private as your repo's
remote URL — anyone who knows it can compute the same id and read/write that stream on any
reachable relay (`bind` prints a reminder of this every time it lands on the derived path). Treat
the relay endpoint as private/trusted until authentication ships; `--create` mints a random,
undiscoverable id instead if you need one that isn't derivable from the remote.

On macOS, a successful bind also installs `nxs sync daemon` as a background (launchd) agent, which
keeps the workspace in sync continuously — on an interval, on wake, and shortly after a local
write — so you rarely need to sync by hand; pass `--no-daemon` at bind time to opt out. On other
platforms nothing is installed automatically: run `nxs sync daemon` yourself in the foreground
under your own process supervisor, or just run a pass by hand:

```bash
nxs sync run
```

Work continues offline between runs; sync exchanges changes and merges them deterministically
(the engine is a CRDT, so concurrent edits converge without a coordinator). The server is the
durable record, not a lock — no one has to be online for you to make progress.

## Migrating across major versions

nexus-flow follows plain SemVer. Within a major version, upgrades (`nxs self-update`) are
drop-in. A **major** version bump (e.g. `1.x` → `2.0`) is the only place a breaking change may
land, and every such change ships a migration note describing what is automatic and what needs
your attention. Those notes are aggregated per major version into a single upgrade guide.

Read the aggregated notes for a major before upgrading to it — they live alongside this guide on
the website `/docs` and in the release notes for the version. Within a `0.x` series there is no
stability guarantee yet, so review the changelog on each update.
