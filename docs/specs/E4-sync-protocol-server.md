# E4 — Sync Protocol + Server (durable, multi-device, realtime) (Design Spec)

> Prerequisite: E2 (Agent CLI MVP, local-first).
> This spec covers the **first slice (structure sync) plus its correctness preconditions**
> and deliberately carves out body Y.Doc sync, realtime push, multi-tenant/auth, and the durable
> backends as **intentionally later, independent slices** (§10).

## 1. Goal & Scope

"Local-only" becomes "from multiple devices/locations". The **single claim** that justifies
E4: issues created/updated offline **converge after reconnect without a manual merge** — the
class of bug familiar from git-style sync (branch-switch drift) is thereby structurally dead,
not merely rarer.

The op-log is already a clean sync payload: an append-only `ops` table whose fold is
order-independent (keep-if-beats LWW + observed-remove OR-set + grow-only tombstones, E1). The
hard CRDT work is **done**. E4 adds: a durable server, a transport, and the **preconditions**
that only bite once two replicas actually merge (prefix distinctness, forward-compat fold).

**Scope.** The server is a **neutral, durable op carrier** (dumb relay) — not a semantic
authority that recomputes state. "Durable server truth" means **durable custody of the
authoritative op-log**, not server-side derivation. Convergence is still proven **client-side
in the unchanged fold**.

## 2. Architectural Decisions

The forks in this slice, each with its rationale and the rejected alternative:

1. **Dumb durable relay instead of a smart server.** The server persists the raw op-log and
   serves "ops since cursor" — it **does not fold, does not derive, does not validate**.
   *Rejected:* a server that folds/validates/rejects-at-ingest collides head-on with `a2p`
   (unknown ops must be stored-not-folded, **never** rejected) and with the "core is
   opinion-free" line. Server-side validation only much later as defense-in-depth, **never** as
   a gatekeeper.

2. **Narrow, SQL-free storage trait, with Postgres/DynamoDB behind it.** Slice 1 carries the
   trait with SQLite and proves convergence; the durable backends are their own later slices
   (§10). *Rejected:* a Postgres schema/migrations/deploy in slice 1 — a horizontal trap that
   puts convergence behind infra and bends the trait design toward SQL.

3. **Offline mint + register/remap-on-join** for prefix distinctness (`bab`, §6).
   *Rejected:* (b) merely widening the prefix space never guarantees distinctness, only
   improbability — it weakens the "bug class is dead" claim; (c) pure detect+remap without a
   registry is conceivable, but with a relay in the loop the registry is the more robust
   coordination point.

4. **A server-assigned, monotonic total-ordered sequence as the cursor** (§5). It maps onto an
   indexed Postgres column *and* a DynamoDB sort key. *Rejected:* a version vector (per-site
   Lamport high-water) is not totally ordered and does not map onto a single SK — unnecessary
   for a relay whose client folds order-independently anyway.

## 3. Crate Layout

Two new crates; the core is touched **additively** (only `a2p`, §7):

```
crates/
  core/      # nexus-flow-core (E1) — op-log, fold, derivation. a2p: ingest accepts
             #   unknown ops, fold skips them (additive, backward-compatible).
  cli/       # nxf (E2) — new verb `nxf sync`; stream bind in the workspace meta.
  sync/      # NEW: protocol types (versioned wire envelope over core::Op) +
             #   client anti-entropy engine. No server requirement, usable from the CLI.
  server/    # NEW: relay binary (Axum). Carries OpStore + PrefixRegistry + HTTP.
             #   Folds nothing, derives nothing.
```

`sync` and `server` share the wire types; `server` does **not** depend on the core's fold code
(it stores opaquely). This keeps the "dumb relay" line visible in the dependency graph as well.

## 4. Data Flow & Sync Protocol

### 4.1 Wire envelope (versioned, server-opaque)

Every op travels as a versioned envelope (serde over `core::Op`):

```
WireOp {
  envelope_version: u16,        // forward-compat discriminator
  op_id, lamport, site,         // identity + LWW metadata (E1)
  target_kind, target_id, field, op_type, value, author, wall_clock
}
```

