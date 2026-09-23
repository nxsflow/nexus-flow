# E3 ②a — Parent edge in the OR-Set + Migration (Plan-Spec, #sp6.3)

Decomposition spec for slice ②a of the plugin-configurable type system (Option B, #sp6.1). It
covers the three points the ticket mandates *before* coding: (a) the edge-kind choice for
parenthood, (b) the migration path for existing `belongs_to` data, (c) the shim that keeps
derivation + invariants behavior-identical. Siblings ②b (#sp6.6, invariants/matrix), ②c
(#sp6.7, multi-parent derivation) and ②d (#sp6.8, parent cycle check) build on this.

## 0. Goal & non-goals

**Goal:** move hierarchy parenthood from the single-parent LWW register `belongs_to` onto a
**`parent` edge in the existing observed-remove OR-set**, so the *substrate* is n:m-capable like
`dep`/`contributes_to`/`mentions` — while staying **behavior-preserving**: every existing
derivation (`ready`/`blocked`/`next`) and invariant test passes unchanged, single-parent still
enforced at the write seam.

**Non-goals (later slices):** relaxing to true n:m + the relationship-matrix enforcement (②b),
multi-parent derivation (②c), the parent-cycle write-time check (②d), and the plugin-declared
type/parent/cardinality schema (③, #sp6.4). This slice changes the *substrate and migration only*.

## 1. Edge-kind choice (a)

Add a dedicated **`EdgeKind::Parent`** (wire string `"parent"`).

- **Direction:** `from = child`, `to = parent` (the child points at its parent), mirroring the
  old `belongs_to` value (the parent id) stored on the child. So `store.targets_of(child, Parent)`
  yields the child's parent(s), exactly as `deps_of(child)` yields its blockers.
- **Why a new variant, not reuse:** `ContributesTo` already means "n:m contribution", carries no
  hierarchy semantics, and is read by `show`/derivation differently — overloading it would muddle
  both. A distinct kind lets the invariant/derivation SQL filter `kind='parent'` precisely, just
  like `kind='dep'`.
- **Substrate reuse:** the OR-set fold is kind-agnostic — `edge_adds`/`edge_removes`/`present_edges`
  and `TaskReducer::parse_edge`/`fold_edge_add`/`fold_edge_remove` accept any `EdgeKind::parse`-able
  kind. Adding the variant to `EdgeKind::{as_str,parse}` makes the existing add/remove + observed-
  remove convergence apply to `parent` automatically (the differential oracle is extended in §4).

## 2. The `belongs_to` shim — a `present_parent` view (c)

`belongs_to` **leaves `ITEM_LWW_FIELDS`** (no longer an independently writable LWW register and no
longer a column on fresh DBs). Instead a **SQL view `present_parent(child_id, parent_id)`** projects
the single **LWW-winning** parent edge per child:

```sql
-- per child, the present `parent` edge whose add-op has the max (lamport, site, to_id)
CREATE VIEW present_parent AS
  SELECT a.from_id AS child_id, a.to_id AS parent_id
  FROM edge_adds a JOIN ops o ON o.op_id = a.tag
  WHERE a.kind='parent' AND NOT EXISTS (SELECT 1 FROM edge_removes r WHERE r.tag=a.tag)
    AND NOT EXISTS (
      SELECT 1 FROM edge_adds a2 JOIN ops o2 ON o2.op_id=a2.tag
      WHERE a2.from_id=a.from_id AND a2.kind='parent'
        AND NOT EXISTS (SELECT 1 FROM edge_removes r2 WHERE r2.tag=a2.tag)
        AND (o2.lamport,o2.site,a2.to_id) > (o.lamport,o.site,a.to_id));
```

- A pure, convergent function of the converged edge set + add-op metadata — one row per child, the
  `(lamport,site,to_id)` total order guaranteeing uniqueness. Under the ②a write-time single-parent
  constraint there is normally exactly one present parent edge, so it equals the pre-migration LWW
  `belongs_to`; in the transient multi-parent state a concurrent merge can produce it still resolves
  to one deterministic winner (the LWW-equivalent). A **view, not a reducer-maintained column**, so
  there is no op-ordering hazard (an edge that folds before its child's create needs no fix-up) and
  no phantom item rows.
- **Behaviour-preserving payoff:** `get_item`/`list_items` select `present_parent.parent_id AS
  belongs_to` (so `ItemRow.belongs_to` is unchanged for every Rust reader — `read::parent_of`,
  `write::{live_descendants,live_ancestors}` + archive cascade, `record.rs`, CLI display), and
  `invariant::reference_violations` joins `present_parent` instead of the old `i.belongs_to` column.
  Those two SQL spots are the only readers that change; ②c migrates them onto the multi-parent edge
  set directly and retires the single-value projection.

### Write seam (single-parent shim)
`create --parent P` and `update --set parent=P` stop emitting `belongs_to/set` and instead write a
**parent edge** via new store helpers `set_parent`/`clear_parent`. Re-pointing is observed-remove-
then-add (remove the child's current parent edge(s), add the new) so single-parent is *enforced at
write time* — the temporary constraint ②c later lifts. `update`'s existing parent guards (live,
project-typed via `CONTAINER_TYPE`, not-self) are unchanged; only the persisted representation
changes. `store.set_field` keeps rejecting `belongs_to` (now: not in the whitelist), so no code path
can resurrect the register.

## 3. Migration of existing data (b) — a breaking, lossless op-log rewrite

Moving parenthood from an LWW register to an OR-set edge is **not alt-binary-safe**: an old binary
writing `belongs_to/set` (LWW) cannot coherently converge with a new binary's OR-set `parent`
edge — the two resolution rules disagree on a concurrent re-point. Per the migration checklist
(`foundation/schema.rs` rule 4) repurposing a read/written column is the case that **must** raise
the compatibility floor. This is the codebase's documented *first breaking migration*.

- **`SCHEMA_VERSION` 3 → 4**, **`MIN_COMPATIBLE_SCHEMA_VERSION` 0 → 4** (lock out pre-②a binaries
  by design — they would diverge on parenthood). flow and memory ship in **one** binary and fold
  the **same** op-log, so they upgrade together; there is no flow-vs-memory version skew, only the
  intended "use a new-enough nxs" gate.
- **Lossless op-log rewrite** in the flow `Store` open path (modelled on `Store::remap_prefix`:
  rewrite ops in the `ops` table, then `clear_views` + `refold` in one transaction). The substrate
  migrates+stamps the version *before* flow runs, so flow cannot read the pre-migration version —
  the rewrite is therefore **gated by data shape, not by version**, which also keeps the flow-edge
  encoding (the `␟` separator, `kind='parent'`) entirely inside flow (no layering leak into the
  foundation `migrate`). For each child that has a `belongs_to/set` op **and no present parent edge
  yet**, take its **LWW-winning** `belongs_to` op:
  - winner sets a non-null parent → **rewrite that op in place** to a `parent` edge *add*:
    `target_kind item→edge`, `target_id child → child␟parent␟parent`, `field belongs_to→present`,
    `op_type set→add`, `value parent→NULL`; **`op_id`/`lamport`/`site` unchanged** so the edge's
    add-tag is the original op_id and **dedups across replicas** that each migrate independently
    (same converged log ⇒ same winner ⇒ identical rewritten op).
  - winner is a clear (NULL) or there is none → no edge.
  - All **non-winning** `belongs_to` ops are left as-is; with `belongs_to` no longer foldable they
    become store-but-don't-fold (§7) — inert, lossless, ignored on refold.
- **Idempotent + cheap:** the "no present parent edge yet" gate excludes already-migrated children,
  so a re-open finds nothing to rewrite and skips the `clear_views`+`refold` entirely. A child whose
  winning value was a clear is re-scanned but produces no rewrite (a no-op). Convergent under the
  floor: every replica is ≥ v4, migrates at open before any sync, and migrates identically, so no
  un-migrated `belongs_to` op ever crosses the wire.

## 4. Tests — TDD + differential oracle

- **Substrate:** `add_parent_edge`/`remove` round-trip; observed-remove (concurrent add vs remove
  keeps the edge); the `belongs_to` projection equals the winning parent and is order-independent.
- **Differential oracle** (`core/tests/differential.rs`): extend `Observation` with a `parents`
  field (the converged `parent` targets of a child), alongside `deps_a`/`deps_b`. TEST 1 (conflict-
  free) cross-checks against Automerge; TEST 2 (concurrent parent add-vs-remove + concurrent
  re-point) asserts intra-substrate convergence. This is the §7-flagged risk surface — exercised
  hardest here.
- **Migration:** a v3 DB with `belongs_to` set, opened by the v4 binary, yields the same parent
  (projection) and the same `ready`/`blocked`/`parent_of`/archive-cascade results; the winning op
  is an edge, superseded ops are inert; re-running open is a no-op.
- **Behaviour-preservation:** the entire existing derivation/invariant/facade/CLI suite stays green
  with single-parent still enforced (the 9 direct `set_field("belongs_to", …)` test sites move to
  the new parent-edge store API).

## 5. Acceptance (mirrors #sp6.3)

`parent` edge exists in the OR-set and is n:m-capable in the substrate; the `belongs_to` register
is removed (projection only); existing `belongs_to` data is losslessly migrated to edges;
derivation + invariant tests stay green with single-parent enforced; differential oracle +
`cargo test`/`--release`/`clippy`/`fmt` green.
