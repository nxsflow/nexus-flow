# Runbook: purging a stream from the relay (`nxf-relay purge`)

`nxf-relay purge` removes everything the relay holds for ONE stream — on SQLite, Postgres and
DynamoDB — with a dry run that shows what would go, a delete bound to that dry run, and a confirm
run that proves nothing is left (6j6v.f6yb; the tool's reference is the module doc of
`crates/server/src/purge.rs`). It is the relay half of an account deletion; `nxf-fold-purge` is the
board-fold half (6j6v.dbmd, below). The account sequence below is **consumer contract v2.1** — ONE
contract for both purges, superseding f6yb's "v1.4" and dbmd's "v2" (it adds the authorizer's
fail-closed clause, scoped ids only, the canary per environment and entry point, the deny table by
ARN and its permanence): foundations (kwv2) runs it for nexflow.it; a self-hosted operator runs the
same steps by hand.

**What "deleted" means is the whole sequence, not one exit 0.** A single `--yes` proves the
logical delete of the plan it was bound to; the confirm runs, the fence and the spacing below are
what make it final.

## The three invocations

```bash
# 1. Dry run: prints the plan and its digest; deletes nothing.
nxf-relay purge <stream-id> --dry-run --expect-identity <pinned identity digest>
# 2. Delete, bound to that dry run's plan_digest.
nxf-relay purge <stream-id> --yes --plan <plan_digest> --expect-identity <pinned identity digest>
# 3. Confirm: exit 0 only when nothing of the stream is left (a DynamoDB stream lease included).
nxf-relay purge <stream-id> --dry-run --expect-empty --expect-identity <pinned identity digest>
```

Configuration comes from the environment only and **nothing has a default**:

