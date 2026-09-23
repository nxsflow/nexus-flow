# E0 — Spike: Sync Substrate, Body CRDT & ID Strategy

> Status: **DONE — GO** · As of: 2026-06-08
> Upstream: [Product Vision](../vision/nexus-flow.md) · Ref: `beads-dashboard-av4`
> **Type: Spike.** Time-boxed, throwaway code (`spike/`). The goal is *validation*, not production code.

## Go/No-Go Report (E0.6) — Result: **GO for E1**

All axes validated (30 tests green across 9 suites, throwaway code in `spike/`).

| Axis | Verdict | Decision / Evidence |
|---|---|---|
| **Structure CRDT** | ✅ GO | **In-house build** (relational, SQL-native, ~250 lines) as primary; tracers 1/2/3/7 green. **Automerge** as differential-test oracle. cr-sqlite rejected (dormant). |
| **Body CRDT** | ✅ GO | **yrs**; offline merge (case 4) + genuine **yrs↔yjs interop** (both directions, multibyte). |
| **ID strategy** | ✅ GO | **ULID + replica prefix** → collision-free offline (case 5), time-sortable, short display form. |
| **Derivation** | ✅ GO | ready/blocked + tombstone exclusion + cycle detection, pure & deterministic (SQL/CTE). |
| **Cold start / size** | ✅ GO | ~1.2 MB **single static binary**, ~5 ms cold start — thresholds (50 MB / 50 ms) met by orders of magnitude. |

**Risks carried into E1+ (consciously left open):**
- **Long tail of CRDT correctness** for the in-house build beyond the 4 cases → covered by the Automerge oracle (shared conformance suite).
- **Incremental sync**: only full-state export/merge validated; watermark/delta sync is E4.
- **UTF-16 indexing** for position-based body edits with non-BMP characters → a detail for E2.

Details per axis in the `E0.x findings` below.

## Goal

To **empirically validate** the foundational decisions of nexus-flow before durable work
(E1–E4) is built on top of them. The spike answers one core question — *which structure CRDT
carries an offline-first, SQL-queryable project/task model?* — and confirms two secondary axes
(body CRDT, ID strategy) against a single end-to-end scenario.

## Decisions already made (input, not part of the benchmark)

- **Language/runtime: Rust.** Rationale: cold start *is* agent ergonomics (the CLI is invoked
  dozens of times per session); a single static binary is the best adoption driver for a
  downloadable engine; Yrs + cr-sqlite are native.
- **Body CRDT: Yrs** (Rust port of Yjs). In the spike, only *maturity + merge correctness against
  the web Y.Doc* are to be confirmed — no choice of alternatives.
- **IDs: ULID/UUID + per-replica prefix** (collision-free offline) + short display form.

## What the spike is NOT

- No server (transport is deliberately dumb — see below); the real server is E4.
- No plugin system, no real CLI UX, no complete data model.
- No code that survives into E1+ — the insight is the product, not the binary.

## Axes & candidates

| Axis | Candidates | Stance |
|---|---|---|
| **Structure CRDT** | _originally:_ cr-sqlite (primary) · Automerge (fallback) · hand-rolled (last resort). **→ revised after E0.2 (see "Substrate decision"): the hand-rolled in-house build becomes primary, cr-sqlite is the reference.** | cr-sqlite places CRDT semantics *inside* SQLite tables → SQL-queryable, which is what the derivation (`ready`/`blocked` = graph traversal + status filter) needs. ElectricSQL is out (TS-first, awkward from Rust). |
| **Body CRDT** | **Yrs** | settled; only verify merge against web Yjs |
| **IDs** | **ULID + replica prefix** | settled; only verify collision-freedom under offline concurrency |

## Tracer bullet — the decisive scenario

Two replicas **A** and **B** (each with its own local SQLite DB + CLI instance) both go *offline*,
work concurrently, then reconnect. **Transport:** changesets are copied as a file back and forth
between the DBs (simulating the later server relay). Success = everything converges **without a
manual merge step**.

