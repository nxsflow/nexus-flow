# Derived ranking keys — ranking over the graph, declared by the plugin (6j6v.e354)

**Status: DRAFT for owner approval.** Nothing here is built until the owner approves this document.
Fourth cut of `6j6v.kfs7` (cut with the owner on 2026-10-09). The three cuts before it add the
plumbing this one ranks with:

- `15ed`: `next` filters by type and container, and pages.
- `z9jk`: a ranking order per type for `next`.
- `n698`: the same for `list`.

## 0. What this supersedes

`next-finish-first-tiers.md` §0 lists as a non-goal: *"we do not add a graph-aware key kind
(speculative generality — rejected)."* That was right when nobody needed one. A product now needs
one, with three concrete orders (§2), and the owner cut this item to answer them. This document
replaces that one non-goal. **Everything else in the tiers spec stands**:

- the tiering of `next` is still computed by the engine, not declared;
- derived keys order candidates **within** a tier, exactly as field keys do today.

## 1. Goal and non-goals

**Goal.** A plugin can rank items by facts about their NEIGHBOURS in the graph, not only by their
own fields. It declares this in the same `[ranking.*]` key list it already uses. The engine knows
no product types: every derived key names edge kinds, item types and fields the plugin declares.

**Non-goals.**
- No scripting and no expressions: a fixed, small set of key kinds (§3), each of bounded cost.
- No new edge kinds and no new fields: the keys read what the board already holds.
- No change to which items a verb returns. Keys order; filters (`15ed`) select.
- No ranking by text. A mention counts only as an edge (`mentions`), never as a substring of a
  note. A product that wants note mentions counted writes the edge when it writes the note. The
  requester does this today through `mention_add`, and through a `ref:<id>` label until it can read
  mentions back (see the note on `e354`).

## 2. The orders the requester needs (stated generically)

These cases are the acceptance test of the key language. The type names are the requester's; the
engine never sees them as anything but strings the plugin declared.

1. **Projects.** First by the declared order of the goals they `contributes_to`, then by their own
   priority, then by the number of their open child actions (more first).
2. **Persons** (ranked by `list`, decision of 2026-10-09). By how often items of type project
   mention them, and by the number of open actions labelled `waiting` that mention them. Then by
   the best rank among the projects they contribute to.
3. **Actions.** By the rank of their parent project, then by priority.

## 3. The key kinds

A key in `order = [...]` is either a FIELD key or a PRECEDENCE key, as today, or exactly one of the
three DERIVED keys below. Each derived key yields one value per candidate. The values compare like
a field key's, with `dir` and `nulls`. The id stays the final tiebreak, so every order is total and
deterministic.

### 3.1 `related` — a value taken from neighbours

```toml
{ related = "contributes_to", type = "goal", of = "rank", pick = "min" }
{ related = "parent", of = "rank" }
{ related = "contributes_to", type = "project", of = "rank", pick = "min" }
```

| part | meaning |
|---|---|
| `related` | the edge kind to follow outward from the candidate: `parent`, `contributes_to`, `dep`, `mentions` |
| `type` | optional: only neighbours of this item type |
| `of` | what to read on each neighbour: `"rank"`, its position in ITS OWN type's declared order (§4), or a field key `{ field = ..., precedence = [...] }` |
| `pick` | how several neighbours become one value: `min` (default) or `max` |
| `dir`, `nulls` | as for a field key; a candidate with no such neighbour is null |

### 3.2 `count` — how many neighbours match

```toml
{ count = "children", type = "action", status = ["open", "in_progress"], dir = "desc" }
{ count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"], dir = "desc" }
{ count = "mentioned_by", type = "project", dir = "desc" }
```

| part | meaning |
|---|---|
| `count` | the direction and kind to count: `children` (inbound `parent`), `contributors` (inbound `contributes_to`), `dependents` (inbound `dep`), `mentioned_by` (inbound `mentions`), or an outbound kind as in §3.1 |
| `type`, `status`, `label` | optional filters on the counted neighbour; `status` is a list |
| `dir` | `desc` puts the most first; a count is never null |

`mentioned_by` with `type = "project"` IS the requester's "mention frequency": the number of items of
that type with a `mentions` edge to the candidate.

### 3.3 `sum` — several counts as one value

Only if the owner wants it (open question 3): `{ sum = [<count>, <count>], dir = "desc" }`. Case 2
reads more naturally as one combined weight than as two keys in sequence.

## 4. `"rank"` of a neighbour, and why it terminates