| Backend | Required | Optional |
|---|---|---|
| all | `NXF_RELAY_BACKEND` (`sqlite`, `postgres`, `dynamodb`) | `NXS_LOG_HMAC_KEY` (log-id v1; without it the report's `sid` is `-`) |
| sqlite | `NXF_RELAY_DB`, `NXF_RELAY_REGISTRY_DB` (both, even when they are the same file) | |
| postgres | `NXF_RELAY_PG_URL` (`DATABASE_URL` is NOT read) | `NXF_RELAY_PG_CA_FILE` |
| dynamodb | `NXF_RELAY_DDB_TABLE`, `NXF_RELAY_DDB_REGISTRY_TABLE`, `NXF_RELAY_DDB_REGION` | `NXF_RELAY_DDB_ENDPOINT` |

The released `nxf-relay` carries SQLite and Postgres. DynamoDB is a source build with
`--features dynamodb` (foundations run it from their own image).

### The board-fold purge (`nxf-fold-purge`, 6j6v.dbmd)

The same three invocations against the board-fold table (no `--keys`; the tool's reference is the
"Purge" and "Deny table" sections of the crate doc of `crates/fold-ddb`). It is in no release
tarball: build it from the SAME `nxs-fold-ddb` pin as the deployed board-fold,
`cargo build -p nxs-fold-ddb --bin nxf-fold-purge --features dynamodb`, and check the dry run's
`tool_version` and `covers.index_revision` (an older build refuses a newer layout with 78,
`UnknownKind`, rather than deleting less).

| Required | Optional |
|---|---|
| `NXF_FOLD_DDB_TABLE`, `NXF_FOLD_DDB_REGION`, `NXS_DENY_TABLE` | `NXF_FOLD_DDB_ENDPOINT`, `NXS_LOG_HMAC_KEY` |

It reads the deny table once a run and reports what it read as `fence` (`denied`, `not_denied`,
`unscoped`) and the table as `deny.table`/`deny.table_arn`. **`--yes` and the confirm exit 78
(`NotFenced`) unless `fence` is `denied`**; the dry run is allowed before the fence (step A).
Without `NXS_DENY_TABLE`, or with a table that is not a deny table, every mode exits 78
(`NotADenyTable`). Its exit codes are the relay's table above without 2 and 5.

## Exit codes, and what to do

| Code | Meaning | Next |
|---|---|---|
| 0 | dry run printed / confirm found nothing / delete complete | go on |
| 1 | a store error stopped the run; `deleted` says what went | a fresh dry run, then a bound `--yes` (after a crash on DynamoDB, the lease recovery below first) |
| 2 | DynamoDB: the stream lease is held by another holder, or this run lost it | wait 15 min, then a fresh dry run |
| 3 | delete: something was written after the plan (residue); confirm: something is still there | the fence leaks — stop and find the writer |
| 4 | the plan is not the one `--plan` names; nothing deleted | a fresh dry run ONCE; a second 4 for the same id is an alarm: stop |
| 5 | SQLite: deleted, but a WAL checkpoint stayed busy (`physical: "pending"`) | later: a fresh dry run (its plan is empty) and a bound `--yes`, which checkpoints again |
| 64 | usage, or the stream id is refused (`refused.rule`) | see "A refused id" |
| 78 | configuration: a variable missing/malformed, not a relay store, `IdentityMismatch`, `EmptyStore`, `UnknownKind` | stop; read `error` |

## The identity pin

Every report carries `store.identity` and its SHA-256, `store.identity_digest`. Take the digest
from a first dry run against the store you mean, record it for the environment, and pass it as
`--expect-identity` on every later run. It refuses any other store (78, `IdentityMismatch`), and it
is what lets the confirm after a store's LAST stream pass: without it, a store that holds nothing
is refused as `EmptyStore` (78), because "nothing found" against the wrong store would look like
success.

- **SQLite**: canonical path and the `relay_store_identity.store_id` of each file. A file deleted
  and re-created at the same path is another store; a byte copy restored in place is the same.
- **Postgres**: `current_database()`, `current_schema()` and `store_id` — host and port are in the
  report as information only, so a failover or an endpoint rename keeps the pin (MF-10). **Who
  re-issues a pin:** the operator, after a dry run that shows the same `store_id` it had before
  (a changed `store_id` is a different database, not a moved one — stop).
- **DynamoDB**: `TableArn`, `TableId`, `CreationDateTime` of both tables. A re-created table is
  another store.

**DynamoDB `ItemCount` lags** (MF-6): `store.populated` reads the ops table's `ItemCount`, which
AWS refreshes about every six hours. A table created minutes ago reads `populated: false`, and a
table emptied by this purge still reads its old count. Both are expected; the pinned
`--expect-identity` is the answer to both — never act on `populated` alone.

## Start the current relay once

The purge creates nothing. A store written by a relay older than this release has no
`relay_store_identity` row, and one older than presence (6j6v.f0b5) has no `machine_presence` table:
both are `NotARelayStore` (78) naming what is missing. Start the CURRENT relay once against that
store (its own open adds the table and writes the row), stop it, then purge.

## The account sequence (consumer contract v2.1)

**Day 0** is the day the deletion order is given. "Stop" below means: nothing further for this
account, the fence stays where it is, escalate to the owner.

**A. Read-only sanity, BEFORE any fence.** Nothing is written or fenced in this step.

- **A1.** Enumerate the account's stream ids S: the relay ops table (`seq = 0` items,
  `begins_with(stream_id, "n-<h>-")`, plus `u-<h>` exactly), the relay registry table (the same),
  and the board-fold table's partition keys (the same).