The server treats the envelope as **opaque**: it stores all fields, indexes only by the
assigned sequence, and **never** rejects on an unknown `target_kind`/`op_type`/
`envelope_version` (§7). This is the server half of the forward-compat story.

### 4.2 Stream bind (create vs. join)

A **stream** is the unit of sync = one workspace, addressed by a durable `stream_id`. When
binding to a remote there are **exactly two paths**:

- **Create** (first replica): generates a fresh `stream_id`, records it in the workspace meta,
  registers it with the relay.
- **Join** (every additional device): binds to the **existing** `stream_id` shared out-of-band
  (slice 1: via flag/config, no discovery) and creates **no** new one.

This is correctness-relevant for `bab`: the prefix registry hangs off `(stream_id, prefix)`. If
a second device accidentally binds to a *different* `stream_id`, it registers in a different
stream and the distinctness guarantee does not hold. Bind is therefore an **explicit step**,
not an implicit side effect of `nxf sync`.

### 4.3 Sync flow (client-driven, realtime deferred)

The client holds **two watermarks**:

- `pushed_through` — the durable **local insertion sequence** (SQLite `rowid` of the `ops`
  table). Deliberately *not* a ULID/wall-clock: under concurrent `nxf` processes (WAL), ULIDs
  from different processes interleave, whereas `rowid` stays monotonic per store.
- `pulled_through` — the **server cursor** (§5).

`nxf sync` runs as:

```
Step 0  Ensure stream bind (§4.2);
        register_prefix(stream_id, prefix, replica_uuid)  -> Granted | Reassigned(new)
        on Reassigned: local remap pass (§8) BEFORE anything else  // gates the push
Step 1  PUSH: local ops with rowid > pushed_through to the relay (append) — in
        bundled batches of <= page_limit ops each (ceil(N/page) requests, mirroring
        the pull); advance pushed_through PER batch. The relay sets a deliberate
        request limit (DefaultBodyLimit + op-count cap, 413 on overflow); client
        pagination keeps every batch below it, so a large history is not
        rejected (k64). A partial push leaves pushed_through exact → the next pass
        resumes without pushing an op twice or skipping one.
Step 2  PULL: read_since(stream_id, pulled_through) -> (WireOps, next_cursor);
        apply each op via core::apply() (dedup-by-op_id, §4.4);
        pulled_through = next_cursor; paginate until exhausted
```

Step 0 is **upstream and gates the push** — otherwise unmapped IDs leak into the stream.

### 4.4 Idempotence & dedup

Double-apply is safe: the fold ops are set-/max-based (keep-if-beats LWW, OR-set add by
`op_id` tag, grow-only tombstones), so they are stable on re-application. In addition, the
**local ingest explicitly deduplicates by `op_id`** (the op-log is an idempotent union) — this
is a named invariant with its own test, not a mere assertion. Re-sync after a partial
push/pull therefore converges without double-counting.

### 4.5 Machine presence (added 2026-09-21, `6j6v.f0b5`)

A later addition, and the relay's one piece of state that is **not history**. After every pass it
syncs, a machine's background service announces itself — `POST /streams/{id}/machines` with
`{machine_id, name, interval_secs}` — and anybody may ask `GET /streams/{id}/machines` for the
stream's machines, most recently seen first. The relay stamps each announcement with its own clock
and answers with the age; it judges nothing. Whether a machine is online is decided client-side, in
one place (`nxs_sync::presence`: `age <= 2 × cadence + 60 s`, 11 minutes at the default cadence).

- **Not in the op-log.** One row per `(stream, machine)`, overwritten on every announcement. A
  heartbeat as an op would grow every stream's history forever and replay into every replica.
- **Only the service announces**, never a manual `nxs sync run` — "online" promises that something
  on the machine attends the stream.
- **A machine is a service home** (`nxs_service::machine`): one per background-service instance.
- **Compatible both ways.** A relay without the route answers 404; the client reads that as "no
  presence here" and the pass still succeeds. An old client never calls the route and syncs as
  before; it is simply not listed.
