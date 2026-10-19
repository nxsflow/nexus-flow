# Derived ranking keys — ranking over the graph, declared by the plugin (6j6v.e354)

**Status: decided by the owner on 2026-10-09** (§10). The cuts that build it are listed in §11.
**Built locally (x0wf, §12):** the three key kinds, their load-time rules and the type chain over
the SQL path, in `next` and `list`. **Built on the server for `next` (1c5s, §6):** the same keys
over the hosted board, with the rule the owner decided on 2026-10-11 (§10.5). **Built on the server
for `list` (xkr7, §6):** the `typed` index, a ranked hosted `list`, and a hosted `next` whose keys
read a `[ranking.list]` rank.
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

**`next`** (built in 1c5s; decided by the owner on 2026-10-11, §10.5).
- **A key reads every LIVE neighbour, on the server as locally** (§12, "Live neighbours"): a
  deleted neighbour is gone, a closed or archived one counts. This replaces the first reading of
  this section, that a counterpart which is not an active ticket counts as absent. That rule stays
  the rule of the LANES (`graph::Board`); it is not the rule of the keys. Only the scope of
  `of = "rank"` is the active set: a neighbour outside `next`'s candidates has no rank (§4), on
  both paths.
- **Where it lives.** The facade's `ActiveBoard` — the store-free lanes' input, `graph::Board`
  plus each active ticket's row and joins — gains a `RankingGraph`: every present edge of the kinds
  the keys name (`contributes_to`, `mentions`, `dep`, `parent` as the `present_parent` projection)
  with an active end and both ends live, and each live neighbour that is not active with its row,
  labels and custom values. `graph::Board` itself is unchanged: it holds no rows, and the lanes read
  no neighbour that is not active. `read::next_ranking_reads(cfg)` names what the graph must hold
  for a plugin; `read::next_active` refuses a plugin with derived keys while its board lacks it, and
  ranks by the same evaluation the store path runs once it is there.
- **What the fold writes.** Every edge add already had an adjacency entry under each end
  (`.adj#<item>#<tag>`), whatever its kind and its ends' activity. Since index revision 3 each entry
  also copies the add's `from_id`, `to_id`, `kind`, `lamport` and `site` and says whether the add is
  removed, kept up to date when the remove is folded. So one Query on an item's prefix reads every
  edge touching it, in and out, with its presence: inbound `contributes_to`, `mentions` both ways,
  and the `parent` edges a child's current parent is picked from. No new index, no new key
  attribute: 1c5s alone leaves the table contract unchanged — the release that ships it changes it
  all the same, with xkr7's GSI `typed` below. Revision 3 makes every stream folded before it not
  current; it answers `NotCurrent` from the moment the fold run clears it, and `Refolding` until
  the refold reaches the relay position the clear named (review of PR #55: `Folder::clear` leaves
  a sentinel watermark without revisions and that position, so a half-refolded stream is never
  served).
- **What the server reads** (`records::with_ranking`): one `GetItem` of the watermark (the read
  refuses a stream that is not current or still refolding itself); one Query per active ticket on
  its adjacency; the rows of the neighbours that are not active by `BatchGetItem` (100 keys a
  request) and, per live one, one Query on its own adjacency for its current parent, plus one Query
  each for its labels and custom values when a key reads them.
- **A `[ranking.list]` rank in a `next` key** (refused on the server in 1c5s, ranked since xkr7): a
  `[ranking.next]` order that reads — directly or down its type chain — the rank of a type only
  `[ranking.list]` orders (§12, first point). That rank is computed over the type's live,
  unarchived items, which the active board does not hold; `read::next_ranking_reads` names such a
  type in `RankingReads::list_types`, and the server reads its items off the `typed` index into the
  graph (`RankingGraph::listed`), with the edges around them.
- **What xkr7 builds on.** The self-describing adjacency entries are written for EVERY edge between
  ANY items, so xkr7's "widened adjacency" needed no new entries for the edges: a `list`-ranked item
  reads its edges with the same Query (verified in xkr7: its `.adj#` entries give its current
  parent, its `contributes_to` targets, its parents for `parent_closed_reason` and every edge a key
  follows or counts). What xkr7 adds is the `typed` index (the `list` scope of a type), the read
  that ranks a type's live items from it, and lifting the refusal above.

**`list` needs another index (decided §10.4).** Objects that `list` ranks, such as a person, are
long-lived and have no "active" status. If they did, they would rank through `next`, and `list`
would not be needed for them. So the server cannot rank them from the `active` index. The table
gains a third sparse index — after `active` and `dated` —, keyed by type:

| what | name | type | role |
|---|---|---|---|
| GSI `typed` | `nxf_typed` / `sk` | S/S | `<stream>#<type>` on every LIVE item row (not deleted). Projection ALL. One Query lists a type's items. |

- **Kept by the fold like `active`.** Every write to an item's `type` or `deleted` cell sets or
  removes the attribute. It is replay-safe in the same way: the attribute is written after the row.
