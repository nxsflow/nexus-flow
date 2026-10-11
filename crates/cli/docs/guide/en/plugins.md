# Plugins

A plugin maps the universal [core](nxf-core-concepts) to a consumer's language and policy. Nothing
in the CLI hardcodes vocabulary — it all flows from the active plugin config, chosen once at
`nxf init`. A plugin sets four things:

- **Vocabulary**: the words for the two item types and the three statuses.
- **Priority labels**: the names for priority levels 0–4.
- **Ranking**: how `next` orders the ready set.
- **Presentation**: which fields `list` and `show` display.

## The two shipped plugins

- **issue-tracker** — Software issue tracking: epics and issues, P0–P4 priorities,
  dependency-aware ranking.
- **personal-todo** — Personal to-do list: lists and todos, now/soon/later priorities for
  everyday tasks.

Their vocabularies differ:

- **issue-tracker**: a project is an `epic`, a task is an `issue`; statuses are
  `open` / `in progress` / `closed`; priorities are `P0`–`P4`; `next` ranks by priority, then
  due date, then id.
- **personal-todo**: a project is a `project`, a task is a `todo`; statuses are
  `todo` / `doing` / `done`; priorities are `now` / `soon` / `later` / `someday` / `icebox`;
  `next` ranks by priority, then id (no due-date tiebreak).

## Resolving named variants: types and priorities

`type` and `priority` are both *named variants* — a small, plugin-defined set with a fixed order.
`nxf schema --json` exposes both as a **keyed map**, so one rule resolves either field on any item:

```text
schema.types[item.type]           # "task" -> "issue"
schema.priorities[item.priority]  # "0"    -> "P0"
```

The value stored on an item is the *handle*, not the label. For `priority` that handle is an
**ordinal** — `"0"` is the highest priority, counting up — deliberately plugin-independent so
`next` can rank numerically and synced items never carry another plugin's labels. The
`schema.priorities` map is what turns the ordinal back into a human label.

**Ordinal-stability contract.** Within a schema version the ordinal→meaning binding is **stable
and append-only**: `priority.labels` may gain entries at the end, but **reordering or inserting**
shifts the meaning of already-stored items and is therefore a **breaking** schema change — a
schema-version bump with migration, not an in-place edit.

## Same data, side by side

The seam is clearest when you run the **same** commands on the **same** data under each plugin.
Both workspaces below hold three tasks: two at priority `1` (one due 2026-02-01, one due
2026-03-01) and one at priority `2`.

`next` shows two differences at once — the **vocabulary** and the **ranking**. Under
issue-tracker, the earlier-due P1 issue (Book venue) wins the tiebreak:

```console
$ nxf next

0002  P1  open  [feature]  Book venue
0001  P1  open  [feature]  Draft proposal
0003  P2  open  [feature]  Order badges
```

Under personal-todo, the same two `soon` todos tie on priority and fall back to id, so the
first-created (Draft proposal) comes first — and the labels are now/soon/later, todo/doing/done:

```console
$ nxf next

0001  soon  todo  [todo]  Draft proposal
0002  soon  todo  [todo]  Book venue
0003  later  todo  [todo]  Order badges
```

Yet the stored record is the same — `list --json` is id-ordered and does not depend on the active
plugin. The plugin overlays presentation and ranking; it never changes the data. The one field a
plugin owns is `type`: the type set is plugin-declared, so issue-tracker stores `feature` where
personal-todo stores `todo`. Strip that one field and the records are **byte-for-byte the same**:

```console
$ nxf list --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Outline the event proposal","design":null,"due":"2026-03-01","id":"ab12.0001","priority":"1","priority_label":"P1","status":"open","title":"Draft proposal","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Reserve the event space","design":null,"due":"2026-02-01","id":"ab12.0002","priority":"1","priority_label":"P1","status":"open","title":"Book venue","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Print attendee name badges","design":null,"due":null,"id":"ab12.0003","priority":"2","priority_label":"P2","status":"open","title":"Order badges","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

`show` makes the vocabulary difference concrete on a single item. Issue-tracker:

```console
$ nxf show ab12.0002