- **Unauthenticated, therefore small** (see `6j6v.6aza`): a random machine id, a name the owner can
  change, the cadence. No path, no user, no replica identity. A sighting is a claim, never an
  authorization.

- **Bounded, because nobody is authenticated.** A machine not heard from for 30 days is forgotten
  (not listed; pruned where the backend can, `expires_at` for an optional DynamoDB TTL); one answer
  carries at most 100 machines and says `truncated` when the relay holds more; an announcement body
  is capped at 4 KiB; the longest cadence is an hour, which bounds how long anybody can keep a
  stopped machine reading "online" to about two hours. A flood inside the window is only stopped by
  relay authentication (`6j6v.6aza`).
- **"Online" survives one failed pass at every cadence**, because the service announces
  `max(interval, retry backoff)` — the backoff is a fixed 300 s, and the interval alone would not
  cover it below about 240 s.

The storage is an optional capability of the registry: `PrefixRegistry::presence()` returns the
backend's `PresenceStore` (`crates/server/src/presence.rs`), defaulted to `None` so a registry
written before presence keeps compiling and the relay then answers the route like one that predates
it. All three in-repo backends have one; on DynamoDB it needs `dynamodb:Query` on the registry
table.

## 5. Storage Traits

**Two** responsibilities, **two** traits — both SQL-free, both with explicit atomic/conditional
write semantics (that is the actual backend risk). (A third, `PresenceStore`, came later as an
optional capability of the registry — §4.5; it holds state, not history, and needs no atomicity
beyond an upsert.)

```
trait OpStore {
    // Appends the op and assigns a monotonically increasing sequence per stream.
    // ATOMIC under concurrent writers (backend: atomic counter / conditional put).
    fn append(&self, stream: &StreamId, op: &WireOp) -> Result<Cursor>;
    // Range SK > cursor, paginated (limit + next-cursor); never everything at once.
    fn read_since(&self, stream: &StreamId, cursor: Cursor, limit: usize)
        -> Result<(Vec<WireOp>, Cursor)>;
}

trait PrefixRegistry {
    // Claim-if-free: binds prefix to replica_uuid in the stream if free; otherwise reports
    // the existing owner. ATOMIC (conditional put / unique constraint).
    fn register(&self, stream: &StreamId, prefix: &Prefix, replica: &ReplicaUuid)
        -> Result<PrefixGrant>;   // Granted | Reassigned(Prefix)
}
```

**Cursor.** `Cursor` = the server-assigned monotonic sequence per stream. Backend mapping:

- **Postgres:** indexed `BIGINT` column, PK `(stream_id, seq)`. `read_since` =
  `WHERE stream_id = ? AND seq > ? ORDER BY seq LIMIT ?`. **Impl note (slice 3):** the seq is
  assigned via `MAX(seq)+1` under a per-stream `pg_advisory_xact_lock`, **not** via
  `GENERATED AS IDENTITY`/sequence. Reason: a global identity/sequence is globally monotonic →
  gappy per stream and not starting at 1, which would break *value-for-value* parity with the
  SQLite impl (contiguous-from-1, gapless, idempotent re-append consumes nothing). The lock
  makes the recompute-from-MAX atomic; it is namespaced per subsystem (op-store vs. registry
  lock class), since advisory locks share a DB-global keyspace.
- **DynamoDB:** PK = `stream_id`, SK = `seq`. `read_since` = range query `SK > cursor`. The
  **main risk is the atomic monotonic sequence per stream** (atomic counter item / conditional
  put), **not** the 400 KB item limit — the trait already handles that via `limit` +
  `next cursor` pagination.

Slice 1: a single SQLite impl carries **both** traits and proves convergence. The durable
backends (§10) are split **per trait**, because `PrefixRegistry` only comes into being with
`bab` (slice 2) — a graph claiming a half-trait backend "done after T5" would be wrong.

## 6. `bab` — Prefix Distinctness (P1 precondition)