| # | Concurrent (A ∥ B offline) | Must converge to |
|---|---|---|
| 1 | A: `status=in_progress` on T1 ∥ B: `priority=0` + new dep on T1 | **field-level LWW** — no lost update, both fields survive |
| 2 | A: add dep T1→T2 ∥ B: remove dep T1→T3 | **OR-set** (observed-remove) correct |
| 3 | A: dep T1→T2 ∥ B: dep T2→T1 | after merge a **cycle** → invariant repair detects it, `ready`/`blocked` stays deterministic |
| 4 | A: edit T1 body at pos. X ∥ B: edit T1 body at pos. Y | **Yrs** merges both, nothing lost |
| 5 | A and B each create new tasks | **no ID collision** (ULID + replica prefix) |
| 6 | (after convergence) | `ready`/`blocked` computes correctly from the merged state |
| 7 | A: *delete* T1 (tombstone) ∥ B: *update* T1 | **tombstone wins deterministically** (the row owns the lifecycle); no "zombie" row, no crash of the derivation |

## Go / No-Go thresholds

- **Convergence:** all 7 cases converge without a manual merge step. *(hard core)*
- **Cold start:** binary invocation **< ~50 ms** (agent-ergonomics yardstick).
- **Binary size:** target **< ~50 MB**.
- **Decision:** if cr-sqlite handles everything → primary confirmed. If it breaks → an
  Automerge+SQLite projection must handle it, otherwise hand-rolled. The result is a documented
  **Go/No-Go per axis** with rationale.

## Deliverables (the spike's actual product)

1. A runnable Rust mini-harness that reproducibly plays through the tracer scenario.
2. Measurements: cold start, binary size, convergence result per case.
3. **Go/No-Go report** per axis (structure CRDT, body CRDT, ID) with rationale and the
   recommendation for E1+ (chosen substrate, known pitfalls).

## Risks

- **cr-sqlite maturity/maintenance** — the main reason for the fallback path.
- **Yrs ↔ Yjs interop** — merge compatibility with the later web Y.Doc must be checked early,
  not first in E4.
- **Convergence ≠ Validity** — case 3 (cycle) and case 7 (tombstone) are exactly the places
  where convergence alone is not enough; the derivation layer must repair.

## Measurement baseline (ongoing)

Updated per task; E0.6 consolidates it into the Go/No-Go report. Values on
Darwin x86_64, Rust 1.96, release profile (`opt-level=z`, `lto`, `strip`, `panic=abort`).

| Task | Binary size | Cold start (mean) | Note |
|---|---|---|---|
| **E0.1** (skeleton, no deps) | **277 KB** | **4.6 ms** /200, incl. shell fork | Baseline before CRDT substrates; only `libSystem` as a dep, clippy `-D warnings` clean |
| **E0.2** (cr-sqlite via rusqlite) | **277 KB** (misleading) | 5.9 ms | `version` binary unchanged, because LTO strips unused SQLite. The real engine size is only measurable once the CLI actually uses cr-sqlite (E2). |
| **E0.3** (yrs + rusqlite linked) | **512 KB** binary + 1.1 MB dylib ≈ **1.6 MB** | — | Realistic proxy: the `interop_check` example links *both* substrates. This is the true engine order of magnitude. |
| **E0.9** (automerge + rusqlite) | **1.3 MB**, **single static binary** (no dylib) | — | Automerge is pure Rust → statically linkable, unlike cr-sqlite. |
| **E0.8** (hand-roll + rusqlite) | **1.2 MB**, **single static binary** | — | Only rusqlite, no extra CRDT lib. Smallest footprint. |

> Thresholds: size < ~50 MB, cold start < ~50 ms. Both met by orders of magnitude.

### E0.2 findings (cr-sqlite) — passed, with two caveats

