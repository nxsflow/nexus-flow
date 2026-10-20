//! [`DynamoDbOpStore`] and [`DynamoDbPrefixRegistry`]: the durable DynamoDB [`OpStore`] and
//! [`PrefixRegistry`](crate::registry::PrefixRegistry) backends. Both live in this one file
//! — unlike Postgres, which splits them across `store_pg.rs` / `registry_pg.rs` — because
//! the epic scoped it that way; there is no shared state between the two beyond the
//! process-wide `block_on` bridge and `ddb_err` mapping below.
//!
//! # The table contract
//!
//! Both tables are provisioned **externally** — by a stack the web package owns. Production
//! code here never creates a table and never migrates one; only the integration-test harness
//! (`tests/dynamodb_local.rs`) creates them, per test, for isolation. That makes this block
//! the contract, and the only place it is written down: whoever authors the
//! CDK/CloudFormation has to match it exactly, because a mismatch (say `seq` declared `S`
//! instead of `N`) is invisible until runtime, where it arrives as a `ValidationException`
//! behind a bare 500.
//!
//! **Ops table** — `NXF_RELAY_DDB_TABLE`, default `nxf_stream_ops`. PK `stream_id` (S),
//! SK `seq` (N):
//!
//! | `seq`  | item                         | attributes                                               |
//! |--------|------------------------------|----------------------------------------------------------|
//! | `0`    | the per-stream counter       | `next_seq` (N) — the highest seq taken                   |
//! | `>= 1` | one op                       | one attribute per [`WireOp`] field (see below)           |
//! | `< 0`  | one op's claim (`6j6v.tm4k`) | `op_id` (S), `op_seq` (N) — the seq that op is stored at |
//!
//! An op item's attributes are named exactly as the `WireOp` field: `envelope_version` (N),
//! `op_id` (S), `lamport` (N), `site` (N), `domain` (S), `target_kind` (S), `target_id` (S),
//! `field` (S), `op_type` (S), `value` (S), `author` (S), `wall_clock` (S), `extra` (S — the
//! JSON passthrough for envelope fields this build does not know, `6j6v.5crb`, omitted when
//! there are none). `value` is the
//! one `Option`: the attribute is **omitted entirely** when the field is `None`, so presence
//! (not emptiness) decides on the way back and a legitimate `Some("")` never collapses to
//! `None`.
//!
//! **Registry table** — `NXF_RELAY_DDB_REGISTRY_TABLE`, default `nxf_prefix_claims`. PK
//! `stream_id` (S), SK `sk` (S), with two item kinds sharing one partition per stream:
//!
//! | `sk`                  | attribute            | role                                     |
//! |-----------------------|----------------------|------------------------------------------|
//! | `prefix#<prefix>`     | `replica_uuid` (S)   | the claim-if-free key                    |
//! | `uuid#<replica_uuid>` | `prefix` (S)         | the idempotent "what did this replica get?" lookup — this backend's stand-in for SQLite/Postgres' `UNIQUE (stream_id, replica_uuid)` |
//! | `machine#<machine_id>` | `name` (S), `last_seen` (N), `interval_secs` (N), `expires_at` (N) | presence (6j6v.f0b5): one item per machine that announced itself for the stream, overwritten by every announcement — this backend's stand-in for the `machine_presence` table |
//!
//! The third kind needs no new table or index: it lives in the registry table the deployment already
//! has, under the same key schema. A relay from before presence never reads or writes it, and a
//! presence-aware relay never reads the other two kinds with the same query (its `begins_with`
//! names `machine#` only). Two things a least-privilege deployment has to know:
//!
//! - **IAM: the registry table now also needs `dynamodb:Query`** (and `dynamodb:PutItem`, which the
//!   op table's role already has). Until presence it only saw `GetItem` and `TransactWriteItems`; a
//!   role scoped to those answers every `GET /streams/{id}/machines` with a 500.
//! - **TTL is optional.** `expires_at` is when the item leaves the retention window, in the shape
//!   DynamoDB's TTL reads. Enabling TTL on that attribute removes expired machines for free; without
//!   it they stay stored and are filtered out of every read.
//!
//! Neither table needs a secondary index or a DynamoDB stream, and TTL is optional (above);
//! billing mode is the deployment's call (the test harness uses `PAY_PER_REQUEST`). Every op and
//! claim read this file issues is **strongly consistent** (`.consistent_read(true)`): the relay's
//! own pull path must see an append it just made, and eventual consistency would make the
//! acceptance suite flaky on top of that. Provisioning must therefore not put the tables behind
//! anything that only serves eventually-consistent reads (a global table replica read in a
//! non-writer region, say). The one exception is the presence read, which is eventually
//! consistent on purpose (see `PresenceStore for DynamoDbPrefixRegistry`).
//!
//! **Note for the Phase-2 reader.** The nexus-flow engine will later fold these same ops
//! from this same table. A naive `Query stream_id = X` picks up the counter item at
//! `seq = 0` and every claim item below it along with the ops — and they carry none of the
//! envelope attributes, so item-to-`WireOp` conversion fails on them. A reader must constrain
//! the query to `seq > 0`, as [`DynamoDbOpStore::read_since`] does. The same holds for a
//! consumer of a DynamoDB stream on this table: an item with `seq <= 0` is bookkeeping, never
//! an op. Sequence numbers are unique and monotone, and a seq is visible only once every
//! lower seq this build wrote is (see the allocation section below, with what that rests on).
//!
//! # IAM — what the relay's role needs
//!
//! **Upgrade step, required since `6j6v.r3jf`: grant `dynamodb:ConditionCheckItem` on the op table
//! BEFORE deploying a relay of this build.** Every append past a stream's first op now carries a
//! `ConditionCheck` in its transaction (below), and DynamoDB authorises each action inside a `TransactWriteItems`
//! as its own IAM action; a role without `ConditionCheckItem` gets every push refused with
//! `AccessDeniedException`, surfaced as a 500, while reads keep working.
//!
//! - **Op table:** `dynamodb:Query`, `dynamodb:PutItem`, `dynamodb:UpdateItem`,
//!   `dynamodb:ConditionCheckItem`.
//! - **Registry table:** `dynamodb:GetItem`, `dynamodb:PutItem`, `dynamodb:Query`.
//!
//! # Design notes
//!
//! **Storage is opaque** (mirrors `store_pg.rs`): a [`WireOp`]'s fields map to attributes
//! one-for-one. No constraint or validation touches `op_type` / `target_kind` /
//! `envelope_version` — an unknown shape is stored and read back verbatim, the same
//! forward-compat contract SQLite and Postgres already honour. The relay is a dumb
//! op-carrier and must never reject an op it does not recognise.
//!
//! **The sync/async seam.** `OpStore` is a synchronous trait (shaped for the blocking
//! SQLite/Postgres drivers); `aws-sdk-dynamodb` is async. [`block_on`] bridges the two on a
//! dedicated, process-wide runtime — never the caller's — because the relay calls `append`
//! from inside an axum handler; see `block_on`'s doc for why that specific shape (channel
//! handoff, not a second `Runtime::block_on`) is required.
//!
//! **Sequence allocation: a seq is taken by the write that commits it** (`6j6v.r3jf`). Each
//! append round knows a counter value `c` (read from the counter item, `seq = 0`, attribute
//! `next_seq`, absent counting as 0 — or learned without a read, below) and sends ONE
//! `TransactWriteItems` that
//!
//! 1. sets `next_seq` to `s = c + 1`, conditioned on `next_seq` still being `c`;
//! 2. puts the op's claim (below), conditioned on its key being free;
//! 3. for `s > 1`, checks the op at `s - 1` with a `ConditionCheck` — `attribute_exists`;
//! 4. puts the op at `s`, conditioned on that key being free.
//!
//! A round that finds the counter moved has lost to a concurrent append and retries from the
//! counter the failed check hands back (`ReturnValuesOnConditionCheckFailure = ALL_OLD`), after a
//! short random pause ([`append_with_retry`]).
//!
//! Why not an `UpdateItem` `ADD` ahead of the write, as before: an `ADD` hands out the seq before
//! the op exists. Writer A got 10, writer B got 11, B committed first, and a reader at 9 was
//! handed `[11]` and stored cursor 11 — so A, committed at 10 a moment later, sat below every
//! cursor that reader would send again, missing for good, and a fold watermark moved past it too.
//!
//! *What AWS documents, and nothing more:* `TransactWriteItems` is **serializable** against other
//! transactions (and against single-item writes) — they behave as if they ran one at a time, and a
//! transaction that touches an item another transaction is still writing is cancelled with
//! `TransactionConflict`. A `Query` is only **read-committed** against a transaction: it never sees
//! uncommitted data, but a transaction's items need not all become visible to it at the same
//! instant, so a `Query` can see one item of a committing transaction in its new state and another
//! still in its old one. Each per-item reason in a cancellation is reported as that item's own
//! outcome (`None` when its condition held).
//!
//! *What the design builds on those two facts:*
//!
//! - **The counter condition orders the transactions.** The transaction that takes `s` expects
//!   the counter at `c = s - 1`, which only the transaction that took `c` can have set. By
//!   serializability, `T_s` runs after `T_c`. That alone does NOT order their *visibility* to a
//!   `Query`: `T_c` can show its counter before its op, `T_s` can read that counter, write op `s`
//!   — a different item — and commit while op `c` is still invisible, and a reader in that window
//!   is handed `[s]`. (Review of PR #59, Integrity #1.)
//! - **The `ConditionCheck` on op `s - 1` closes that window.** It puts op `c`'s item into
//!   `T_s`, so `T_s` and `T_c` now share an item: while `T_c` is still writing op `c`, `T_s` is
//!   cancelled with `TransactionConflict` (a lost round, retried); once `T_c` is done, op `c`
//!   exists and `T_s` may commit. So op `s` cannot become visible before op `s - 1` has been
//!   written — transaction against transaction, the case AWS documents — and by induction a
//!   strongly consistent `read_since` that returns seq `n` has every op below `n` this build
//!   wrote. That is the order SQLite (a mutex) and Postgres (an advisory lock held to commit)
//!   give, and what a reader storing its cursor unconditionally, and a snapshot at P (spec
//!   `streams-without-lifetime-growth` §4.1), need.
//! - **When op `s - 1` is legitimately absent, the check flips, it is never dropped.** A failed
//!   `attribute_exists` with the counter condition holding means: the counter stands at `c`, no
//!   transaction is writing op `c` (that would have been a conflict), and op `c` is not there. No
//!   transaction of this build can still write it — the only one that could is the one that moved
//!   the counter to `c`, and it writes counter and op together. So `c` is a seq an earlier build's
//!   `ADD` burned (or one an `ADD`-era relay still holds, see the rolling upgrade). The next round,
//!   for the SAME `s` only, checks op `c` with `attribute_not_exists` instead: still an item of the
//!   transaction, so a writer of op `c` still conflicts, and should op `c` appear meanwhile the check
//!   fails and the round after goes back to `attribute_exists`. Seq 1 has no predecessor and no
//!   check. A floor step (below) takes `s` one above an occupied seq, so its check finds that op. A
//!   legacy gap therefore costs one extra round, once, and never wedges the stream: the round that
//!   steps over it moves the counter to an op that exists.
//! - **The op's own condition stays as the brace.** A seq occupied by an op the counter does not
//!   account for (a stream written before the counter existed, an item put there out of band) is
//!   stepped over, never overwritten: the round finds the highest occupied seq above the counter
//!   with ONE reverse `Query` (`Limit 1`) and takes the seq after it, so a run of any length costs
//!   one extra round, not one per seq. Seqs are `i64`; a counter at `i64::MAX` is a loud error.
//!
//! If a future change deletes ops (truncation below a snapshot, spec slice 2), it must keep the op
//! the counter points at, or the next append takes one extra round over the gap — correct, but it
//! is the kind of gap the flip above treats as legacy.
//!
//! **What it costs.** Uncontended: one transaction per op (four items: counter, claim, the
//! predecessor check, op) plus one strongly consistent `Query` of the counter for the first op of
//! a push — the rest of the push carries the seq it wrote forward as the counter to expect
//! ([`OpStore::append_batch`]), so a push of `n` ops costs `n` transactions and one read.
//! DynamoDB bills a transaction at twice the plain write per item; the `ConditionCheck` is a read
//! of op `s - 1` billed at transactional read rates, sized by that op. Appends to ONE stream
//! serialise on the counter item, as on the other two backends, and appends to different streams
//! never contend.
//!
//! Under contention a lost round is a whole cancelled transaction, and **cancelled transactions
//! consume capacity like committed ones**, so `N` writers racing for one stream cost up to about
//! `N` times the capacity per op stored. Two transactions can also both be cancelled in one round,
//! so contention is not "one winner per round". The per-stream ceiling is about one append per
//! counter round-trip — on the order of 30-60 appends per second per stream on real AWS by
//! latency arithmetic; **unmeasured** (filed as a chore, see the PR). [`CONTENTION_ROUNDS`] bounds
//! the rounds one append loses in a row, [`APPEND_ATTEMPTS`] the rounds that fail for any other
//! reason, and the push deadline ([`OpStore::append_batch`], `app::PUSH_DEADLINE`) bounds the
//! wall clock of a whole push; each ends in a loud `StoreError`, and the client re-pushes, which
//! the claims make safe.
//!
//! **Density.** A seq is taken only together with its op, so a stream written by this build has
//! no gaps: neither a re-push nor a lost round nor the SDK's retry of a write whose response was
//! lost consumes one (the retry finds the claim taken). Streams written before it carry the
//! gaps the `ADD` left. Nothing downstream depends on density — `read_since` is `seq > cursor`
//! and the client follows the returned cursor — so read it as a property, not a contract.
//!
//! **Rolling upgrade.** First grant `dynamodb:ConditionCheckItem` (above). A relay of an earlier
//! build still allocates with `ADD` and can still expose a seq ahead of an uncommitted one while it
//! serves; mixed with this build nothing is overwritten (both condition the op's put), the counter
//! never moves backwards (this build's `SET` is conditioned on the value it read, and an `ADD`
//! landing in between fails that condition), and the ordering guarantee holds once every relay
//! writing a table runs this build. Meanwhile the old relays answer more pushes with a 500: their
//! `ADD` on the counter is itself cancelled with `TransactionConflict` whenever one of this build's
//! transactions holds the counter, and the old build does not retry that.
//!
//! **Per-item size ceiling.** DynamoDB rejects any item over **400 KB**, so a single op
//! whose `value` exceeds roughly that is a hard `StoreError` here where SQLite and Postgres
//! would have stored it. The relay's own push guard is much looser (`MAX_PUSH_BYTES`, 8 MB
//! for a whole batch), so this ceiling is reached inside the store, not at the HTTP edge.
//! It is a real Phase-1 constraint of this backend, not a shared one.
//!
//! **Append is idempotent by `op_id`** (`6j6v.tm4k`), as on SQLite and Postgres, where
//! `UNIQUE (stream_id, op_id)` does it. DynamoDB has no secondary unique constraint, so each op
//! carries a **claim item** in the same partition: its range key is a negative number derived
//! from the op's `op_id` ([`claim_seq`]), and it records the op's id and the seq the op went in
//! at. The op and its claim are written by ONE `TransactWriteItems`, each conditioned on its
//! key being free, so a claim exists exactly when its op does. A re-pushed op finds its claim
//! taken, reads it, and returns the op's EXISTING seq: the log does not grow, and the SDK's
//! own retry of a write whose response was lost in flight — a duplicate source SQLite and
//! Postgres never had — is absorbed the same way.
//!
//! What it costs and where it differs from the other two backends:
//!
//! * **The claim rides the append's transaction.** It is an item of the same
//!   `TransactWriteItems` that moves the counter and puts the op, so a re-push is turned back by
//!   its taken claim before the counter moves: it consumes no seq, as on SQLite and Postgres.
//!   A taken claim outranks a moved counter, so a re-push that also lost the counter race still
//!   answers with its stored seq. At most four items per transaction, far below DynamoDB's 100.
//! * **Ops stored before the claim items existed have none.** Re-pushing such an op stores it
//!   once more, with a claim this time; every later re-push is then absorbed.
//! * **The claim key is a 63-bit hash, so two op ids can share it.** The claim records the op
//!   id it was written for, so a shared key is detected rather than mistaken for a duplicate:
//!   the second op is then stored WITHOUT a claim, exactly as before this change, and only it
//!   loses the dedupe. Nothing is ever lost or overwritten by a collision.
//! * **No new table or index.** The claim lives in the op table under the same key schema, it is
//!   written with `PutItem` inside the transaction and read with `Query`. (The transaction's
//!   `ConditionCheck`, `6j6v.r3jf`, is what needs the one new IAM action — see "IAM" above.)

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aws_sdk_dynamodb::error::ProvideErrorMetadata;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{
    AttributeValue, ConditionCheck, Put, ReturnValue, ReturnValuesOnConditionCheckFailure,
    TransactWriteItem, Update,
};
use aws_sdk_dynamodb::Client;
use nxs_sync::protocol::{Cursor, MachineHello, RegisterOutcome, StreamId};
use nxs_sync::wire::WireOp;

