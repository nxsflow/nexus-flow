# Streams without lifetime growth — snapshot at P, then truncation below P (6j6v.kg3e)

**Status: decided by the owner on 2026-10-12** (§11). Nothing here is built yet. The relay seq
fix (§4.1, `r3jf`) lands first, as its own PR; then slice 1, then slice 2.
Board: epic `6j6v.kg3e` (direction 9, coordinator `47jy.d28m`). Slice 1 is `6j6v.0cfa` (a reader
that starts from a snapshot), slice 2 is `6j6v.5vxp` (truncation). Related: `6j6v.r3jf` (relay seq
order) and `6j6v.3201` (text CRDT).
Owner deadline: **solved by the end of 2026.**

Every claim about today's code cites `file:line` on `main` at `130c048` (v0.210.0). Anything not
checked against code is marked **[unverified]**. Anything reasoned from code but not observed
running is marked **[inferred]**.

## 0. Summary

- **The cap is the hosted engine's own count, not the relay's.** engine-server counts every op it
  pulls since hydration (§1.1). So the hosted cap stops counting history as soon as the hosted
  replica starts from a state that carries no log and pulls only the tail. **Slice 1 plus one
  engine-server change already meets the deadline's goal.** Slice 2 bounds the relay's storage and
  gives long-offline devices a defined way back. It is not needed for the cap (decision D1).
- **One snapshot format.** The existing `nxs_sync::snapshot` envelope gets a second format: views
  plus a *base* (relay position P, clock floor, residue), with no log below P. The fold table loads
  the same bytes. No third format (§4.2, D3).
- **Published next to the stream, on the relay.** It is published under `/streams/{id}/snapshot`,
  so it is behind the same per-stream authorization as `/ops` and deleted with the stream (§4.4).
- **The views already are a self-contained CRDT state.** Every register carries its
  `(lamport, site)`. Every OR-set add is keyed by its op id, and every remove is a tombstone row. 13
  things still read the op log. §3 lists them, and §4.5 says how each is handled.
- **Arrival order (§2).** The reducers converge. A delayed stream lands at its causal place
  deterministically, which is not divergence. Order does decide the result in seven places. The one
  that hits honest replicas is the DynamoDB relay: a seq becomes visible before a lower one, so
  pullers and the fold can skip an op for good (r3jf, worse than filed). A snapshot at P that misses
  an op ≤ P passes the miss to every replica that starts from it. **Fixing it is a precondition of
  slice 1, and it lands separately and first:** its own small PR, with a reproduction test (owner,
  2026-10-12; §4.1).
- **Truncated ops go to a cold archive** (owner, 2026-10-12, D4): not served, not counted, deleted
  with the stream, and the last resort for a refold (§5.6).

## 1. Problem and constraints

### 1.1 What is capped, measured

- **The cap is in app-foundations' engine-server, not in this repo.** `MAX_STREAM_OPS = 30_000` and
  `MAX_STREAM_BYTES = 32 MiB` are at `app-foundations/crates/engine-server/src/hosted/replica.rs:74,90`.
  `Capped::pull` (`:354-378`) adds each page's op count and wire bytes to a running total and refuses
  the page that would cross either cap. Over the cap, the engine answers `413 {code:"io"}`.
- **A snapshot does not lower the count today.** The count starts from the snapshot's
  `header.ops` and unpacked bytes (`replica.rs:609-616`), because today's snapshot carries the whole
  log (`crates/foundation/src/image.rs:1-10`, `crates/sync/src/snapshot.rs:70-100`).
- **The block does not use snapshots at all.** bb-hosted-engine does not set `NXS_ENGINE_SNAPSHOTS`
  (`app-foundations/packages/bb-hosted-engine/src/index.cdk.ts:185-197`), so it hydrates by full
  replay on every cold start.
- Measured at the cap (engine-server `AGENTS.md:412-464`):
  - 29,500 ops / 28 MiB are served in 11.1–12.1 s;
  - 35,000 ops are refused after about 13 s;
  - the pass budget is 18 s.
- r6ef staging numbers for manufakt's real board (about 5,860 distinct ops):
  - full hydration: 3.2 s, 25 pages, 6.5 MB;
  - from a with-log snapshot: 1.56 MB gzipped, 0.33–0.40 s import.

### 1.2 Nothing shortens a stream

- The substrate refuses to delete an op. Append-only triggers forbid it
  (`crates/foundation/src/schema.rs:474-503`), and E1 §9 says so too.
- A chunk supersede only adds rows (`crates/core/src/task_reducer.rs:264-299`).
- The relay has no delete, compaction or epoch path. Its routes are `ops` POST/GET, `register` and
  `machines` (`crates/server/src/app.rs:89-98`).
- The only stream deletion is manufakt-io's account deletion
  (`manufakt-io/apps/web/src/backend/loeschauftrag/loeschung.ts`), which removes every row.

### 1.3 Constraints carried into the design

- **E4 decision 4 stands.** One relay position is the whole watermark, with no version vector
  (`docs/specs/E4-sync-protocol-server.md` §2.4 and §5.1, owner 2026-10-06). A snapshot is only
  taken of what the relay already holds.
- **The relay stays a dumb carrier** (E4 §2.1). It stores and serves snapshot bytes. It never folds
  and never builds one.
- **Convergence does not depend on trust; actions do** (E4-auth §2.4). A snapshot replaces ops, so
  for the first time trust decides STATE. That is the new question in §4.3.
- **Compatibility (`41j0.bgwn`, open, no base version in force).** A snapshot format becomes part
  of the compatibility promise.
- **app-foundations is paused.** What it must change is a hand-over (§7), not a dependency this
  repo waits on.

## 2. Arrival order — where it decides the result today (measured)

The owner's second weakness: a machine that delivers late should not change the outcome. Two
different effects hide in that sentence. They have to be kept apart.

**(a) A late op lands at its causal place.** Ordering is by `(lamport, site)` everywhere. That
covers LWW registers (`crates/foundation/src/change.rs:246-272`), notes
(`crates/core/src/store.rs:1546-1550`), chunks (`:1522-1527`), chat messages
(`crates/chat/src/schema.rs:837,849`) and chat's turn watermark (`crates/chat/src/store.rs:375-377`).

So an edit made offline yesterday loses to an edit made this morning, on every replica, in every
arrival order. A message written yesterday appears among yesterday's messages. This is
deterministic. It is what Yjs does too: it orders by causality, not by arrival. **It is not
divergence.**

What breaks is a READER that remembers a position in arrival order, such as "read until here".
manufakt.io already replaced such a marker by a set of seen messages (note on `47jy.d28m`,
2026-09-21). In this repo the position readers are:

- `pushed_through` (local rowid, own site): correct.
- `next_page_cache` (local rowid plus the op at it): correct, machine-local.
- `pulled_through` and fold-ddb's `folded_through` (relay seq): correct only if relay seq order is
  visibility order. That is finding 1.

The chat message reducer's "first write wins by ARRIVAL", recorded on `47jy.d28m` on 2026-09-23,
**is fixed**. Messages and thread roots are ranked lowest-wins registers since vvw6
(`crates/chat/src/message_reducer.rs:54-98,182-198`).

**(b) Places where two replicas holding the same ops end up different, or miss ops:**

