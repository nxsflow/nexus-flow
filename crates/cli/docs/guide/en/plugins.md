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