use crate::presence::{MachineRecord, MachinesPage, PresenceStore};
use crate::registry::{
    candidate_prefix, LeaseItem, LeaseRenewed, LeaseTaken, PrefixRegistry, StreamLease,
    REASSIGN_ATTEMPTS,
};
use crate::store::{
    decode_extra, encode_extra, page_limit, past_deadline, OpStore, StoreError, StoreResult,
};

/// The DynamoDB SDK is async; `OpStore`/`PrefixRegistry` are sync (they were shaped for the
/// sync SQLite/Postgres drivers). Bridging on a DEDICATED runtime — never the caller's — is
/// what makes that safe: the relay calls `append` from inside an axum handler, so blocking the
/// calling thread on its own runtime would panic. Here the future runs on this runtime's
/// worker threads and the caller merely waits on a channel, exactly as the blocking Postgres
/// driver makes its caller wait on a socket.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("nxf-ddb")
            .build()
            .expect("build the DynamoDB bridge runtime")
    })
}

/// Run one SDK future to completion from sync code. `OnceLock` keeps the runtime alive for the
/// process, so it is never dropped from an async context (which would also panic).
///
/// The caller's thread is genuinely blocked for a full network round-trip, and `app.rs`'s
/// push handler calls `append` once PER OP — so a 500-op push would otherwise pin one axum
/// worker for 1000 sequential round-trips. [`tokio::task::block_in_place`] tells tokio that
/// this worker is about to block, so it hands the worker's remaining tasks to a replacement
/// thread instead of stalling them behind us. (Batching the appends themselves is the real
/// fix and is tracked separately; this keeps one slow push from looking like a dead relay.)
///
/// The flavor guard is load-bearing, not defensive: `block_in_place` **panics** on a
/// current-thread runtime, and the relay is embedded on one in the DynamoDB smoke test. Off
/// a runtime entirely (the common case — the acceptance suite calls the store from plain
/// `#[test]` threads) there is nothing to hand off, so the plain recv is also the right call.
pub(crate) fn block_on<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime().spawn(async move {
        let _ = tx.send(fut.await);
    });
    let on_multi_thread = matches!(
        tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()),
        Ok(tokio::runtime::RuntimeFlavor::MultiThread)
    );
    let received = if on_multi_thread {
        tokio::task::block_in_place(|| rx.recv())
    } else {
        rx.recv()
    };
    received.expect("DynamoDB bridge task panicked")
}

/// Map an SDK operation error to the trait's opaque [`StoreError`] — keeps `aws_sdk_dynamodb`
/// types out of the trait signature, exactly as `store_pg::pg_err` keeps `postgres` out.
/// Generic over any per-operation `SdkError<E, R>`, all of which the SDK's own `Error` type
/// converts from, so one helper covers GetItem/PutItem/Query alike.
fn ddb_err<E: Into<aws_sdk_dynamodb::Error>>(e: E) -> StoreError {
    let err = e.into();
    // `Display` on the SDK's unified `Error` renders an unmatched variant as the useless
    // "unhandled error (ValidationException)" — the service's own sentence, the part that
    // actually names WHICH validation failed, is only reachable through the error metadata.
    // Reading code + message explicitly is what turns "something went wrong" into
    // "ValidationException: Item size has exceeded the maximum allowed size".
    let code = err.code().unwrap_or("UnknownError");
    let message = match err.message() {
        Some(detail) => format!("{code}: {detail}"),
        None => code.to_string(),
    };
    // `app.rs` maps every `StoreError` to a bare 500 with no body, so without this line a
    // wrong table name, a wrong region, expired credentials, an unprovisioned table and an
    // over-400 KB item are all indistinguishable to an operator: an empty 500 and silence.
    // Postgres does not need this because `connect()` fails loudly at boot; DynamoDB builds
    // its client without I/O, so its first evidence of any misconfiguration is an in-flight
    // request failing. Every SDK error in this file funnels through here, which is why one
    // line covers all of them — and why the ordinary, expected outcomes (a conditional-check
    // collision, a claim-if-free loss) deliberately do NOT come through here and stay quiet.
    eprintln!("nxf-relay: dynamodb error: {message}");
    StoreError(message)
}

/// An SDK error as its error CODE only (log-id v1 C10, owner decision 2026-10-20): the SDK's own
/// message can quote a key or a value, so neither the log nor a purge report ever carries it. Used
/// by the stream lease and the purge; unlike [`ddb_err`] it writes nothing to stderr itself.
pub(crate) fn ddb_code_err<E: Into<aws_sdk_dynamodb::Error>>(e: E) -> StoreError {
    let err = e.into();
    StoreError(format!(
        "dynamodb error: {}",
        err.code().unwrap_or("UnknownError")
    ))
}

/// Build a client from explicit config. The store and the registry take their table name,
/// endpoint, and region as CONSTRUCTOR ARGUMENTS rather than reading the environment
/// themselves, so both stay directly testable against DynamoDB Local; reading the
/// `NXF_RELAY_DDB_*` variables is `backends()`'s job at boot (`src/main.rs`), the single
/// place the process's environment is consulted.
/// `region`/`endpoint` fall back to the standard AWS chain when `None`; DynamoDB Local
/// overrides `endpoint` (and the test harness sets a region via env, since the chain still
/// needs one). Building a client does no I/O and cannot fail — connectivity and the table's
/// existence are only ever proven by an actual request. Shared by
/// [`DynamoDbOpStore::new`] and [`DynamoDbPrefixRegistry::new`]: identical client
/// construction, two independently provisioned tables.
pub(crate) async fn build_client(endpoint: Option<String>, region: Option<String>) -> Client {
    let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .timeout_config(request_timeouts());
    if let Some(region) = region {
        loader = loader.region(aws_sdk_dynamodb::config::Region::new(region));
    }
    if let Some(endpoint) = endpoint {
        loader = loader.endpoint_url(endpoint);
    }
    Client::new(&loader.load().await)
}

/// Per-attempt ceiling on one HTTP round-trip to DynamoDB. Generous next to DynamoDB's
/// single-digit-millisecond norm, but low enough that a black-holed connection is abandoned
/// rather than waited on.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// Ceiling on one whole SDK operation, INCLUDING the SDK's own internal retries. This is the
/// number that actually bounds a caller: [`block_on`] hands the calling thread to a channel
/// receive with no deadline of its own, so whatever bounds the future bounds the thread.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(15);

/// Explicit request timeouts. The SDK's defaults cover only a ~3.1 s **connect** timeout —
/// nothing bounds a connection that is established and then goes quiet, which is exactly what
/// a black-holed endpoint, a mid-failover table, or a saturated NAT looks like. Without this,
/// a stalled endpoint parks an axum worker on `block_on`'s receive indefinitely, and the relay
/// degrades to "alive but answering nothing" instead of returning errors the client can retry.
/// `store_pg.rs` sets an explicit connection-acquire timeout for the same reason; this is that
/// precedent applied to the transport DynamoDB actually uses.
///
/// Deliberately constants rather than `NXF_RELAY_DDB_*` knobs: this file never reads the
/// process environment (see [`build_client`] — that is `backends()`'s job), and a timeout the
/// operator can raise is a timeout that will be raised to paper over a real fault.
fn request_timeouts() -> aws_config::timeout::TimeoutConfig {
    aws_config::timeout::TimeoutConfig::builder()
        .operation_attempt_timeout(ATTEMPT_TIMEOUT)
        .operation_timeout(OPERATION_TIMEOUT)
        .build()
}

/// Read a required `N` (number) attribute and parse it. Malformed shape (missing attribute,
/// wrong DynamoDB type, unparsable text) is a loud [`StoreError`], not a silent default —
/// these attributes are our own invariant, not user input, so a mismatch means the table was
/// touched out of band and must not be papered over.
fn parse_n<T: std::str::FromStr>(
    item: &HashMap<String, AttributeValue>,
    key: &str,
) -> StoreResult<T>
where
    T::Err: std::fmt::Display,
{
    item.get(key)
        .ok_or_else(|| StoreError(format!("item missing required attribute {key:?}")))?
        .as_n()
        .map_err(|_| StoreError(format!("attribute {key:?} is not a DynamoDB number")))?
        .parse::<T>()
        .map_err(|e| StoreError(format!("attribute {key:?} is not a valid number: {e}")))
}

/// Read a required `S` (string) attribute.
fn parse_s(item: &HashMap<String, AttributeValue>, key: &str) -> StoreResult<String> {
    Ok(item
        .get(key)
        .ok_or_else(|| StoreError(format!("item missing required attribute {key:?}")))?
        .as_s()
        .map_err(|_| StoreError(format!("attribute {key:?} is not a DynamoDB string")))?
        .clone())
}

/// The sentinel range key for a stream's counter item. Real ops start at seq 1; every
/// `read_since` query is `seq > cursor` with `cursor >= 0`, so this item is structurally
/// invisible to reads — no filter needed to keep it out of a page.
const COUNTER_SEQ: i64 = 0;

/// The range key of an op's claim item: the first 63 bits of `SHA-256(op_id)`, negated and
/// moved down by one, so it lands in `i64::MIN..=-1` — below the counter at 0, where no
/// `seq > cursor` read can ever reach it. A hash, not the op id itself, because the range key
/// is a number (`seq` is `N`); SHA-256, not `std`'s hasher, because the key is stored and must
/// come out the same from every build that ever reads it. Two op ids sharing a key is handled,
/// not ruled out (see the module doc).
#[doc(hidden)] // public only so the DynamoDB Local suite plants a claim at the real key
pub fn claim_seq(op_id: &str) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(op_id.as_bytes());
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    -((u64::from_be_bytes(head) >> 1) as i64) - 1
}

/// Bound on the append rounds that end WITHOUT another append having committed in between: a
/// taken seq, a predecessor check that did not hold, a throttled transaction, a claim key found
/// taken by another op id or read back absent. Counted over the whole append. Exhausting it means
/// something is structurally wrong or the table is saturated, and that must surface as a loud
/// `StoreError`, never spin the caller's thread forever.
const APPEND_ATTEMPTS: u32 = 32;

/// Bound on the rounds one append loses IN A ROW to a concurrent append of the same stream — the
/// counter moved under it, or DynamoDB cancelled it with a `TransactionConflict`. Any round that
/// ends otherwise resets the count. Separate from [`APPEND_ATTEMPTS`], and larger, because a lost
/// round usually means another writer committed: the stream is making progress, only this writer
/// is losing the race for the next seq.
///
/// A backstop, not a promise about fairness: two transactions can also cancel each other in the
/// same round, so "one writer wins every round" does not hold on real AWS. What bounds the wall
/// clock of a push is its deadline ([`OpStore::append_batch`]). Without one (a bare
/// [`OpStore::append`]), one append's ceiling is about `CONTENTION_ROUNDS` x (one transaction,
/// ~10-30 ms on AWS, plus at most 16 ms of jitter) per run of losses — 5-12 s — plus at most
/// `APPEND_ATTEMPTS - 1` stalled rounds, each of whose SDK calls [`OPERATION_TIMEOUT`] caps.
const CONTENTION_ROUNDS: u32 = 256;

/// What one append round reads and writes: the counter value it expects to find (`0` when the
/// stream has no counter yet) and the seq it moves the counter to and puts the op at, in ONE
/// transaction. `PartialEq`/`Debug` exist for the unit tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    expected: i64,
    seq: i64,
}

impl Slot {
    /// The slot after `counter`: one above it, or one above `floor` when that is higher. `floor`
    /// is the highest seq an earlier round found occupied although the counter had not reached
    /// it — a stream written before the counter existed, or an item put there out of band.
    /// Stepping over such a seq is safe: it already holds a committed op. A seq past `i64::MAX`
    /// is a loud error, never a wrap into the negative range the claims live in.
    fn after(counter: i64, floor: i64) -> StoreResult<Slot> {
        let seq = counter.max(floor).checked_add(1).ok_or_else(|| {
            StoreError(format!(
                "append: the next seq would pass i64::MAX (counter {counter}, highest occupied \
                 seq {floor})"
            ))
        })?;
        Ok(Slot {
            expected: counter,
            seq,
        })
    }

    /// The seq the predecessor check looks at: the one below, `None` for a stream's first seq.
    fn predecessor(self) -> Option<i64> {
        if self.seq > 1 {
            Some(self.seq - 1)
        } else {
            None
        }
    }
}

/// Where a round's counter value comes from. Only [`Counter::Unknown`] costs a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counter {
    /// Not known: read the counter item first.
    Unknown,
    /// Handed back by the previous round's failed counter check (`ALL_OLD`).
    Hint(i64),
    /// The value the previous round's counter condition held at, in a round that failed on
    /// something else (a taken seq, a predecessor check).
    Known(i64),
    /// The seq the previous op of the same push wrote ([`OpStore::append_batch`]).
    Carried(i64),
}

impl Counter {
    /// The name a test seam reports it by.
    fn source(self) -> &'static str {
        match self {
            Counter::Unknown => "read",
            Counter::Hint(_) => "hint",
            Counter::Known(_) => "known",
            Counter::Carried(_) => "carried",
        }
    }
}

/// How a round checks the op below the seq it takes (module doc, "Sequence allocation").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Predecessor {
    /// Seq 1: there is nothing below.
    None,
    /// `attribute_exists` — every round but the one below.
    Exists,
    /// `attribute_not_exists` — the round right after an `Exists` check at the same seq found the
    /// op below absent: a legacy gap. Still an item of the transaction, so a writer of it still
    /// conflicts.
    Absent,
}

/// What the next round of [`append_with_retry`] starts from: the counter value to expect, the
/// highest seq found occupied above the counter, whether the op still goes in with its claim, and
/// the one seq whose predecessor a round just found absent (`gap`, cleared by every other outcome).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Round {
    counter: Counter,
    floor: i64,
    claimed: bool,
    gap: Option<i64>,
}

impl Round {
    fn first(counter: Counter) -> Round {
        Round {
            counter,
            floor: 0,
            claimed: true,
            gap: None,
        }
    }

    /// The predecessor check for `slot`: none for seq 1, `Absent` for the seq a previous round
    /// found the op below missing at, `Exists` otherwise.
    fn predecessor_check(&self, slot: Slot) -> Predecessor {
        match slot.predecessor() {
            None => Predecessor::None,
            Some(_) if self.gap == Some(slot.seq) => Predecessor::Absent,
            Some(_) => Predecessor::Exists,
        }
    }
}

