# Architecture

This page is the map: what the pieces are, which way the arrows point, and which seams are
contracts. It is deliberately readable on its own — the picture below illustrates it, it does not
carry it, because the terminal cannot show you a picture and an agent reading `nxs guide` deserves
the same account you get.

![nexus-flow architecture at a glance: callers, surfaces, engines, the shared op-log, and the sync edge](/nxs/docs/assets/nexus-flow-simplified.webp)

The rendered diagram is on this topic's page at
`nxsflow.com/docs/develop/architecture`; in a terminal you are reading the text version,
which says the same thing.

## Five layers, top to bottom

**1 — Who talks to the system.** Coding agents (each active block installs its own SessionStart
hook), humans at a terminal, MCP hosts, and embedding apps like manufakt.io and nexflow.it.

**2 — Surfaces.** ONE binary, four personas dispatched by `argv[0]`: `nxs` is the umbrella
(`init`, `prime`, `doctor`, `status`, `sync`, `self-update`, `mcp serve`), and `nxf`, `nxm`, `nxc`
are the three blocks. They are thin callers — no surface holds a model of its own.

**3 — Engines.** flow, memory and chat, each behind its own engine seam. Only flow's
(`crates/facade`) is a SemVer contract; the other two are seams without that promise yet. Plugins
register here at compile time, which is how a consumer supplies a vocabulary the open-source
engine never ships.

**4 — Substrate.** One workspace, one log. `.nxs/db.sqlite` is git-ignored and device-local, and
an append-only `ops` table inside it is the single source of truth. Every module appends to THE
SAME log.

**5 — The outward edge.** An anti-entropy client, driven by a background service, pushes local ops
to `nxf-relay` — a dumb, durable op carrier on SQLite or Postgres — where other replicas converge
over the same stream. The server stores and hands back ops; it never folds, derives or validates.

Two arrows are worth naming on their own. An embedding app links the **engine seam directly** and
never shells out to the CLI — the app and the CLI are equals over one core, which is why every CLI
verb has a counterpart on the seam. And the substrate arrow runs one way only:

```
ops (append-only)  →  fold  →  materialized views  →  derivation
```

`nxf next` and `nxf blocked` are **computed in SQL over the views, never stored**. That is what
keeps the work list consistent no matter which order ops arrived in — and it is why merging two
replicas cannot produce a board that disagrees with itself.

## What the picture does not show

Two things, and both are deliberate.

It is a **building-block view**: it says what the parts are and which way they call each other, not
what happens over time. Everything above is the map; none of it is the trip — and the trip has its
own page: [the journey of one operation](develop-the-journey-of-one-op) follows a single verb from
the surface down to the log and back up to an answer nobody stored.

And it is **coarse at crate level**: six blocks for a crate map of fourteen. `nxs-foundation`, the
sync client's internals, the background service that drives it, and the relay's storage backends
are all folded into the band that owns them rather than drawn.

For the layer you actually need next: [the workspace](nxs-the-workspace) says what `nxs init`
leaves on disk, and [modules](nxs-modules) says which block to reach for.