0002 P1 Book venue
==================

TYPE: feature
STATUS: open

DESCRIPTION
-----------

Reserve the event space

DEFINITION OF DONE
------------------



DESIGN
------



NOTES
-----
```

personal-todo, the same `ab12.0002`:

```console
$ nxf show ab12.0002

0002 soon Book venue
====================

TYPE: todo
STATUS: todo

WHY
---

Reserve the event space

DONE WHEN
---------



PLAN
----



LOG
---
```

That doubling is not redundancy — it *is* the explanation. The `--json` is the meaning; the
plugin is how a particular audience reads and ranks it. Picking a plugin at `init` is therefore
a real choice; run `nxf init --help` to see the options before you commit.

## Declaring a ranking

A plugin declares how `next` ranks under `[ranking.next]` and how `list --sort rank` ranks under
`[ranking.list]`. Both take the same keys and the same rules:

- `order = [...]` is the default order: a list of keys, compared in turn, with the id as the final
  tiebreak.
- `[ranking.next.<type>]` (or `[ranking.list.<type>]`) gives one type its own `order`.
- `types = [...]` says how the types stand against each other once some have their own order.

A key is a **field key**, such as `{ field = "priority", dir = "asc" }`. With a `precedence` list,
a field key ranks by the value's position in that list instead. In a per-type order, or in the
`[ranking.list]` default `order`, a key can also be a **derived key**, which ranks an item by its
neighbours in the graph:

```toml
[ranking.next]
types = ["project", "action"]
order = [ { field = "priority" } ]

# related: a value read on the neighbours over one outbound edge (parent, contributes_to, dep,
# mentions). `of = "rank"` reads the neighbour's position in its own type's order;
# `of = { field = "..." }` reads a field, a custom field included. `pick` is min (default) or max.
[ranking.next.action]
order = [
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { field = "priority" },
]

# count: how many neighbours match. children, contributors, dependents and mentioned_by count the
# inbound parent, contributes_to, dep and mentions edges; an outbound kind counts the other way.
# `type`, `status` (a list) and `label` filter the counted neighbours.
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order" }, nulls = "last" },
  { count = "children", type = "action", status = ["open", "in_progress"], dir = "desc" },
]

# sum: several counts, added with equal weight. `dir` belongs to the sum.
[ranking.list]
types = ["person"]
[ranking.list.person]
order = [
  { sum = [ { count = "mentioned_by", type = "project" },
            { count = "mentioned_by", type = "action", label = "waiting" } ], dir = "desc" },
]
```

Derived keys count only live neighbours: a deleted one is gone, a closed or archived one counts.
A rank is computed over the type's candidates under the verb that ranks it, and items that tie on
every key share a rank. For `next` these candidates are its actionable items; for `list` they are
the live, unarchived items. A neighbour outside that set, such as a closed project under `next`,
has no rank and reads as null. A `list` key may read the rank of a type that only `next` ranks;
`next`'s candidates are then taken at the current time (`NXF_NOW` pins it). Only `list --sort rank`
reads that time.

The plugin is refused at load, with the key named, when:

- a key mixes kinds or names none;
- a key names an unknown edge kind, type, status or field;
- a key carries a part its kind does not take, such as `pick` on a count or `nulls` on a count or
  sum (a count is never null);
- `of = "rank"` names a type without an order of its own;
- the types rank by each other in a cycle.

A `related ... of = "rank"` key without `type` reads the rank of every type with an order, its own
included. That is a cycle, so name the neighbour's `type`; the `[ranking.list]` default `order`
refuses such a key too. Derived keys are not allowed in the
`[ranking.next]` default `order`, because that order also ranks `blocked` and the lanes'
`--sort rank`.
