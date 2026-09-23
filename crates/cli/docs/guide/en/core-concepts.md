# Core Concepts

This is the **plugin-free** heart of nexus-flow. Everything here is universal — the same model
underlies every plugin; only the words on top change ([plugins](nxf-plugins)).

## The data model

There are two kinds of **item**:

- A **project** groups work (a plugin may call it an *epic* or a *project*).
- A **task** is a unit of work (an *issue*, a *todo*).

Items carry a small, fixed set of engine-level fields — visible verbatim in any `--json`
record: `id`, `type`, `title`, `description`, `design`, `status` (`open` / `in_progress` / `closed`), `priority`,
`due`, `defer_until`, `assignee`, `belongs_to`, and a `closing_comment`. Two relationships
connect items:

- **belongs-to**: a task or project belongs to one parent project (set with `--parent`). This is
  containment — the breakdown of work.
- **dependencies**: a directed *must-finish-first* edge between any two items (project or task),
  independent of containment.

Beyond dependencies, items can carry **mentions** — free-text short-id references that record
"this text talks about that item" without ever blocking it. Every change is kept as **history**,
and closing an item records a **closing comment** (the *why*, not just the *that*).

## Preserving the original intent

A **title** and **description** are fixed shortly after you create an item and then kept stable
— they are the record of *what you set out to do*. Correct them once right after creation (for
example to fold in a review), and after that leave them alone. New context and everything you
learn while working go into the **append-only notes** stream (`nxf note add`), not into a rewrite
of the original fields. (This is a convention today; a future release may enforce it with an
irreversible per-field lock.)

When an item genuinely no longer makes sense, **do not** reshape it into something unrelated —
that would erase its history. Instead open a **new** item and **close the old one with a reason
that points to the new one** (closing is the only way an item leaves the board; there is no hard
delete). The original item stays, closed, as part of the record.

Why this matters: over time it builds the pair **"this is what we wanted to achieve"** (the
description and design) **↔ "this is how we actually closed it"** (the closing comment and notes).
That intent-vs-outcome pair is exactly what you learn from later. Overwrite the original intent
and you lose half of it — and with it the ability to learn. (Preserving intent is only useful if
you can find it again: `nxf search` looks through closed items too, so the pair stays retrievable.)

## Dependencies and blocking

A dependency says the *from* item must wait for the *to* item. Suppose `ab12.0002` ("Write the
CLI") cannot start until `ab12.0003` ("Spec sign-off") is done — add the edge:

```console
$ nxf dep add ab12.0002 ab12.0003 --json
{"msg":"ab12.0002 -> ab12.0003","ok":true}

```

While the blocker is open, the dependent item is **blocked**:

```console
$ nxf blocked --json
[{"archived":null,"assignee":null,"belongs_to":"ab12.0001","blockers":[{"id":"ab12.0003","status":"open"}],"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Build the command-line tool","design":null,"due":"2026-12-31","id":"ab12.0002","priority":"1","priority_label":"P1","status":"open","title":"Write the CLI","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

## Derivation: blocked and next

`blocked` and `next` are **derived**, never stored. They are a deterministic computation over the
items and their edges — there is no *ready* flag to set or forget. An item is *ready* when it is
open and has no open blocker; the project and the unblocked task are ready, so `next` lists them,
while the blocked task is absent:

```console
$ nxf next --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Cut the first release","design":null,"due":null,"id":"ab12.0001","parent":null,"priority":"1","priority_label":"P1","status":"open","title":"Ship v1","type":"epic","type_label":"epic","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Approve the final spec","design":null,"due":null,"id":"ab12.0003","parent":null,"priority":"2","priority_label":"P2","status":"open","title":"Spec sign-off","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

`next` is that same ready set put in order by the active plugin's ranking policy — same
derivation, one more step. *Ready* is a state an item is **in**, not a lane you can ask for:
`next` and `blocked` are the two questions there are. Because it is all computed, closing the
blocker instantly makes the dependent item ready on the next query; nothing has to be re-flagged.

## Time: due and defer

Two date fields shape an item over time. `due` is a target date (it can influence ranking).
`defer_until` hides an item until a date arrives — useful for work you cannot start yet.
Derivation is time-sensitive but still deterministic: pass `--now` to pin the reference instant.
In a fresh workspace, a deferred task:

```console
$ nxf create --type feature --title "Pay quarterly taxes" --description "File the quarterly tax return" --priority P3 --defer 2026-07-01 --json
{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":"2026-07-01","deleted":null,"description":"File the quarterly tax return","design":null,"due":null,"id":"ab12.0001","priority":"3","status":"open","title":"Pay quarterly taxes","type":"feature"}
```

Before the defer date it is **not** ready:

```console
$ nxf next --now 2026-06-15T00:00:00Z --json
[]

```

On or after it, the very same query returns the task — only `--now` changed:

```console
$ nxf next --now 2026-08-01T00:00:00Z --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":"2026-07-01","deleted":null,"description":"File the quarterly tax return","design":null,"due":null,"id":"ab12.0001","parent":null,"priority":"3","priority_label":"P3","status":"open","title":"Pay quarterly taxes","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

That determinism — same inputs, same `--now`, byte-identical output — is what makes nexus-flow
safe for agents to drive. Next: see how a plugin renders all of this in [plugins](nxf-plugins), or
walk the full command set in [commands](nxf-commands).
