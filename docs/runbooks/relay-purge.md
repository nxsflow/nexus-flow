# Runbook: purging a stream from the relay (`nxf-relay purge`)

`nxf-relay purge` removes everything the relay holds for ONE stream — on SQLite, Postgres and
DynamoDB — with a dry run that shows what would go, a delete bound to that dry run, and a confirm
run that proves nothing is left (6j6v.f6yb; the tool's reference is the module doc of
`crates/server/src/purge.rs`). It is the relay half of an account deletion. The account sequence
below is consumer contract v1.3: foundations (kwv2) runs it for nexflow.it; a self-hosted operator
runs the same steps by hand.

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

## The account sequence (consumer contract v1.3)

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
- **A3. The ownership check** (positive and mandatory). (i) The subject `(issuer, sub)` comes from
  the deletion-order record that POST /deletion created from the authenticated request — never
  typed in by an operator. (ii) Recompute `<h>` = the first 32 lowercase hex characters of
  SHA-256(`<issuer>:<sub>`) in your own code; where the order stores an `<h>` or a `subjectHash`,
  both must be equal to it — no normalisation of the issuer (a trailing `/` is a mismatch). (iii)
  Every id A1 found must parse as exactly `u-<h>` or `n-<h>-<k>` with `<k>` matching `^[0-9]+$`.
  Any failure: stop.
- **Re-entry.** A2's stop conditions apply on the FIRST entry for an id only. On every later entry
  (after exit 1, 2, 4 or 5, or step 8's re-run) compare the dry run with the previous report for
  that id instead: the same `identity_digest`, no `found` larger than before, and — when the
  previous `--yes` reported `deleted > 0` — an empty plan is expected, not a stop.
  Example after an exit 5: step 1 (two dry runs, equal digests, empty plans) → step 2 (`--yes`,
  empty plan, the checkpoint) → step 4.

**0. Fence, permanent and account-scoped.**

- The authorizer's deny list gets `u-<h>` and the prefix `n-<h>-` (WITH the trailing hyphen: an id
  is denied iff it equals `u-<h>` or begins with `n-<h>-`). The account's tokens are revoked; the
  hosted engine instance for the account is stopped.
- Board-fold admission closed: every id of the account is removed from `NXS_FOLD_STREAMS` in every
  board-fold function, as one configuration update at time T_u. **Stop when `NXS_FOLD_STREAMS` is
  unset or empty in any of them** (unset admits every stream). Then the admission proof: (i) the
  configuration read back lacks every id of the account and is non-empty; (ii) board-fold logs its
  configuration fingerprint (`cfg=<24 hex>`) in its start line and every EMF line: `LastUpdateStatus`
  is `Successful` at T_u, and after T_u + timeout every invocation logs the NEW fingerprint, over at
  least 15 min with at least one invocation per function; (ii-b) a canary id `n-<h_canary>-1`,
  removed in the same update, gets a progress and a rescue message after T_u + timeout: both are
  refused at `streams.admits`, logged under the new fingerprint, and reach the DLQ without a write;
  (iii) at least the function's timeout has passed since T_u. Until (ii) and (ii-b) have passed once
  in staging, step 8b applies.
- **The pre-purge 403 probe.** Deny list first, then the probe, then token revocation. Through the
  public endpoint: `POST /streams/{id}/ops` with `{"ops":[]}` and
  `GET /streams/{id}/ops?since=0&limit=1`. Fenced = 403 on both AND the authorizer's log has
  `deny_list` with the request's `sid` token for both. `200 {"appended":0}`: NOT fenced — stop. A
  401/403 without the `deny_list` line, or anything else: inconclusive — stop. Run it also for one
  synthetic, never-used `n-<h>-<k>` (a `<k>` above every enumerated one), which proves the prefix rule.

**1. Two relay dry runs, at least 10 minutes apart**, each with `--expect-identity`. (1a) A2's
checks (first entry) or the re-entry compare; (1b) the same ids as in A; (1c) no growth since A:
`max_seq` and every `found` not larger than in A2 — growth means something wrote after the fence;
(1d) the two `plan_digest`s are equal. Unequal digests after the fence: stop, do not loop.

**2. `nxf-relay purge <id> --yes --plan <digest of the second dry run> --expect-identity <pin>`.**
0: go on. 4: back to step 1 ONCE; a second 4 is an alarm — stop. 3: the fence leaks — stop. 2: wait
15 min, back to 1. 1: back to 1 (after a DynamoDB crash, the lease recovery first). 5: back to 1
later. 64 or 78: stop.

**3. Fold pass 1:** `nxf-fold-purge` dry run, then `--yes --plan <digest>` (6j6v.dbmd). Its exit 3
does not block.

**4. At least 1 h after step 2:** relay confirm, exits 0.

**5. At least 25 h after the `finished_at` of step 4's report:** fold pass 2, a fold confirm, exits
0. On 3: a fold dry run and a bound `--yes`, check step 0's admission proof again, repeat 4 and 5.

**6. After step 5:** relay confirm again, exits 0.

**7. Post-purge 403 check:** step 0's probe again, judged the same way, recorded with the run.

**8. Completeness:** A1's enumeration again over all three tables. An id not in S that matches the
fence runs steps A2 to 7 itself; one that does not match: the fence is not what step 0 says — stop.
The account is deleted when step 8 finds no id outside those already processed.

**8b. Only when step 0's (ii) could not be shown — the queue window:** wait `R_q` after the later of
T_u (+15 min) and step 5's `finished_at`, then a fold confirm and a relay confirm, both exit 0.
`R_q` is the queues' `MessageRetentionPeriod` (the larger of the two for standard queues, their sum
for FIFO): 14 days for the known deployments.

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
it (for example when check 1c later fails): take it off the deny list and put it back into
`NXS_FOLD_STREAMS` — two people, with the proof and the time recorded.

**Unfencing does NOT undo the rest of step 0** (MF-9): the account's tokens stay revoked — the user
signs in again and gets new ones — and the stopped engine instance stays stopped until foundations
restart it. Tell the owner both, with the unfence record.

After a destructive step on a wrong id nothing in this tool undoes it: escalate at once. Within the
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