- **A2.** For each id, a relay dry run with `--expect-identity`. Stop on any exit other than 0, on
  `populated: false`, or on a `tool_version`/`covers` below the minimum (the build's `tool_version`
  at least the deployed relay's, `covers` including every kind the deployed relay writes). An
  account whose `u-<h>` holds nothing is a valid state, not a stop.
- **Scoped ids only.** The fence can name only `u-<h>` and ids beginning `n-<h>-`. A product that
  promises deletion through this contract issues no other stream id, enforced where ids are issued
  (nexflow.it's `authorizeStream` does), and "deleted" is defined for scoped ids only. A stack that
  runs board-fold with the deny table folds scoped ids only: board-fold writes nothing for an id the
  list cannot name (owner decision 2026-10-23). Which `<h>` an id belongs to is pinned by the vector
  file `crates/fold-ddb/contract/deny-account-vectors.json`; foundations run it against the
  authorizer, and the hand-over says it passed.
- **A3. The ownership check** (positive and mandatory). (i) The subject `(issuer, sub)` comes from
  the deletion-order record that POST /deletion created from the authenticated request — never
  typed in by an operator. (ii) Recompute `<h>` = the first 32 lowercase hex characters of
  SHA-256(`<issuer>:<sub>`) in your own code; where the order stores an `<h>` or a `subjectHash`,
  both must be equal to it — no normalisation of the issuer (a trailing `/` is a mismatch). (iii)
  Every id A1 found must parse as exactly `u-<h>` or `n-<h>-<k>` with `<k>` matching `^[0-9]+$`.
  Any failure — an enumerated id outside the scoped forms included — stop.
- **Re-entry.** A2's stop conditions apply on the FIRST entry for an id only. On every later entry
  (after exit 1, 2, 4 or 5, or step 8's re-run) compare the dry run with the previous report for
  that id instead: the same `identity_digest`, no `found` larger than before, and — when the
  previous `--yes` reported `deleted > 0` — an empty plan is expected, not a stop.
  Example after an exit 5: step 1 (two dry runs, equal digests, empty plans) → step 2 (`--yes`,
  empty plan, the checkpoint) → step 4.

**0. Fence, permanent and account-scoped.**

- **The deny table is permanent.** It is provisioned with deletion protection, a retain policy on
  stack removal and point-in-time recovery, and the stack documents its restore: an emptied,
  replaced or restored-too-early table silently unfences every deleted account, and a table of the
  right shape passes every check however empty. So steps 5, 6 and 8 each read the deny item back.
- **The deny item.** Write the account's item `{"h": "<h>", "v": 1, "since": "<RFC 3339 UTC>"}`
  into the shared deny table (key `h`, a string: the 32-hex `<h>`) with `PutItem`; **T_w** is the
  time that `PutItem` was acknowledged.
  The authorizer reads that table and denies `u-<h>` and every id beginning `n-<h>-` (WITH the
  trailing hyphen: an id is denied iff it equals `u-<h>` or begins with `n-<h>-`) — every
  `/streams/{id}/` route answers 403 — at the latest `deny_cache_ttl` (≤ 60 s, hits and misses) after
  T_w. Then revoke the account's tokens and stop the hosted engine instance for the account.
- **The authorizer fails closed** (binding): a read error, or a deny table it cannot read or is not
  configured with, answers **503 `deny_list_unavailable`** and never allows; an error is never cached
  as a miss. The accepted consequence: a deny-table outage is an outage of all stream access. The
  hand-over carries the evidence — an authorizer test, or a kwv2 staging run with the table
  unreachable.
- **Board-fold fenced through the same table.** Board-fold's live fold, rescue, progress and
  backfill check the deny table (uncached) before EVERY chunk; for the account a chunk writes
  nothing, errs, is redriven and lands in the DLQ, logged with the reason `deny_list`.
  **`NXS_FOLD_STREAMS` is neither changed nor read by any step here** — it is an operational
  allow-list (unset = every stream), and a Lambda environment's 4 KB could never list every stream
  anyway. The proof, all three:
  - **(i)** the deny item, read back with a consistent `GetItem`, exists; and every board-fold
    function of the environment has `NXS_DENY_TABLE` naming the table whose **`TableArn`** equals
    the ARN the authorizer reports — compared by ARN, never by name (a name is ambiguous across
    regions and accounts). `nxf-fold-purge` gets its `NXS_DENY_TABLE` from the same source, and its
    report's `deny.table_arn` is checked against that ARN. **A function without `NXS_DENY_TABLE`, a
    different ARN, or a report naming another ARN: stop.**
  - **(ii) The canary**, run IN EACH ENVIRONMENT against the board-fold build deployed there, and
    again after every deploy of board-fold: a reserved `<h_canary>` — the first 32 hex of
    SHA-256(`nxs-canary:<environment>`); `nxs-canary` is no issuer, so no account derives it —
    written to the deny table the same way. After the canary item's OWN write time +
    `deny_cache_ttl` + the largest function timeout, one message for `n-<h_canary>-1` through EVERY
    entry point that writes — a live-fold change-stream INSERT, a rescue message, a progress message
    and a backfill — is refused at `admission`, logged with `deny_list`, and reaches the DLQ (the
    backfill exits non-zero) without a write to the board-fold table (a Query of the canary's
    partition stays empty). The canary proves the fence for the entry points it exercised, and the
    hand-over names them. Canary items are never deleted. Until the canary has passed in an
    environment, step 8b applies there.
  - **(iii) The wait:** step 1 — and with it every destructive step — not before
    **T_w + `deny_cache_ttl` + the LARGEST timeout of any board-fold function that can write**
    (live fold, rescue, progress, backfill; ≤ 60 s + ≤ 15 min; 960 s on the staging probe): the
    authorizer's cache has expired, and a chunk that read the table before T_w has finished. Every
    writing entry point is a Lambda invocation; a backfill run outside Lambda (a CLI, a container)
    reads the deny table before every chunk too and adds its own longest chunk time to the wait, or
    is not run during a deletion.
  A queued progress message for the account is then refused at its `admission` check and lands in
  the DLQ without writing, as is a change-stream record from before the fence, so the queues are
  not waited out. If (ii) cannot be shown, step 8b applies.
- **The pre-purge 403 probe.** The deny item first; then — **not before T_w + `deny_cache_ttl`**, a
  cached miss would still answer 200 — the probe; then token revocation. Through the public
  endpoint: `POST /streams/{id}/ops` with `{"ops":[]}` and `GET /streams/{id}/ops?since=0&limit=1`.
  Fenced = 403 on both AND the authorizer's log has `deny_list` with the request's `sid` token for
  both. `200 {"appended":0}`: NOT fenced — stop. A 401/403 without the `deny_list` line, or anything
  else: inconclusive — stop. Run it also for one synthetic, never-used `n-<h>-<k>` (a `<k>` above
  every enumerated one), which proves the prefix rule. **A 503 `deny_list_unavailable` is
  inconclusive — stop**; it never counts as fenced.

**1. Two relay dry runs, at least 10 minutes apart**, each with `--expect-identity`. (1a) A2's
checks (first entry) or the re-entry compare; (1b) the same ids as in A; (1c) no growth since A:
`max_seq` and every `found` not larger than in A2 — growth means something wrote after the fence;
(1d) the two `plan_digest`s are equal. Unequal digests after the fence: stop, do not loop.

**2. `nxf-relay purge <id> --yes --plan <digest of the second dry run> --expect-identity <pin>`.**
0: go on. 4: back to step 1 ONCE; a second 4 is an alarm — stop. 3: the fence leaks — stop. 2: wait
15 min, back to 1. 1: back to 1 (after a DynamoDB crash, the lease recovery first). 5: back to 1
later. 64 or 78: stop.

**3. Fold pass 1:** `nxf-fold-purge` dry run, then `--yes --plan <digest>` (6j6v.dbmd), each with
`--expect-identity`; the dry run's `fence` must be `denied` and `deny.table_arn` the authorizer's.
Its exit 3 does not block.

**4. At least 1 h after step 2:** relay confirm, exits 0.

**5. At least 25 h after the `finished_at` of step 4's report:** fold pass 2, a fold confirm, exits
0 — which reads the deny item back (`fence: denied`). On 3: a fold dry run and a bound `--yes`,
check step 0's proof again (the deny item still there, every board-fold function's `NXS_DENY_TABLE`
still the authorizer's table by ARN, and the canary passed against the build deployed NOW — a deploy
since step 0 means the canary runs again), repeat 4 and 5.

**6. After step 5:** relay confirm again, exits 0; read the deny item back (a consistent `GetItem`).

**7. Post-purge 403 check:** step 0's probe again, judged the same way, recorded with the run.

**8. Completeness:** A1's enumeration again over all three tables, and the deny item read back. An
id not in S that matches the fence runs steps A2 to 7 itself; one that does not match — any id
outside the scoped forms — the fence is not what step 0 says: stop.
The account is deleted when step 8 finds no id outside those already processed.

**8b. Only where step 0's canary (ii) has not passed in this environment against the deployed build,
for every writing entry point — the queue window:** wait `R_q`
after the later of T_w + `deny_cache_ttl` + the function timeout and step 5's `finished_at`, then a
fold confirm and a relay confirm, both exit 0.
`R_q` is the queues' `MessageRetentionPeriod` (the larger of the two for standard queues, their sum
for FIFO): 14 days for the known deployments.

**"Deleted" for a stream** means: steps 2, 4, 5 and 6 exited 0 in that order with the required
spacing, step 0's probe and step 7 answered 403, step 0's proof holds (or 8b passed), and the fence —
the deny item — is still in place. For an account: every id, and step 8 found no new one.

**Inputs the deployer reports with the hand-over:** the ops table's stream view type; board-fold's
`MaximumRecordAgeInSeconds` and Lambda timeout; both queues' `MessageRetentionPeriod` and type; the
PITR window; the pinned `identity_digest` per environment (relay and board-fold); the deny table's
name and `TableArn`, its deletion protection, retain policy and PITR, the authorizer's
`deny_cache_ttl`, the timeout of every writing board-fold function, the evidence used for step 0 (i)
and (ii) — the canary's environment, build and entry points included — the evidence for the
authorizer's fail-closed clause, and the vector file's run against the authorizer.

**Timing rule.** Every destructive step on a table with point-in-time recovery (step 2, step 3,
every repeat, every `--yes` for an id step 8 finds) finishes by day 30 minus the PITR window
(nexflow.it: 7 days → day 23). One that would run later is escalated to the owner first.

**Outside "deleted"**, each named: DynamoDB point-in-time recovery and AWS Backup; Postgres backups
and WAL archives; S3 versioning (off, required); the ops table's DynamoDB stream (NEW_IMAGE: INSERT
records of the last 24 h carry content until 24 h after their write; both have aged out before step
5); SQS rescue/DLQ messages (the pseudonymous stream id, never content, ≤ 14 d); CloudWatch logs
(365 d; ids only as log-id v1 tokens from 6j6v.01wt on).