/// Outcome of one append round. `PartialEq`/`Debug` exist for the unit tests.
#[derive(Debug, PartialEq, Eq)]
enum PutOutcome {
    /// The op is stored at this seq, and the counter stands at it.
    Written(i64),
    /// The op was already stored, at this seq: the append is done, and that is its answer.
    Existing(i64),
    /// Another append of this stream committed first (the counter moved), or is committing (a
    /// `TransactionConflict`). Carries the counter value the failed check saw, when DynamoDB
    /// handed it back, so the next round needs no read.
    Lost(Option<i64>),
    /// The counter held at `counter`, but the seq is occupied by an op the counter does not
    /// account for; `top` is the highest occupied seq above the counter.
    SeqTaken { counter: i64, top: i64 },
    /// The counter held at `counter`, but the op below `seq` was not as the check expected:
    /// `absent` under an `Exists` check (a legacy gap: flip the check for this seq), present under
    /// an `Absent` one (it appeared meanwhile: check for it again).
    Predecessor {
        counter: i64,
        seq: i64,
        absent: bool,
    },
    /// Throttled or over capacity: wait ([`claim_backoff`]), then try again.
    Throttled,
    /// The claim key belongs to another op id: store this op without a claim (module doc).
    ClaimForeign,
    /// The claim was reported taken and read back absent: try again.
    ClaimVanished,
}

/// How an append settled: written by this call, or found already stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    Written(i64),
    Existing(i64),
}

impl Settled {
    fn seq(self) -> i64 {
        match self {
            Settled::Written(seq) | Settled::Existing(seq) => seq,
        }
    }
}

/// The error an append stops with at the push deadline. `app.rs` answers it 503 by the clock, not
/// by this text; [`OpStore::append_batch`] replaces it with how far the push got.
fn deadline_passed(stream_id: &str) -> StoreError {
    StoreError(format!(
        "append to stream {stream_id:?} stopped at the push deadline"
    ))
}

