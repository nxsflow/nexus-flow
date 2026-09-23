# E1 — Core Data Model, Invariants & Derivation Layer (Design Spec)

> Status: Design · As of: 2026-06-08
> Predecessor: E0 spike (`docs/specs/E0-sync-substrate-spike.md`, verdict GO).
> This is the semantic heart of the engine — **opinion-free**. It defines the universal
> slots, an op-based CRDT substrate, the pure derivation layer, and the invariants.

## 1. Goal & Scope

E1 turns the throwaway spike into the **real production core as a library**: the full
data model of all vision slots, the CRDT substrate, the deterministic derivation
(`ready`/`blocked`/`next`), and invariant detection.

**In E1:**
- Schema for all structural slots on the CRDT substrate
- Op-based substrate: append-only operation log as the single source of truth, materialized views
- Changeset export/apply as wire form (the sync seam; no network/server yet)
- Derivation of `ready`/`blocked`/`next` (pure, deterministic)
- Invariant detection (cycles, referential integrity) — **non-destructive**
- Complete change history per item

**Not in E1 (deferred):**
- yrs/Yjs body runtime (reading/editing the body) → **E2** (CLI, explicit)
- File persistence, CLI, plugin seam → **E2**
- Sync server, realtime, multi-device transport → **E4**
- Log compaction → **E4** (see §9)
- Ranking policy override → **E3** (the core ships only a default ranking)

In-memory SQLite is sufficient for E1; the store already accepts an optional path,
so that E2 only has to open it.

## 2. Crate Structure

A fresh Cargo workspace at the repo root. The spike (`spike/`) stays untouched as a reference
and is deleted later.

```
Cargo.toml                    # [workspace]
crates/core/                  # nexus-flow-core — the E1 deliverable (lib)
  src/
    lib.rs
    id.rs            # ReplicaIds: {prefix}.{ULID} + short form (concept from spike E0.4)
    model.rs         # Domain types: ItemType, Status, Priority, EdgeKind, Op, Note
    schema.rs        # SQLite DDL: ops (log) + materialized views items/edges/notes
    crdt.rs          # Fold rules: LWW register, observed-remove OR-set
    store.rs         # Substrate: local writes (append op + materialize), merge/apply, export
    derive.rs        # ready / blocked / next (pure, deterministic)
    invariant.rs     # cycle/reference checks, isolation set
    history.rs       # change history per item from the op-log
```

## 3. Substrate: Operation Log as the Single Source of Truth

Key decision (a revision relative to the state-based spike export): an **append-only
operation log** is the single source of truth. It unifies three concerns:

1. **Change history** — every field change stays visible, including concurrent edits that are
   later overwritten (motivating case: user 1/computer 1 changes the title, user 2/computer 3
   changes the title concurrently, user 1/computer 2 changes the status — all three remain in the history).
2. **Sync payload** — E4 ships ops, not the current state.
3. **CRDT convergence** — the log is a grow-only set (union by `op_id`); the current state is
   a **deterministic fold** over it.

### 3.1 The `ops` table (the log)

| Column | Type | Meaning |
|---|---|---|
| `op_id` | TEXT PK | ULID of the writing replica → globally unique, time-sortable per replica. In OR-sets it also serves as the add tag. |
| `lamport` | INTEGER | Lamport counter of the replica (counted past everything merged in). |
| `site` | INTEGER | Replica/site id (tiebreak). |
| `target_kind` | TEXT | `item` \| `edge` \| `note` |
| `target_id` | TEXT | Item id; edge composite `{from}{to}{kind}`; or note id. |
| `field` | TEXT | Affected attribute (`title`, `status`, `present`, `body`, …). |
| `op_type` | TEXT | `set` (LWW) \| `add` / `remove` (OR-set) \| `note_add`. |
| `value` | TEXT/NULL | New value (NULL allowed). For `remove`: list of observed add tags. |
| `author` | TEXT | Who (for display). |
| `wall_clock` | TEXT | Timestamp **for display only**; canonical ordering remains `(lamport, site)`. |

`lamport` is the counter of the REPLICA, not of an open handle — a distinction the implementation
lost for a while and 6j6v.fc5p restored: every local write re-reads the log's high-water mark under
the write lock, so two processes on one replica cannot mint the same counter. That gives the log a
second uniqueness beside the primary key, enforced by `UNIQUE INDEX ops_lamport_site ON ops(lamport,
site)`: **`(lamport, site)` identifies at most one op.** Across sites a counter says nothing — that
is what Lamport clocks are for — but the same counter twice from one site is not a race outcome, it
is proof that a writer used a stale clock, and the fold's strict keep-if-beats would discard the
second write in silence. A log written before that fix may carry such pairs; it opens unchanged,
with the index created non-unique (`schema::ensure_op_coordinate_index`).

Convergence: two replicas with an identical set of ops fold to an identical state. This is
backed by the Automerge differential oracle (see §8).

### 3.2 Fold Rules (`crdt.rs`)

- **LWW register** (all scalar item fields): the materialized value = the op with max `(lamport, site)`
  per `(target_id, field)`.
- **Observed-remove OR-set** (edges: `dep`, `contributes_to`): an `add` creates a tag
  (`op_id`); a `remove` tombstones the **observed** add tags. An element is *present* iff there
  is a live add tag. Concurrent add+remove → **add wins** (observed-remove, as
  required by the vision for dependencies; the spike had simplified this to an LWW bool).
- **Grow-only notes**: each note is a `note_add` with immutable content; redaction via a
  separate LWW `deleted` field on the note (tombstone), not an edit.

### 3.3 Materialized Views

