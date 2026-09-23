# flow — what needs doing

The issue tracker of the suite: items, the dependencies between them, due and defer dates, an
append-only note stream, and the reason each item was closed. That is the record of what a project
is doing, and why.

One property separates it from a list of tickets, and it is worth knowing before anything else:
**the work list is derived, not stored**. `nxf next` and `nxf blocked` are computed from the
dependency graph at the moment you ask for them, so they cannot go stale and cannot disagree with
the graph. Nothing sets an item to *actionable*; closing its last prerequisite is what makes it so.

That shapes the first move. It is not "write a task" — it is *record what exists, and what it waits
on*:

```bash
nxf create --type feature --title "Ship the 1.0 release" --priority P1
nxf dep add <item> <prerequisite>
nxf next
```

What an item *is* comes from a **plugin**: the types you may create, the vocabulary you read and
write, and the ranking policy behind `next`. The choice is made once per workspace, because it
shapes everything you type afterwards — a coding project's issue tracker and a personal to-do list
are the two that ship, and both drive the same engine.

Every command answers `--json` with byte-stable output, and that is the surface agents build on: an
agent can drive the whole board without a human reading a table first.

## Where to go from here

- [Getting started](nxf-getting-started) — from an empty directory to a planned, tracked piece of
  work.
- [Core concepts](nxf-core-concepts) — items, dependencies, and how the two lanes are derived
  rather than stored.
- [Deferring and waiting](nxf-deferring-and-waiting) — the one distinction that is easy to get
  wrong: a real calendar date, versus waiting on something that has to ship first.