## A refused id

Exit 64 with `refused.rule` (`empty`, `leading_hash`, `slash`, `control`, `too_long` — over 1,000
bytes). For nexflow.it the ids are `u-<hex>` and `n-<hex>-<n>`, which no rule refuses, so a refusal
means the account → stream mapping produced a wrong id: stop, keep the fence, escalate; never retry
with a modified id. For a self-hosted operator with such an id on SQL, outside the tool, with the
relay stopped, the id as a BOUND parameter and nothing else:

```sql
DELETE FROM stream_ops       WHERE stream_id = ?;
DELETE FROM prefix_claims    WHERE stream_id = ?;
DELETE FROM machine_presence WHERE stream_id = ?;
```

## The unfence procedure

Only for an id or an `<h>` proven NOT to belong to the account, before any destructive step touched
it (for example when check 1c later fails): its deny item is deleted from the shared deny table —
under the break-glass unfence role, the only role with `DeleteItem` on the table (kwv2 has `PutItem`
only; the authorizer, board-fold and both purges `GetItem` only), by two people, with the proof and
the time recorded. Canary items are never unfenced. `NXS_FOLD_STREAMS` plays no part. The deletion
takes effect at the authorizer within `deny_cache_ttl`; board-fold reads the table uncached.

**Unfencing does NOT undo the rest of step 0** (MF-9): the account's tokens stay revoked — the user
signs in again and gets new ones — and the stopped engine instance stays stopped until foundations
restart it. Tell the owner both, with the unfence record.