| # | Where | What decides | Two-op example | Who hits it | Snapshot at P + tail ≠ full replay? |
|---|---|---|---|---|---|
| 1 | DynamoDB relay, before #59 (`r3jf`): the seq was allocated by an `UpdateItem ADD` on the counter item before the op was written by its own transaction (`allocate_seq` and `try_append_op` in `crates/server/src/store_ddb.rs` at `130c048`). **Fixed by #59, see §4.1.** `read_since` returns `seq > since` and sets `next` to the last seq seen. The client stores that cursor unconditionally (`crates/sync/src/engine.rs:486`). fold-ddb advances to the batch's max seq (`crates/fold-ddb/src/fold.rs:336-338`), and `advance` never moves back (`:343`). | Commit order vs. seq order of concurrent appends | Writer A gets seq 10, writer B gets 11, and B commits first. A reader at `since=9` gets [11] and stores cursor 11. Then A@10 commits. That reader never asks below 11 again, so A is **missing for good** on it. A replica that pulled later has A. | **Honest replicas**, whenever two pushes to one stream overlap: a service and a manual pass, two devices, the hosted writer and a device. SQLite and Postgres relays are immune: a mutex (`crates/server/src/store.rs:206-212`) and an advisory lock held to commit (`crates/server/src/store_pg.rs:334,352-356`). | **Yes, and it propagates.** A snapshot at P from a replica that skipped op 10 claims P and omits 10. Every replica started from it misses 10 too. The anchor check only compares the op AT P (`crates/sync/src/snapshot.rs:422-440`). r3jf rated live reading harmless. That holds for the fold, which folds a late op when it arrives through the DynamoDB stream, but **not for pullers**. **[reproduced: #59's test `a_reader_is_never_handed_a_seq_above_an_append_still_in_flight`, backported onto the `ADD` allocation, fails with the second pull empty]** |
| 2 | Rows holding several registers whose version columns default to 0: `items` (`crates/core/src/schema.rs:69-73`), and chat `channels`, `profiles`, `threads`. The fresh-row INSERT writes unconditionally (`change.rs:246-272`; DynamoDB `crates/fold-ddb/src/write.rs:147-156`). `lamport_in_bound` bounds only the top (`crates/foundation/src/model.rs:100-102`). | Which op creates the row | A sets `title=x` at (−1, 5). B sets `status=open` at (5, 2). In order AB the row is created with `title=x`. In order BA, B creates the row with `title_v=0`, and A needs (−1,5) > (0,0), so it is refused: `title` stays NULL. | Hostile or malformed ops only: honest clocks start at 1 (`crates/foundation/src/store.rs:226-231`). | Yes: the snapshot freezes its source's arrival order. A local full refold folds in `(lamport, site)` order (`store.rs:573-578`), so it can disagree even with the replica's own incremental fold. |
| 3 | Two different ops with the same `(lamport, site)`. Locally the unique coordinate index makes the second one `Collided` and it is never stored (`store.rs:499-531`). The server folds both, and an equal version keeps the first (`fold.rs:38-44`). | Which copy arrives first | A and B are both at (7,3) and both set `title`. Order AB keeps A, order BA keeps B. | Logs written before fc5p only: four pairs in this board's own log (`hehx` point 4). | Yes: every snapshot lineage inherits its source's choice. |
| 4 | Two copies of one `op_id`: `INSERT OR IGNORE` keeps the first copy's content, signature and verdict (`store.rs:496-531`). | Which copy arrives first | A stripped copy arrives first and is stored `unsigned`. The signed copy arrives later and is ignored. `acting_ops` then differs between replicas (`crates/foundation/src/trust.rs:123-127`), and so do chat's `held` and `answers` (`crates/chat/src/store.rs:344-363,417-438`). | A stripping or hostile transport. | State: no (same content). Actions: yes. A snapshot carries its source's `key_id`/`sig`; the importer re-verifies them but has no other copy (`crates/foundation/src/image.rs:468-470`). |
| 5 | Server fold, row size: whichever op makes a row pass 400 KB is refused (`fold.rs:185-195,245-275`). | Relay order | A and B each add about 250 KB to one item. Order AB folds A and refuses B. Order BA folds B and refuses A. Local replicas hold both. | Server only, oversized rows. | Yes for a fold-table snapshot. No for a replica snapshot. |
| 6 | Legacy `belongs_to` migration rewrites the log from what has arrived so far (`crates/core/src/store.rs:716-777`). | When the migration ran | x (lamport 5) is migrated into a parent edge. y (lamport 7, from an old binary) arrives after the migration and is never migrated, so parent = x. A replica that held both before migrating rewrites y, so parent = y. | Pre-v4 stores only. **[inferred]** | Not for snapshots: an import is schema v8 and never runs this migration. |
| 7 | Ops that arrived before an upgrade made their shape foldable stay unfolded unless the fold revision goes up. The watermark moves past stored-not-folded ops (`store.rs:545-550,902-920`). Only `bind_views` rebuilds, and only on a revision change (`store.rs:798-829`). | When the op arrived relative to the upgrade | Op u of a new shape arrives before the upgrade: it stays unfolded. Op u′ of the same shape arrives after: it folds. | Latent: a release that forgets the revision bump. **[inferred]** | Yes, unless the snapshot carries the residue (§4.5). |

**Not order, but listed because the owner named them:**

- **Text is whole-value LWW.** `CrdtText` falls back to LWW (`crates/core/src/task_reducer.rs:72-81`),
  as do custom fields (`:102-120`) and memory bodies (`crates/memory/src/fact_reducer.rs:140-157`).
  Of two concurrent edits, one whole string is lost, the same one on every replica. That is a merge
  quality problem, `3201`/`eqeh`. It does not affect snapshot equivalence: a register is
  self-contained. nexflow.it already carries text as Yjs updates in chunks.
- **History misses `set:crdt-text` ops.** `item_history` filters `op_type='set'`
  (`crates/core/src/history.rs:28-30`).

**Scope.**

- Finding 1 is fixed IMMEDIATELY, before slice 1, as its own PR with a reproduction test (owner,
  2026-10-12; §4.1).
- Finding 2 is a one-line refusal of `lamport < 1` in `folder_for`
  (`crates/foundation/src/reducer.rs:81-92`). It changes which ops fold, so it needs a fold revision,
  and every stream refolds, locally and on the server. Recommended: ride the next revision bump.
  Not a slice-1 blocker.
- Finding 3 is scoped as "pre-fc5p logs". A snapshot can be checked for it: `schema.rs:2400`
  counts duplicate coordinates.
- Findings 4–7 are scoped and named, not solved here.
- The owner's DoD line "proven on the running system" needs the r3jf fix deployed to manufakt's
  relay. That relay is pinned to an old nexus-flow (v0.35.0 per the 2026-09-21 note, **[unverified]**
  today).

## 3. What in today's state is NOT self-contained (measured)

**Self-contained already:**

- Each `items` cell is `(value, f_v, f_site)` (`crates/core/src/schema.rs:69-73`).
- `custom_fields` and `item_timestamps` carry their own versions.
- The OR-sets are add rows keyed by the add's op id: `edge_adds` and `thread_link_adds`, both with
  `(lamport, site)`, and `label_adds` without (`crates/core/src/task_reducer.rs:156-217`). Each has
  grow-only remove rows keyed by tag (`:175-185`).
- `notes` and `note_tombstones`; `chunks` and `chunk_superseded` (`:219-310`).
- Chat: `messages` with `(claim, lamport, site)`, `threads.root_*`, LWW channel, profile and thread
  fields, and `membership_*` (`crates/chat/src/message_reducer.rs`).
- Memory: `memories` with `_v`/`_site` pairs and `created_*` (`crates/memory/src/fact_reducer.rs:105-169`).

Every effect is commutative and idempotent: Ensure, Put, Register keep-if-beats
(`crates/foundation/src/change.rs:10-22`). So **a snapshot row is itself a described change.**
Applying a view row with its stored version is a join, which is what §5.3 relies on.

**What still reads the op log:**

| # | What | Where | Effect on a replica without the ops ≤ P |
|---|---|---|---|
| L1 | Lamport clock | `high_water_mark` reads `MAX(ops.lamport)` (`crates/foundation/src/store.rs:1211-1219`); seeded at open, refreshed per write (`:256-261,414-415`) | **Correctness.** A fresh replica's clock would be 0. Its first `title` write ticks to 1 and loses to the snapshot's `title_v = 900`. The clock floor cannot be derived from the views: tombstones and `label_adds` carry no lamport. It must be carried. |
| L2 | Dedup by `op_id` | `ops` primary key; `INSERT OR IGNORE` (`store.rs:499-531`) | A re-delivered op ≤ P is stored and folded again. The state is unchanged because effects are idempotent, but the log gains a row. In slice 1 a based replica never pulls ≤ P (§4.5). |
| L3 | Coordinate-collision check | unique `ops_lamport_site` (`store.rs:521-531`) | A collision with an op ≤ P goes unseen. Pre-fc5p only (finding 3). |
| L4 | Refold on a fold-revision or schema change | `bind_views`, `force_refold`, `fold_whole_log` (`store.rs:747-760,798-829,1163-1169`) | **Cannot run.** The replica must start from a snapshot folded by its own revisions (§4.5, D8). |
| L5 | Stored-not-folded ops (unknown shapes, oversized chunks, lamport > `MAX_LAMPORT`, `emit_unfolded`) | `folder_for` returns None, giving `Ingest::Deferred` (`store.rs:538-555`; `reducer.rs:28-31`) | Lost unless carried. The snapshot carries them verbatim as the **residue** (§4.2). |
| L6 | Memory migration mark: a register with no view | written unfolded (`crates/memory/src/store.rs:185-200`), read off `ops` (`:211-235`), used at `crates/memory/src/migration.rs:1480` | Covered by the residue, because it is stored-not-folded. |
| L7 | Item history | `crates/core/src/history.rs:25-45` reads `ops`; only a test calls it (`crates/core/src/store.rs:4175`) | History before P is empty on a based replica. After truncation the archive still holds it (§5.6, D4); a history read for ops ≤ P′ is an archive read, not built in these slices. |
| L8 | Chat: whether an op acts (provenance) | `acting_ops` view over `ops.provenance` (`crates/foundation/src/trust.rs:123-127`); chat joins it by `(lamport, site)` (`crates/chat/src/store.rs:344-350`); `held_sql` reads `open` ops (`:417-438`) | **Fail-closed but wrong.** Every message ≤ P would act nowhere, so every thread opened ≤ P is held. The E4-auth rule "a verdict is never taken from outside" forbids carrying verdicts (§4.5). |
| L9 | Border-thread copy | `crates/chat/src/border.rs:537-547` selects op ids from `ops` | Same domain as L8. |
| L10 | Prefix remap | `crates/core/src/store.rs:566-626` rewrites every op carrying the old prefix, then refolds | Needs the log. It runs on the empty store before an import (E4-continuous §10, CLI order), so a based replica never needs it for ops ≤ P. |
| L11 | Legacy `belongs_to` migration | `crates/core/src/store.rs:716-777` | Never runs on a schema-v8 import. |
| L12 | Relay downgrade detection | `downgrades` (`crates/sync/src/engine.rs:637-677`) asks `ops` about earlier signatures | Weaker for ops ≤ P (nothing to compare against). Diagnostic only. |
| L13 | Push watermark / "unpushed" | `local_ops_since`: rowid > `pushed_through` AND own site (`crates/sync/src/engine.rs:714-725`) | A fresh replica has no own ops ≤ P. Unaffected. |

