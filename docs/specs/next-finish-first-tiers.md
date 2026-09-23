# next — finish-first tiers (work-selection ranking)

Decision record + decomposition spec for making the `next` recommendation push toward **finishing**
what is started before starting new work (Kanban: *stop starting, start finishing*). Today `next`
ranks purely by the plugin's field-based policy and hides in-progress work behind a flag; this spec
adds a **graph-derived tiering** on top of the unchanged plugin rank. Prompted by a PM bug report
against the `issue-tracker` plugin (board `41j0`).

## 0. Goal & non-goals

**Goal:** reorder `next` so it surfaces, in this order, (1) in-progress work that can be **closed
now**, (2) the open children of **started epics**, grouped under their epic, then (3) the general
ready backlog. The change is **graph-derived and type-agnostic** — it reads only the `parent` edge
and `status`, never plugin vocabulary (no "epic", no priority). The plugin's `[ranking.next]` policy
(priority → type → due → id) is unchanged and still decides order *within* a tier.

**Design stance.** This respects the deliberate boundary recorded at `crates/core/src/derive.rs`
(the removed `derive::next` that hardcoded a priority range): **ranking stays plugin policy**. What
this spec adds to the core is **tier *assignment*** — a pure graph fact ("is this item finish-first /
a promoted child / general"), the same kind of convergent SQL over the materialized views as
`ready`/`blocked`/`suppressed`. The final order is composed in the facade: `(tier, cluster, plugin
rank)`.

**Non-goals:**

- No change to the derived **sets** `ready`/`blocked`/`deferred`/`in_progress` themselves, nor to the
  07a parent-gating / suppression / closed-mask rules. Tiering is a **selection + ordering** concern
  layered on top; a suppressed/blocked/deferred item is still excluded exactly as before.
- No new plugin-config surface. Tiering is not expressible as a declarative field key and we do **not**
  add a graph-aware key kind (speculative generality — rejected).
- Tiering applies to the **`next` verb only**. `list --sort rank` and `blocked` keep their flat,
  field-based rank (the shared `order_by`/`sort_cmp` are untouched).

## 1. Current behavior (the two root causes)

`crates/facade/src/read.rs::next` builds `ready` (`derive::ready`) and, **only when
`include_in_progress` is set**, also folds in `derive::in_progress`; it then sorts the union with the
plugin's `[ranking.next]` keys via `rank_cmp`. That ranking is **purely field-based on each item
itself** — status precedence, priority, type, due, id. It has no vocabulary for "rank me higher
because my *parent* is in-progress."

Two distinct symptoms follow:

1. **Scattered children (ranking gap).** The open children of an in-progress epic are ranked by their
   *own* priority and mix into the general backlog. Newer, higher-priority ready work outranks
   finishing what is already started.
2. **Vanishing done epic (visibility default).** Plain `nxf next` defaults to
   `include_in_progress = false`, so an in-progress epic whose children are all closed is **absent
   entirely** from the default command — it never surfaces, so it never gets closed. (`nxs prime`
   passes the flag, which is why prime's snapshot shows in-progress rows.)

## 2. Desired behavior

`next` **always** includes actionable in-progress work (the `--include-in-progress` flag is
removed — §5). The candidate set is `ready ∪ in_progress` (both from `derive`, both already
respecting suppression/gating/closed-mask). It is then ordered into three tiers:

### Tier 1 — closeable now

Actionable in-progress items with **no live, non-closed child** — i.e. every child is closed (or the
item has no children). Covers empty/all-done epics **and** in-progress leaves (a claimed bug/chore).
These are exactly the items an agent can finish/close immediately. Ordered internally by the plugin
rank.

### Tier 2 — started epics (carry to done)

Each **actionable** in-progress parent (in the `in_progress` set) that has **≥1 live non-closed
child** forms a **cluster**:

- the **parent as a header row**, followed by
- its **open, actionable children** (the ones already in `ready`).

Clusters are ordered by the **parent's** plugin rank (then parent id); children within a cluster by
their own plugin rank. This is where the **non-empty epic is visible** — surfacing the parent as a
header is a deliberate refinement over the original report (which hid it), driven by the consuming
app's need to display the epic as a container (§5).

A started epic whose only remaining non-closed children are blocked/deferred/in-progress (none
currently actionable) appears as a **lone header** with no child rows — visible, but without
"do-this-now" child noise.

### Tier 3 — general backlog

The remaining `ready` items — those that are **not** open children of an in-progress parent — ordered
by the plugin rank exactly as today.

### Worked example (board `41j0`)

- `a0gw` (P1 in-progress epic, 10/10 children closed) → **Tier 1** (surface to close). Previously
  absent.
- `cd6k` (in-progress epic) + its open children `9z2b`, `nwh3`, `sbek`, `zj10` → **Tier 2**, the epic
  as header, the four children grouped beneath it by priority. Previously scattered across positions
  4/8/11/16.
- everything else ready → **Tier 3**, unchanged priority order.

## 3. Core derivation (`crates/core/src/derive.rs`)

Two new pure-SQL functions, sitting beside `ready`/`in_progress`/`suppressed_by_*`. Both are
deterministic, id-sorted where they return single ids, and **fallible** (a db error maps to `io`, like
the neighbours — never `.unwrap()`).

### 3.1 `finishable_in_progress(conn, now) -> Vec<String>`

The **Tier-1 set**: the same actionability filter as `in_progress` (unblocked, acyclic, not deferred,
and respecting the 07a suppression + closed-mask, so a claimed item under a blocked ancestor stays
hidden), **plus** `AND NOT EXISTS` a live child (`present_edges.kind='parent'`, `from_id`=child,
`to_id`=this item) whose status is `open` **or** `in_progress`. "Live" = not deleted. A child that is
closed does not count; a child that is still open or in-progress disqualifies the parent (it is not
yet finishable). Reuses the shared `BLOCKED_GATING_CTE`/`DEFERRED_GATING_CTE`/`cyclic` machinery so it
tracks the same suppression as `actionable`.

### 3.2 `promoted_children(conn, now) -> Vec<(String, String)>`

The **Tier-2 child→cluster map**: `(child_id, promoting_parent_id)` for every child that is in the
`ready` set (open + actionable) whose `parent` (`present_edges.kind='parent'`) is an **actionable
in-progress** item — i.e. the parent is itself in the `in_progress` set (§3.3), a genuine Tier-2
header. One row per child; if a child has several in-progress parents (multi-parent, not possible in
`issue-tracker` but the core is generic), the lexicographically smallest parent id is chosen so
clustering is unambiguous and deterministic.

### 3.3 Deriving the Tier-2 headers

The facade needs the set of **started epics** (Tier-2 headers). It is derived from the two functions
above, not a third query:

```
headers = in_progress \ finishable_in_progress
```

= actionable in-progress items **with** a live non-closed child = the started epics (including the
"lone header" case whose non-closed children are none of them actionable). Because §3.2 restricts a
promoting parent to `∈ in_progress`, **every** promoted child's parent is already in this set, so no
cluster is ever headerless — no union trick, no header fetched from outside `next`'s candidate set.

**Blocked parents stay in `blocked` (07a, unchanged — the explicit invariant #07a.2).** A blocked
(open) parent suppresses its **entire subtree** into the `blocked` lane: neither the parent nor any
descendant appears in `ready`/`in_progress`/`next`, so none of them enters any tier here. A child is
therefore promoted (§3.2) **only** if it is in `ready`, which already means it is *not* suppressed —
so its promoting parent is never a blocked ancestor. The rare in-progress-but-blocked parent (its own
open dep) is not in the `in_progress` set, so its child is not promoted and simply falls to Tier 3;
this spec deliberately does **not** change whether such a child is suppressed (that is 07a's concern,
left exactly as-is per §0).

## 4. Facade ordering (`crates/facade/src/read.rs`)

### 4.1 `next` signature & candidate set

```
pub fn next(cfg: &PluginConfig, store: &Store, now: &str, sort: Option<SortKey>) -> Result<Vec<ItemRow>>
```

(The `include_in_progress: bool` parameter is removed — §5.) Candidates = `resolve_items(ready ∪
in_progress)`. Every Tier-2 header is already in `in_progress` and every promoted child in `ready`
(§3.3), so all header and child rows come from this one candidate set — no rows are fetched from
outside it.

### 4.2 Tiering is `next`-local

The shared `order_by`/`sort_cmp`/`rank_cmp` are **not** modified (so `list`/`blocked` keep flat rank).
`next` gets a dedicated tiered assembly used **only for the default `rank` order**. An explicit
`--sort id` (or any non-`rank` key) on `next` **bypasses tiering** and gives the flat order (escape
hatch), via the existing `order_by`.

### 4.3 The tiered order

Build a per-candidate classification from §3, then order by the tuple:

1. **tier**: 1 < 2 < 3.
2. within **tier 1** and **tier 3**: the existing `rank_cmp(cfg, …)` (priority → type → due → id).
3. within **tier 2**: `cluster_rank` = the *parent's* `rank_cmp` then parent id (stable cluster
   ordering); then **header-before-children** (header sorts ahead of its own children); then, among a
   cluster's children, the child's `rank_cmp`; id as the final tiebreak.

The whole order is total and deterministic (id is always the last tiebreak).

### 4.4 The `parent` join is retained

`next_to_value`/`prime_next_value` already attach each row's resolved `parent` (`parent_of` →
id/title/type). This is **kept** — combined with the Tier-2 header it is what lets the app render
"epic → its children" whether it groups by the header row or by each child's `parent` field.

## 5. Remove `--include-in-progress`; SemVer

The flag is redundant once in-progress is always included, and is removed end-to-end:

- **CLI** (`crates/cli/src/lib.rs`, `crates/cli/src/commands/mod.rs`): drop the `--include-in-progress`
  arg and its plumbing into `commands::next`.
- **MCP** (`crates/nxs/src/mcp/params.rs`, `crates/nxs/src/mcp/flow.rs`): drop the
  `include_in_progress` param. **The param struct must tolerate a still-present `include_in_progress`
  field from an older client** (no `deny_unknown_fields` that would hard-error) — verified as part of
  the "engine doesn't break" audit.
- **Facade** (`crates/facade/src/read.rs`): drop the bool from `read::next`.

**SemVer.** Removing the parameter from the `crates/facade` public `next` is a **signature change** →
a break under the facade contract (`docs/specs/release-management.md` §4.3–4.5), which belongs on a
**minor** bump, not a patch. The changelog fragment is flagged `facade: breaking`. `facade-semver`
(cargo-semver-checks) will confirm it is caught on a patch and passes on a minor.

**Consumer audit ("die Engine bricht nicht").** Walk every `next` consumer — CLI `nxf next`, MCP
`flow::next`, `prime`, and the board app (`--json`) — and confirm: (a) the app still sees the non-empty
epic via the Tier-2 header + the retained `parent` join; (b) no consumer relies on the removed flag in
a way that now errors (MCP tolerates the stale field; CLI/prime are updated in-tree).

## 6. Prime (`crates/facade/src/read.rs`, `crates/cli/src/commands/mod.rs`)

- `prime` calls `next(cfg, store, now, None)` — the tiered default (in-progress always included). The
  old `include_in_progress = true` argument is dropped with the flag.
- Raise `PRIME_NEXT_LIMIT` **7 → 15** (`read.rs:1721`), so a started epic with ~10 children is not
  truncated mid-cluster and an agent does not decide on a partial list.

## 7. Edge cases

- **In-progress leaf** (no children): Tier 1 (finishable) — the type-agnostic "finish what you
  started" rule.
- **In-progress epic, mixed children (some open, some closed):** Tier-2 header; its open actionable
  children are its Tier-2 rows; closed children ignored.
- **In-progress epic, remaining children all blocked/deferred/in-progress:** lone Tier-2 header, no
  child rows.
- **In-progress child of an in-progress parent:** the child is in-progress (not "open"), so it is a
  Tier-1 candidate (finishable if it has no open child of its own), **not** a Tier-2 child. When it
  closes, its parent may become finishable and move to Tier 1.
- **Blocked (open) parent:** 07a-suppressed — parent and its whole subtree are in the `blocked` lane
  only, never in any tier (unchanged; §3.3).
- **In-progress-but-blocked parent** (its own open dep): not in `in_progress`, so it is not a Tier-2
  header and its child is **not** promoted — the child falls to Tier 3. Whether that child is itself
  suppressed is 07a's concern, left exactly as-is.
- **Suppressed subtree** (blocked/deferred ancestor): unchanged — the whole subtree is excluded from
  `ready`/`in_progress`, so it contributes to no tier.
- **Multi-parent child** (generic core; not `issue-tracker`): clustered under its smallest in-progress
  parent id.
- **Deeper hierarchies** (generic core): the rules are local (own status + own children + parent
  status), so they generalise beyond the 2-level `issue-tracker` shape.

## 8. Testing (TDD)

**Core** (`derive.rs` unit tests, in the existing style):

- all-children-closed in-progress epic → in `finishable_in_progress`; epic with one open child → not
  finishable, that child appears in `promoted_children` with the epic as parent.
- in-progress leaf → finishable.
- in-progress epic with an in-progress child → not finishable (in-progress child is non-closed).
- suppression: a finishable-looking in-progress item under a blocked ancestor is excluded.
- multi-parent child → smallest parent id chosen; determinism (id-sorted output).

**Facade** (`read.rs` tests):

- Tier 1 before Tier 2 before Tier 3; within-tier order is the plugin rank.
- Tier-2 cluster: parent header immediately precedes its children; clusters ordered by parent rank;
  children by priority (the `41j0` scenario).
- Lone header (no actionable children) still appears.
- **Blocked-parent invariant preserved:** a child under a blocked (open) parent appears in **no**
  tier of `next` (07a suppression untouched); the subtree stays in `blocked`.
- `next` no longer takes the flag; in-progress is present by default.
- `next --sort id` bypasses tiering (flat id order).
- `PRIME_NEXT_LIMIT` = 15; prime snapshot uses the tiered default.

**Gates:** `cargo test`, `cargo test --release`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` all green.

## 9. Changelog & release

- One `changes/<slug>.md` fragment, EN + DE, `type: changed`, flagged `facade: breaking` (the removed
  `next` parameter). Validate with `cargo xtask changelog check origin/main` before pushing.
- Lands via a `feat/` branch + PR (never direct to `main`).

## 10. Out of scope

- Any change to the `blocked`/`deferred`/`closed`/`archived` lane verbs or their orders.
- A graph-aware `[ranking.next]` config DSL (rejected as speculative generality).
- Board/app UI work — this spec only guarantees the app *can* render the epic (header + `parent`
  join); the rendering lives in the consumer.