`items`, `edges`, `notes` are **materialized projections** of the log — solely for fast
SQL derivation. Write path: append the op **and** materialize the affected cell. Merge path:
apply new ops by union **and** re-fold the affected cells. The current state is never stored
authoritatively; it is reconstructible from the log at any time.

## 4. Data Model — Slots

### 4.1 `items` (one node per row; each column an LWW register)

| Slot (vision) | Field | CRDT | Note |
|---|---|---|---|
| Identity | `id` (PK), `type` | id immutable; `type` LWW | `type ∈ {project, task}` |
| Title | `title` | LWW | |
| Description (body) | `body` | LWW text (**placeholder**) | Replaced by a yrs doc in E2. Slot reserved. |
| Completion criterion | `completion_criterion` | LWW | DoD / acceptance criteria (plugin label) |
| Status | `status` | LWW | Canonical core lifecycle `open` \| `in_progress` \| `closed`. Plugins map only display labels, not the lifecycle. |
| Priority | `priority` | LWW | `0–4`, 0 = highest (modeled after `bd`) |
| Time | `due`, `defer_until` | LWW | nullable timestamps (ISO-8601) |
| Assignment | `assignee` | LWW | nullable |
| belongs-to (1:n) | `belongs_to` | LWW | **Single parent** → register, not an edge. nullable item id. |
| Closing comment | `closing_comment` | LWW | single, overwritable (separate from the note stream) |
| Tombstone | `deleted` | LWW | soft delete |

### 4.2 `edges` (type-agnostic n:m edges, observed-remove OR-set)

PK `(from_id, to_id, kind)`:
- `kind = dep` — dependency; applies automatically on **both levels** (from/to can be
  `project` or `task` → task↔task **and** project↔project).
- `kind = contributes_to` — contributes-to (n:m).

`belongs-to` is deliberately **not** an edge, but a single-parent LWW register on the item.

### 4.3 `notes` (append-only history stream, grow-only)

`id` (its own ULID), `item_id`, `author`, `body`, `deleted` (LWW tombstone). Ordering by the
ULID (time-sortable per replica). The **closing comment** is separate (an LWW slot on the item).

## 5. Derivation Layer (`derive.rs`) — Pure & Deterministic

All pure SQL queries over the materialized views; **no stored state**
(vision: "Convergence ≠ Validity"). Time-dependent queries take `now` as a parameter → they stay
deterministic.

- **`ready(now)`** — an item is `open`, not `deleted`, **not** part of a dep cycle, has no
  present `dep` on an item that is still `open`/undeleted, and `defer_until` is NULL or ≤ `now`.
- **`blocked(now)`** — `open`, not `deleted`, but with ≥1 present `dep` on an open/undeleted
  item **or** part of a cycle.
- **`next(now)`** — `ready(now)` ordered by `(priority ASC, due ASC NULLS LAST, id ASC)`.
  A fixed **default ranking**; plugins replace the policy in E3. The core stays opinion-free,
  yet is usable out of the box.

## 6. Invariants (`invariant.rs`) — Detect + Isolate, Non-Destructive

After a merge the graph may be invalid. E1 **detects** violations deterministically and
**isolates** affected items out of `ready`, marking them derivably as `needs_attention` — an edge is
**never** deleted automatically (no data loss; a human/agent decides).

Checks:
- **Dependency cycles** over `dep` edges (both levels), via a recursive CTE (the procedure from
  spike E0.5, which terminates even on cyclic graphs).
- **Referential integrity**: `belongs_to` points to an existing, non-deleted item of type
  `project` (no `belongs_to` → `task`); edges point to existing items.

Violations are a **derived view**, not stored state.

## 7. History (`history.rs`)

Complete change history per item: `SELECT … FROM ops WHERE target_id = ? ORDER BY
lamport, site`. Per entry: field, old→new value (old = the previous fold winner), `author`,
`wall_clock` (display). Surfaces concurrent, later-overwritten edits in full, once the
ops have propagated.

## 8. Tests — TDD + Differential Oracle

Test-first for **all** behavior (scaffolding/config exempt). Quality gates as in the spike:
`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.

- **Differential oracle**: the Automerge test from the spike (`spike/tests/differential.rs`) is
  ported to `core`. Property: any application order of the ops across multiple replicas
  converges to an identical materialized state. Automerge remains **only** a test dependency,
  not in the binary.
- **Unit tests** per slot (LWW tiebreak), OR-set (concurrent add/remove → add wins),
  notes (grow-only), derivation (`ready`/`blocked`/`next`), invariants (cycle after a merge,
  `belongs_to` break), history (the C1/C3 scenario from §3).

## 9. Open Points / Deliberately Deferred

- **Log compaction**: the op-log grows unbounded. A compaction/snapshot strategy is an
  **E4 topic** (transport/server context). E1 keeps the log complete. (Snapshots arrived with
  6j6v.mxt2 WITHOUT compaction — a snapshot carries the whole log beside the folded views, see
  E4-continuous-sync-client §10 — and since 6j6v.hehx the substrate refuses to delete an op at all.)
- **Body CRDT**: the `body` placeholder is replaced by the yrs doc in E2.
- **Ranking policy**: plugin override in E3; E1 ships only the default ranking.

## 10. Acceptance Criteria

- [ ] Slots as schema on the CRDT substrate (op-based)
- [ ] `ready`/`blocked`/`next` are pure deterministic derivations with tests
- [ ] Invariant detection catches cycles (non-destructive, isolated)
- [ ] Complete change history per item (including concurrent edits)
- [ ] Convergence backed by the differential oracle