Cross-replica uniqueness today rests solely on the 4-char prefix, which is minted per replica
at random within 32⁴≈1M **without coordination** (`crates/core/src/id.rs`, a documented
hazard). Birthday-bounded, two replicas can draw the same prefix; on merge, same-id creates
from different replicas then silently alias into **one** item. Harmless in single-replica E2,
but a real convergence risk once sync is in play.

**Mechanism (offline mint + register/remap-on-join):**

- A durable, globally unique `replica_uuid` lives in the workspace meta — **separate** from the
  4-char display prefix and from `site`. It is the identity anchor that keeps two replicas with
  a colliding prefix apart.
- The prefix is still minted **offline** (offline-first stays intact; a never-connected replica
  needs no server).
- On the **first sync** against a stream, the replica calls `register(stream_id, prefix,
  replica_uuid)`. If the prefix already belongs to a **different** `replica_uuid`, the relay
  answers `Reassigned(new_prefix)` → the joining replica runs a local **remap pass** (§8)
  **before** the merge.

**Guarantee:** no two replicas share a prefix in the same stream at merge time. The collision
is resolved exactly when the merge would happen — never earlier.

## 7. `a2p` — Forward-Compat (Cross-Version Guarantee)

Today, skipped (unknown/malformed) ops are **not persisted at all**
(`crates/core/src/store.rs`: "skipped op is never persisted", `op_count == 0`). There is thus
**no** resurface path: an older peer that does not understand an op from a newer peer would
lose it for good.

E4 makes forward-compat whole in **two separate halves**:

- **Server opacity** (§4.1, T2/T3): the relay stores/relays all ops regardless of
  `target_kind`/`op_type`/`envelope_version`.
- **Client store-then-skip + re-fold** (core ingest change, **additive & backward-compatible**):
  the local `ops` log **accepts** ops with an unknown `op_type`/`target_kind`, and the fold
  **skips** them instead of matching/dropping them. After an upgrade that understands the op
  shape, the stored ops are **re-folded** (resurfaced). Local writes keep the strict guard.

**Distinction from the slice-1 plan:** on the convergence happy path (both replicas on the same
version) **no** unknown op arises → `a2p` is **not** the T5 blocker but the **cross-version
guarantee**. It runs in parallel with the spine because it is core ingest and touches T2/T4 —
not because T5 waits on it.

## 8. Remap + Free-Text References (`T-remap`, `dqj`, `bm0`)

On `Reassigned(new_prefix)` (§6) the replica remaps locally — **only the prefix component** of
the short IDs is swapped, the **suffixes stay** (within-replica uniqueness is intact via
`mint_unique`, no need to re-draw). Affected:

1. **Structural:** `ops` (`target_id`, edge composite IDs) + materialized views — the `T-remap`
   pass orchestrates the prefix swap.
2. **Free text:** short-ID references in description/note. For this there is **one** shared
   rewrite engine (`dqj`) that rewrites deterministically based on the **reference edges**
   (`bm0`) — **no** second impl in the remap path.

**Chain:** `bm0` (reference edge kind `mentions`, a pure reference without blocking semantics, +
prime rule) → `dqj` (free-text rewrite engine over the reference edges) → `T-remap`
(orchestrates the structural prefix swap **and** calls the `dqj` engine for the free text).
`T-remap` therefore depends on `dqj`; it does not stand beside it.

`bm0` semantics note: do **not** use a dep edge (it would distort `ready`/`blocked`) — a
dedicated edge kind `mentions` without blocking semantics.

## 9. Determinism & Tests (TDD)

- **Convergence (T5, acceptance):** two replicas, each with offline-created/updated issues, sync
  via a local relay → identical materialized state, **without a manual merge**. Prefixes in
  slice 1 coordinated via fixtures.
- **Idempotence/dedup (§4.4):** double push/pull and re-sync do not change the state; explicit
  dedup-by-`op_id` test.
- **Order-independence:** pull in either order → same state (inherited from E1, repeated here
  over the wire).
- **Forward-compat (`a2p`):** an op with an unknown `op_type` is stored-not-folded (no panic,
  `op_count` rises) and re-folded after "understanding" (resurface).