After a destructive step on a wrong id nothing in this tool undoes it: escalate at once.

## A deny-table outage

Every authorizer request answers 503 and every board-fold chunk errs and goes to the DLQ. Nothing is
lost from the relay: once the table answers again, backfill the board-fold copy from the relay for
the affected streams (chunks the DLQ dropped after its retention included). No deletion step runs
during an outage — step 0's probe is inconclusive. Within the
PITR window a point-in-time restore of the affected items is the only recovery, and that is the
owner's decision.

## DynamoDB: the stream lease after a crash

A `--yes` takes a lease item `("#sweep", "stream#<id>")` before it plans and releases it on every
exit except a process abort. After a crash the item stays until it expires (900 s): wait, then a
fresh dry run and a bound `--yes` — its plan may be empty — take over the expired lease and release
it, then the confirm again. A confirm that finds a lease item exits 3.

## SQLite

- **Lock time.** The `--yes` holds the store's write lock for the plan's re-read and one `DELETE`
  per table; the relay's own pushes wait up to `busy_timeout` (5 s) and are retried by the client
  after that (append is idempotent: nothing is lost). Measured 2026-10-20 (release build, a
  developer Mac on an external disk, ops of 400-byte values; `cargo test --release -p nxs-server
  --test purge_sqlite -- --include-ignored --nocapture the_delete_step_of`, which reads the
  `lock_ms`/`checkpoint_ms` the `--yes` prints on stderr): **5,000 ops: write lock 87 ms +
  checkpoint 11 ms; 100,000 ops (~40 MB): write lock 3,111 ms + checkpoint 24 ms** — under the 5 s
  `busy_timeout`, so the initial threshold stands. Rule: **above 100,000 ops or 64 MB of
  `value_bytes`, stop the relay before the `--yes`**; the dry run says so with `large: true`.
  Re-measure on your own hardware before trusting the margin: the lock time grows with the
  stream, not with the store.
