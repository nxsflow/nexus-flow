# Modules

The suite is three building blocks over one store. This page is the map: what each one is for, and
which one to reach for. Read it before the three detail guides — they each assume you already know
which block you are in.

You activate blocks **per workspace**, with `nxs init`. Activating one does not oblige you to the
others: a workspace can carry flow alone, or all three.

## flow (`nxf`) — what needs doing

The issue tracker: items, the dependencies between them, due and defer dates, an append-only note
stream and a closing reason. Its distinguishing property is that **the work list is derived, not
stored** — `nxf next` and `nxf blocked` are computed from the dependency graph every time you ask,
so they cannot go stale or disagree with the graph.

What an item *is* comes from a **plugin**: the types you may create, the vocabulary you read, and
the ranking policy behind `next`. `nxs init` asks which one, because the choice shapes everything
you will type afterwards. Reach for flow when the question is *what should I do next, and what is
blocking it*.

→ [flow's guide](nxf-getting-started)

## memory (`nxm`) — what the project knows

Durable facts that outlive a session: conventions, gotchas, decisions and the reasons behind them.
`remember` writes one, `recall` reads it back in full, and a stable key lets a fact be **corrected
in place** rather than accumulating contradictory copies.

The point is not storage but replay: memories are handed back at the start of every session, so a
new agent starts out knowing what the last one learned. They can also be projected into a generated
`NEXUS_MEMORY.md`, which lets a contributor who has not installed the suite read the same context.
Reach for memory when something learned the hard way must not be learned twice.

→ [memory's guide](nxm-getting-started)

## chat (`nxc`) — the channel between you and your agents

The channel between you and the agents in a workspace: `send --to` starts something,
`reply --thread` answers it, and every message is durable and replayable rather than a message in
flight. Who may be
addressed, and how a group of agents proceeds, is **declared** in files under `.nxs-personas/` —
a persona per agent, and channels that carry a flow.

Reach for chat when more than one agent is working and a hand-off has to survive a session
ending. Scratch files do not: they are not delivered, not synced, and never replayed.

→ [chat's guide](nxc-getting-started)

## What they share

One `.nxs/` workspace and one store underneath — see [the workspace](nxs-the-workspace). Each block
folds its own view over the same append-only log, which is why activating a second block adds a
view rather than a second database, and why one sync stream carries all of them.

The umbrella itself owns no domain. `nxs` sets workspaces up (`init`), restores context (`prime`),
diagnoses (`doctor`), migrates the schema, syncs, updates itself, and serves the suite over MCP.
Everything else belongs to a block.
