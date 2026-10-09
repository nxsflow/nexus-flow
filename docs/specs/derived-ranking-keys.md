# Derived ranking keys — ranking over the graph, declared by the plugin (6j6v.e354)

**Status: decided by the owner on 2026-10-09** (§10). The cuts that build it are listed in §11.
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

### 3.3 `sum` — several counts as one value (decided §10.3)

```toml
{ sum = [ { count = "mentioned_by", type = "project" },
          { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"] } ],
  dir = "desc" }
```

The listed counts are added with equal weight and compared as one value. A summand takes only the
parts of a `count`; `dir` belongs to the `sum`. An optional `weight` per summand is deliberately
LEFT OUT. The syntax keeps room for it, so adding it later breaks no declaration. It comes when
real data shows that equal weights rank badly.

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
- **The scope of "rank" is the type's candidates under the verb that ranks it.** For `next` these
  are the actionable items of that type. For `list` they are the live items of that type that
  `list` would return without paging (not deleted; archived only where `list` includes them). A
  neighbour outside that scope has no rank and reads as null. For example: a closed project under
  `next`, or a deleted person. The server follows the same rule (§6).

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

**`next`.**
- `Board` gains the edge kinds the keys need (`contributes_to`, `mentions`), and each ticket's
  type, status, labels and the fields the keys read.
- The rule that makes this possible is the one `Board` already has: a counterpart that is not an
  active ticket counts as absent, which is exactly §4's scope for `next`.

**`list` needs a second index (decided §10.4).** Objects that `list` ranks, such as a person, are
long-lived and have no "active" status. If they did, they would rank through `next`, and `list`
would not be needed for them. So the server cannot rank them from the `active` index. The table
gains a second sparse index, keyed by type:

| what | name | type | role |
|---|---|---|---|
| GSI `typed` | `nxf_typed` / `sk` | S/S | `<stream>#<type>` on every LIVE item row (not deleted). Projection ALL. One Query lists a type's items. |

- **Kept by the fold like `active`.** Every write to an item's `type` or `deleted` cell sets or
  removes the attribute. It is replay-safe in the same way: the attribute is written after the row.
- **Generic.** The fold indexes every live item by its type, so it needs no plugin knowledge of
  which types `list` ranks. The price is one index write per item, not only for the ranked types.
- **Edges.** A derived key on a `list`-ranked type reads that item's `contributes_to` and `mentions`
  neighbours, and the counts read their inbound edges. Today the adjacency entries (`.adj#`) are
  kept only for `dep`/`parent` edges that touch an active ticket. They are widened to every present
  edge of a kind some key names, between live items. That widening is the open design work of the
  server cut (§11), together with what it costs per edge write.
- **Archived items.** They are in the index, because they are live. `list` filters them at read,
  as it does locally.
- **It changes the table contract.** The deploying stack (the web package's CDK) must add the GSI
  before a fold that writes `nxf_typed` runs against it. A fold against a table without the index
  still writes the attribute, which costs nothing. Only the `list` read needs the index. The
  rollout order is therefore: the stack first, then the read.

A differential test holds the SQL path and the server path to the same order on generated boards,
for `next` and for `list`, as for the lanes today.

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
  { sum = [ { count = "mentioned_by", type = "project" },
            { count = "mentioned_by", type = "action", label = "waiting", status = ["open", "in_progress"] } ],
    dir = "desc" },
  { related = "contributes_to", type = "project", of = "rank", pick = "min", nulls = "last" },
]
```

`order` on a goal stands for the field that carries the goals' declared sequence: a custom field
the plugin declares (`plugin-custom-fields.md`), as decided in §10.2.

## 9. Tests

- **Per kind:** unit tests over hand-built boards: a value from one neighbour, from several
  (`min`/`max`), from none (null placement); counts with each filter.
- **Chains:** actions by project rank by goal order across three types, and a refused cycle.
- **Differential:** SQL path against `graph::Board` on generated boards with derived keys, as
  `fold_differential` does for the lanes.
- **Determinism:** shuffled edge order gives the same result, and ties fall to the id.
- **Bounded cost:** a ratio guard in the style of `xn8s`. A query with derived keys runs the edge
  pass once, not once per candidate.

## 10. Decisions (owner, 2026-10-09)

1. **The tiers spec's non-goal is superseded** (§0): derived keys become part of the ranking
   language; the tiers stay. The alternatives were declined. A separate mechanism outside
   `[ranking.*]` would be two ways to the same thing. Ranking only in the product would give a
   paging order (15ed) that is not the order the product shows.
2. **A goal's declared order is a plugin custom field** (for example a numeric `order`), read with
   `of = { field = ... }`. `of = "rank"` stays available, so the product can move to ranking goals
   by their own order without an engine change. The engine is indifferent; the product's plugin
   decides.
3. **Persons rank by `sum`** (§3.3), equal weights, `weight` left out until data asks for it.
   Strictly sequential keys were declined: one more mention would always beat any number of open
   waiting-fors.
4. **A hosted `list` with derived keys uses a second index** (§6, GSI `typed`). Ranking only active
   items was declined: the objects `list` ranks have no active status by design. If they had one,
   they would rank through `next`, and `list` would not be needed. Leaving `list` unranked on the
   server was declined because the product reads through the hosted path.

## 11. Cuts

- **Derived keys, local.** The three key kinds, load-time validation and the type chain over the
  SQL path. Usable in `next` (after `z9jk`) and `list` (after `n698`).
- **Derived keys on the server, `next`.** `graph::Board` with the extra edges and fields; the
  differential test.
- **The `typed` index and a ranked hosted `list`.** The GSI, the widened adjacency, the read, and
  the table-contract note for the deploying stack, coordinated with app-foundations.
- **Shared with `6j6v.q29p`.** The reverse mention lookup and `count = "mentioned_by"` use one
  implementation.