**Validated (all test-first, green):**
- cr-sqlite v0.16.3 loads via `rusqlite` (bundled SQLite, since Apple's system `sqlite3` does not allow `.load`).
- **Case 1 (LWW):** concurrent edits of *different columns* of the same row → both survive (field-level, no lost update).
- **Case 2 (OR-set):** add dependency edge ∥ remove a different one → both propagate.
- **Hard OR-set case:** concurrent add ∥ remove of *the same* edge → **converges** (both replicas identical, no split-brain). Which operand wins is causal-order-dependent — to be pinned in E0.5 if invariant-relevant.
- Changeset transport via `crsql_changes` (SELECT → INSERT), version-robust via column introspection.

**Caveat 1 — maintenance:** v0.16.3 is the *last* release (Jan 2024). The maturity risk from the
spec is real; mitigated by the Automerge fallback (E0.7) and the fact that the CRR semantics live
in stable SQLite.

**Caveat 2 — distribution:** cr-sqlite is a *runtime-loaded* extension (`crsqlite.dylib`, 1.1 MB,
separate). The engine is thus **not a single file** (binary + dylib), unless we statically link
cr-sqlite from source (extra effort). The size budget stays uncritical (< ~3 MB total), but the
"one binary" ideal from E0.1 is not free with cr-sqlite.

### E0.3 findings (Yrs body CRDT) — passed

**Validated (test-first, green; interop separately as a harness):**
- `yrs` 0.21.3 embeds cleanly; together with cr-sqlite only a **512 KB** binary.
- **Case 4 (body merge):** two replicas edit the body offline at different positions
  → they converge, no edit lost.
- **Projection:** the body reads out as plaintext/Markdown (`Body::text()`).
- **Interop yrs ↔ yjs (the crux):** a genuine cross-runtime round trip via Node — Rust
  produces a v1 update, JS reads it losslessly, and vice versa; **multibyte-UTF-8 safe**
  (`Grüße`, `café`). Harness: `cargo run --example interop_check` (needs `interop/node_modules`,
  `npm i` in `interop/`). Meaning: CLI and web share *one* body wire format — the promise
  "the CLI edits the body, even offline, convergent with the web editor" is substantially backed.

**Note:** yrs/yjs index text in **UTF-16 units** — relevant only for *position-based* edits with
non-BMP characters (emoji); round-tripping whole strings is unaffected. Flag this as a detail for
E2 (CLI body editing).

### Substrate decision (revised) — in-house build instead of cr-sqlite as primary

**Context:** cr-sqlite is *dormant*. Last `main` commit June 2024, last release Jan 2024;
the original team (Rocicorp) has moved on to **Zero/Replicache**. MIT-licensed,
community forks exist, but no strong successor for our case (Rust-embeddable,
SQL-queryable CRDT). A dormant project as the **foundation** under a product is the
wrong risk.

**Direction:** away from cr-sqlite as *primary* (dormant). cr-sqlite was the quick *proof that
the approach holds* (E0.2) and remains the **reference implementation**. We decide the primary via
a **bake-off** between two equally weighted, *maintainable* candidates — with data, not by
argument (lesson from cr-sqlite):

| Candidate | Strength | Price |
|---|---|---|
| **In-house build** (relational, E0.8) | SQL-native (ready/blocked directly), we own it | we carry the CRDT correctness burden (subtle: tombstone GC, concurrent edges) |
| **Automerge** (document, E0.9) | actively maintained (Ink & Switch), bulletproof merge correctness, Rust-native | document model → a projection layer is needed to query in SQL |

The in-house build trades *abandonment risk* for *self-borne correctness risk*; Automerge is
strong exactly there. Both against the **same** tracer cases (1, 2, 3, 7). Decisive
measure: for Automerge the projection cost (doc → SQL-queryable), for the in-house build the
correctness/effort burden. **Body stays yrs** (web interop); Automerge would be structure only.

Both sit behind a `SyncSubstrate` trait (`changes_since`/`apply`/`db_version`), to which the
`Replica` abstraction is formalized → interchangeable.

**Consequences:**
- **E0.8** — `SyncSubstrate` trait + hand-rolled in-house build against the tracer cases.
- **E0.9** — Automerge candidate against the same cases + projection into a SQL-queryable form.
- **E0.6** depends on *both* and chooses the primary with data.

### E0.9 findings (Automerge candidate) — passed, with a clear profile

The `SyncSubstrate` trait is in place; the 4 tracer cases are **written once generically** and
run against any candidate that way. Automerge (0.5.12):

- **Cases 1/2/3/7 green** + one projection test (`ready` query over materialized SQLite).
- **Footgun found (case 3 caught it):** concurrent *object creation* at the same key
  (two replicas independently create the `deps` map) → conflict, one edge is lost.
  Mitigation: **flat scalar keys on ROOT** instead of nested maps. Lesson: Automerge's merge
  is robust, but the *modeling* still carries a diligence burden — "zero correctness burden"
  is not quite right.
- **Projection cost (the differentiator):** Automerge is a document, not SQL. For
  `ready`/`blocked` the doc must be **materialized** into SQLite — ~40 lines + a full
  O(keys) rescan after *every* merge. That is exactly what a relational substrate does **not** pay.
- **Distribution advantage:** Automerge is *pure Rust* → statically linked, **a single binary**
  (no dylib like cr-sqlite). 1.3 MB with rusqlite.
- **Maintenance:** actively maintained (Ink & Switch) — the reason it is in the running at all.

### E0.8 findings (in-house build) — passed

Schema A (LWW per column: Lamport version + `site_id` tiebreak; OR-set/tombstone as an LWW boolean;
changeset as a row-wise export/merge on *pure* SQLite, ~250 lines):

- **The same** generic tracer cases 1/2/3/7 — **green on the first try** (cr-sqlite + Automerge as
  a reference for the semantics paid off).
- **ready/blocked SQL-native**: a direct query on `tasks`/`deps`, **no projection** (contrast to
  Automerge). Test `ready_query_is_native` green.
- **Smallest footprint**: 1.2 MB, single static binary, **zero external CRDT dependency**.

### Bake-off result & recommendation (input for E0.6)

| Criterion | **In-house build (E0.8)** | **Automerge (E0.9)** | cr-sqlite (reference) |
|---|---|---|---|
| Tracer 1/2/3/7 | ✅ on the first try | ✅ (case 3 found a model footgun) | ✅ |
| ready/blocked | **SQL-native, free** | projection needed (rescan/merge) | SQL-native |
| Distribution | **single binary 1.2 MB** | single binary 1.3 MB | binary + dylib (dormant) |
| Maintenance | we own ~250 lines | actively maintained | dormant |
| Correctness risk | **we carry the long tail** | carried by the lib | — |

**DECISION (made): in-house build as primary + Automerge as differential-test oracle.**
SQL nativity is the decisive advantage for an engine whose core feature is the `ready`/`blocked`
derivation; on top of that, the smallest footprint and zero abandonment risk. The real price is the
**long tail of CRDT correctness** beyond the 4 tracer cases (causal delivery, incremental
sync in E4, tombstone GC) — covered by the oracle.

**Mitigation (an elegant side effect of the bake-off):** **keep Automerge as a differential-test
oracle** — run the shared `SyncSubstrate` conformance suite against both the in-house build *and*
Automerge; if they ever diverge, we have a bug. This turns Automerge from a *dependency* into a
*correctness net* for our in-house build, without landing in the product binary.

### E0.5 findings (derivation + invariants) — passed, and a data point for the substrate choice

Pure, deterministic derivation over `tasks`/`deps` (no stored state):

- **ready/blocked** (1-hop blocker, cycle-safe) + **tombstone exclusion** (a deleted blocker
  does not block) — green.
- **Cycle detection** via a recursive CTE (bounded depth → terminates even on cyclic
  graphs); **deterministic across converged replicas** (two replicas that converge to the same
  cyclic graph detect the same cycle → they stay in lockstep). Validates
  "Convergence ≠ Validity": cycles *converge* (case 3) **and** are *detected* by the derivation
  (to flag for human/agent).

**Relevance for the substrate decision:** the derivation is inherently **SQL** (joins for
ready, a recursive CTE for cycles). This widens the in-house build's advantage — it computes
*directly*; an Automerge primary would have to project **before every derivation** (ready, blocked,
cycles — all of it).

## Open points (to be clarified going into the spike)

- The concrete cr-sqlite changeset format & how we serialize it as a file.
- Minimal schema: which slots the spike creates at all (as few as possible, but deps +
  status + body + IDs must be in, otherwise the scenario tests nothing).