**Not affected:** `remove_edge` and `remove_label` read the live add tags from the views, not the
log (`crates/core/src/store.rs:785-806,1022-1043`). The SQL views `present_edges`, `present_labels`,
`present_parent` and `present_thread_links` read only view tables (`crates/core/src/schema.rs:171-178,240-272`).

**Chat and memory share the stream.** One `db.sqlite` holds all three products
(`crates/foundation/src/workspace.rs:112-113`). The push filters by site, not domain
(`engine.rs:719-724`). A stream's snapshot therefore covers three reducers. Today's
`nxs sync snapshot` opens a flow-only store (`crates/core/src/store.rs:123,144`), and chat and
memory refold from the log it carries. Without a log they cannot, so a based snapshot must carry
every platform reducer's views. fold-ddb's `platform_reducers()` already does
(`crates/fold-ddb/src/fold.rs:72-78`).

## 4. Slice 1 — a reader that starts from a snapshot at P (`0cfa`)

### 4.1 Precondition: relay seq order equals visibility order (r3jf) — lands separately

**Owner, 2026-10-12: fixed immediately, as its own small PR with a reproduction test, before slice
1.** Slice 1 assumes it is merged and released; this section records what that PR (#59) built, so
the spec stays readable on its own. The reproduction test came first: two appends on
dynamodb-local, the first one held by a test seam after it has chosen its seq and before it sends
its write, the second one committing meanwhile, and a reader polling between them — which on the
`ADD` allocation stores a cursor past the missing seq.

A snapshot at P must contain every op with seq ≤ P. The DynamoDB relay now assigns the seq inside
the write, as the other two backends do.

- **Fix.** Each append round knows a counter value `c` — read with one strongly consistent `Query`
  of the counter item for the first op of a push, otherwise learned without a read (handed back by
  a failed counter check through `ReturnValuesOnConditionCheckFailure = ALL_OLD`, or carried from
  the op the same push wrote before). Its `TransactWriteItems` then carries up to four items:
  - `Update counter SET next_seq = :seq` (`s = c + 1`) with the condition `next_seq = :expected`
    (`c`; for `c = 0`, also "no counter yet");
  - the claim (as before);
  - for `s > 1`, a `ConditionCheck attribute_exists` on the op at `s − 1` — the **predecessor
    check**;
  - the op, conditioned on its key being free.

  Concurrent appends to a stream serialize on the counter item; a loser retries from the counter
  the failed check handed back, after a jitter of at most 16 ms.
- **Why the predecessor check (review of #59, Integrity #1).** AWS documents transactions as
  serializable against each other, but a `Query` only as read-committed against a transaction: a
  transaction's items need not become visible to a reader at the same instant. The counter
  condition alone orders the transactions, not what a reader sees — the transaction that took `c`
  can show its counter before its op, the next one can write op `c + 1` and commit, and a reader in
  that window is handed `[c + 1]`. The predecessor check makes op `c`'s item part of the next
  transaction, so that transaction conflicts (`TransactionConflict`, retried as a lost round) while
  op `c` is still being written and can commit only once it exists. Commit visibility then follows
  seq order — transaction against transaction, the case AWS documents — as with Postgres' advisory
  lock.
- **Legacy gaps.** When op `s − 1` is legitimately absent (a seq an `ADD`-era relay burned), a failed
  `attribute_exists` with the counter condition holding proves it: no transaction of this build can
  still write that seq. The next round, for the same `s` only, checks `attribute_not_exists`
  instead — still an item of the transaction, so a writer of it still conflicts — and steps over
  the gap. One extra round per legacy gap; the stream never wedges. Seq 1 has no check; a floor
  step (below) checks the occupied seq it steps over.
- **Occupied seqs above the counter** (a stream older than the counter, out-of-band writes) are
  found with one reverse `Query` (`Limit 1`), and the next round takes the seq above the top one —
  one extra round for a run of any length, not one per seq. Seqs past `i64::MAX` are a loud error.
- **IAM — changed.** The predecessor check needs **`dynamodb:ConditionCheckItem`** on the op table.
  It must be granted **before** a relay of this build is deployed; without it every push past a
  stream's first op fails with `AccessDeniedException` (a 500). The op table's role now needs
  `Query`, `PutItem`, `UpdateItem` and `ConditionCheckItem`. Wire protocol and table shape are
  unchanged.
- **Cost.** Uncontended, one transaction per op plus one counter read per push. A transaction is
  billed at twice the plain write per item; the predecessor check is a transactional read of op
  `s − 1`. Under contention a lost round is a cancelled transaction, and cancelled transactions
  consume capacity like committed ones, so `N` writers racing for one stream cost up to about `N`
  times the capacity per stored op; two transactions can also cancel each other, so there is not
  always a winner per round. The per-stream ceiling is about one append per counter round-trip —
  30-60 appends/s by latency arithmetic. **[unverified]:** throughput and capacity under contention
  on real AWS; not measurable on dynamodb-local, which serialises transactions (chore filed with
  #59). Bounds: 256 rounds lost in a row (`CONTENTION_ROUNDS`), 32 rounds failing otherwise
  (`APPEND_ATTEMPTS`), and a per-push deadline of 20 s (`PUSH_DEADLINE`, configurable as
  `NXF_RELAY_PUSH_DEADLINE_SECS`), below the ~30 s of API Gateway and CloudFront in front of the
  hosted relay; checked between ops, between rounds and against the round in flight, answered 503,
  which the client re-pushes like any failed push.
- **Density.** A stream written by this build has no gaps: a seq is taken only together with its
  op, so neither a re-push, a lost round nor an SDK retry consumes one. Streams written before
  carry the `ADD`'s gaps. Nothing downstream depends on density.
- **What is rejected.** A "visibility horizon" (serving only seqs older than N seconds): a time
  bound is a guess about commit latency, not an ordering, and the old streams' burned seqs make a
  hole indistinguishable from a gap anyway. Also rejected (review of #59): bounding `read_since`
  by the counter and stopping a page at an unexpected gap — it would put the ordering into every
  reader instead of the one writer path, and legacy gaps would stall it.
- **Rolling upgrade.** Grant `ConditionCheckItem` first. Mixed with an `ADD`-era relay nothing is
  overwritten and the counter never moves backwards; the ordering guarantee holds once every relay
  writing a table runs the new build. Meanwhile the old relays answer more pushes with a 500 (their
  `ADD` is cancelled by a `TransactionConflict` whenever a new relay's transaction holds the
  counter).
- **What it does not repair.** Ops already skipped by today's replicas stay skipped. The writer
  check in §4.3 (count and digest ≤ P) detects such a replica before it publishes.

### 4.2 The snapshot object: one format

The `nxs_sync::snapshot` envelope (`crates/sync/src/snapshot.rs:70-100`) gains **format 2**. It
stays gzipped JSON with a header and an image. Format 1 stays readable.

```
Header {
  format: 2,
  stream_id, pulled_through: P, anchor: <op_id at P>,     // as format 1
  engine, schema_version, min_compatible,                  // as format 1
  revisions: { task: 2, message: 1, fact: 0 },             // per-reducer fold revisions (y3r4)
  base: {                                                  // NEW — absent = format-1 semantics
    clock: <high_water_mark at export>,                    // L1
    ops_through_p: <count of distinct op ids the writer holds>,  // §4.3 check
    digest_through_p: <hash over the sorted op ids ≤ P>,   // §4.3 check
    residue: <count>,                                      // L5/L6
    kept_domains: ["message"],                             // §4.5, L8
  },
  writer: { key_id: "ed25519:…" }, sig: "<over the canonical header + body hash>"
}
Image {
  views: every platform reducer's view tables (task, message, fact), rows as today,
  ops: ONLY  (a) the residue: stored-not-folded ops ≤ P, verbatim with key_id/sig
             (b) every op of a kept domain ≤ P (chat, §4.5)
}
```

- **Relation to fold-ddb's snapshot.** `nxs_fold_ddb::snapshot::Snapshot`
  (`crates/fold-ddb/src/snapshot.rs:43-52`) is tables plus `folded_through` plus revisions, which
  is a subset of the above. It has no production caller in this repo; tests call it. Its readers
  and writers become a projection of format 2:
  - `import` reads the views and P from the format-2 body;
  - `export` and `from_sqlite` write format 2.

  The format-1 struct stays readable for one release. Then there is ONE format (D3).
- **Relation to r6ef's S3 snapshot.** It IS the `nxs_sync::snapshot` format 1, with the whole log
  (app-foundations `crates/engine-server/src/hosted/snapshot.rs:16-17,43`). Format 2 replaces it in
  place: the same key, the same loader, the new header.
- **Views per reducer revision, not per engine version.** Format 1 takes views only when
  `ENGINE_VERSION` matches, and otherwise refolds from the carried log (`image.rs:26-43`;
  `store.rs:1022-1043`). A based image has no log to refold. It is taken iff every reducer's
  revision and the schema's compatibility floor match. Otherwise the importer refuses with
  `NeedsRevision` and the caller falls back (§4.5).
- **Size.** The views of a board, not its history, except for hidden payloads:
  - superseded chunks keep their rows and bodies (`chunk_superseded` only hides them);
  - removed edges and labels keep their add rows;
  - redacted notes keep their bodies.

  Slice 1 elides those payloads and keeps every tombstone (owner, 2026-10-12, D10). That is safe:
  a re-delivered append or add is hidden by the tombstone it meets. It is the one lever that bounds
  a snapshot by live content for a chunk-heavy stream like nexflow.it's. The data stays on the
  relay's log, and after truncation in the archive (§5.6). **[unverified]:** the size of a views-only image of
  manufakt's board. The with-log image is 1.56 MB gzipped; slice 1 measures the views-only one.
- **Bounds.** `MAX_UNPACKED_BYTES` stays 256 MiB (`crates/sync/src/snapshot.rs:76`). A relay
  refuses an upload over its own limit (§4.4).

### 4.3 Who writes it, how often, and integrity

- **Writers.** Any replica that is caught up and holds a signing key:
  - the local background service, for local-first streams;
  - the hosted engine, for hosted streams. A web-only nexflow.it user has no other replica.

  The relay never builds one.
- **When.**
  - The replica has no unpushed ops (today's refusal, `crates/sync/src/snapshot.rs:233-240`).
  - The anchor at P matches the relay (`:242-253`).
  - The relay's last snapshot is K ops behind P. K is 5,000, with at most one snapshot per stream
    per hour (decided, D7).
  - The writer's `ops_through_p` and `digest_through_p` equal the relay's (below). Otherwise it
    refuses as `Incomplete`, which is the r3jf victim detector.
- **The relay's half of the check.** `GET /streams/{id}/digest?through=P` returns the count and an
  XOR-of-hashes over the op ids with seq ≤ P. It is a scan of the stream's op ids: no fold, no
  interpretation. The relay may cache it per P. It is the only new relay computation.
- **Signing.**
  - The writer signs the header, which carries a hash of the body, with its replica key. That is
    the E4-auth Ed25519 key (`crates/foundation/src/signing.rs`) under a new domain tag
    `nxs-snapshot/1`, so a snapshot signature can never be read as an op signature.
  - The ops carried in the residue keep their own signatures and are re-verified at import, as
    today.
  - The views cannot be verified against anything. Whom a reader believes is decision D2.
- **Finding the latest.** `GET /streams/{id}/snapshot` returns the latest bytes, or 404.
  `HEAD` (or the header alone) is enough to decide whether it is newer than what a reader has.

### 4.4 On the relay

| Route | Body / answer | Rules |
|---|---|---|
| `PUT /streams/{id}/snapshot` | format-2 bytes | Accepted iff the header's `anchor` is the op the relay holds at seq P (so a stream that was deleted and recreated, or a forged position, is refused), and P > the stored P for the same reducer revision set (a conditional put: the latest wins per revision set, older uploads answer 409; D8). Size limit 32 MiB gzipped **[unverified: right number]**. 413 above it. |
| `GET /streams/{id}/snapshot?revisions=…` | latest bytes for that revision set (default: the newest P of any set), or 404 | `ETag` = P |
| `GET /streams/{id}/digest?through=P` | `{count, digest}` | §4.3 |

- **Storage.** A new optional `SnapshotStore` capability of the registry, like `PresenceStore`
  (`crates/server/src/presence.rs`). A relay without it answers 404, and clients read that as
  "no snapshot here", exactly as for presence (E4 §4.5).
  - SQLite and Postgres keep a blob row per stream.
  - DynamoDB keeps a pointer item `snapshot#<revision set>` in the registry table, and the bytes in S3 under
    `<stream>/<P>.nxs-snapshot`. An item is limited to 400 KB.
- **Authorization (requirement R4).** This repo's relay authenticates nobody (`crates/server/src/main.rs:2`).
  All three routes live under `/streams/{id}/`, so manufakt's authorizer
  (`manufakt-io/apps/web/src/backend/relay-authorizer.ts`) applies its per-person stream ownership
  (`iss`+`sub`) to them unchanged.
  - Its route map must put `GET …/snapshot` and `GET …/digest` under `read` (read-only engine
    tokens included), and `PUT …/snapshot` under `write`.
  - There is no snapshot route without a stream id, and no shared-key-only path beyond the one the
    authorizer already has for every route.
- **Deletion (requirement R1).** The snapshot lives under the stream's key, so deleting the stream
  deletes it: blob row, pointer item, S3 prefix.
  - The upload rule (anchor at P must be held) means a cached snapshot cannot resurrect a deleted
    stream: there is no op at P any more.
  - Re-pushing old ops into a deleted stream recreates it from ops, not from a snapshot. That is the
    known limit of the shared key (loeschung.ts) and manufakt's stream tombstone (`dqem.09af`), not
    this design's.

### 4.5 The import: "state without the ops before P"

**Local (SQLite).** `Store::load_image` (`crates/foundation/src/store.rs:995`) accepts a based
image into an empty store:

1. **Check before writing anything.** Revisions and schema floor (else `NeedsRevision`), signature
   and trust (D2, else `Untrusted`), the header against the body, and cell types. These are today's
   checks (`image.rs`) plus the new ones.
2. **Write the views as they are.** Then the residue and kept-domain ops into `ops`, re-verified as
   today.
3. **Record the base** in a new substrate table `log_base(relay_position, clock, residue_count)`.
   It is machine-local and not a view.
4. **Clock.** `high_water_mark` becomes `MAX(today's, log_base.clock)` (L1).
5. **Refuse the paths that need the whole log on a based store**, with one error `NeedsHistory`:
   - `refold`, `force_refold` and `bind_views` when a revision changed (L4);
   - `remap_prefix` when an op ≤ P carries the prefix (L10).

   The caller's answer is a re-bootstrap from a snapshot of its own revisions.
6. **Fold the residue for newly understood shapes.** That is additive, because effects are joins
   (§3). It answers L5/L7 without a full refold. A revision that changes what an ALREADY folded op
   produces still needs `NeedsHistory` (D8).
7. **Watermarks.** `pulled_through = P`, `pushed_through` = the store's max rowid. An anchor
   mismatch on a based import does **not** "pull from 0" as format 1 does
   (`crates/sync/src/snapshot.rs:411-413`). It answers `AnchorMismatch`, and the caller falls back
   (slice 1: a full replay into a fresh store).

**Kept domain: chat (L8, L9).** The message reducer's views are self-contained for state, but
chat's ACTIONS read per-op provenance, and a verdict is never taken from outside (E4-auth §2.4). A
based snapshot therefore keeps every `message`-domain op ≤ P in its log. Chat stays an ordinary
replica, and only task and fact drop their log.

The cost is that chat ops still count against the tail cap. **[unverified]:** the chat share of
manufakt's or nexflow.it's streams. A later, separate step can bound chat by carrying the
`(lamport, site, key_id, sig)` of the ops chat's action reads reference, re-verified at import (D9).

**Hosted.** engine-server already calls `nxs_sync::snapshot::import`
(`app-foundations/crates/engine-server/src/hosted/replica.rs:550-656`), so a format-2 snapshot loads
through the same call. What changes there is §7.

The fold table starts from the same bytes. `nxs_fold_ddb::snapshot::import` keeps its sentinel
discipline (`crates/fold-ddb/src/snapshot.rs:239-256`) and rebuilds the index from the rows
(`board::reindex`, `:250`). So an index revision never needs history; only a reducer revision does.

**Fallback chain in slice 1.** Every refusal ends in a full replay from the relay, which still holds
everything: relay snapshot → (hosted only) S3 → full replay. Slice 2 removes the last link for
streams it has truncated (§5.2).

### 4.6 The hosted cap after slice 1 (requirement R6)

With §7 applied, engine-server's `Capped` starts at:

- `header.base.residue` + the kept-domain ops (not `header.ops`);
- the snapshot's unpacked bytes.

It then counts the tail as today.

- **Ops: at most 30,000 in the tail since the snapshot**, not over the lifetime. With K = 5,000
  and writers keeping up, a hosted stream would need 25,000 un-snapshotted ops to hit the cap.
- **Bytes: snapshot (unpacked) + tail ≤ 32 MiB.** The snapshot's size is the board's live state:
  hidden payloads are elided (D10).
- **Unchanged by slice 2.** Truncation's archive is never pulled during hydration (§5.6).

The cap numbers themselves are the engine's to keep or change. The spec proposes keeping them: they
are sized against the 18 s pass budget, and a based replica stays far below both.

## 5. Slice 2 — truncation below P (`5vxp`), at the level needed to decide

### 5.1 What the relay does

- The relay first copies the op items with seq ≤ P′ into the stream's cold archive (§5.6, D4),
  where P′ ≤ the latest snapshot's P. Only then does it delete them from the live stream and record
  `base = P′` per stream (counter item attribute or registry row).
- **The tm4k claim items stay** (D6), as does the counter at seq 0.
- `GET ops?since=s` with s < base answers **`410 Gone {base, snapshot: true}`** instead of a page.
  A client of today reads 410 as an error and fails the pass loudly. It does not silently diverge.
- **What is counted afterwards** (requirement R6):
  - **The hosted cap** counts the snapshot's unpacked bytes, its residue and kept chat ops, and the
    tail since the snapshot, against 30,000 ops / 32 MiB (§4.6). The archive is never pulled
    during hydration, so it counts for nothing.
  - **Live relay storage** holds the retention window W plus the tail, the latest snapshot, and
    the claims. Claims grow with the lifetime op count, about 60 bytes an op **[unverified]**, and
    are never served.
  - **Lifetime growth moves into the cold archive.** It is not served, not counted, and deleted
    with the stream (§5.6).

### 5.2 Devices offline since before P′ (requirements R5, `5vxp` open question a)

A pass pushes before it pulls (`crates/sync/src/engine.rs:407-541`), and the push does not depend
on the pull cursor. So:

1. **Unpushed ops are pushed first, as today.** They get new seqs above the base. Their old
   lamports place them causally (§2a): they lose to later edits and win over earlier ones, on every
   replica. Nothing is lost, and nothing is reordered by arrival.
2. **The pull then answers 410.** The client fetches the latest snapshot and **merges it into its
   existing store**: each view row is applied as the described change it is (§3), with its stored
   version. That is the CRDT join of two states, so it commutes with everything the device already
   folded.
   - A new reducer method, `row_changes(table, row)`, turns a row back into its changes. It is
     derived from the same column lists the reducer already declares.
   - The residue and kept-domain ops are applied as ops.
   - Then `pulled_through = P` and the tail is pulled.
3. **The device's own log keeps its old ops.** It becomes a based store with `log_base` = P. Its
   own ops ≤ P are all on the relay already, because they were pushed before.
4. **Fallback:** a fresh store from the snapshot, then re-push the device's own ops that were never
   pushed. This is simpler, but it loses the device's machine-local tables. Not recommended.

The one test that must hold (§8): offline device with unpushed ops → truncation past its cursor →
return. Final state equals the reference fold, every op id once on the relay, nothing lost.

### 5.3 Dedup of re-pushed old op ids (`5vxp` open question b)

Who re-pushes ops ≤ P′:

- a partial push that died before its watermark moved;
- `bind --rebind`, which resets both watermarks and re-pushes everything
  (`docs/specs/E4-continuous-sync-client.md` §4.4);
- a store restored from backup;
- ops stored before tm4k, which are stored once more on their first re-push
  (`crates/server/tests/dynamodb_local.rs:338-371`).

| Option | How | Trade-off |
|---|---|---|
| **A. Keep the claims (decided, D6)** | DynamoDB keeps its claim items at negative seq when the op items go. SQLite/Postgres keep `(stream_id, op_id, seq)` with the body dropped. A re-push answers the old seq and stores nothing. | Exact, server-side, already built for DynamoDB. About 60 bytes a claim **[unverified]**, never served, never counted by the cap. Grows with lifetime op count: a million ops ≈ 60 MB of claims per stream. |
| B. Op-id set in the snapshot | Exact set of op ids ≤ P in the snapshot; clients drop known ids. | 26+ bytes per op ever written, in every snapshot, so the snapshot grows with history again. The relay still stores the duplicate and the cap still counts it. A Bloom filter would drop genuine ops on a false positive: unacceptable. |
| C. Per-site frontier (a version vector) | Header carries `max lamport per site ≤ P`. An op with lamport ≤ F[site] is "known". | Small, and it is the owner's Yjs picture ("version state per source"). **But it is only correct if each site's ops reach the relay in lamport order.** That holds for honest pushes (rowid order, own site, `engine.rs:714-725`). A foreign op squatting a site with a high lamport makes F jump, and genuine lower ops of that site would be dropped as known: **data loss from one hostile op.** Pre-fc5p duplicate coordinates break it too. Also against E4 decision 4. |

A re-pushed op that is NOT deduped changes no state, because effects are idempotent. It costs a log
row and a cap count. The choice is about cost and cleanliness, not correctness.

### 5.4 Who triggers, retention window (`5vxp` open questions c and d)

- **Decided (D7): the relay truncates on its own schedule**, never past:
  - the latest snapshot's P;
  - ops younger than the retention window W (proposed **30 days**, matching presence's
    forget-after, E4 §4.5, decided);
  - the DynamoDB stream's 24 h retention (so board-fold has seen every op it deletes).

  The relay checks positions only and still never folds.
- **The alternative** is the snapshot writer calling `POST /streams/{id}/truncate {through}`. That
  puts a destructive call in every writer's hands. Rejected as the default; kept as an operator
  route behind the stream's `write` authorization (D7). Either way the archive is written first
  (§5.6).
- **W is a trade:**
  - a device offline less than W catches up from the tail (cheap, no merge);
  - one offline longer takes the §5.2 path;
  - a longer W keeps more ops on the relay.

### 5.5 Truncation vs. the fold path (requirement R3)

- **board-fold reads INSERT records of the DynamoDB stream.** It skips REMOVE records
  (`app-foundations/crates/board-fold/src/batch.rs:13,97-98`) and claim and counter items
  (`record.rs:160-165`). Truncation's deletes are REMOVE records, so a live fold is unaffected.
- **A fold that starts fresh** cannot backfill from position 0 below the base:
  - a new partition build (`9mj5`);
  - a refold after a reducer revision.

  It starts from the relay snapshot (`nxs_fold_ddb::snapshot::import` of format 2), then folds from
  P. Today's backfill from 0 (gnbg, the stream keeps only 24 h) becomes "snapshot, then from P".
- **An index revision (fold-ddb index revision 3) never needs history.** `import` rebuilds the
  index from the rows (`crates/fold-ddb/src/snapshot.rs:179-190,250`).
- **A reducer revision needs a snapshot folded by the new reducers.** So the release order is:
  1. a writer on the new revision publishes;
  2. then the fold runs switch (D8). If no writer can, the archive rebuild of §5.6 is the last
     resort.
- **The claim items with negative seq and the counter at seq 0 stay**, as above. Readers already
  skip seq ≤ 0 (`store_ddb.rs:65-72`; board-fold `record.rs:160-165`).

### 5.6 The cold archive (owner, 2026-10-12, D4)

Truncated ops are kept, not destroyed. This keeps the signed audit log (E4-auth §2.2), the data
superseded chunks still hold, item history, and the last-resort refold of D8. The archive keeps
them **off every path a sync pass, a hydration or the cap sees.**

- **Who writes it.** The relay itself, as a step of truncation. Copying op items it already stores
  is custody, not folding, so the relay stays a dumb carrier (E4 §2.1).
- **Format.** One segment per truncation step: `(from_seq, to_seq]` as gzipped JSON lines. Each line
  is one `WireOp`, verbatim, with its seq, its `extra`, and its `key_id`/`sig`. A segment header
  carries `{stream_id, from_seq, to_seq, count, digest}`, where the digest is the same op-id digest
  as §4.3. Segments are append-only and never rewritten.
- **Order of writes, crash-safe.** Steps 1–3 can be redone: a rerun rewrites the same segment and
  finds nothing more to delete.
  1. Write the segment.
  2. Re-read its count and digest and compare them with the relay's digest of the same range.
  3. Record it in the stream's archive index (`archive#<to_seq>`).
  4. Delete the op items. The claims stay (D6).
  5. Move `base`.

  A crash before step 4 leaves the live stream whole. A crash after step 4 but before step 5 leaves
  the base behind the deletions, so a pull below the actual items still returns what remains, and
  the next run finishes.
- **Storage.**
  - SQLite and Postgres: one blob row per segment, keyed `(stream_id, to_seq)`.
  - DynamoDB: S3 objects `<stream>/archive/<to_seq>.nxs-archive` in the snapshot bucket, next to
    `<stream>/<P>.nxs-snapshot` (§4.4). The storage class is the deployer's choice.
  - Archive storage is an optional capability, like `SnapshotStore`. **A relay without it never
    truncates.** Truncating without an archive would contradict D4.
- **Not served.** Neither `GET …/ops` nor the snapshot routes ever return archived ops. One
  separate route, `GET /streams/{id}/archive` (the index) and `GET /streams/{id}/archive/{to_seq}`
  (one segment), exists for the two readers below. It sits under the stream's `read`
  authorization, but manufakt's authorizer maps it to the owner's own tokens only, never to
  delegated read-only engine tokens (hand-over §7).
- **Not counted.** No hydration reads it. engine-server never calls the archive route, and its
  `Capped` sees only snapshot and tail.
- **Who reads it.** Two readers, both offline jobs, never on a request path:
  - **D8's last resort.** A replica on a new reducer revision with no snapshot of its revision
    runs `nxs sync rebuild-snapshot`. It folds the archive segments in seq order into a fresh store,
    then the live tail, then publishes a format-2 snapshot at the new revision.
  - **A future history or audit reader.** Not built in these slices (L7).
- **Deletion (requirement R1).** The archive lives under the stream's key: blob rows keyed by
  stream, the S3 prefix `<stream>/archive/` inside `<stream>/`.
  - Deleting a stream deletes the whole prefix `<stream>/`: snapshots and archive in one go, plus
    the archive index and claim items. manufakt.io's account deletion must cover the prefix
    (hand-over §7).
  - For the backends in this repo, slice 2 adds `purge_stream(stream)` to the store traits. It
    deletes ops, claims, counter, snapshot and archive, for tests and self-hosted relays.
  - **No resurrection.** An archive is never re-imported into a live stream. A segment can only
    feed a rebuild into a fresh store, whose snapshot upload then needs the anchor op at P on the
    live relay (§4.4). A deleted stream has none.

## 6. Requirements from manufakt.io (note on `kg3e`, 2026-10-11) — where each is met

| # | Requirement | Where / how | Open? |
|---|---|---|---|
| R1 | Deletion reaches snapshots; no resurrection | §4.4: the snapshot lives under the stream key, and an upload needs the anchor op at P on the relay. §5.6: the cold archive lives under the same key (`<stream>/archive/`), is deleted with the stream's whole prefix, and is never re-imported into a live stream. Truncated state is snapshot, claims and archive, all under the stream key. Hand-over: loeschung.ts deletes `snapshot#*`, `archive#*`, the S3 prefix `<stream>/` and the claim items. It already deletes every ops-table row. | Decided (D4, D5) |
| R2 | One format, not two | §4.2: format 2 of the `nxs_sync::snapshot` envelope. fold-ddb's struct becomes a projection of it, and r6ef's S3 snapshot is the same envelope (format 1 today). Compatibility: D8. The archive is not a snapshot: it holds ops in their wire form (§5.6). | Decided (D3) |
| R3 | Truncation vs. the fold path, the index revision, claims and counter | §5.5 | Decided (D8) |
| R4 | Snapshot reads go through per-person authorization | §4.4: the routes live under `/streams/{id}/`, read vs. write mapping in the authorizer. No stream-less route. The archive route (§5.6) goes to the owner's own tokens only. | Hand-over to manufakt-io |
| R5 | Long-offline devices get a defined path, measured together with arrival order | §5.2 (410 → merge snapshot → tail; unpushed ops pushed first and placed causally) and §2 (finding 1 is the one honest-replica order bug; its fix lands first, separately) | Decided (D6, D7) |
| R6 | The cap after the change | §4.6 and §5.1: 30,000 ops / 32 MiB counted over the tail since the snapshot, plus the snapshot's unpacked bytes, plus residue and kept chat ops. The archive counts for nothing. Lifetime growth remains only in the claims (live, never served) and the cold archive. | Decided (D1, D4, D10) |

## 7. Hand-over: what app-foundations' engine-server must change (it is paused)

### 7.0 The format-2 contract, as BUILT (6j6v.0cfa, slice 1's smallest cut)

**Status: built and merged in nexus-flow.** This subsection is the normative one for
app-foundations — §7.1 onwards is the original plan, and where the two differ, this one is what the
code does. Everything here is `cargo doc`-visible on the named items.

**The seam is still `nxs_sync::snapshot`.** Two functions, one envelope, and format 1 stays
readable by every build:

```rust
// Take one (the local service, or the hosted engine for a hosted stream):
nxs_sync::snapshot::export_based(
    &store, local_site, &stream, &marks, &transport,
    nxs_sync::snapshot::PLATFORM,          // the ONE product list; see below
) -> Result<Vec<u8>, SnapshotError>

// Start a replica from one — UNCHANGED signature, so `start_from_snapshot` keeps its call:
nxs_sync::snapshot::import(&mut store, &bytes, &stream, &transport) -> Result<Imported, SnapshotError>

// Size one without loading it (what `Capped` needs):
nxs_sync::snapshot::peek(&bytes) -> Result<Header, SnapshotError>

// The envelope opened, for the board fold rather than a replica:
nxs_sync::snapshot::unpack_image(&bytes) -> Result<(Header, nxs_foundation::image::Image), SnapshotError>
nxs_fold_ddb::snapshot::from_image(&image, stream_id) -> Result<Snapshot, FromImageError>
```

**Three things the hosted engine MUST do that it does not do today.**

1. **Register every platform reducer on the store before importing.** One database holds all three
   products, and a based snapshot carries all three products' views with no log to fold them from.
   A store that registers only flow's task reducer is refused by name
   (`SnapshotError::NeedsRevision { ours: None, domain: "message" }`) rather than silently dropping
   chat's and memory's state. The two lines:

   ```rust
   store.register_reducer(Box::new(nexus_chat::message_reducer::MessageReducer));
   store.register_reducer(Box::new(nexus_memory::fact_reducer::FactReducer));
   ```

   Only for a based snapshot. On the format-1 path they would change the store's summed fold
   revision and make every format-1 import refold what it could have taken.

2. **Count `header.base`, not `header.ops`.** `Capped` starts from what the FILE carries — the
   residue plus the kept domains, which is `header.ops` of a format-2 file — and the snapshot's
   unpacked bytes. `header.base.residue` says how many of those ops no reducer could fold, and
   `header.base.ops_through_p` how many ops the writer held below the position (what was NOT
   carried). The budget then counts the tail as today. Update `AGENTS.md` "Limits".

3. **Branch the fallback chain on these names.** Each one means "this snapshot cannot start this
   replica, nothing was written, try the next source":

   | `SnapshotError` variant | what it means |
   |---|---|
   | `NeedsRevision { of, domain, snapshot, ours }` | a product's views were folded by other rules than this build's (`ours: Some`), or this store registers no reducer for a product the snapshot carries (`ours: None`). A based snapshot's views are TAKEN or refused — there is no log to fold instead. |
   | `MissingViews { domain }` | the snapshot carries no views for a product THIS build folds. |
   | `AnchorMismatch { position }` | the relay does not hold the op the snapshot names at its position. A format-1 snapshot answers this by pulling from 0; a based one cannot, so it refuses. |
   | `FormatTooNew { found, supported }` | a newer nxs wrote it. |
   | `Corrupt(String)` | the header and the body disagree, or the bytes are not a snapshot. |
   | `Unpushed`, `Unbacked`, `NotFolded` | export-side refusals, for the publish step. |

   `Untrusted` and `Incomplete` are NOT in this cut: the trust list (D2) and the relay's
   count-and-digest route (§4.3) are out of its scope. The digest IS carried
   (`header.base.digest_through_p`, `ops_through_p`), so the check can be added without a format
   change.

**What the format carries, and what it does not.** `nxs_sync::snapshot::Header` gains
`revisions: BTreeMap<String, i64>` (per-domain fold revision) and `base: Option<Base>`;
`Base` is `{ clock, ops_through_p, digest_through_p, residue, kept_domains }`. The body's
`nxs_foundation::image::Image` gains `base: Option<Base>` with
`{ relay_position, clock, residue, kept_domains, view_bases }`, and the two halves are checked
against each other at import. `base: None` is format-1 semantics in both. Hidden-payload elision
(D10) is NOT in this cut (owner, 2026-10-14): a based snapshot still carries superseded chunk
bodies, removed add rows and redacted note bodies. That is the one size lever still unpulled, and
it is tracked separately.

**The three substrate consequences, which an embedder must know about:**

- **The clock travels in the snapshot.** `log_base.clock` is a term of `high_water_mark`, so a
  based replica's first write mints above the state it imported. Without it every first write would
  lose, silently (§3, L1).
- **A based database carries its OWN compatibility floor** (`schema::BASED_MIN_COMPATIBLE = 9`,
  stamped into that file's `application_id` by the import). A binary that cannot read `log_base`
  refuses the file instead of minting against a clock of zero. The GLOBAL floor stays at 4, so no
  ordinary workspace is affected.
- **Rebuilding a based store's views is refused, by name.** `refold_if_behind` and `force_refold`
  now return `Result<(), nxs_foundation::image::NeedsHistory>`, and the three product `open`s
  surface it as a handled sqlite error (as the skew gate does). A rebuild's first act is to EMPTY
  the views it would re-derive from the whole log; a based store has no such log, so the honest
  answer is a re-bootstrap from a snapshot at this build's revisions. `remap_prefix` is the other
  path, and it is why a replica registers its id prefix BEFORE importing, not after.

### 7.1 The rest of the hand-over (plan, unchanged)

**engine-server** (`crates/engine-server`):

1. **Pin** the nexus-flow release that ships format 2 (today it pins `v0.209.0`,
   app-foundations `Cargo.toml:277-329`). The board-fold pin brings index revision 3 at the same
   time, which is already pending (5aej/9yt8 notes).
2. **Load from the relay first.** In `start_from_snapshot` (`hosted/replica.rs:550-656`), fetch
   `GET {relay}/streams/{id}/snapshot` with the person's token (the same `x-nxs-reader` /
   delegated-token path the ops pull uses). Then fall back to `NXS_ENGINE_SNAPSHOTS` (S3), then to a
   full replay.
   - Handle the new refusals by name, each falling through to the next source: `NeedsRevision`,
     `Untrusted`, `AnchorMismatch`, `Incomplete`.
3. **Count only what came in without a snapshot.** In `Capped` (`replica.rs:609-616`), start from
   `header.base.residue` + the kept-domain op count, not `header.ops`. Bytes stay the unpacked size.
   Update `AGENTS.md` "Limits" (`:412-464`) to say what is counted.
4. **Publish snapshots to the relay.** After a pass that folded something (`replica.rs:872-878`,
   `writeback.rs`), when no write is unpushed and the relay's snapshot is K ops behind, `export`
   format 2 and `PUT` it. The S3 write-back writes the same bytes (one format) or is retired (D3).
5. **A stable signing key for the hosted engine.** Today each hydrated write replica gets its own
   site (k6e7), and a key per throwaway replica cannot be trusted by anyone. Snapshots must be
   signed by a key that survives cold starts: a secret, loaded at start, `NXS_ENGINE_SIGNING_KEY`
   **[proposed name]**.
   - Plus the list of keys whose snapshots the engine accepts (`NXS_ENGINE_SNAPSHOT_TRUST`
     **[proposed name]**, D2).
6. **Slice 2 only:**
   - treat `410 Gone` on a pull as "re-hydrate from the latest snapshot";
   - never fall back to a full replay for a stream whose base is above 0.

**board-fold** (`crates/board-fold`):

7. **Start a build from the relay snapshot** (`nxs_fold_ddb::snapshot::import` of format 2), then
   fold from P, instead of backfilling from 0 (`backfill.rs`). Needed by slice 2. Optional in
   slice 1, where it shortens a backfill.
8. Nothing changes for REMOVE, claim or counter records: they are already skipped.

**bb-hosted-engine** (CDK):

9. Set the snapshot source, so the block stops hydrating by full replay
   (`packages/bb-hosted-engine/src/index.cdk.ts:185-197` sets no `NXS_ENGINE_SNAPSHOTS`).
10. Give the relay in the same image (`nxf-relay`) the snapshot store: the S3 bucket and prefix,
    IAM, and a lifecycle rule for superseded snapshot objects.
11. Slice 2: the same bucket holds the cold archive under `<stream>/archive/` (§5.6). No lifecycle
    expiry on that prefix: the archive is deleted with the stream, not by age. The storage class
    for it is the deployer's choice.

**Not app-foundations, but needed** (manufakt-io, listed so it can be forwarded):

- the authorizer's route map for the three snapshot routes (§4.4);
- the archive routes (§5.6) mapped to the stream owner's own tokens only, never to delegated
  read-only engine tokens;
- loeschung.ts deleting the snapshot and archive pointers (`snapshot#*`, `archive#*`), the whole S3
  prefix `<stream>/` (snapshots and archive), and the claim items (R1);
- the manufakt relay's nexus-flow pin moving past the r3jf fix (§4.1). That fix ships in its own
  release, before slice 1, so the pin can move ahead of everything else here.

## 8. Tests

**Slice 1 acceptance (`0cfa`):**

- **Differential, the core test.** Generated streams of more than 30,000 ops on one relay:
  - **Ops:** task items with every LWW field and retypes; edges and labels with removes, including
    removes before adds; thread links; notes and redactions; custom fields; chunks with supersedes,
    including supersedes before appends; chat messages, threads and memberships; memory facts and
    forgets; a few stored-not-folded ops (unknown shape, oversized chunk, lamport > `MAX_LAMPORT`).
  - **Writers:** three or more sites.
  - **Snapshots:** several positions P, published by one replica.
  - **The two paths:**
    - Path 1: a fresh replica imports the snapshot and pulls only the tail. It must request no
      position ≤ P (asserted on the test relay).
    - Path 2: a fresh replica pulls everything.
  - **What must be equal:** every view table as a row set, `next`, `list`, `show`, chunks, a chat
    thread view, memory recall, and every op id in `ops` above P.
  - **Shuffled arrival** where it is legal: the tail is delivered in shuffled page order to a third
    replica, which must match too.
  - **Where:** `crates/sync/tests/`, beside today's "snapshot plus the rest" test
    (`crates/sync/src/snapshot.rs:753-839`).
- **Hosted equivalence.** The same snapshot imported into fold-ddb (`mem` table and dynamodb-local),
  then the tail folded, equals folding everything. Same harness as `fold_differential`.
- **The cap.** A test transport counting like engine-server's `Capped`, over a 35,000-op stream with
  a snapshot at 33,000. It is served within 30,000 counted ops. A replay of the same stream is
  refused.
- **r3jf** is tested in its own PR, before slice 1 (§4.1). Slice 1 re-runs the snapshot
  differential on the DynamoDB relay under concurrent pushes, with that fix in place.
- **Hidden payloads (D10).** A snapshot omits superseded chunk bodies, removed edge and label add
  rows, and redacted note bodies, but keeps every tombstone. A tail that re-delivers such an
  append or add leaves the based replica equal to the full replay.
- **Clock (L1).** A based replica's first write to a field the snapshot holds at `_v = 900` wins.
- **Residue (L5/L6).** An unknown-shape op ≤ P survives the import and folds once the reducer
  learns the shape. The memory migration mark reads the same before and after.
- **Chat (L8).** A thread opened ≤ P by a trusted key is not held on the based replica.
- **Refusals.** Each one falls back without writing: revision mismatch, untrusted signer, anchor
  mismatch, an `Incomplete` writer (a replica that skipped a seq), a format newer than the build,
  a body that disagrees with its header. And `NeedsHistory` on refold and remap of a based store.
- **Relay routes.** All three backends: latest wins, an older P answers 409, a missing anchor is
  refused, 404 without the capability. Deleting the stream deletes the snapshot.

**Slice 2 acceptance (`5vxp`):**

- **Offline then return.** Device A holds unpushed ops and its cursor is at 1,000. Device B writes
  40,000 ops, a snapshot is published, and the relay truncates to 39,000. A returns:
  - push, then 410, then merge snapshot, then tail;
  - final state equals the reference fold of all ops;
  - every op id is on the relay exactly once (claims), A's ops are present, and nothing is
    duplicated in A's log.
  - **Variants:** A's unpushed ops older (lower lamport) than ops ≤ P′; A having never pulled at
    all; A rebinding.
- **Re-push.** Re-pushing every op of a truncated stream stores nothing and returns the old seqs.
- **Written after truncation.** A stream that received more than 30,000 ops in total serves and
  accepts writes from a fresh hosted replica.
- **Fold.** board-fold starts a build from the snapshot after truncation, and it equals the serving
  build.
- **Archive.**
  - After truncation, the archive segments hold exactly the truncated ops: the same count and
    digest, verbatim signatures.
  - No pull and no hydration touches them: the test relay asserts that the archive route is never
    called during a pass.
  - A truncation that dies between any two steps of §5.6 and runs again ends in the same live
    stream and archive.
  - `nxs sync rebuild-snapshot` from archive plus tail equals the full fold.
  - Purging the stream removes ops, claims, counter, snapshot and archive.
  - A relay without archive storage refuses to truncate.

## 9. What slice 1 changes in code (for sizing, not a plan)

- `crates/server` (the seq fix of §4.1 is NOT here: it lands first, in its own PR):
  - the `SnapshotStore` capability for all three backends;
  - the three routes.
- `crates/sync`:
  - snapshot format 2: header, base, signature, the writer check against `digest`, hidden payloads
    elided (D10);
  - import outcomes;
  - the engine's pass gets a "publish if due" step, opt-in.
- `crates/foundation`:
  - `Image` with a base;
  - `log_base` table;
  - clock floor;
  - `NeedsHistory` on refold and remap;
  - residue fold;
  - the `nxs-snapshot/1` signing tag.
- `crates/fold-ddb`: `snapshot` reads and writes format 2.
- `crates/nxs`: `nxs sync snapshot --publish` and `nxs sync bind --from-relay`. The service
  publishes when due.

## 10. Cuts

**Before slice 1, its own PR (owner, 2026-10-12):** the r3jf fix with its reproduction test (§4.1),
released on its own, so manufakt's relay pin can take it first.

**Slice 1 (`0cfa`) builds:**

- format 2 with the base, residue, chat kept, signature and digest check, and hidden payloads
  elided (§4.2–4.3, D10);
- relay snapshot routes and storage on all three backends (§4.4);
- the based import, local and fold-ddb (§4.5);
- the publish step in the local service;
- the hand-over (§7) and the tests (§8, slice 1).

**Slice 2 (`5vxp`) builds:**

- relay truncation with base and 410, claims kept (§5.1, §5.3);
- the cold archive: segments, index, archive routes, `purge_stream`, `nxs sync rebuild-snapshot`
  (§5.6, D4);
- the scheduled trigger with window W (§5.4);
- the snapshot merge into an existing store (§5.2, `row_changes`);
- the slice-2 tests.

**Deferred:**

- finding 2's `lamport < 1` refusal (next fold revision);
- findings 4–7;
- the text CRDT (`3201`);
- bounding chat's kept log (D9);
- a history or audit reader over the archive (L7);
- a per-call read budget (`wq9e`);
- relay authentication in this repo (`ax54`).


## 11. Decisions (owner, 2026-10-12)

The draft put ten decisions to the owner, each with options and a recommendation. On 2026-10-12 the
owner accepted nine as recommended and chose the cold archive for D4. Separately, the owner ruled
that the relay seq bug is fixed immediately, in its own PR (§4.1). Each decision below keeps the
options and the reasons, as the derived-ranking spec does in its §10.

0. **The relay seq bug (§2 item 1, `r3jf`) is fixed immediately**, as its own small PR with a
   reproduction test, before slice 1. Slice 1 names it as a precondition that lands separately
   (§4.1, §10). It also stands on its own: it can make a puller miss an op for good on today's
   DynamoDB relay, snapshot or not.

1. **D1 — The deadline is met by slice 1 plus the engine-server hand-over; slice 2 follows.** The
   hosted cap then counts only the tail since the last snapshot (§4.6). Relay storage keeps growing
   until slice 2.
   - *Declined:* both slices by the deadline. That puts destructive relay work under deadline
     pressure, and the cap does not need it.
   - *Named:* the cap is only reached through §7, so app-foundations must take the hand-over before
     the deadline.

2. **D2 — A reader believes only snapshots signed by a key on its own trust list**, otherwise it
   falls back to a full replay. That means the hosted engine's stable key, and the user's own
   devices. `nxs sync bind` offers to trust the key that signed the latest snapshot after showing
   it, as `trust list` does for op signers.
   - *Declined:* trusting any snapshot the relay accepted from an authorized writer. That would let
     the relay operator, or anyone with the stream's write token, forge a whole board.
   - *Declined:* a split rule, strict for local replicas and lenient for the hosted engine.
   - *Reason:* E4-auth's rule that trust is local and explicit, now that a remote party decides
     state for the first time.

3. **D3 — The one format is `nxs_sync::snapshot` format 2.** fold-ddb's tables-plus-watermark
   becomes a projection of it, and r6ef's S3 objects become format 2.
   - *Compatibility promise:* every build reads every older format, and format 1 stays readable at
     least through the bgwn base window.
   - *Declined:* fold-ddb's struct as the format. It has no anchor, no engine or schema version,
     and no production caller.
   - *Declined:* keeping both formats, which requirement R2 forbids.

4. **D4 — Truncated ops go to a cold archive** (§5.6). It is not served, not counted against the
   cap, deleted with the stream, and the last resort for D8. It keeps the signed audit log (E4-auth
   §2.2), item history and the data superseded chunks hold.
   - *Declined:* dropping truncated ops for good. That is simplest, but it ends the audit log and
     history.
   - *Declined:* never truncating. That keeps everything at the cost of unbounded relay storage.
   - *Cost, named:* a second store per stream, and a second thing deletion must reach (R1, covered
     in §5.6 and §7).

5. **D5 — Snapshots live on the relay** under `/streams/{id}/snapshot` (§4.4). This reverses
   E4-continuous §10 ("not on the relay"). That decision rested on a relay that authenticated
   nobody and a snapshot nobody could check. Signing (D2), the anchor rule, and manufakt's
   per-person authorizer now answer both.
   - *Declined:* snapshots only in S3 next to the hosted engine. Local replicas and slice 2 need
     them on the relay, and a second store per product would have to be deleted and authorized
     separately.

6. **D6 — Re-pushed op ids below P′ are deduplicated by keeping the claims** (§5.3). DynamoDB keeps
   the tm4k claim items; SQLite and Postgres keep slim `(stream_id, op_id, seq)` rows.
   - *Declined:* an op-id set in the snapshot. It grows with history, and the relay would still
     store and count duplicates.
   - *Declined:* a per-site frontier. It is small and matches the Yjs picture, but one hostile op
     squatting a site could make genuine ops read as known: data loss.
   - *Cost, named:* claims grow with the lifetime op count, about 60 bytes an op **[unverified]**.
     They are never served or counted.

7. **D7 — Cadence, trigger and window.**
   - A snapshot every K = 5,000 ops, at most hourly, written by the local service and the hosted
     engine.
   - The relay truncates on its own schedule, never past the latest snapshot, never ops younger
     than W = 30 days, never younger than the DynamoDB stream's 24 h.
   - Writer-triggered truncation is kept only as an operator route.
   - *Declined:* K = 1,000 or 10,000, W = 7 or 90 days, and writer-triggered truncation as the
     default.
   - *To revisit if* manufakt.io or nexflow.it expect laptops to stay away longer than 30 days.

8. **D8 — Fold-revision compatibility.**
   - The relay keeps the latest snapshot per reducer revision set.
   - The release order is "a writer on the new revision publishes, then readers and fold runs
     switch". The release runbook gains that step.
   - The archive rebuild (`nxs sync rebuild-snapshot`, §5.6) is the last resort.
   - *Declined:* requiring every rewriting revision to ship a view migration. Not always possible:
     fold revision 1 needed op coordinates no view held.

9. **D9 — Chat keeps its log in a based snapshot.** Every `message`-domain op ≤ P rides in the
   snapshot, so chat's actions keep reading their own verdicts (E4-auth: a verdict is never taken
   from outside).
   - *Declined for now:* carrying `(lamport, site, key_id, sig)` of the referenced ops. That would
     be a second provenance mechanism.
   - *To revisit when* a chat-heavy stream approaches the cap.

10. **D10 — Hidden payloads are elided from snapshots, from slice 1 on.** That covers superseded
    chunk bodies, removed edge and label add rows, and redacted note bodies. Tombstones are kept,
    so convergence is unaffected (§4.2).
    - *Reason:* it bounds a snapshot by live content, which matters most for nexflow.it's
      chunk-carried text. Redaction then actually stops a note's body reaching new devices.
    - *Declined:* carrying the payloads, which lets the snapshot grow with edit history.
    - The data itself stays on the relay's log, and after truncation in the archive.