- **Generic.** The fold indexes every live item by its type, so it needs no plugin knowledge of
  which types `list` ranks. The price is one index write per item, not only for the ranked types.
- **Edges.** A derived key on a `list`-ranked type reads that item's `contributes_to` and `mentions`
  neighbours, and the counts read their inbound edges. Corrected in 1c5s: the adjacency entries
  (`.adj#`) were always written for every edge add, under both ends; only the `active` INDEX flag is
  limited to `dep`/`parent` edges with an active end. Since index revision 3 the entries say what
  their edge is and whether it is removed (`next` above), so a `list`-ranked item reads its edges
  with one Query on its own prefix, at no extra cost per edge write. Since index revision 4
  (6j6v.9tx3) the `.radj#` reverse-reference hints and their `.rsrc#` mirrors are the second kind
  of plain adjacency row (plugin-custom-fields §5.3, "The hosted read"); nothing in ranking reads
  them.
- **Archived items.** They are in the index, because they are live. `list` filters them at read,
  as it does locally.
- **It changes the table contract.** The deploying stack (the web package's CDK) must add the GSI
  before a fold that writes `nxf_typed` runs against it. A fold against a table without the index
  still writes the attribute, which costs nothing. Only the `list` read needs the index. The
  rollout order is therefore: the stack first, then the read.

**`list` as built (xkr7).**
- **The key.** `nxf_typed` is `<stream>#<type>` with the type escaped as a key value is (`%` →
  `%25`, `#` → `%23`), so `a` + `b#c` and `a#b` + `c` cannot share a partition; the sort key is the
  table's `sk`, so a Query lists a type's items in id order. An item without a type is on no
  partition. A type whose partition would pass the 2,048 bytes DynamoDB allows an index key is
  refused at fold time, as an op with a sort key too long is.
- **Index revision.** The attribute ships in revision 3 with the self-describing adjacency entries
  (1c5s and xkr7 reach the same beta), so revision 3 still makes every older stream not current
  and one refold sets both; no revision 4.
- **Where it lives.** The facade's `read::list_reads(cfg, query)` names what a hosted `list` reads
  (`ListReads`: the listed types — the query's type filter, else every type —, whether `next`'s
  board is needed, and the graph); `nxs_fold_ddb::records::list_board` reads it into a `ListBoard`
  (the listed items with the joins a `list` record and its filters carry, `next`'s board, the
  `RankingGraph`); `read::list_board_page` and `read::list_board_value` filter, rank and project it
  by the code `list_page` runs, and `read::list_board` reads the same board from a store, the
  parity oracle.
- **Without a type filter** the server lists every live item: it reads the partition's `items#`
  rows (strongly consistent, one Query — paged by DynamoDB at 1 MB —, no index), since the index
  lists one type at a time.
- **What a list costs** (review of PR #55, Integrity #1). A `list` that is not ranked orders and
  filters by the item rows alone, so `ListReads::select` lets the server keep and cut BEFORE the
  joins: status and type are asked of the row, a container or label filter reads the one join it
  needs per kept row, and only the page (at most `limit` rows) reads the rest — three Queries a
  row, the timestamps and outside parents by `BatchGetItem`. `list --limit 10` costs the rows'
  Query and about 33 requests, however many items the stream holds. A RANKED `list` reads every
  live, unarchived item of its types in full — three Queries each — because a rank is computed
  over the type's whole `list` scope (§12, "The scope of a `list` rank": every live, unarchived item of the type,
  whatever the query's status, filter or limit). That cost is the scope's, not the answer's, and it
  is not capped: a hard per-call budget is filed as `6j6v.wq9e` (E4 spec §5.1).
- **Paging.** As for the hosted `next`: the store path's page cache is machine-local and a
  server's paging is an open decision (15ed), so the store-free `list` is not paged — `limit` cuts
  it with the total disclosed, `paginate` and a token are refused with `validation`, and a server
  cuts the list itself.
- **`now`.** A `list` key that reads a rank `next` gives takes `next`'s candidates at the query's
  `now`, as locally (§12); without one the read answers `validation` where a ranked row needs it,
  and `next`'s board is not read for it.

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

How items of DIFFERENT types compare is the per-type ordering's own rule (`z9jk`, PR #44): an
optional `types = [...]` under `[ranking.next]` (and, with `n698`, `[ranking.list]`) gives the order
of the types; items of different types compare by that position first, then each by its own type's
order, then id. A type with its own table must be listed; an unlisted type sorts after the listed
ones. Derived keys only ever compare items of the SAME type, so they need nothing more (owner,
2026-10-10).

```toml
[ranking.next]
types = ["project", "action"]

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
- **Differential:** SQL path against the server path on generated boards with derived keys, as
  `fold_differential` does for the lanes. Built for `next` in 1c5s
  (`crates/fold-ddb/tests/derived_differential.rs`): the SQL path, the store-free path over the
  store's own board and graph, and the same over the folded tables give one order. Built for
  `list` in xkr7 (`crates/fold-ddb/tests/list_differential.rs`): the same three paths on
  generated boards of four types with retypes, every key kind and cross-section ranks both ways,
  over a set of queries (types, none, container, label, status, sort, limit), and the JSON.
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
4. **A hosted `list` with derived keys uses a second index** (§6, GSI `typed`; the table's third,
   counting `dated`, which this decision did not count). Ranking only active
   items was declined: the objects `list` ranks have no active status by design. If they had one,
   they would rank through `next`, and `list` would not be needed. Leaving `list` unranked on the
   server was declined because the product reads through the hosted path.
5. **On the server a key reads every live neighbour** (owner, 2026-10-11, during 1c5s). The first
   text of §6 said a counterpart that is not an active ticket counts as absent; the local build
   (§12) reads every live neighbour, closed and archived ones included. The owner chose the local
   rule for both paths, so a server and a replica rank alike: the fold keeps self-describing
   adjacency entries (index revision 3) and the server reads the neighbours that are not active.
   Narrowing the local rule to active neighbours was declined, as was refusing on the server every
   key whose value can depend on a neighbour that is not active (it would refuse §8's first order).
   A `[ranking.next]` order that reads the rank of a type only `[ranking.list]` orders was refused on
   the server until the `typed` index (xkr7) gave it that type's scope; it ranks there since.

## 11. Cuts

- **Derived keys, local.** The three key kinds, load-time validation and the type chain over the
  SQL path. Usable in `next` (after `z9jk`) and `list` (after `n698`).
- **Derived keys on the server, `next`** (1c5s, built). The `RankingGraph` on the store-free
  lanes' board, the self-describing adjacency entries (index revision 3), and the differential test
  (SQL path = Board path = fold-ddb path on generated boards with closed, archived and deleted
  neighbours, mentions and inbound `contributes_to`).
- **The `typed` index and a ranked hosted `list`** (xkr7, built). The GSI, the read
  (`ListBoard`, `records::list_board`, `read::list_board_page`), the `[ranking.list]` rank in a
  hosted `next`, the differential test, and the table-contract note for the deploying stack
  (`nxs-fold-ddb` crate doc, E4 §5.1), coordinated with app-foundations. The widened adjacency
  turned out to be 1c5s's self-describing entries; nothing more was needed.
- **Shared with `6j6v.q29p`.** The reverse mention lookup and `count = "mentioned_by"` use one
  implementation.

## 12. The local cut as built (x0wf)

How x0wf reads the points this spec leaves to the build. Each one is a load-time refusal or a
reading of the text, so a later decision can loosen it without breaking a declaration.

- **Which order `of = "rank"` reads.** The asking section's own `[ranking.<verb>.<type>]`, else
  the other section's. That is how §8's persons, ranked by `list`, read the rank of the projects
  that `next` ranks. A `list` query that needs `next`'s candidates takes them at a reference time:
  `ListQuery::now`, which the CLI and MCP fill from the clock for a ranked list (`NXF_NOW` pins
  it). Without one, such a query answers `validation`. The query's filters narrow the rows before
  they are ranked, so only the orders the kept rows rank by are computed.
- **The scope of a `list` rank** is the live, unarchived items of the type, whatever `--status`,
  `--type`, `--in` or `--label` the query filters by. Filters select, keys order. `next`'s scope
  is its candidates over the whole board, before its filters.
- **A dense rank** (§4) means that items equal on every key of their type's order share a rank,
  and the next distinct item takes the next integer. The id orders the sort, never the rank.
- **Live neighbours.** A `related` or `count` key reads only live neighbours: a deleted one is
  gone, a closed or archived one counts. This is the rule `mentioners_of` (q29p) applies, and
  `count = "mentioned_by"` reads through it (§11).
- **`of = { field, dir, nulls }`.** `dir` and `nulls` belong to the `related` key. Inside `of`
  they are accepted at their defaults only, so §8's `dir = "asc"` loads, and refused otherwise.
- **An untyped `related ... of = "rank"`** reads the rank of every type with an order, its own
  included, so in a per-type order it is a cycle and refused. Name the neighbour's `type`. The
  `[ranking.list]` default order refuses it too: there it would read nothing without per-type
  orders, and every ranked type with them (review of PR #48).
- **The `[ranking.next]` default `order` refuses derived keys**, because that order also ranks
  `blocked` and the lanes' `--sort rank`, which compute none. The `[ranking.list]` default order
  takes them.
- **`nulls` on a `count` or `sum`** is refused: a count is never null (§3.2).
- **The store-free lanes** (`read::next_active`) rank a `[ranking.next]` with derived keys from
  the board's `RankingGraph` since 1c5s (§6), with the live-neighbour rule above; since xkr7 an
  order that reads the rank of a type only `[ranking.list]` orders too, from the graph's listed
  items. The store-free `list` (`read::list_board_page`, xkr7) ranks a `[ranking.list]` the same
  way.
- **TOML.** An inline table may not span lines outside its arrays (TOML 1.0), so a `sum`'s `dir`
  stands on the line its array closes on. §8 above breaks that line for reading only.