`of = "rank"` reads the neighbour's position in its own type's declared order (`z9jk` / `n698`).
That order may itself use `related`: an action ranks by its project's rank, and the project ranks
by its goals' rank. Load-time rules keep this finite and cheap:

- **The type graph must be acyclic.** Draw an edge from a type to every type it ranks by. A cycle
  (project by person, person by project) is refused at load with an error naming the cycle. Case 2
  (persons by projects) and case 1 (projects by goals) form a chain, not a cycle.
- **Ranks are computed bottom-up, once per query.** First the types nothing depends on, then their
  dependents. Each type's rank is a dense position over its items. A neighbour's rank is therefore
  a lookup, never a recursion.
- **The scope of "rank" is the type's candidates under the same verb rules.** For `next` these are
  the actionable items of that type. For `list` they are the items `list` would return for that
  type without paging. A neighbour outside that scope (closed, archived, deleted, or a goal `next`
  would not list) has no rank and reads as null. The same rule makes the server agree (§6).

## 5. Cost

One query computes, for the candidate set and the types it ranks by:

- **Edge passes:** one pass over the present edges of the kinds the keys name, to count and to
  collect neighbours.
- **Ranking:** one sort per type in the dependency chain.

So the cost is O(E + N log N) for E relevant edges and N items in scope. Nothing walks deeper than
the type chain, and the chain is bounded by the number of declared types. A plugin that names no
derived key pays nothing.

## 6. The hosted server

`nxs-fold-ddb` answers `next` from a sparse index of ACTIVE tickets through `graph::Board`
(6j6v.k7w7). Derived keys must give the same order there.

- `Board` gains the edge kinds the keys need (`contributes_to`, `mentions`) and each ticket's type,
  status, labels and the fields the keys read.
- **The rule that makes this possible is the one `Board` already has:** a counterpart that is not an
  active ticket counts as absent. For `next` that is exactly §4's scope.
- For `list` it is not. A person is long-lived and may be in a status `next` never sees. A
  server-side `list` with derived keys therefore needs either a second index of the types `list`
  ranks, or a rule that `list` ranks only active items (open question 4).
- A differential test holds the SQL path and the `Board` path to the same order on generated
  boards, as for the lanes today.

## 7. Validation at load

The plugin is refused with an error that names the key, as `RankSpec::validate` does today, when:

- a key names an edge kind, a direction or a filter field that does not exist;
- a key mixes kinds (for example `related` together with `count`);
- `of = "rank"` names a type with no declared order;
- the type graph has a cycle (§4);
- `pick` is set on a `count`, or `dir`/`nulls` on a precedence key (unchanged).

## 8. The requester's orders, declared

```toml
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order", dir = "asc" }, pick = "min", nulls = "last" },
  { field = "priority", dir = "asc" },
  { count = "children", type = "action", status = ["open", "in_progress"], dir = "desc" },
]

[ranking.next.action]
order = [
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { field = "priority", dir = "asc" },
]

[ranking.list.person]
order = [
  { count = "mentioned_by", type = "project", dir = "desc" },
  { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"], dir = "desc" },
  { related = "contributes_to", type = "project", of = "rank", pick = "min", nulls = "last" },
]
```

`order` on a goal stands for the field that carries the goals' declared sequence: a custom field
the plugin declares (`plugin-custom-fields.md`) or the priority (open question 2).

## 9. Tests

- **Per kind:** unit tests over hand-built boards: a value from one neighbour, from several
  (`min`/`max`), from none (null placement); counts with each filter.
- **Chains:** actions by project rank by goal order across three types, and a refused cycle.
- **Differential:** SQL path against `graph::Board` on generated boards with derived keys, as
  `fold_differential` does for the lanes.
- **Determinism:** shuffled edge order gives the same result, and ties fall to the id.
- **Bounded cost:** a ratio guard in the style of `xn8s`. A query with derived keys runs the edge
  pass once, not once per candidate.

## 10. Open questions for the owner

1. **Approve superseding the tiers spec's non-goal** (§0)?
2. **What carries a goal's "declared order"?** A custom field (proposal: the plugin declares a
   numeric field, here called `order`), or the priority? The engine needs no answer; the requester's
   plugin does.
3. **Combined weight for persons:** two keys in sequence (§8 as written), or a `sum` (§3.3)?
4. **`list` on the server:** does a hosted `list` with derived keys need a second index, or may it
   rank only active items, as `next` does? Locally both work; this only decides the server.