/// Drive append rounds until one settles, within [`APPEND_ATTEMPTS`], [`CONTENTION_ROUNDS`] and,
/// when given, `deadline` — checked before every round and raced against the round in flight. A
/// round cut short may still commit on DynamoDB's side; the client's re-push then finds its claim.
///
/// Generic over the round so the retry policy — which outcome retries with what, after how long,
/// within which budget — is unit-tested without a table: DynamoDB Local serialises transactions,
/// so most of these branches never occur against it.
async fn append_with_retry<F, Fut>(
    stream_id: &str,
    start: Counter,
    deadline: Option<Instant>,
    mut round: F,
) -> StoreResult<Settled>
where
    F: FnMut(Round) -> Fut,
    Fut: std::future::Future<Output = StoreResult<PutOutcome>>,
{
    let mut next = Round::first(start);
    let (mut stalls, mut lost) = (0u32, 0u32);
    loop {
        let outcome = match deadline {
            None => round(next).await?,
            Some(deadline) => {
                if Instant::now() >= deadline {
                    return Err(deadline_passed(stream_id));
                }
                let until = tokio::time::Instant::from_std(deadline);
                match tokio::time::timeout_at(until, round(next)).await {
                    Ok(outcome) => outcome?,
                    Err(_) => return Err(deadline_passed(stream_id)),
                }
            }
        };
        next.gap = None;
        match outcome {
            PutOutcome::Written(seq) => return Ok(Settled::Written(seq)),
            PutOutcome::Existing(seq) => return Ok(Settled::Existing(seq)),
            PutOutcome::Lost(seen) => {
                lost += 1;
                if lost >= CONTENTION_ROUNDS {
                    return Err(StoreError(format!(
                        "append: lost {CONTENTION_ROUNDS} rounds in a row to concurrent appends \
                         of stream {stream_id:?}; the stream is under more write contention than \
                         one append waits out, retry the push"
                    )));
                }
                next.counter = seen.map_or(Counter::Unknown, Counter::Hint);
                tokio::time::sleep(contention_jitter(lost)).await;
            }
            stalled => {
                lost = 0;
                stalls += 1;
                if stalls >= APPEND_ATTEMPTS {
                    return Err(StoreError(format!(
                        "append: exhausted {APPEND_ATTEMPTS} attempts writing an op to stream \
                         {stream_id:?} without a successful conditional write (last: {stalled:?})"
                    )));
                }
                next.counter = Counter::Unknown;
                match stalled {
                    PutOutcome::SeqTaken { counter, top } => {
                        next.counter = Counter::Known(counter);
                        next.floor = next.floor.max(top);
                    }
                    PutOutcome::Predecessor {
                        counter,
                        seq,
                        absent,
                    } => {
                        next.counter = Counter::Known(counter);
                        if absent {
                            next.gap = Some(seq);
                        }
                    }
                    PutOutcome::ClaimForeign => next.claimed = false,
                    PutOutcome::Throttled => {
                        let base = claim_backoff(stalls - 1);
                        tokio::time::sleep(base + jitter(base)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A uniformly random duration in `[0, cap]`. Seeded from `RandomState`, whose keys `std` draws
/// from the OS once per thread and steps per instance — enough to de-synchronise writers, which
/// is all a backoff needs, without a `rand` dependency.
fn jitter(cap: Duration) -> Duration {
    use std::hash::BuildHasher;
    let cap_us = cap.as_micros().max(1) as u64;
    let draw = std::collections::hash_map::RandomState::new().hash_one(Instant::now());
    Duration::from_micros(draw % (cap_us + 1))
}

/// The pause before the next round after losing one to a concurrent append: full jitter up to
/// 1, 2, 4, 8, then 16 ms. Small and capped, unlike a throttle backoff, because a lost round means
/// the stream moved on — waiting longer would only hand the next seq to whoever did not wait —
/// and random, so the losers do not all come back for the same seq at the same instant.
fn contention_jitter(lost: u32) -> Duration {
    jitter(Duration::from_millis(1u64 << lost.min(4)))
}

/// Which item of an append's `TransactWriteItems` sits at which index — the order
/// [`DynamoDbOpStore::try_append_op`] adds them in: the counter, then the claim when the op
/// carries one, then the predecessor check when the seq has one, last the op. DynamoDB reports
/// cancellation reasons positionally, so this is the one place the order is decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TxLayout {
    claimed: bool,
    checked: bool,
}

impl TxLayout {
    const COUNTER: usize = 0;

    fn claim(self) -> Option<usize> {
        self.claimed.then_some(1)
    }

    fn predecessor(self) -> Option<usize> {
        self.checked.then_some(1 + usize::from(self.claimed))
    }

    fn op(self) -> usize {
        1 + usize::from(self.claimed) + usize::from(self.checked)
    }
}

/// Why an append's `TransactWriteItems` was cancelled, when the cancellation is an outcome rather
/// than a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppendCancellation {
    /// The claim's key is taken: this op — or one whose id shares the key — is stored.
    Claimed,
    /// The counter is not at the value this round expected: another append committed first.
    CounterMoved,
    /// The counter held but the op's seq is occupied: step over it.
    SeqTaken,
    /// The counter held but the predecessor check did not.
    PredecessorFailed,
    /// A concurrent transaction on one of the items: another append is committing.
    Conflict,
    /// Throttled or over capacity.
    Throttled,
}

/// Classify a cancelled append from its positional per-item codes, laid out as `layout` says.
/// Pure for the same reason [`classify_cancellation`] is: DynamoDB Local serialises transactions,
/// so the transient codes never reach an acceptance test.
///
/// A taken claim outranks everything: it is definitive, and it means the op is already stored, so
/// the seq question no longer matters. A moved counter outranks the rest: the next round starts
/// from the new counter and so picks another seq and another predecessor anyway. A taken seq
/// outranks a failed predecessor check: the next round steps above the occupied seqs, whose top
/// one is then the predecessor, and it exists.
fn classify_append_cancellation(
    layout: TxLayout,
    codes: &[Option<&str>],
) -> Result<AppendCancellation, String> {
    const CCF: Option<&str> = Some("ConditionalCheckFailed");
    let at = |index: Option<usize>| index.and_then(|i| codes.get(i).copied().flatten());
    let counter = at(Some(TxLayout::COUNTER));
    let claim = at(layout.claim());
    let predecessor = at(layout.predecessor());
    let op = at(Some(layout.op()));
    if claim == CCF {
        return Ok(AppendCancellation::Claimed);
    }
    if counter == CCF {
        return Ok(AppendCancellation::CounterMoved);
    }
    if op == CCF {
        return Ok(AppendCancellation::SeqTaken);
    }
    if predecessor == CCF {
        return Ok(AppendCancellation::PredecessorFailed);
    }
    let codes = [counter, claim, predecessor, op];
    if codes.contains(&Some("TransactionConflict")) {
        return Ok(AppendCancellation::Conflict);
    }
    if codes.into_iter().any(is_retryable_code) {
        return Ok(AppendCancellation::Throttled);
    }
    Err(format!(
        "append transaction cancelled for a reason other than a moved counter, a taken claim or \
         seq, or a predecessor check (counter item code: {counter:?}, claim item code: {claim:?}, \
         predecessor check code: {predecessor:?}, op item code: {op:?})"
    ))
}

/// What a classified cancellation asks for next: an outcome right away, or one more read first.
#[derive(Debug, PartialEq, Eq)]
enum Settle {
    Outcome(PutOutcome),
    /// A taken claim: read it, then [`settle_claim`].
    ReadClaim,
    /// A taken seq: find the highest occupied seq above the counter.
    FindTop,
}

/// Map a cancellation to the next step. `hint` is the counter the failed counter check handed
/// back ([`counter_hint`]). Pure, so the mapping is unit-tested; the two reads it may ask for are
/// [`DynamoDbOpStore::try_append_op`]'s.
fn settle_cancellation(
    cancellation: AppendCancellation,
    slot: Slot,
    check: Predecessor,
    hint: Option<i64>,
) -> Settle {
    match cancellation {
        AppendCancellation::Claimed => Settle::ReadClaim,
        AppendCancellation::SeqTaken => Settle::FindTop,
        AppendCancellation::CounterMoved => Settle::Outcome(PutOutcome::Lost(hint)),
        AppendCancellation::PredecessorFailed => Settle::Outcome(PutOutcome::Predecessor {
            counter: slot.expected,
            seq: slot.seq,
            absent: check == Predecessor::Exists,
        }),
        // Another transaction holds the counter, the predecessor (the append below this one,
        // still writing its op) or the claim (a racing re-push of this very op): it is
        // committing. The next round reads the counter and, should that transaction have been
        // this op's own, finds the claim taken.
        AppendCancellation::Conflict => Settle::Outcome(PutOutcome::Lost(None)),
        AppendCancellation::Throttled => Settle::Outcome(PutOutcome::Throttled),
    }
}

/// What the claim read back after a taken claim means: the op's existing seq when it was written
/// for this op id, a foreign claim (store without one) when for another, and — read back absent —
/// a race with something that deletes claims (nothing does today): try again.
fn settle_claim(found: Option<(String, i64)>, op_id: &str) -> PutOutcome {
    match found {
        Some((claimed_for, stored)) if claimed_for == op_id => PutOutcome::Existing(stored),
        Some(_) => PutOutcome::ClaimForeign,
        None => PutOutcome::ClaimVanished,
    }
}

/// The counter item's value: `next_seq`, or 0 when the item or the attribute is absent (a stream
/// nothing has been appended to by a counter-aware build).
fn counter_value(item: Option<&HashMap<String, AttributeValue>>) -> StoreResult<i64> {
    match item {
        Some(item) if item.contains_key("next_seq") => parse_n(item, "next_seq"),
        _ => Ok(0),
    }
}

/// The counter a failed counter check handed back (`ALL_OLD`): `None` when DynamoDB returned no
/// item — the next round then reads it — otherwise its value as [`counter_value`] reads it.
fn counter_hint(item: Option<&HashMap<String, AttributeValue>>) -> StoreResult<Option<i64>> {
    item.map(|item| counter_value(Some(item))).transpose()
}

fn item_to_wire(item: &HashMap<String, AttributeValue>) -> StoreResult<WireOp> {
    Ok(WireOp {
        envelope_version: parse_n(item, "envelope_version")?,
        op_id: parse_s(item, "op_id")?,
        lamport: parse_n(item, "lamport")?,
        site: parse_n(item, "site")?,
        domain: parse_s(item, "domain")?,
        target_kind: parse_s(item, "target_kind")?,
        target_id: parse_s(item, "target_id")?,
        field: parse_s(item, "field")?,
        op_type: parse_s(item, "op_type")?,
        // Attribute is OMITTED entirely for None — an empty string is a legitimate
        // Some("") and must not collapse to None, so presence (not emptiness) decides.
        value: match item.get("value") {
            Some(v) => Some(
                v.as_s()
                    .map_err(|_| StoreError("attribute \"value\" is not a DynamoDB string".into()))?
                    .clone(),
            ),
            None => None,
        },
        author: parse_s(item, "author")?,
        wall_clock: parse_s(item, "wall_clock")?,
        // Like `value`, the attribute is omitted rather than written empty when there is nothing
        // to carry — and an item stored before the attribute existed reads the same way.
        extra: match item.get("extra") {
            Some(v) => {
                decode_extra(v.as_s().map_err(|_| {
                    StoreError("attribute \"extra\" is not a DynamoDB string".into())
                })?)?
            }
            None => Default::default(),
        },
    })
}

/// DynamoDB-backed [`OpStore`]. `Client` is cheap to clone (an `Arc` under the hood), so each
/// call clones it into its own `'static` future for the [`block_on`] bridge rather than
/// threading a lifetime through the async SDK calls.
pub struct DynamoDbOpStore {
    client: Client,
    table: String,
    /// Always `None` outside a build with the `test-seams` feature, which alone can set it.
    before_write: Option<BeforeWrite>,
}

/// The test seam's hook, see `DynamoDbOpStore::with_before_write`.
type BeforeWrite = std::sync::Arc<dyn Fn(&WriteAttempt) + Send + Sync>;

#[cfg(feature = "test-seams")]
pub use seam::WriteAttempt;
#[cfg(not(feature = "test-seams"))]
use seam::WriteAttempt;

/// The test seam's argument. Public only through the `test-seams` feature (the re-export above):
/// in a shipped build it is private to this file.
mod seam {
    /// One append attempt, as the test seam sees it right before the attempt sends its write.
    /// Without the feature nothing can install a hook, so nothing reads the fields.
    #[derive(Debug, Clone)]
    #[cfg_attr(not(feature = "test-seams"), allow(dead_code))]
    pub struct WriteAttempt {
        pub op_id: String,
        /// The seq the attempt takes.
        pub seq: i64,
        /// The counter value the attempt's condition expects.
        pub expected: i64,
        /// The predecessor check: `None` for seq 1, `Some(true)` for `attribute_exists` on
        /// `seq - 1`, `Some(false)` for `attribute_not_exists` (the flip over a legacy gap).
        pub predecessor: Option<bool>,
        /// Where `expected` came from: `"read"`, `"hint"` (handed back by a failed counter check),
        /// `"known"` (the previous round's counter condition held) or `"carried"` (the seq the
        /// previous op of the same push wrote).
        pub counter_from: &'static str,
    }
}

impl DynamoDbOpStore {
    /// See [`build_client`] for the constructor-argument rationale.
    pub fn new(
        table: impl Into<String>,
        endpoint: Option<String>,
        region: Option<String>,
    ) -> DynamoDbOpStore {
        let table = table.into();
        let client = block_on(build_client(endpoint, region));
        DynamoDbOpStore {
            client,
            table,
            before_write: None,
        }
    }

    /// Test seam (`6j6v.r3jf`), only in a build with the `test-seams` feature — which only this
    /// package's own tests turn on, so the relay binary and every embedder never carry it. `hook`
    /// is called on every append attempt with what the attempt is about to write, after it has
    /// decided its seq and before it sends the write. A test blocks in it to hold one append
    /// between those two points while another append commits, or acts as an older relay in that
    /// window. It runs on the bridge runtime inside `block_in_place`, so blocking in it is
    /// allowed.
    #[cfg(feature = "test-seams")]
    pub fn with_before_write(
        mut self,
        hook: impl Fn(&WriteAttempt) + Send + Sync + 'static,
    ) -> Self {
        self.before_write = Some(std::sync::Arc::new(hook));
        self
    }

    /// Read the stream's counter: the highest seq written, `0` when the stream has none yet. A
    /// strongly consistent `Query` on the counter's exact key rather than a `GetItem`, for the
    /// reason [`Self::read_claim`] gives: the op table's role needs no action it did not have.
    async fn read_counter(client: &Client, table: &str, stream_id: &str) -> StoreResult<i64> {
        let resp = client
            .query()
            .table_name(table)
            .key_condition_expression("stream_id = :sid AND seq = :counter")
            .expression_attribute_values(":sid", AttributeValue::S(stream_id.to_string()))
            .expression_attribute_values(":counter", AttributeValue::N(COUNTER_SEQ.to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(ddb_err)?;
        counter_value(resp.items().first())
    }

    /// The highest seq occupied above `counter`, or `None` when nothing is. ONE strongly
    /// consistent reverse `Query` with `Limit 1`, so a run of occupied seqs above the counter —
    /// of any length — costs one read, not one round per seq (review of PR #59, Integrity #4).
    async fn read_top_seq(
        client: &Client,
        table: &str,
        stream_id: &str,
        counter: i64,
    ) -> StoreResult<Option<i64>> {
        let resp = client
            .query()
            .table_name(table)
            .key_condition_expression("stream_id = :sid AND seq > :counter")
            .expression_attribute_values(":sid", AttributeValue::S(stream_id.to_string()))
            .expression_attribute_values(":counter", AttributeValue::N(counter.to_string()))
            .projection_expression("#seq")
            .expression_attribute_names("#seq", "seq")
            .scan_index_forward(false)
            .limit(1)
            .consistent_read(true)
            .send()
            .await
            .map_err(ddb_err)?;
        resp.items()
            .first()
            .map(|item| parse_n(item, "seq"))
            .transpose()
    }

    /// The counter's half of an append transaction: move `next_seq` to `slot.seq`, on the
    /// condition that it still stands at `slot.expected` (or, for `0`, that the stream has no
    /// counter yet). This condition orders the append transactions of a stream; the predecessor
    /// check ([`Self::predecessor_check`]) orders what a reader sees. On a failed check DynamoDB
    /// hands back the counter as it found it, so the next round needs no read.
    fn counter_update(table: &str, stream_id: &str, slot: Slot) -> StoreResult<Update> {
        let update = Update::builder()
            .table_name(table)
            .key("stream_id", AttributeValue::S(stream_id.to_string()))
            .key("seq", AttributeValue::N(COUNTER_SEQ.to_string()))
            .update_expression("SET next_seq = :seq")
            .expression_attribute_values(":seq", AttributeValue::N(slot.seq.to_string()))
            .expression_attribute_values(":expected", AttributeValue::N(slot.expected.to_string()))
            .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld);
        let update = if slot.expected == 0 {
            update.condition_expression("attribute_not_exists(next_seq) OR next_seq = :expected")
        } else {
            update.condition_expression("next_seq = :expected")
        };
        update.build().map_err(|e| StoreError(e.to_string()))
    }

    /// The predecessor check of an append transaction (module doc, "Sequence allocation"): a
    /// `ConditionCheck` on the op at `below` — `attribute_exists` normally, `attribute_not_exists`
    /// for the flip over a legacy gap. Either way the item is part of the transaction, so a
    /// transaction still writing it cancels this one with `TransactionConflict`. Needs the
    /// `dynamodb:ConditionCheckItem` IAM action.
    fn predecessor_check(
        table: &str,
        stream_id: &str,
        below: i64,
        check: Predecessor,
    ) -> StoreResult<ConditionCheck> {
        let condition = match check {
            Predecessor::Absent => "attribute_not_exists(stream_id)",
            Predecessor::Exists | Predecessor::None => "attribute_exists(stream_id)",
        };
        ConditionCheck::builder()
            .table_name(table)
            .key("stream_id", AttributeValue::S(stream_id.to_string()))
            .key("seq", AttributeValue::N(below.to_string()))
            .condition_expression(condition)
            .build()
            .map_err(|e| StoreError(e.to_string()))
    }

    /// The op's item at `seq`: one attribute per [`WireOp`] field (see the module doc).
    fn op_item(stream_id: &str, seq: i64, op: WireOp) -> HashMap<String, AttributeValue> {
        // Encoded before the field-by-field moves below consume `op`.
        let extra = encode_extra(&op);
        let mut item: HashMap<String, AttributeValue> = HashMap::new();
        item.insert("stream_id".into(), AttributeValue::S(stream_id.to_string()));
        item.insert("seq".into(), AttributeValue::N(seq.to_string()));
        item.insert(
            "envelope_version".into(),
            AttributeValue::N(op.envelope_version.to_string()),
        );
        item.insert("op_id".into(), AttributeValue::S(op.op_id));
        item.insert("lamport".into(), AttributeValue::N(op.lamport.to_string()));
        item.insert("site".into(), AttributeValue::N(op.site.to_string()));
        item.insert("domain".into(), AttributeValue::S(op.domain));
        item.insert("target_kind".into(), AttributeValue::S(op.target_kind));
        item.insert("target_id".into(), AttributeValue::S(op.target_id));
        item.insert("field".into(), AttributeValue::S(op.field));
        item.insert("op_type".into(), AttributeValue::S(op.op_type));
        // Omit the attribute entirely for None so Option round-trips; Some("") still
        // inserts an (empty) S attribute, so it is never confused with absence.
        if let Some(v) = op.value {
            item.insert("value".into(), AttributeValue::S(v));
        }
        item.insert("author".into(), AttributeValue::S(op.author));
        item.insert("wall_clock".into(), AttributeValue::S(op.wall_clock));
        // The envelope passthrough (6j6v.5crb). Omitted when the op carried nothing this build
        // does not know — the overwhelmingly common case — so a table written before the
        // attribute existed and one written after are the same shape for the same op.
        if !op.extra.is_empty() {
            item.insert("extra".into(), AttributeValue::S(extra));
        }
        item
    }

    /// One append round: ONE `TransactWriteItems` laid out as [`TxLayout`] says — the counter
    /// moved to `slot.seq` ([`Self::counter_update`]), the op's claim at [`claim_seq`] when
    /// `claimed`, the predecessor check per `check` ([`Self::predecessor_check`]), and the op at
    /// `slot.seq`. The claim and the op are each conditioned on their key being free —
    /// `attribute_not_exists(stream_id)`, which on a composite key reads "no item at this exact
    /// key", not "the stream is absent". So a seq is taken by exactly the transaction that also
    /// writes its op, only after the transaction that wrote the seq below, and not before that op
    /// is written (`6j6v.r3jf`).
    ///
    /// A taken claim means the op is already stored, so this answers with the seq the claim
    /// recorded — unless the claim was written for a different op id, in which case the next
    /// round stores the op without one.
    async fn try_append_op(
        client: &Client,
        table: &str,
        stream_id: &str,
        slot: Slot,
        check: Predecessor,
        op: &WireOp,
        claimed: bool,
    ) -> StoreResult<PutOutcome> {
        let claim = claim_seq(&op.op_id);
        let layout = TxLayout {
            claimed,
            checked: check != Predecessor::None,
        };
        let mut tx = client.transact_write_items().transact_items(
            TransactWriteItem::builder()
                .update(Self::counter_update(table, stream_id, slot)?)
                .build(),
        );
        if claimed {
            // `build()` can only fail on a required field left unset, and every one is set here;
            // mapped rather than unwrapped for the reason `try_claim` gives.
            let claim_put = Put::builder()
                .table_name(table)
                .item("stream_id", AttributeValue::S(stream_id.to_string()))
                .item("seq", AttributeValue::N(claim.to_string()))
                .item("op_id", AttributeValue::S(op.op_id.clone()))
                .item("op_seq", AttributeValue::N(slot.seq.to_string()))
                .condition_expression("attribute_not_exists(stream_id)")
                .build()
                .map_err(|e| StoreError(e.to_string()))?;
            tx = tx.transact_items(TransactWriteItem::builder().put(claim_put).build());
        }
        if let (true, Some(below)) = (layout.checked, slot.predecessor()) {
            let check = Self::predecessor_check(table, stream_id, below, check)?;
            tx = tx.transact_items(TransactWriteItem::builder().condition_check(check).build());
        }
        let op_put = Put::builder()
            .table_name(table)
            .set_item(Some(Self::op_item(stream_id, slot.seq, op.clone())))
            .condition_expression("attribute_not_exists(stream_id)")
            .build()
            .map_err(|e| StoreError(e.to_string()))?;
        tx = tx.transact_items(TransactWriteItem::builder().put(op_put).build());

        let reasons = match tx.send().await {
            Ok(_) => return Ok(PutOutcome::Written(slot.seq)),
            Err(err) => match err.into_service_error() {
                TransactWriteItemsError::TransactionCanceledException(e) => {
                    e.cancellation_reasons().to_vec()
                }
                other => return Err(ddb_err(other)),
            },
        };
        let codes: Vec<Option<&str>> = reasons.iter().map(|r| r.code()).collect();
        let cancellation = match classify_append_cancellation(layout, &codes) {
            Ok(cancellation) => cancellation,
            Err(msg) => {
                // The service's own sentences (an item past 400 KB, say) are carried along:
                // the codes alone do not say which validation failed.
                let detail = reasons
                    .iter()
                    .filter_map(|r| r.message())
                    .collect::<Vec<_>>()
                    .join("; ");
                let message = format!("append: {msg}: {detail} (stream {stream_id:?})");
                eprintln!("nxf-relay: dynamodb error: {message}");
                return Err(StoreError(message));
            }
        };
        let hint = counter_hint(reasons.first().and_then(|r| r.item()))?;
        match settle_cancellation(cancellation, slot, check, hint) {
            Settle::Outcome(outcome) => Ok(outcome),
            Settle::ReadClaim => {
                let found = Self::read_claim(client, table, stream_id, claim).await?;
                let outcome = settle_claim(found.clone(), &op.op_id);
                if let (PutOutcome::ClaimForeign, Some((claimed_for, _))) = (&outcome, found) {
                    eprintln!(
                        "nxf-relay: op {:?} shares its claim key {claim} with op \
                         {claimed_for:?} in stream {stream_id:?}; stored without a claim, \
                         so a re-push of it is stored again",
                        op.op_id
                    );
                }
                Ok(outcome)
            }
            Settle::FindTop => {
                // The counter held (its condition passed), so it stands at `slot.expected`, and
                // the next round needs no read of it.
                let top = Self::read_top_seq(client, table, stream_id, slot.expected).await?;
                Ok(PutOutcome::SeqTaken {
                    counter: slot.expected,
                    top: top.unwrap_or(slot.seq).max(slot.seq),
                })
            }
        }
    }

    /// Read the claim at `claim`: the op id it was written for and the seq that op is stored
    /// at. A strongly consistent `Query` on the exact key rather than a `GetItem`, so the op
    /// table's role needs no action it did not already have.
    async fn read_claim(
        client: &Client,
        table: &str,
        stream_id: &str,
        claim: i64,
    ) -> StoreResult<Option<(String, i64)>> {
        let resp = client
            .query()
            .table_name(table)
            .key_condition_expression("stream_id = :sid AND seq = :claim")
            .expression_attribute_values(":sid", AttributeValue::S(stream_id.to_string()))
            .expression_attribute_values(":claim", AttributeValue::N(claim.to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(ddb_err)?;
        match resp.items().first() {
            Some(item) => Ok(Some((parse_s(item, "op_id")?, parse_n(item, "op_seq")?))),
            None => Ok(None),
        }
    }

    /// Append one op: rounds of [`Self::try_append_op`] under [`append_with_retry`], starting
    /// from `start` (what the caller already knows of the counter) and stopping at `deadline`.
    /// Uncontended it runs once: one transaction, plus one read when `start` is unknown.
    async fn append_one(
        client: &Client,
        table: &str,
        stream_id: &str,
        op: &WireOp,
        start: Counter,
        deadline: Option<Instant>,
        before_write: Option<&BeforeWrite>,
    ) -> StoreResult<Settled> {
        append_with_retry(stream_id, start, deadline, move |round| async move {
            let counter = match round.counter {
                Counter::Unknown => Self::read_counter(client, table, stream_id).await?,
                Counter::Hint(c) | Counter::Known(c) | Counter::Carried(c) => c,
            };
            let slot = Slot::after(counter, round.floor)?;
            let check = round.predecessor_check(slot);
            if let Some(hook) = before_write {
                let attempt = WriteAttempt {
                    op_id: op.op_id.clone(),
                    seq: slot.seq,
                    expected: slot.expected,
                    predecessor: match check {
                        Predecessor::None => None,
                        Predecessor::Exists => Some(true),
                        Predecessor::Absent => Some(false),
                    },
                    counter_from: round.counter.source(),
                };
                tokio::task::block_in_place(|| hook(&attempt));
            }
            Self::try_append_op(client, table, stream_id, slot, check, op, round.claimed).await
        })
        .await
    }
}

impl OpStore for DynamoDbOpStore {
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        let op = op.clone();
        let before_write = self.before_write.clone();
        block_on(async move {
            let settled = Self::append_one(
                &client,
                &table,
                &stream_id,
                &op,
                Counter::Unknown,
                None,
                before_write.as_ref(),
            )
            .await?;
            Ok(Cursor(settled.seq()))
        })
    }

    /// The push path: [`OpStore::append_batch`]'s contract, with the deadline also checked before
    /// every round and raced against the round in flight, and the seq each written op took
    /// carried to the next op as the counter to expect — so an uncontended push of `n` ops costs
    /// one read and `n` transactions. A wrong guess (another writer appended in between) costs one
    /// lost round, which hands back the real counter.
    fn append_batch(
        &self,
        stream: &StreamId,
        ops: &[WireOp],
        deadline: Instant,
    ) -> StoreResult<()> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream = stream.clone();
        let ops = ops.to_vec();
        let before_write = self.before_write.clone();
        block_on(async move {
            let stream_id = stream.as_str();
            let mut start = Counter::Unknown;
            for (done, op) in ops.iter().enumerate() {
                if Instant::now() >= deadline {
                    return Err(past_deadline(&stream, done, ops.len()));
                }
                let settled = Self::append_one(
                    &client,
                    &table,
                    stream_id,
                    op,
                    start,
                    Some(deadline),
                    before_write.as_ref(),
                )
                .await;
                match settled {
                    Ok(Settled::Written(seq)) => start = Counter::Carried(seq),
                    // An op already stored says nothing new about the counter.
                    Ok(Settled::Existing(_)) => {}
                    Err(_) if Instant::now() >= deadline => {
                        return Err(past_deadline(&stream, done, ops.len()));
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })
    }

    fn read_since(
        &self,
        stream: &StreamId,
        cursor: Cursor,
        limit: usize,
    ) -> StoreResult<(Vec<WireOp>, Cursor)> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        // Clamp the cursor to >= 0 and the page to MAX_PAGE — same guards as the other two
        // backends, reused via the shared helper rather than re-derived.
        let since = cursor.0.max(0);
        let page = page_limit(limit);
        block_on(async move {
            let mut ops = Vec::new();
            let mut next = cursor;
            let mut start_key: Option<HashMap<String, AttributeValue>> = None;

            // A `Query` is NOT a SQL `LIMIT n`: DynamoDB stops at 1 MB of read data and
            // returns fewer items than `Limit` together with a `LastEvaluatedKey`. Ops big
            // enough to trip the cap are the expected case, not a pathology — the prose body
            // rides the same LWW-longtext substrate field, so a 500-op page averaging over
            // ~2 KB per op already exceeds 1 MB. So continue from the key until the page is
            // full or the stream really is exhausted, which is exactly what SQLite and Postgres
            // do for free; nothing above this line may be able to tell the backends apart.
            //
            // This is now page-SIZE parity, not correctness: the client's pull loop stops on a
            // cursor that stands still, not on a short page (invariant 4, `6j6v.xsf3`), so a
            // short page would cost an extra round-trip rather than silently end the pass. Left
            // in place because that parity is worth keeping — an ACL filter (E4) will make short
            // pages ordinary, and the backends should not differ in how many of them they serve.
            while (ops.len() as i64) < page {
                let remaining = page - ops.len() as i64;
                let resp = client
                    .query()
                    .table_name(&table)
                    .key_condition_expression("stream_id = :sid AND seq > :since")
                    .expression_attribute_values(":sid", AttributeValue::S(stream_id.clone()))
                    .expression_attribute_values(":since", AttributeValue::N(since.to_string()))
                    .scan_index_forward(true)
                    .limit(remaining as i32)
                    .set_exclusive_start_key(start_key.take())
                    // Strongly consistent: the relay's own pull path must see an append it
                    // just made, and eventual consistency would make the suite flaky too.
                    .consistent_read(true)
                    .send()
                    .await
                    .map_err(ddb_err)?;

                for item in resp.items() {
                    next = Cursor(parse_n(item, "seq")?);
                    ops.push(item_to_wire(item)?);
                }

                // An ABSENT (or empty) LastEvaluatedKey is DynamoDB's only "that was
                // everything" signal — an item count below `Limit` is not one.
                match resp.last_evaluated_key() {
                    Some(key) if !key.is_empty() => start_key = Some(key.clone()),
                    _ => break,
                }
            }
            Ok((ops, next))
        })
    }
}

// ----- DynamoDbPrefixRegistry: claim-if-free via one TransactWriteItems ---------------

/// The claim-if-free key: does `prefix` already have an owner in this stream?
fn prefix_sk(prefix: &str) -> String {
    format!("prefix#{prefix}")
}

/// The idempotent "what did this replica get?" lookup key — DynamoDB's stand-in for
/// SQLite/Postgres' `UNIQUE (stream_id, replica_uuid)`.
fn uuid_sk(replica_uuid: &str) -> String {
    format!("uuid#{replica_uuid}")
}

/// Outcome of one [`DynamoDbPrefixRegistry::try_claim`] attempt. `PrefixTaken` is the
/// ordinary claim-if-free collision (someone else owns this candidate; try the next one).
/// `UuidRace` is the one race `register` must not resolve by trying another candidate — see
/// `try_claim`'s doc. `Retryable` is contention or capacity, not an answer at all: re-run the
/// SAME claim (see [`CLAIM_ATTEMPTS`]). `PartialEq`/`Debug` exist for
/// `classify_cancellation`'s unit tests.
#[derive(Debug, PartialEq, Eq)]
enum ClaimOutcome {
    Claimed,
    PrefixTaken,
    UuidRace,
    Retryable,
}

/// Bound on re-running the SAME claim after a [`ClaimOutcome::Retryable`] cancellation.
/// Small, because these clear in milliseconds or not at all: the contention window is one
/// `TransactWriteItems`, and a bound that lets a genuinely saturated table stall a request
/// for seconds is worse than a loud failure the client can retry on its own schedule.
const CLAIM_ATTEMPTS: u32 = 5;

/// Backoff before re-running a claim: 5, 10, 20, 40 ms. Doubling (rather than a fixed sleep)
/// is what stops N racers that all conflicted from re-colliding in lockstep on every round;
/// the absolute values stay small because [`CLAIM_ATTEMPTS`] is the real ceiling.
fn claim_backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(5u64 << attempt.min(3))
}

/// Cancellation reason codes that mean "this transaction lost to concurrency or capacity",
/// not "this transaction is wrong". `TransactionConflict` in particular is routine: AWS
/// returns it whenever two `TransactWriteItems` touch the same item concurrently — exactly
/// what N replicas first-syncing against one offline-minted prefix produce — and the
/// documented remedy is to retry the whole transaction. It has to be handled HERE because
/// the AWS SDK's default retry classifier does not auto-retry `TransactionCanceledException`
/// at all: from the outside it cannot distinguish a legitimate condition failure (which must
/// never be retried) from a conflict (which must be). DynamoDB Local serialises transactions
/// and so effectively never emits these, which is why they are pinned by unit test rather
/// than by the acceptance suite.
fn is_retryable_code(code: Option<&str>) -> bool {
    matches!(
        code,
        Some("TransactionConflict" | "ThrottlingError" | "ProvisionedThroughputExceeded")
    )
}

/// Classify a cancelled `TransactWriteItems`' positional per-item status codes (index 0 =
/// the prefix item, index 1 = the uuid item — see `try_claim`'s doc for the full picture).
/// Pulled out as a pure function over plain `&str` codes, rather than inlined into
/// `try_claim`, specifically so this three-way boundary can be unit-tested directly:
/// DynamoDB Local has no reliable way to provoke a `TransactionConflict` /
/// `ProvisionedThroughputExceeded` / `ThrottlingError` / `ValidationError` cancellation on
/// demand, but those are real possibilities under the exact contention this code exists for.
/// None of them may be silently folded into "prefix taken" (which would steer `register` at
/// a wrong candidate, or exhaust `REASSIGN_ATTEMPTS` with a misleading error), and the
/// transient ones must not be reported as hard failures either — on real AWS that turns
/// routine first-sync contention into a 500 for the client's whole sync pass.
///
/// A reported `ConditionalCheckFailed` is checked first and outranks everything else,
/// because it is definitive: DynamoDB emits it only for an item whose condition genuinely
/// evaluated false, so the answer will not change on a retry. Within that, the uuid item
/// takes priority even when the prefix item ALSO failed — see `try_claim`'s doc for why a
/// self-race must never be treated as an ordinary collision. Only once neither condition
/// failed does a transient code ([`is_retryable_code`]) make this a [`ClaimOutcome::Retryable`].
/// Everything left over — a `ValidationError`, or no recognised code at all — is a hard
/// error, matching `classify_append_cancellation`'s stricter pattern above: `register` must surface a real
/// fault rather than misread it as "try another candidate" or spin on it.
fn classify_cancellation(
    prefix_item_code: Option<&str>,
    uuid_item_code: Option<&str>,
) -> Result<ClaimOutcome, String> {
    if uuid_item_code == Some("ConditionalCheckFailed") {
        return Ok(ClaimOutcome::UuidRace);
    }
    if prefix_item_code == Some("ConditionalCheckFailed") {
        return Ok(ClaimOutcome::PrefixTaken);
    }
    if is_retryable_code(prefix_item_code) || is_retryable_code(uuid_item_code) {
        return Ok(ClaimOutcome::Retryable);
    }
    Err(format!(
        "transaction cancelled for a reason other than an ordinary claim-if-free collision \
         (prefix item code: {prefix_item_code:?}, uuid item code: {uuid_item_code:?})"
    ))
}

/// Re-run one claim while it comes back [`ClaimOutcome::Retryable`], bounded by
/// [`CLAIM_ATTEMPTS`] with a doubling [`claim_backoff`]. Generic over the attempt for the
/// same reason [`append_with_retry`] is: DynamoDB Local serialises transactions and so never
/// emits the transient cancellation codes that reach this branch, leaving both the retry and
/// the exhaustion error untestable against a live endpoint. A stand-in attempt reaches both.
async fn retry_claim<F, Fut>(
    stream_id: &str,
    prefix: &str,
    replica_uuid: &str,
    mut attempt_claim: F,
) -> StoreResult<ClaimOutcome>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = StoreResult<ClaimOutcome>>,
{
    for attempt in 0..CLAIM_ATTEMPTS {
        match attempt_claim().await? {
            ClaimOutcome::Retryable => tokio::time::sleep(claim_backoff(attempt)).await,
            settled => return Ok(settled),
        }
    }
    Err(StoreError(format!(
        "register: {CLAIM_ATTEMPTS} claim attempts for prefix {prefix:?} (stream \
         {stream_id:?}, replica {replica_uuid:?}) were all cancelled by transaction \
         conflict or throughput limits"
    )))
}

/// Wall-clock ceiling on ONE `register` call, checked between reassignment candidates.
///
/// [`REASSIGN_ATTEMPTS`] is 100_000 — a sane bound when a candidate costs a SQLite row read,
/// and a wildly unsafe one when each costs a network round-trip that [`CLAIM_ATTEMPTS`] may
/// itself retry five times over. Multiplied out, a single unauthenticated `register` request
/// could issue hundreds of thousands of sequential round-trips and never return. The attempt
/// count stays as it is (it is shared with the other backends, and the "prefix space
/// exhausted" error must stay identical across them); this budget is the bound that actually
/// matters for a backend whose candidates cost milliseconds each, and it is independent of
/// both counts. Exceeding it is a loud error naming the budget: the client's own retry is the
/// right layer for a stream this contended, not an open-ended HTTP request.
const REGISTER_BUDGET: Duration = Duration::from_secs(5);

/// DynamoDB-backed [`PrefixRegistry`]. Claim-if-free with the exact observable
/// outcomes of `SqlitePrefixRegistry` / `PostgresPrefixRegistry` — read/write ordering
/// mirrors both step for step; only the mechanism differs (a `TransactWriteItems` here
/// instead of a mutex or an advisory lock).
pub struct DynamoDbPrefixRegistry {
    client: Client,
    table: String,
}

impl DynamoDbPrefixRegistry {
    /// A separate registry table/client rather than reusing the op store's: the two tables
    /// are provisioned independently and have unrelated lifecycles. See [`build_client`]
    /// (shared with [`DynamoDbOpStore::new`]) for the constructor-argument rationale.
    pub fn new(
        table: impl Into<String>,
        endpoint: Option<String>,
        region: Option<String>,
    ) -> DynamoDbPrefixRegistry {
        let table = table.into();
        let client = block_on(build_client(endpoint, region));
        DynamoDbPrefixRegistry { client, table }
    }

    /// Step 1: the idempotent "what did this replica get?" lookup by `uuid#<replica_uuid>`.
    /// Strongly consistent so a replica that just won a claim sees it on its very next
    /// call — required both for crash-safety retries and for `resolve_uuid_race` below.
    async fn owned_prefix(
        client: &Client,
        table: &str,
        stream_id: &str,
        replica_uuid: &str,
    ) -> StoreResult<Option<String>> {
        let resp = client
            .get_item()
            .table_name(table)
            .key("stream_id", AttributeValue::S(stream_id.to_string()))
            .key("sk", AttributeValue::S(uuid_sk(replica_uuid)))
            .consistent_read(true)
            .send()
            .await
            .map_err(ddb_err)?;
        match resp.item() {
            Some(item) => Ok(Some(parse_s(item, "prefix")?)),
            None => Ok(None),
        }
    }

    /// Attempt to claim `prefix` for `replica_uuid` with ONE `TransactWriteItems` writing
    /// BOTH the `prefix#<prefix>` and `uuid#<replica_uuid>` items. Atomicity is the entire
    /// reason this is a transaction and not two `PutItem`s: either both land or neither
    /// does, so a replica can never end up owning a prefix with no uuid-lookup item — which
    /// would make `owned_prefix` miss it on the replica's very next (idempotent) register.
    /// Each `Put` is conditioned on `attribute_not_exists(stream_id)`, evaluated against the
    /// item at THAT put's own `(stream_id, sk)` key — "no item here yet", not "stream_id is
    /// absent from the table" (mirrors `try_append_op` in the op store above).
    ///
    /// A cancelled transaction MIGHT mean someone else won a race on one of the two keys — or
    /// it might be transient contention (a conflict, a throttle), or a genuine fault (a
    /// validation failure), all of which arrive wrapped in the same
    /// `TransactionCanceledException` shape. `TransactWriteItemsError` is matched on its
    /// typed variant, never a `Display` substring, and the per-item `cancellation_reasons`
    /// (positional: index 0 = the prefix item, index 1 = the uuid item) are handed to
    /// [`classify_cancellation`] — see its doc for the full three-way decision, and
    /// [`Self::claim_with_retry`] for what absorbs the transient case. Note that this method
    /// reports [`ClaimOutcome::Retryable`] rather than retrying itself, so it stays exactly
    /// one round-trip and the retry policy lives in one place.
    /// The two collision outcomes: if ONLY the prefix item's condition failed, this candidate
    /// is simply taken — the ordinary claim-if-free collision `register` retries with the
    /// next candidate. If the UUID item's condition failed, a *concurrent register call for
    /// this SAME replica* won first (racing itself, e.g. two retries in flight at once) —
    /// trying another candidate here would give this one replica a second, aliasing claim, so
    /// `register` must not do that; it re-runs step 1 instead (`resolve_uuid_race`).
    /// Misclassifying a uuid-item loss as an ordinary `PrefixTaken` is worse than a wrong
    /// answer: the uuid item stays claimed forever (the winner owns it), so EVERY subsequent
    /// candidate's uuid `Put` would also lose, and `register` would grind through all of
    /// [`REASSIGN_ATTEMPTS`] before giving up — confirmed by temporarily forcing this
    /// misclassification, which turned
    /// `concurrent_registers_by_the_same_replica_converge_on_one_stable_answer_dynamodb`
    /// from an 8-second pass into a run that did not finish in two minutes.
    async fn try_claim(
        client: &Client,
        table: &str,
        stream_id: &str,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<ClaimOutcome> {
        // These two `build()` calls are the one place a `StoreError` is constructed inline
        // instead of going through `ddb_err`. Not a second convention: `BuildError` is a
        // builder-shape error (a required field left unset), not a service error, and it does
        // NOT implement `Into<aws_sdk_dynamodb::Error>`, so `ddb_err`'s bound does not admit
        // it. It also cannot happen at runtime here — every required field is set literally
        // above — which is why it is mapped rather than logged like a real fault.
        let prefix_put = Put::builder()
            .table_name(table)
            .item("stream_id", AttributeValue::S(stream_id.to_string()))
            .item("sk", AttributeValue::S(prefix_sk(prefix)))
            .item("replica_uuid", AttributeValue::S(replica_uuid.to_string()))
            .condition_expression("attribute_not_exists(stream_id)")
            .build()
            .map_err(|e| StoreError(e.to_string()))?;
        let uuid_put = Put::builder()
            .table_name(table)
            .item("stream_id", AttributeValue::S(stream_id.to_string()))
            .item("sk", AttributeValue::S(uuid_sk(replica_uuid)))
            .item("prefix", AttributeValue::S(prefix.to_string()))
            .condition_expression("attribute_not_exists(stream_id)")
            .build()
            .map_err(|e| StoreError(e.to_string()))?;

        let result = client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().put(prefix_put).build())
            .transact_items(TransactWriteItem::builder().put(uuid_put).build())
            .send()
            .await;

        match result {
            Ok(_) => Ok(ClaimOutcome::Claimed),
            Err(err) => match err.into_service_error() {
                TransactWriteItemsError::TransactionCanceledException(e) => {
                    let reasons = e.cancellation_reasons();
                    let prefix_item_code = reasons.first().and_then(|r| r.code());
                    let uuid_item_code = reasons.get(1).and_then(|r| r.code());
                    classify_cancellation(prefix_item_code, uuid_item_code).map_err(|msg| {
                        StoreError(format!(
                            "register: {msg} (stream {stream_id:?}, candidate prefix \
                             {prefix:?}, replica {replica_uuid:?})"
                        ))
                    })
                }
                other => Err(ddb_err(other)),
            },
        }
    }

    /// [`Self::try_claim`], with [`ClaimOutcome::Retryable`] absorbed: re-run the SAME claim,
    /// bounded by [`CLAIM_ATTEMPTS`] with a doubling [`claim_backoff`]. Every caller wants
    /// this — a transient cancellation carries no information about the candidate — so the
    /// retry lives here rather than being repeated at each call site, and `try_claim` stays a
    /// single honest round-trip. Exhausting the bound is a loud `StoreError`: at that point
    /// the table really is saturated and the client's own retry is the right layer to handle
    /// it, not an unbounded spin inside one HTTP request.
    async fn claim_with_retry(
        client: &Client,
        table: &str,
        stream_id: &str,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<ClaimOutcome> {
        retry_claim(stream_id, prefix, replica_uuid, || {
            let (client, table, stream_id, prefix, replica_uuid) =
                (client, table, stream_id, prefix, replica_uuid);
            async move { Self::try_claim(client, table, stream_id, prefix, replica_uuid).await }
        })
        .await
    }

    /// Resolve a [`ClaimOutcome::UuidRace`]: re-run step 1 rather than trying another
    /// candidate (see `try_claim`'s doc for why). The winner's write is already visible
    /// under strongly consistent reads, so this cannot itself race further.
    async fn resolve_uuid_race(
        client: &Client,
        table: &str,
        stream_id: &str,
        requested_prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        Self::owned_prefix(client, table, stream_id, replica_uuid)
            .await?
            .map(|existing| stable_outcome(existing, requested_prefix))
            .ok_or_else(|| {
                StoreError(format!(
                    "register: uuid item for replica {replica_uuid:?} vanished after a \
                     transaction cancellation reported it as taken (stream {stream_id:?})"
                ))
            })
    }
}

/// `existing` is this replica's actual claim (from `owned_prefix`); comparing it against
/// what it originally asked for produces the same stable answer step 1 returns in
/// `SqlitePrefixRegistry` / `PostgresPrefixRegistry`.
fn stable_outcome(existing: String, requested_prefix: &str) -> RegisterOutcome {
    if existing == requested_prefix {
        RegisterOutcome::Registered
    } else {
        RegisterOutcome::Reassigned {
            new_prefix: existing,
        }
    }
}

impl PrefixRegistry for DynamoDbPrefixRegistry {
    fn presence(&self) -> Option<&dyn PresenceStore> {
        Some(self)
    }

    fn stream_lease(&self) -> Option<&dyn StreamLease> {
        Some(self)
    }

    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        let prefix = prefix.to_string();
        let replica_uuid = replica_uuid.to_string();
        block_on(async move {
            let started = Instant::now();

            // 1. Already registered? Same stable, idempotent answer as the other backends
            //    (crash-safety: survives a client that lost its adopted prefix).
            if let Some(existing) =
                Self::owned_prefix(&client, &table, &stream_id, &replica_uuid).await?
            {
                return Ok(stable_outcome(existing, &prefix));
            }

            // 2. Try the requested prefix.
            match Self::claim_with_retry(&client, &table, &stream_id, &prefix, &replica_uuid)
                .await?
            {
                ClaimOutcome::Claimed => return Ok(RegisterOutcome::Registered),
                ClaimOutcome::UuidRace => {
                    return Self::resolve_uuid_race(
                        &client,
                        &table,
                        &stream_id,
                        &prefix,
                        &replica_uuid,
                    )
                    .await;
                }
                // `claim_with_retry` never hands back `Retryable` — it either settles or
                // fails — so only the ordinary collision reaches step 3.
                ClaimOutcome::PrefixTaken | ClaimOutcome::Retryable => {}
            }

            // 3. Collision: walk candidate_prefix(replica_uuid, attempt) — imported from
            //    crate::registry, NOT re-derived, so a reassignment lands on the exact same
            //    value SQLite/Postgres compute for the same inputs. That shared derivation
            //    is what makes cross-backend parity value-for-value rather than merely
            //    shape-for-shape.
            for attempt in 0..REASSIGN_ATTEMPTS {
                // See REGISTER_BUDGET: the attempt count alone does not bound a
                // network-bound walk, so the wall clock does.
                if started.elapsed() >= REGISTER_BUDGET {
                    return Err(StoreError(format!(
                        "register: exceeded the {REGISTER_BUDGET:?} budget for stream \
                         {stream_id:?} after {attempt} reassignment candidates (replica \
                         {replica_uuid:?}); the stream is too contended to settle inside one \
                         request — retry"
                    )));
                }
                let cand = candidate_prefix(&replica_uuid, attempt);
                match Self::claim_with_retry(&client, &table, &stream_id, &cand, &replica_uuid)
                    .await?
                {
                    ClaimOutcome::Claimed => {
                        return Ok(RegisterOutcome::Reassigned { new_prefix: cand });
                    }
                    ClaimOutcome::UuidRace => {
                        return Self::resolve_uuid_race(
                            &client,
                            &table,
                            &stream_id,
                            &prefix,
                            &replica_uuid,
                        )
                        .await;
                    }
                    // See step 2: `claim_with_retry` has already absorbed `Retryable`.
                    ClaimOutcome::PrefixTaken | ClaimOutcome::Retryable => continue,
                }
            }
            Err(StoreError(format!(
                "prefix space exhausted for stream {stream_id} after {REASSIGN_ATTEMPTS} attempts"
            )))
        })
    }
}

// ----- the stream lease (6j6v.f6yb design §7a) --------------------------------------------------

/// The partition of the per-stream lease items (and of 5vxp-a's global `lease` item, which the
/// purge never reads or writes).
pub const SWEEP_PARTITION: &str = "#sweep";

/// The lease item's sort key for one stream. `validate_purge_id` keeps ids at 1,000 bytes so this
/// stays inside DynamoDB's 1,024-byte sort key (MF-R3-4).
pub fn lease_sk(stream_id: &str) -> String {
    format!("stream#{stream_id}")
}

fn lease_item(item: &HashMap<String, AttributeValue>) -> StoreResult<LeaseItem> {
    Ok(LeaseItem {
        holder: parse_s(item, "holder")?,
        expires_at: parse_n(item, "expires_at")?,
    })
}

fn unix_secs(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl DynamoDbPrefixRegistry {
    /// One conditional `PutItem` of the lease item; `Ok(Err(item))` when the condition failed
    /// (the item as it stood, from `ReturnValuesOnConditionCheckFailure`).
    async fn put_lease(
        client: &Client,
        table: &str,
        stream_id: &str,
        holder: &str,
        expires_at: i64,
        condition: &str,
        values: Vec<(&str, AttributeValue)>,
    ) -> StoreResult<Result<Option<LeaseItem>, Option<LeaseItem>>> {
        use aws_sdk_dynamodb::operation::put_item::PutItemError;
        let mut put = client
            .put_item()
            .table_name(table)
            .item("stream_id", AttributeValue::S(SWEEP_PARTITION.into()))
            .item("sk", AttributeValue::S(lease_sk(stream_id)))
            .item("holder", AttributeValue::S(holder.into()))
            .item("expires_at", AttributeValue::N(expires_at.to_string()))
            .condition_expression(condition)
            .return_values(ReturnValue::AllOld)
            .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld);
        for (name, value) in values {
            put = put.expression_attribute_values(name, value);
        }
        match put.send().await {
            Ok(out) => Ok(Ok(out.attributes().map(lease_item).transpose()?)),
            Err(e) => match e.into_service_error() {
                PutItemError::ConditionalCheckFailedException(c) => {
                    Ok(Err(c.item().map(lease_item).transpose()?))
                }
                other => Err(ddb_code_err(other)),
            },
        }
    }
}

impl StreamLease for DynamoDbPrefixRegistry {
    fn take(
        &self,
        stream: &StreamId,
        holder: &str,
        ttl: Duration,
        now: std::time::SystemTime,
    ) -> StoreResult<LeaseTaken> {
        let (client, table) = (self.client.clone(), self.table.clone());
        let (stream_id, holder) = (stream.as_str().to_string(), holder.to_string());
        let now_s = unix_secs(now);
        let expires_at = unix_secs(now + ttl);
        block_on(async move {
            let put = Self::put_lease(
                &client,
                &table,
                &stream_id,
                &holder,
                expires_at,
                "attribute_not_exists(sk) OR expires_at < :now",
                vec![(":now", AttributeValue::N(now_s.to_string()))],
            )
            .await?;
            Ok(match put {
                Ok(previous) => LeaseTaken::Taken {
                    expires_at,
                    previous,
                },
                Err(Some(held)) => LeaseTaken::Held(held),
                Err(None) => {
                    return Err(StoreError(
                        "dynamodb error: the lease condition failed without an item".into(),
                    ))
                }
            })
        })
    }

    fn renew(
        &self,
        stream: &StreamId,
        holder: &str,
        ttl: Duration,
        now: std::time::SystemTime,
    ) -> StoreResult<LeaseRenewed> {
        let (client, table) = (self.client.clone(), self.table.clone());
        let (stream_id, holder) = (stream.as_str().to_string(), holder.to_string());
        let expires_at = unix_secs(now + ttl);
        block_on(async move {
            let put = Self::put_lease(
                &client,
                &table,
                &stream_id,
                &holder,
                expires_at,
                "holder = :me",
                vec![(":me", AttributeValue::S(holder.clone()))],
            )
            .await?;
            Ok(match put {
                Ok(_) => LeaseRenewed::Renewed { expires_at },
                Err(_) => LeaseRenewed::Lost,
            })
        })
    }

    fn release(&self, stream: &StreamId, holder: &str) -> StoreResult<bool> {
        use aws_sdk_dynamodb::operation::delete_item::DeleteItemError;
        let (client, table) = (self.client.clone(), self.table.clone());
        let (stream_id, holder) = (stream.as_str().to_string(), holder.to_string());
        block_on(async move {
            let deleted = client
                .delete_item()
                .table_name(&table)
                .key("stream_id", AttributeValue::S(SWEEP_PARTITION.into()))
                .key("sk", AttributeValue::S(lease_sk(&stream_id)))
                .condition_expression("holder = :me")
                .expression_attribute_values(":me", AttributeValue::S(holder))
                .send()
                .await;
            match deleted {
                Ok(_) => Ok(true),
                Err(e) => match e.into_service_error() {
                    DeleteItemError::ConditionalCheckFailedException(_) => Ok(false),
                    other => Err(ddb_code_err(other)),
                },
            }
        })
    }

    fn read(&self, stream: &StreamId) -> StoreResult<Option<LeaseItem>> {
        let (client, table) = (self.client.clone(), self.table.clone());
        let stream_id = stream.as_str().to_string();
        block_on(async move {
            let resp = client
                .query()
                .table_name(&table)
                .key_condition_expression("stream_id = :p AND sk = :sk")
                .expression_attribute_values(":p", AttributeValue::S(SWEEP_PARTITION.into()))
                .expression_attribute_values(":sk", AttributeValue::S(lease_sk(&stream_id)))
                .consistent_read(true)
                .send()
                .await
                .map_err(ddb_code_err)?;
            resp.items().first().map(lease_item).transpose()
        })
    }
}

// ----- presence (6j6v.f0b5): a third item kind in the registry table ---------------------------

/// The presence item key for one machine.
fn machine_sk(machine_id: &str) -> String {
    format!("{MACHINE_SK_PREFIX}{machine_id}")
}

const MACHINE_SK_PREFIX: &str = "machine#";

/// How many `Query` pages one presence read follows at most. Each is up to 1 MB — thousands of
/// machines — so an honest stream never gets near it. It exists because the route is
/// unauthenticated: without it, a stream flooded with invented machine ids would turn every read
/// into an unbounded walk. Reaching it marks the answer `truncated`, so the reader says the list is
/// partial instead of presenting it as complete (the page order is by machine id, not recency, so
/// what lies beyond the ceiling may include recent machines — hence the flag, not a silent cut).
const PRESENCE_QUERY_PAGES: usize = 2;

impl PresenceStore for DynamoDbPrefixRegistry {
    fn announce(
        &self,
        stream: &StreamId,
        hello: &MachineHello,
        now: i64,
        forget_before: i64,
    ) -> StoreResult<()> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        let hello = hello.clone();
        // No per-stream delete here (it would cost a query and a write per expired item on every
        // announcement). Instead the item carries when it expires, in the shape DynamoDB's TTL reads
        // — a deployment MAY enable TTL on `expires_at` to have old items removed for free — and the
        // read filters expired items out either way, so the listing is the same with or without it.
        let expires_at = now.saturating_add(now.saturating_sub(forget_before));
        block_on(async move {
            // An unconditional put: the latest announcement IS the state, so there is nothing
            // to protect against overwriting.
            client
                .put_item()
                .table_name(&table)
                .item("stream_id", AttributeValue::S(stream_id))
                .item("sk", AttributeValue::S(machine_sk(&hello.machine_id)))
                .item("name", AttributeValue::S(hello.name))
                .item("last_seen", AttributeValue::N(now.to_string()))
                .item(
                    "interval_secs",
                    AttributeValue::N(hello.interval_secs.to_string()),
                )
                .item("expires_at", AttributeValue::N(expires_at.to_string()))
                .send()
                .await
                .map_err(ddb_err)?;
            Ok(())
        })
    }

    fn machines(
        &self,
        stream: &StreamId,
        seen_since: i64,
        limit: usize,
    ) -> StoreResult<MachinesPage> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        block_on(async move {
            let mut records = Vec::new();
            let mut start_key: Option<HashMap<String, AttributeValue>> = None;
            let mut more_beyond_ceiling = false;
            for page in 0..PRESENCE_QUERY_PAGES {
                // Eventually consistent, unlike every other read in this file: presence is judged
                // in minutes, a sighting a second late changes nothing, and it halves the read cost
                // of a route anybody can call.
                let resp = client
                    .query()
                    .table_name(&table)
                    .key_condition_expression("stream_id = :sid AND begins_with(sk, :kind)")
                    .filter_expression("last_seen >= :since")
                    .expression_attribute_values(":sid", AttributeValue::S(stream_id.clone()))
                    .expression_attribute_values(
                        ":kind",
                        AttributeValue::S(MACHINE_SK_PREFIX.to_string()),
                    )
                    .expression_attribute_values(
                        ":since",
                        AttributeValue::N(seen_since.to_string()),
                    )
                    .set_exclusive_start_key(start_key.take())
                    .send()
                    .await
                    .map_err(ddb_err)?;
                for item in resp.items() {
                    let sk = parse_s(item, "sk")?;
                    let machine_id = sk
                        .strip_prefix(MACHINE_SK_PREFIX)
                        .ok_or_else(|| StoreError(format!("presence item with key {sk:?}")))?
                        .to_string();
                    records.push(MachineRecord {
                        machine_id,
                        name: parse_s(item, "name")?,
                        last_seen: parse_n(item, "last_seen")?,
                        interval_secs: parse_n(item, "interval_secs")?,
                    });
                }
                match resp.last_evaluated_key() {
                    Some(key) if !key.is_empty() => {
                        start_key = Some(key.clone());
                        more_beyond_ceiling = page + 1 == PRESENCE_QUERY_PAGES;
                    }
                    _ => break,
                }
            }
            // The key orders by machine id; the contract is most recently seen first, ties by id
            // in BYTE order — what SQLite's BINARY and Postgres' `COLLATE "C"` give.
            records.sort_by(|a, b| {
                b.last_seen
                    .cmp(&a.last_seen)
                    .then_with(|| a.machine_id.cmp(&b.machine_id))
            });
            let truncated = more_beyond_ceiling || records.len() > limit;
            records.truncate(limit);
            Ok(MachinesPage { records, truncated })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    // `classify_cancellation` is pure, so these run under plain `cargo test --features
    // dynamodb` — no DynamoDB Local needed — which is exactly why the logic was pulled out
    // of `try_claim` in the first place: `TransactionConflict` / `ProvisionedThroughputExceeded`
    // / `ThrottlingError` / `ValidationError` cancellations are real under contention but
    // DynamoDB Local has no reliable way to provoke them from an integration test.

    /// One tiny current-thread runtime for the pure retry-loop tests. They never touch the
    /// network — the stand-in cycles resolve immediately — but `retry_claim` sleeps on
    /// `tokio::time`, so it needs a reactor.
    fn drive<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build the test runtime")
            .block_on(fut)
    }

    // ----- Slot + append_with_retry ------------------------------------------------------
    //
    // DynamoDB Local serialises transactions, so a throttle, a `TransactionConflict` or a
    // contention budget running out never happen against it. These drive the policy directly.

    #[test]
    fn a_slot_takes_the_seq_after_the_counter_and_steps_over_an_occupied_one() {
        let slot = |counter, floor| Slot::after(counter, floor).unwrap();
        assert_eq!(
            slot(0, 0),
            Slot {
                expected: 0,
                seq: 1
            },
            "a stream without a counter starts at 1"
        );
        assert_eq!(
            slot(9, 0),
            Slot {
                expected: 9,
                seq: 10
            }
        );
        assert_eq!(
            slot(0, 1),
            Slot {
                expected: 0,
                seq: 2
            },
            "an op stored before the counter existed is stepped over, the counter still expected"
        );
        assert_eq!(
            slot(12, 3),
            Slot {
                expected: 12,
                seq: 13
            },
            "a floor below the counter changes nothing"
        );
        assert_eq!(slot(0, 0).predecessor(), None, "seq 1 has nothing below");
        assert_eq!(slot(9, 0).predecessor(), Some(9));
        assert_eq!(
            slot(0, 40).predecessor(),
            Some(40),
            "a floor step checks the floor"
        );
    }

    #[test]
    fn a_seq_past_i64_max_is_an_error_not_a_wrap_into_the_claim_range() {
        let err = Slot::after(i64::MAX, 0).expect_err("no seq after i64::MAX");
        assert!(err.to_string().contains("i64::MAX"), "{err}");
        assert!(Slot::after(0, i64::MAX).is_err(), "nor after a floor there");
        assert_eq!(Slot::after(i64::MAX - 1, 0).unwrap().seq, i64::MAX);
    }

    #[test]
    fn the_predecessor_check_flips_only_for_the_seq_found_over_a_gap() {
        let slot = Slot::after(9, 0).unwrap();
        let mut round = Round::first(Counter::Unknown);
        assert_eq!(round.predecessor_check(slot), Predecessor::Exists);
        round.gap = Some(10);
        assert_eq!(round.predecessor_check(slot), Predecessor::Absent);
        round.gap = Some(11);
        assert_eq!(
            round.predecessor_check(slot),
            Predecessor::Exists,
            "a gap found at another seq does not relax this one"
        );
        assert_eq!(
            round.predecessor_check(Slot::after(0, 0).unwrap()),
            Predecessor::None
        );
    }

    /// The rounds a stand-in was asked for, shared with the test.
    type Rounds = std::rc::Rc<std::cell::RefCell<Vec<Round>>>;

    /// What a stand-in round answers.
    type Answer = std::future::Ready<StoreResult<PutOutcome>>;

    /// A stand-in round that records each round it is asked for and answers from `script`.
    fn scripted(script: Vec<PutOutcome>) -> (Rounds, impl FnMut(Round) -> Answer) {
        let rounds: Rounds = std::rc::Rc::default();
        let seen = std::rc::Rc::clone(&rounds);
        let mut script = script.into_iter();
        let round = move |r: Round| {
            seen.borrow_mut().push(r);
            std::future::ready(Ok(script.next().expect("the script covers every round")))
        };
        (rounds, round)
    }

    /// [`append_with_retry`] from an unknown counter and without a deadline.
    fn retry<F, Fut>(round: F) -> StoreResult<Settled>
    where
        F: FnMut(Round) -> Fut,
        Fut: std::future::Future<Output = StoreResult<PutOutcome>>,
    {
        drive(append_with_retry("s", Counter::Unknown, None, round))
    }

    #[test]
    fn a_round_lost_to_a_concurrent_append_retries_from_the_counter_it_saw() {
        // The race this backend now settles inside the transaction (6j6v.r3jf): the loser does not
        // keep a seq, it learns where the counter went and takes the seq after that.
        let (rounds, round) = scripted(vec![
            PutOutcome::Lost(Some(10)),
            PutOutcome::Lost(None),
            PutOutcome::Written(12),
        ]);
        assert_eq!(retry(round).unwrap(), Settled::Written(12));
        let counters: Vec<Counter> = rounds.borrow().iter().map(|r| r.counter).collect();
        assert_eq!(
            counters,
            [Counter::Unknown, Counter::Hint(10), Counter::Unknown],
            "read first; then the counter the failed check saw; read again when none came back"
        );
    }

    #[test]
    fn a_push_carries_the_counter_it_was_given_into_the_first_round() {
        let (rounds, round) = scripted(vec![PutOutcome::Written(8)]);
        drive(append_with_retry("s", Counter::Carried(7), None, round)).unwrap();
        assert_eq!(rounds.borrow()[0].counter, Counter::Carried(7), "no read");
    }

    #[test]
    fn a_taken_seq_raises_the_floor_to_the_top_and_keeps_the_counter_known() {
        let (rounds, round) = scripted(vec![
            PutOutcome::SeqTaken {
                counter: 0,
                top: 40,
            },
            PutOutcome::ClaimForeign,
            PutOutcome::Written(41),
        ]);
        assert_eq!(retry(round).unwrap(), Settled::Written(41));
        let rounds = rounds.borrow();
        assert_eq!(
            rounds[1],
            Round {
                counter: Counter::Known(0),
                floor: 40,
                claimed: true,
                gap: None,
            },
            "one step to the top occupied seq, and no re-read of a counter whose condition held"
        );
        assert_eq!(
            rounds[2],
            Round {
                counter: Counter::Unknown,
                floor: 40,
                claimed: false,
                gap: None,
            },
            "an op whose claim key is another op id's goes in unclaimed, as before the claims"
        );
    }

    #[test]
    fn a_missing_predecessor_flips_the_check_for_that_seq_and_only_for_the_next_round() {
        let (rounds, round) = scripted(vec![
            PutOutcome::Predecessor {
                counter: 9,
                seq: 10,
                absent: true,
            },
            PutOutcome::Lost(Some(10)),
            PutOutcome::Predecessor {
                counter: 10,
                seq: 11,
                absent: false,
            },
            PutOutcome::Written(11),
        ]);
        assert_eq!(retry(round).unwrap(), Settled::Written(11));
        let rounds = rounds.borrow();
        assert_eq!(
            (rounds[1].counter, rounds[1].gap),
            (Counter::Known(9), Some(10)),
            "the same slot again, its predecessor checked for absence"
        );
        assert_eq!(
            rounds[2].gap, None,
            "a lost round moves to another slot, and the check is attribute_exists again"
        );
        assert_eq!(
            (rounds[3].counter, rounds[3].gap),
            (Counter::Known(10), None),
            "a predecessor that appeared is checked for again, not stepped over"
        );
    }

    #[test]
    fn append_gives_up_loudly_after_append_attempts_stalled_rounds() {
        // Never an unbounded spin: rounds that keep failing with no other writer committing are
        // structurally broken or saturated, and the caller must be told so.
        let (rounds, round) = scripted(
            (1..=i64::from(APPEND_ATTEMPTS))
                .map(|seq| PutOutcome::SeqTaken {
                    counter: 0,
                    top: seq,
                })
                .collect(),
        );
        let err = retry(round).expect_err("a stall must not loop forever");
        assert_eq!(rounds.borrow().len() as u32, APPEND_ATTEMPTS);
        assert!(
            err.to_string().contains(&APPEND_ATTEMPTS.to_string()),
            "the error names the bound it hit: {err}"
        );
    }

    #[test]
    fn contention_has_its_own_larger_budget_and_a_clear_error_when_it_runs_out() {
        // Lost rounds are other writers' commits, so they do not eat the stall budget: more of
        // them than APPEND_ATTEMPTS still land...
        let mut script: Vec<PutOutcome> = (0..APPEND_ATTEMPTS * 2)
            .map(|_| PutOutcome::Lost(None))
            .collect();
        script.push(PutOutcome::Written(99));
        let (_, round) = scripted(script);
        assert_eq!(retry(round).unwrap(), Settled::Written(99));

        // ...but they are bounded too, and the error says what happened.
        let (rounds, round) = scripted(
            (0..CONTENTION_ROUNDS)
                .map(|_| PutOutcome::Lost(None))
                .collect(),
        );
        let err = retry(round).expect_err("contention is bounded");
        assert_eq!(rounds.borrow().len() as u32, CONTENTION_ROUNDS);
        let err = err.to_string();
        assert!(
            err.contains("concurrent appends") && err.contains(&CONTENTION_ROUNDS.to_string()),
            "{err}"
        );
    }

    #[test]
    fn the_contention_budget_counts_lost_rounds_in_a_row() {
        // CONTENTION_ROUNDS - 1 losses, one round that ends otherwise, then as many losses again:
        // twice the budget in total, never the budget in a row, so the append still lands.
        let losses = || (1..CONTENTION_ROUNDS).map(|_| PutOutcome::Lost(None));
        let script: Vec<PutOutcome> = losses()
            .chain([PutOutcome::Predecessor {
                counter: 3,
                seq: 4,
                absent: true,
            }])
            .chain(losses())
            .chain([PutOutcome::Written(500)])
            .collect();
        let (_, round) = scripted(script);
        assert_eq!(
            retry(round).expect("a round that is not lost resets the count"),
            Settled::Written(500)
        );
    }

    #[test]
    fn append_propagates_a_hard_error_from_the_round_immediately() {
        let calls = std::cell::Cell::new(0u32);
        let err = retry(|_| {
            calls.set(calls.get() + 1);
            async { Err(StoreError("table gone".into())) }
        })
        .expect_err("a hard error surfaces");
        assert_eq!(calls.get(), 1, "no retry on a fault");
        assert!(err.to_string().contains("table gone"));
    }

    #[test]
    fn a_throttled_round_backs_off_before_the_next() {
        let (_, round) = scripted(vec![
            PutOutcome::Throttled,
            PutOutcome::Throttled,
            PutOutcome::Written(3),
        ]);
        let started = Instant::now();
        assert_eq!(retry(round).unwrap(), Settled::Written(3));
        assert!(
            started.elapsed() >= claim_backoff(0) + claim_backoff(1),
            "both throttled rounds waited at least their backoff"
        );
    }

    #[test]
    fn append_answers_an_already_stored_op_with_its_existing_seq() {
        let (_, round) = scripted(vec![PutOutcome::Existing(3)]);
        assert_eq!(retry(round).unwrap(), Settled::Existing(3));
    }

    #[test]
    fn a_passed_deadline_stops_before_the_next_round() {
        let (rounds, round) = scripted(vec![PutOutcome::Lost(None)]);
        let deadline = Instant::now();
        let err = drive(append_with_retry(
            "s",
            Counter::Unknown,
            Some(deadline),
            round,
        ))
        .expect_err("nothing is sent once the deadline has passed");
        assert!(rounds.borrow().is_empty());
        assert!(err.to_string().contains("deadline"), "{err}");
    }

    #[test]
    fn the_deadline_cuts_a_round_in_flight_short() {
        // A round that would take ten seconds (a stalled endpoint, say) is not waited out.
        let started = Instant::now();
        let deadline = started + Duration::from_millis(50);
        let err = drive(append_with_retry(
            "s",
            Counter::Unknown,
            Some(deadline),
            |_| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(PutOutcome::Written(1))
            },
        ))
        .expect_err("the round is abandoned at the deadline");
        assert!(err.to_string().contains("deadline"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "stopped at the deadline, not after the round: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_jitter_stays_inside_its_cap() {
        for lost in 0..40 {
            assert!(contention_jitter(lost) <= Duration::from_millis(16));
        }
        assert!(jitter(Duration::ZERO) <= Duration::from_micros(1));
        assert!(jitter(Duration::from_millis(5)) <= Duration::from_millis(5));
    }

    #[test]
    fn the_jitter_actually_spreads_its_draws() {
        // A jitter that is always zero (or always one value) would pass the cap test above and
        // bring every loser back for the same seq at the same instant. 200 draws over 16 ms: the
        // chance that a uniform draw lands below 4 ms every time is 0.25^200.
        let cap = Duration::from_millis(16);
        let draws: Vec<Duration> = (0..200).map(|_| jitter(cap)).collect();
        let distinct: std::collections::BTreeSet<Duration> = draws.iter().copied().collect();
        assert!(distinct.len() > 20, "{} distinct draws", distinct.len());
        assert!(
            draws.iter().any(|d| *d >= cap / 4),
            "no draw reached a quarter of the cap"
        );
    }

    // ----- claim_seq + the append transaction's cancellations -----------------------------

    #[test]
    fn a_claim_key_lies_below_the_counter_and_is_stable_per_op_id() {
        for op_id in ["", "o1", "ab12.0001@1", &"x".repeat(4096)] {
            let key = claim_seq(op_id);
            assert!(
                key < COUNTER_SEQ,
                "{op_id:?} -> {key} must sit below the counter"
            );
            assert_eq!(
                key,
                claim_seq(op_id),
                "the key is a pure function of the op id"
            );
        }
        assert_ne!(claim_seq("o1"), claim_seq("o2"));
        // Pinned: the key is STORED, so a build that derived it differently would stop
        // recognising every claim written before it.
        assert_eq!(claim_seq("o1"), -1_272_668_462_371_106_663);
    }

    const ALL: TxLayout = TxLayout {
        claimed: true,
        checked: true,
    };

    #[test]
    fn the_layout_puts_each_item_at_its_index() {
        assert_eq!(
            (ALL.claim(), ALL.predecessor(), ALL.op()),
            (Some(1), Some(2), 3)
        );
        let unclaimed = TxLayout {
            claimed: false,
            checked: true,
        };
        assert_eq!(
            (unclaimed.claim(), unclaimed.predecessor(), unclaimed.op()),
            (None, Some(1), 2)
        );
        let first_seq = TxLayout {
            claimed: true,
            checked: false,
        };
        assert_eq!(
            (first_seq.claim(), first_seq.predecessor(), first_seq.op()),
            (Some(1), None, 2)
        );
        let bare = TxLayout {
            claimed: false,
            checked: false,
        };
        assert_eq!(
            (bare.claim(), bare.predecessor(), bare.op()),
            (None, None, 1)
        );
    }

    #[test]
    fn a_taken_claim_outranks_a_moved_counter_which_outranks_a_taken_seq_and_the_predecessor() {
        let ccf = Some("ConditionalCheckFailed");
        let none = Some("None");
        let conflict = Some("TransactionConflict");
        let classify = classify_append_cancellation;
        assert_eq!(
            classify(ALL, &[ccf, ccf, ccf, ccf]),
            Ok(AppendCancellation::Claimed),
            "a re-push that lost the counter race still answers with its stored seq"
        );
        assert_eq!(
            classify(ALL, &[conflict, ccf, none, none]),
            Ok(AppendCancellation::Claimed)
        );
        assert_eq!(
            classify(ALL, &[ccf, none, ccf, ccf]),
            Ok(AppendCancellation::CounterMoved)
        );
        assert_eq!(
            classify(ALL, &[none, none, ccf, ccf]),
            Ok(AppendCancellation::SeqTaken),
            "a taken seq outranks a failed predecessor check"
        );
        assert_eq!(
            classify(ALL, &[none, none, ccf, none]),
            Ok(AppendCancellation::PredecessorFailed)
        );
        assert_eq!(
            classify(ALL, &[none, none, conflict, none]),
            Ok(AppendCancellation::Conflict),
            "the append below still writing its op: contention, retried"
        );
        assert_eq!(
            classify(ALL, &[none, Some("ThrottlingError"), none, conflict]),
            Ok(AppendCancellation::Conflict),
            "a conflict means another append is committing: contention, not a stall"
        );
        assert_eq!(
            classify(
                ALL,
                &[none, none, none, Some("ProvisionedThroughputExceeded")]
            ),
            Ok(AppendCancellation::Throttled)
        );
    }

    #[test]
    fn the_codes_are_read_at_the_layouts_indexes() {
        let ccf = Some("ConditionalCheckFailed");
        let none = Some("None");
        // Unclaimed: index 1 is the predecessor check, not a claim.
        let unclaimed = TxLayout {
            claimed: false,
            checked: true,
        };
        assert_eq!(
            classify_append_cancellation(unclaimed, &[none, ccf, none]),
            Ok(AppendCancellation::PredecessorFailed)
        );
        assert_eq!(
            classify_append_cancellation(unclaimed, &[none, none, ccf]),
            Ok(AppendCancellation::SeqTaken)
        );
        // Seq 1, claimed: index 1 is the claim, index 2 the op.
        let first_seq = TxLayout {
            claimed: true,
            checked: false,
        };
        assert_eq!(
            classify_append_cancellation(first_seq, &[none, ccf, none]),
            Ok(AppendCancellation::Claimed)
        );
        assert_eq!(
            classify_append_cancellation(first_seq, &[none, none, ccf]),
            Ok(AppendCancellation::SeqTaken)
        );
        let bare = TxLayout {
            claimed: false,
            checked: false,
        };
        assert_eq!(
            classify_append_cancellation(bare, &[ccf, None]),
            Ok(AppendCancellation::CounterMoved),
            "an unclaimed round"
        );
    }

    #[test]
    fn any_other_append_cancellation_is_a_hard_error() {
        let none = Some("None");
        let err = classify_append_cancellation(ALL, &[none, none, none, Some("ValidationError")])
            .expect_err("a validation failure is a fault, never a retry");
        assert!(err.contains("ValidationError"), "{err}");
        assert!(classify_append_cancellation(ALL, &[]).is_err());
    }

    #[test]
    fn each_cancellation_maps_to_its_next_step() {
        let slot = Slot::after(9, 0).unwrap();
        let settle = |c, check, hint| settle_cancellation(c, slot, check, hint);
        let exists = Predecessor::Exists;
        assert_eq!(
            settle(AppendCancellation::CounterMoved, exists, Some(12)),
            Settle::Outcome(PutOutcome::Lost(Some(12))),
            "a moved counter carries the counter the failed check handed back"
        );
        assert_eq!(
            settle(AppendCancellation::Conflict, exists, Some(12)),
            Settle::Outcome(PutOutcome::Lost(None)),
            "a conflict says nothing about the counter: read it"
        );
        assert_eq!(
            settle(AppendCancellation::PredecessorFailed, exists, None),
            Settle::Outcome(PutOutcome::Predecessor {
                counter: 9,
                seq: 10,
                absent: true
            }),
            "attribute_exists failed: the op below is absent"
        );
        assert_eq!(
            settle(
                AppendCancellation::PredecessorFailed,
                Predecessor::Absent,
                None
            ),
            Settle::Outcome(PutOutcome::Predecessor {
                counter: 9,
                seq: 10,
                absent: false
            }),
            "attribute_not_exists failed: the op below appeared"
        );
        assert_eq!(
            settle(AppendCancellation::Throttled, exists, None),
            Settle::Outcome(PutOutcome::Throttled)
        );
        assert_eq!(
            settle(AppendCancellation::Claimed, exists, None),
            Settle::ReadClaim
        );
        assert_eq!(
            settle(AppendCancellation::SeqTaken, exists, None),
            Settle::FindTop
        );
    }

    #[test]
    fn a_claim_read_back_settles_as_existing_foreign_or_vanished() {
        assert_eq!(
            settle_claim(Some(("o1".into(), 7)), "o1"),
            PutOutcome::Existing(7)
        );
        assert_eq!(
            settle_claim(Some(("other".into(), 7)), "o1"),
            PutOutcome::ClaimForeign
        );
        assert_eq!(settle_claim(None, "o1"), PutOutcome::ClaimVanished);
    }

    #[test]
    fn the_counter_reads_as_zero_when_absent_and_the_hint_only_when_handed_back() {
        let with = |n: &str| {
            HashMap::from([
                ("stream_id".to_string(), AttributeValue::S("s".into())),
                ("next_seq".to_string(), AttributeValue::N(n.into())),
            ])
        };
        let without = HashMap::from([("stream_id".to_string(), AttributeValue::S("s".into()))]);
        assert_eq!(counter_value(Some(&with("12"))).unwrap(), 12);
        assert_eq!(counter_value(Some(&without)).unwrap(), 0);
        assert_eq!(counter_value(None).unwrap(), 0);
        assert_eq!(counter_hint(Some(&with("12"))).unwrap(), Some(12));
        assert_eq!(
            counter_hint(Some(&without)).unwrap(),
            Some(0),
            "an item without the attribute is a stream without a counter"
        );
        assert_eq!(
            counter_hint(None).unwrap(),
            None,
            "nothing handed back: the next round reads"
        );
        assert!(
            counter_hint(Some(&with("twelve"))).is_err(),
            "a malformed counter is loud, never a silent 0"
        );
    }

    // ----- retry_claim -------------------------------------------------------------------

    #[test]
    fn a_retryable_cancellation_re_runs_the_same_claim_until_it_settles() {
        let calls = std::cell::Cell::new(0u32);
        let outcome = drive(retry_claim("s", "aaaa", "uuid-A", || {
            let n = calls.get() + 1;
            calls.set(n);
            async move {
                Ok(if n < 3 {
                    ClaimOutcome::Retryable
                } else {
                    ClaimOutcome::Claimed
                })
            }
        }))
        .expect("the third attempt settles");
        assert_eq!(outcome, ClaimOutcome::Claimed);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn retry_claim_gives_up_loudly_after_claim_attempts() {
        // A saturated table must produce a loud error the client can retry on its own
        // schedule — never an unbounded spin inside one HTTP request.
        let calls = std::cell::Cell::new(0u32);
        let err = drive(retry_claim("s", "aaaa", "uuid-A", || {
            calls.set(calls.get() + 1);
            async { Ok(ClaimOutcome::Retryable) }
        }))
        .expect_err("sustained contention must not loop forever");
        assert_eq!(calls.get(), CLAIM_ATTEMPTS, "the bound is CLAIM_ATTEMPTS");
        assert!(
            err.to_string()
                .contains("cancelled by transaction conflict"),
            "the error names the cause: {err}"
        );
    }

    #[test]
    fn retry_claim_returns_a_settled_outcome_without_retrying() {
        // PrefixTaken is an answer, not contention: retrying it would waste round-trips and
        // delay the walk to the next candidate.
        let calls = std::cell::Cell::new(0u32);
        let outcome = drive(retry_claim("s", "aaaa", "uuid-A", || {
            calls.set(calls.get() + 1);
            async { Ok(ClaimOutcome::PrefixTaken) }
        }))
        .unwrap();
        assert_eq!(outcome, ClaimOutcome::PrefixTaken);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn the_register_budget_bounds_the_reassignment_walk_below_its_attempt_count() {
        // REASSIGN_ATTEMPTS is sized for cheap in-process retries (SQLite/Postgres). On the
        // network-bound path the wall clock, not the count, is what keeps one register call
        // inside one HTTP request — so the budget must be small enough to bind first.
        let worst_case_per_candidate: Duration =
            (0..CLAIM_ATTEMPTS).map(claim_backoff).sum::<Duration>() + OPERATION_TIMEOUT;
        assert!(
            REGISTER_BUDGET < worst_case_per_candidate * REASSIGN_ATTEMPTS,
            "the budget must bind long before REASSIGN_ATTEMPTS x CLAIM_ATTEMPTS does"
        );
        // And it must leave room for at least one full candidate, or no walk could ever start.
        assert!(
            REGISTER_BUDGET > worst_case_per_candidate.saturating_sub(OPERATION_TIMEOUT),
            "the budget must allow a candidate's backoff to complete"
        );
    }

    #[test]
    fn request_timeouts_bound_both_one_attempt_and_the_whole_operation() {
        // The SDK's own defaults bound only the CONNECT phase; an established-then-silent
        // connection is what actually parks an axum worker. Pin that both knobs are set, and
        // that a whole operation may span more than one attempt (so the SDK's internal
        // retries have room) but is still finite.
        let cfg = request_timeouts();
        assert_eq!(cfg.operation_attempt_timeout(), Some(ATTEMPT_TIMEOUT));
        assert_eq!(cfg.operation_timeout(), Some(OPERATION_TIMEOUT));
        assert!(
            ATTEMPT_TIMEOUT < OPERATION_TIMEOUT,
            "a single attempt must not be able to consume the whole operation budget"
        );
    }

    #[test]
    fn only_the_prefix_item_failing_is_an_ordinary_collision() {
        assert_eq!(
            classify_cancellation(Some("ConditionalCheckFailed"), Some("None")),
            Ok(ClaimOutcome::PrefixTaken)
        );
    }

    #[test]
    fn the_uuid_item_failing_is_a_self_race_even_if_the_prefix_item_also_failed() {
        // The uuid item takes priority: a self-race must never be read as an ordinary
        // collision, regardless of what the prefix item's condition did.
        assert_eq!(
            classify_cancellation(Some("None"), Some("ConditionalCheckFailed")),
            Ok(ClaimOutcome::UuidRace)
        );
        assert_eq!(
            classify_cancellation(
                Some("ConditionalCheckFailed"),
                Some("ConditionalCheckFailed")
            ),
            Ok(ClaimOutcome::UuidRace)
        );
    }

    #[test]
    fn a_cancellation_with_no_condition_check_failure_is_a_hard_error() {
        // Neither item failed a condition check — e.g. both report "None" because some
        // OTHER item in a larger future transaction caused the cancellation. This must not
        // default to "prefix taken".
        assert!(classify_cancellation(Some("None"), Some("None")).is_err());
    }

    #[test]
    fn a_throttle_or_conflict_on_either_item_is_retryable_not_a_collision() {
        // A conflict or a throttle is not the transaction failing on its merits — it lost to
        // concurrency or capacity, and AWS's documented remedy is to retry the whole
        // transaction. Reading it as `PrefixTaken` would steer `register` at a different
        // candidate (an aliasing claim); reading it as a hard error turns routine contention
        // into a 500 for the client's entire sync pass, because the SDK's own retry
        // classifier does not auto-retry `TransactionCanceledException` — it cannot tell a
        // legitimate condition failure from a conflict, so this code has to.
        for code in [
            "TransactionConflict",
            "ProvisionedThroughputExceeded",
            "ThrottlingError",
        ] {
            assert_eq!(
                classify_cancellation(Some(code), Some("None")),
                Ok(ClaimOutcome::Retryable),
                "{code} on the prefix item must be retryable"
            );
            assert_eq!(
                classify_cancellation(Some("None"), Some(code)),
                Ok(ClaimOutcome::Retryable),
                "{code} on the uuid item must be retryable"
            );
        }
    }

    #[test]
    fn a_validation_error_stays_a_hard_error_not_a_retry() {
        // A `ValidationError` is deterministic — a malformed request, or a table whose key
        // schema does not match the contract at the top of this file. Retrying it just
        // repeats the same failure more slowly and buries the one message that would tell an
        // operator what is actually wrong.
        for reasons in [
            (Some("ValidationError"), Some("None")),
            (Some("None"), Some("ValidationError")),
        ] {
            let result = classify_cancellation(reasons.0, reasons.1);
            assert!(
                result.is_err(),
                "ValidationError must stay a hard error, got {result:?} for {reasons:?}"
            );
        }
    }

    #[test]
    fn a_confirmed_condition_failure_outranks_a_transient_code_on_the_other_item() {
        // A reported `ConditionalCheckFailed` is definitive information — DynamoDB only
        // emits it for an item whose condition actually evaluated false — so it decides the
        // outcome even when the other item reports a transient code. Retrying instead would
        // reach the identical answer one round-trip later.
        assert_eq!(
            classify_cancellation(Some("ConditionalCheckFailed"), Some("TransactionConflict")),
            Ok(ClaimOutcome::PrefixTaken)
        );
        assert_eq!(
            classify_cancellation(Some("TransactionConflict"), Some("ConditionalCheckFailed")),
            Ok(ClaimOutcome::UuidRace)
        );
    }

    #[test]
    fn the_whole_claim_retry_budget_stays_inside_one_http_request() {
        // The retry only helps if it is cheap: `register` runs inside one relay request, and
        // a budget that can stall it for seconds trades a 500 for a hang, which is worse.
        // This pins the bound so raising CLAIM_ATTEMPTS or the backoff is a deliberate act.
        let total: std::time::Duration = (0..CLAIM_ATTEMPTS).map(claim_backoff).sum();
        assert!(
            total < std::time::Duration::from_millis(250),
            "the full retry budget is {total:?}, too long to sit inside one request"
        );
        assert!(
            claim_backoff(1) > claim_backoff(0),
            "backoff must grow, or N racers that all conflicted re-collide in lockstep"
        );
    }

    #[test]
    fn missing_cancellation_reasons_are_a_hard_error() {
        // A response shape we don't recognise (e.g. an empty reasons list) must not be
        // silently read as "prefix taken" either — nor quietly retried.
        assert!(classify_cancellation(None, None).is_err());
    }

    // ----- the sync/async bridge ------------------------------------------------------

    #[test]
    fn block_on_works_from_inside_a_current_thread_runtime() {
        // `block_in_place` PANICS on a current-thread runtime, so the flavor guard in
        // `block_on` is load-bearing: the DynamoDB smoke test embeds the relay on exactly
        // such a runtime (`tests/dynamodb_local.rs::spawn_ddb_relay`), and so may an
        // embedder. Blocking the single thread here is safe only because the future runs on
        // the separate bridge runtime — which is the whole point of the channel handoff.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build a current-thread runtime");
        assert_eq!(rt.block_on(async { block_on(async { 7u32 }) }), 7);
    }

    #[test]
    fn a_blocked_bridge_call_does_not_starve_the_multi_thread_worker_it_runs_on() {
        // The regression this pins: `app.rs`'s push handler calls `append` once per op from
        // an axum worker, so without `block_in_place` a single slow store call parks that
        // worker — and every other task queued on it — for a full network round-trip. One
        // worker thread makes the failure deterministic rather than merely likely: the
        // second task can ONLY run if tokio was told to hand the worker's queue off.
        use std::sync::atomic::{AtomicBool, Ordering};

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build a single-worker multi-thread runtime");
        let blocking_started = Arc::new(AtomicBool::new(false));
        let other_task_ran = Arc::new(AtomicBool::new(false));

        let other_task_was_served = rt.block_on(async {
            let started = Arc::clone(&blocking_started);
            let observed = Arc::clone(&other_task_ran);
            let blocker = tokio::task::spawn(async move {
                started.store(true, Ordering::SeqCst);
                // Blocks the one worker thread until the other task reports progress.
                block_on(async move {
                    for _ in 0..500 {
                        if observed.load(Ordering::SeqCst) {
                            return true;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    false
                })
            });

            // This future runs on the CALLER's thread, not the worker, so a plain sleep here
            // cannot itself be what unblocks anything — it only ensures the blocker is
            // already inside the bridge call before the second task is queued.
            while !blocking_started.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));

            let ran = Arc::clone(&other_task_ran);
            tokio::task::spawn(async move { ran.store(true, Ordering::SeqCst) });
            blocker.await.expect("the blocking task panicked")
        });

        assert!(
            other_task_was_served,
            "a task queued behind a blocked bridge call must still be scheduled — \
             block_on has to tell tokio it is about to block a multi-thread worker"
        );
    }
}