- **Prefix distinctness (`bab`):** two independently (same prefix) minted replicas register →
  one is `Reassigned`, remap runs, the merge aliases **no** foreign item.
- **Stream bind:** joining an existing `stream_id` creates no new one; the registry key stays
  `(stream_id, prefix)`.

CLI contract as in E2: `--json` everywhere, deterministic output.

## 10. Explicitly OUT (later, independent slices)

These are **deliberately** not in the structure-sync slice — their own risks, their own slices:

- **Body Y.Doc sync service** — the second CRDT engine. Hangs off `ra0` (Yrs body backend) and
  becomes its own slice/service (server-side authoritative Y.Doc, the CLI participates in the
  body CRDT). **Not** in this spec.
- **Realtime push** — transport upgrade from client-driven poll/`nxf sync` to server push. Its
  own slice.
- **Auth / multi-tenant** — gated stream access. Slice 1 runs over loopback without auth. Its
  own slice. (Touches `fxn`: `--db`/`NXF_DB` path guard, P3, only once paths come from
  less-trusted sources.)
- **Durable backends** — Postgres and DynamoDB impl of the two traits, split per trait
  (§5/§11). Postgres directly after the convergence proof; DynamoDB later.

## 11. Implementation Order / Decomposition

The hard vertical cut for "closes first" is **T1→T5**; `bab`/slice 2 hardens afterward without
leaving the line; `a2p` runs in parallel (core ingest).

**Slice 1 — convergence over a dumb relay (spine):**

| Ticket | Content | Dep |
|---|---|---|
| T1 | `OpStore` trait (append→atomic monotonic seq, read_since→paginated) + SQLite impl | — |
| T2 | Wire envelope (versioned, serde over `core::Op`, server-opaque) | — (‖ T1) |
| T-bind | Stream bind: `stream_id` create-vs-join in the workspace meta | — |
| T3 | Relay server skeleton (Axum): POST append / GET read_since, loopback, no auth | T1, T2 |
| T4 | Client sync engine + `nxf sync`: 2 watermarks (`pushed_through`=rowid), dedup-by-`op_id` | T2, T3, T-bind |
| T5 | **Acceptance**: 2 replicas offline → sync via relay → convergent (fixtures prefixes) | T4 |

**Core precondition (parallel):**

| Ticket | Content | Dep |
|---|---|---|
| `a2p` | Core store-then-skip + envelope versioning + re-fold/resurface (cross-version guarantee, **not** a T5 blocker) | ↔ T2, ↔ T4 |

**Slice 2 — merge hardening:**

| Ticket | Content | Dep |
|---|---|---|
| `bab` (P1) | `replica_uuid` + `PrefixRegistry` trait (claim-if-free) + SQLite impl + offline-mint + register-on-join + `Reassigned` | T1, T-bind, T4 |
| `bm0` | Reference edge kind `mentions` + prime rule | — |
| `dqj` | Free-text rewrite engine over reference edges | `bm0` |
| T-remap | Prefix swap (prefix component only) on `ops`+views, calls the `dqj` engine; step 0 of the sync | `bab`, `dqj` |

**Slice 3 — durable backends (split per trait):**

| Ticket | Content | Dep |
|---|---|---|
| `pg-opstore` | Postgres `OpStore` (atomic monotonic seq = main risk) — directly after T5 | T1 |
| `pg-registry` | Postgres `PrefixRegistry` (claim-if-free) | `bab` |
| `ddb-opstore` | DynamoDB `OpStore` (atomic counter/conditional-put per stream) | T1 |
| `ddb-registry` | DynamoDB `PrefixRegistry` | `bab` |

**Hardening:**

| Ticket | Content | Dep |
|---|---|---|
| `k64` | Bounded push (client pagination, ceil(N/page) batches) + deliberate relay request limits (DefaultBodyLimit + op-count cap → 413); §4.3 | T4 |

**Deferred (their own slices, §10):** body Y.Doc sync (dep `ra0`), realtime push, auth/multi-tenant,
`fxn` (path guard, P3).
