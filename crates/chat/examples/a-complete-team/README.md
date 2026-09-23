# A complete team

The declarations a real team ships with, taken from a project that runs this engine every day and
generalized: no repository names, no project-specific gate commands, every prompt in English. Copy
`.nxs-personas/` into a project, adjust the prompts to your own rules, and the chain below runs.

`examples/role-runtime-v3` is the SMALLEST set that demonstrates the runtime — one coder and a
review quorum. This one is the OTHER end: ten personas and five channels, with every declaration
surface in use at least once.

## What it declares

| Channel | Shape | What walks through it |
|---|---|---|
| `planning` | three steps, back edge from the review to the draft | an idea becomes an epic plus one ticket per task; the design is judged before it becomes work |
| `coding` | four steps, back edge to the build | one ticket from nothing to a pull request |
| `review` | a three-member quorum, folded into one verdict | the judgement the two chains above ask for |
| `merging` | one member | what happens to finished work: merge now, or let the next ticket ride along |
| `process` | two steps | changing the team itself — a designer writes declarations, an independent reader judges them |

## The things worth copying

**Reachability is declared by the persona that is addressed.** Every file says who may open a
conversation with it, and nothing says whom it may call:

- `addressable: none` — the step targets (`coder`, `verifier`, `finisher`, `merger`,
  `process-designer`, `declaration-check`). They are reached because a channel's step names them,
  never by a direct message.
- `addressable: {personas: [pm]}` — the three reviewers. They are members of `review`, and the PM
  may additionally consult one of them directly, as advice rather than a second review round.
- `addressable: {humans: true}` — the PM. A person may start it; no agent may. That is what lets
  small work skip the planning chain.

**One round, two tasks.** `review` is addressed from two places: the coding chain hands it a diff,
the planning chain hands it a design. The step declares what its target does there (`task:`) and at
which experience band (`stage:`), so one set of reviewer personas serves both.

**A step declares what it is given.** `verify` gets `input: []` — it measures the tree and must not
read the coder's account of it. `finish` gets `input: [verify, judge]`, because it needs the numbers
AND the verdict before it puts work up.

**The working copy is leased.** `coding` declares `working_tree: exclusive`, so a second chain waits
instead of building in the same directory.

## What to change first

1. The gate commands. Every persona that runs the project's checks says "find them the way anybody
   would: `AGENTS.md` or `CLAUDE.md` first, then the manifest's scripts". Say it plainly in your
   own files instead once you know them.
2. The board vocabulary in `pm.yaml` — item types and priorities are declared by your `nxf` plugin.
3. `permissions:` and `tools:`. These personas run with broad permissions because the project they
   come from decided that; decide it for yourself rather than inheriting it.