- **`physical: "file_level"`** means `secure_delete` was on for this run's deletes and every WAL
  checkpoint completed. It says nothing about pages freed earlier by a relay without
  `secure_delete` — every release before this one. **Once, in a maintenance window with the relay
  stopped, run `VACUUM` on each relay file** (`sqlite3 relay.sqlite 'VACUUM;'`): it rewrites the
  file without its free pages. Nor does it cover blocks a copy-on-write file system or a backup
  keeps.
- **Self-hosted "deleted"** also needs the last `--yes` to report `physical: "file_level"`, and a
  fence of your own (the relay stopped, or the stream's routes blocked at your proxy) for steps 0 to
  8 — the relay authenticates nobody. The confirm after a store's last stream needs
  `--expect-identity`.

## Postgres

- **Connect directly, never through a transaction-pooling pooler** (MF-R4-2; e.g. PgBouncer in
  `transaction` mode, Supabase's port 6543): the purge holds session advisory locks across its
  transaction, which such a pooler does not keep. A session-mode pooler or a direct connection is
  fine. The purge waits at most 30 s for a stream's lock (`lock_timeout`), then exits 1.
- **A dedicated schema** (MF-5): a self-hosted Postgres relay belongs in a schema of its own
  (`search_path` in `NXF_RELAY_PG_URL`). The purge refuses (78, `UnknownKind`) when ANY table in its
  schema has a `stream_id` column it does not cover — another application's table included — and it
  sees tables its role holds no privilege on (it reads `pg_catalog`).
- **Least privilege:** `SELECT, DELETE` on `stream_ops`, `prefix_claims`, `machine_presence`;
  `SELECT` on `relay_store_identity`; `USAGE` on the schema.
- **Physical erasure is not attempted** (`physical: "not_attempted"`): deleted rows stay as dead
  tuples (and TOAST) until `VACUUM` and page reuse, and in WAL until it is recycled or archived. Run
  `VACUUM` on the three tables after a purge if your threat model needs it sooner; backups and WAL
  archives are yours to expire.
