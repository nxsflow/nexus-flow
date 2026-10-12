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
//! lower seq is; they are **not** guaranteed dense: see the allocation section below.
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
//! append round reads the counter item (`seq = 0`, attribute `next_seq`; absent counts as 0)
//! and sends ONE `TransactWriteItems` that sets `next_seq` to the seq after it — conditioned on
//! `next_seq` still holding the value read — and puts the op at that seq (and its claim, below).
//! A round that finds the counter moved has lost to a concurrent append; it retries from the
//! counter the failed check reports, after a short random pause ([`append_with_retry`]).
//!
//! Why this shape and not an `UpdateItem` `ADD` ahead of the write, as before: an `ADD` hands
//! out the seq before the op exists. Writer A got 10, writer B got 11, B committed first, and a
//! reader at 9 was handed `[11]` and stored cursor 11 — so A, committed at 10 a moment later,
//! sat below every cursor that reader would send again, missing for good, and a fold watermark
//! moved past it too. Here seq `n + 1` cannot be written before the transaction that wrote `n`
//! has committed, because its condition reads the counter that transaction set. So commit order
//! is seq order, as on SQLite (a mutex) and Postgres (an advisory lock held to commit), and a
//! strongly consistent `read_since` that returns seq `n` has every op below `n` that will ever
//! exist. The op's own condition, `attribute_not_exists(stream_id)` at its `(stream_id, seq)`,
//! stays as the brace: a seq occupied by an op the counter does not account for (a stream
//! written before the counter existed, an item put there out of band) is stepped over, never
//! overwritten.
//!
//! What it costs: one strongly consistent read of the counter and one transaction per append,
//! the same two round-trips as the `ADD` and the transaction it replaces (the read is cheaper
//! than the `ADD`'s write). Appends to ONE stream now serialise on the counter item, as they do
//! on the other two backends; a lost round costs one more transaction, and its retry needs no
//! read when DynamoDB returns the counter with the failed check. Rounds lost to concurrent
//! appends are bounded by [`CONTENTION_ROUNDS`], rounds that fail for any other reason by
//! [`APPEND_ATTEMPTS`]; either bound ends in a loud `StoreError`, and the client re-pushes,
//! which the claims make safe. Appends to different streams never contend. No new table,
//! index, attribute or IAM action: the counter is written with `UpdateItem` (as the `ADD` was)
//! and read with `Query`.
//!
//! **Density.** A seq is taken only together with its op, so a stream written by this build has
//! no gaps: neither a re-push nor a lost round nor the SDK's retry of a write whose response was
//! lost consumes one (the retry finds the claim taken). Streams written before it carry the
//! gaps the `ADD` left. Nothing downstream depends on density — `read_since` is `seq > cursor`
//! and the client follows the returned cursor — so read it as a property, not a contract.
//!
//! **Rolling upgrade.** A relay of an earlier build still allocates with `ADD` and can still
//! expose a seq ahead of an uncommitted one while it serves; mixed with this build nothing is
//! overwritten (both condition the op's put), and the guarantee holds once every relay writing a
//! table runs this build.
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
//! * **The claim rides the append's transaction.** It is item 1 of the same
//!   `TransactWriteItems` that moves the counter and puts the op, so a re-push is turned back by
//!   its taken claim before the counter moves: it consumes no seq, as on SQLite and Postgres.
//!   A taken claim outranks a moved counter, so a re-push that also lost the counter race still
//!   answers with its stored seq. Three items per transaction, far below DynamoDB's 100.
//! * **Ops stored before the claim items existed have none.** Re-pushing such an op stores it
//!   once more, with a claim this time; every later re-push is then absorbed.
//! * **The claim key is a 63-bit hash, so two op ids can share it.** The claim records the op
//!   id it was written for, so a shared key is detected rather than mistaken for a duplicate:
//!   the second op is then stored WITHOUT a claim, exactly as before this change, and only it
//!   loses the dedupe. Nothing is ever lost or overwritten by a collision.
//! * **No new table, index or IAM action.** The claim lives in the op table under the same key
//!   schema, it is written with `PutItem` inside the transaction and read with `Query`, actions
//!   the op table's role already has. Every op write costs a transaction, which DynamoDB bills at
//!   twice the plain write, for three items (counter, claim, op).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aws_sdk_dynamodb::error::ProvideErrorMetadata;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{
    AttributeValue, Put, ReturnValuesOnConditionCheckFailure, TransactWriteItem, Update,
};
use aws_sdk_dynamodb::Client;
use nxs_sync::protocol::{Cursor, MachineHello, RegisterOutcome, StreamId};
use nxs_sync::wire::WireOp;

use crate::presence::{MachineRecord, MachinesPage, PresenceStore};
use crate::registry::{candidate_prefix, PrefixRegistry, REASSIGN_ATTEMPTS};
use crate::store::{decode_extra, encode_extra, page_limit, OpStore, StoreError, StoreResult};

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
async fn build_client(endpoint: Option<String>, region: Option<String>) -> Client {
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
/// taken seq, a throttled transaction, a claim key found taken by another op id or read back
/// absent. Exhausting it means something is structurally wrong or the table is saturated, and
/// that must surface as a loud `StoreError`, never spin the caller's thread forever.
const APPEND_ATTEMPTS: u32 = 32;

/// Bound on the append rounds lost to a CONCURRENT append of the same stream — the counter
/// moved under this round, or DynamoDB reported a `TransactionConflict` on it. Separate from
/// [`APPEND_ATTEMPTS`], and larger, because every such round is another writer's commit: the
/// stream as a whole is making progress, only this writer is losing the race for the next seq.
/// With `n` writers racing, one wins each round, so a writer loses `k` rounds in a row with
/// probability about `((n-1)/n)^k`; at 256 that is below 1e-14 for eight writers. Exhausting it
/// means the stream took 256 other appends while this one waited, and the error says so.
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
    /// Stepping over such a seq is safe: it already holds a committed op.
    fn after(counter: i64, floor: i64) -> Slot {
        Slot {
            expected: counter,
            seq: counter.max(floor) + 1,
        }
    }
}

/// What the next round of [`append_with_retry`] starts from: the counter value to expect, when a
/// lost round already learned it (`None`: read it first), the highest seq found occupied above the
/// counter, and whether the op still goes in with its claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Round {
    counter: Option<i64>,
    floor: i64,
    claimed: bool,
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
    /// The counter held, but this seq is occupied by an op the counter does not account for.
    SeqTaken(i64),
    /// Throttled or over capacity: wait ([`claim_backoff`]), then try again.
    Throttled,
    /// The claim key belongs to another op id: store this op without a claim (module doc).
    ClaimForeign,
    /// The claim was reported taken and read back absent: try again.
    ClaimVanished,
}

/// Drive append rounds until one settles, within [`APPEND_ATTEMPTS`] and [`CONTENTION_ROUNDS`].
///
/// Generic over the round so the retry policy — which outcome retries with what, after how long,
/// within which budget — is unit-tested without a table: DynamoDB Local serialises transactions,
/// so most of these branches never occur against it.
async fn append_with_retry<F, Fut>(stream_id: &str, mut round: F) -> StoreResult<Cursor>
where
    F: FnMut(Round) -> Fut,
    Fut: std::future::Future<Output = StoreResult<PutOutcome>>,
{
    let mut next = Round {
        counter: None,
        floor: 0,
        claimed: true,
    };
    let (mut stalls, mut lost) = (0u32, 0u32);
    loop {
        let outcome = round(next).await?;
        match outcome {
            PutOutcome::Written(seq) | PutOutcome::Existing(seq) => return Ok(Cursor(seq)),
            PutOutcome::Lost(seen) => {
                lost += 1;
                if lost >= CONTENTION_ROUNDS {
                    return Err(StoreError(format!(
                        "append: lost {CONTENTION_ROUNDS} rounds in a row to concurrent appends \
                         of stream {stream_id:?}; the stream is under more write contention than \
                         one append waits out, retry the push"
                    )));
                }
                next.counter = seen;
                tokio::time::sleep(contention_jitter(lost)).await;
            }
            PutOutcome::SeqTaken(_)
            | PutOutcome::Throttled
            | PutOutcome::ClaimForeign
            | PutOutcome::ClaimVanished => {
                stalls += 1;
                if stalls >= APPEND_ATTEMPTS {
                    return Err(StoreError(format!(
                        "append: exhausted {APPEND_ATTEMPTS} attempts writing an op to stream \
                         {stream_id:?} without a successful conditional write (last: {outcome:?})"
                    )));
                }
                next.counter = None;
                match outcome {
                    PutOutcome::SeqTaken(seq) => next.floor = next.floor.max(seq),
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

/// Why an append's `TransactWriteItems` (index 0 = the counter, then the claim when the op
/// carries one, last = the op — see [`DynamoDbOpStore::try_append_op`]) was cancelled, when the
/// cancellation is an outcome rather than a fault.
#[derive(Debug, PartialEq, Eq)]
enum AppendCancellation {
    /// The claim's key is taken: this op — or one whose id shares the key — is stored.
    Claimed,
    /// The counter is not at the value this round expected: another append committed first.
    CounterMoved,
    /// The counter held but the op's seq is occupied: step over it.
    SeqTaken,
    /// A concurrent transaction on one of the items: another append is committing.
    Conflict,
    /// Throttled or over capacity.
    Throttled,
}

/// Classify a cancelled append from its positional per-item codes (`claim_code` is `None` for a
/// round without a claim). Pure for the same reason [`classify_cancellation`] is: DynamoDB Local
/// serialises transactions, so the transient codes never reach an acceptance test.
///
/// A taken claim outranks everything: it is definitive, and it means the op is already stored, so
/// the seq question no longer matters. A moved counter outranks a taken seq: the next round reads
/// the counter afresh and so picks another seq anyway.
fn classify_append_cancellation(
    counter_code: Option<&str>,
    claim_code: Option<&str>,
    op_code: Option<&str>,
) -> Result<AppendCancellation, String> {
    const CCF: Option<&str> = Some("ConditionalCheckFailed");
    if claim_code == CCF {
        return Ok(AppendCancellation::Claimed);
    }
    if counter_code == CCF {
        return Ok(AppendCancellation::CounterMoved);
    }
    if op_code == CCF {
        return Ok(AppendCancellation::SeqTaken);
    }
    let codes = [counter_code, claim_code, op_code];
    if codes.contains(&Some("TransactionConflict")) {
        return Ok(AppendCancellation::Conflict);
    }
    if codes.into_iter().any(is_retryable_code) {
        return Ok(AppendCancellation::Throttled);
    }
    Err(format!(
        "append transaction cancelled for a reason other than a moved counter or a taken claim \
         or seq (counter item code: {counter_code:?}, claim item code: {claim_code:?}, op item \
         code: {op_code:?})"
    ))
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
    before_write: Option<BeforeWrite>,
}

/// A test seam, see [`DynamoDbOpStore::with_before_write`].
type BeforeWrite = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

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

    /// Test seam (`6j6v.r3jf`): `hook` is called with the op id on every append attempt, after
    /// the attempt has decided which seq it writes and before it sends the write. A test blocks
    /// in it to hold one append between those two points while another append commits — the
    /// interleaving that decides whether a reader can be handed a seq above one still in flight.
    /// It runs on the bridge runtime inside `block_in_place`, so blocking in it is allowed.
    #[doc(hidden)]
    pub fn with_before_write(mut self, hook: impl Fn(&str) + Send + Sync + 'static) -> Self {
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
        match resp.items().first() {
            Some(item) if item.contains_key("next_seq") => parse_n(item, "next_seq"),
            _ => Ok(0),
        }
    }

    /// The counter's half of an append transaction: move `next_seq` to `slot.seq`, on the
    /// condition that it still stands at `slot.expected` (or, for `0`, that the stream has no
    /// counter yet). This condition is what makes a seq visible only together with its op: no
    /// round can take seq `n + 1` before the round that wrote `n` has committed. On a failed
    /// check DynamoDB hands back the counter as it found it, so the next round needs no read.
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

    /// One append round: ONE `TransactWriteItems` that moves the counter to `slot.seq` (index 0,
    /// see [`Self::counter_update`]), puts the op's claim at [`claim_seq`] when `claimed` (index
    /// 1), and puts the op at `slot.seq` (last). The claim and the op are each conditioned on
    /// their key being free — `attribute_not_exists(stream_id)`, which on a composite key reads
    /// "no item at this exact key", not "the stream is absent". So a seq is taken by exactly the
    /// transaction that also writes its op, and only once the seq below it is written: commit
    /// order is seq order (`6j6v.r3jf`).
    ///
    /// A taken claim means the op is already stored, so this answers with the seq the claim
    /// recorded — unless the claim was written for a different op id, in which case the next
    /// round stores the op without one.
    async fn try_append_op(
        client: &Client,
        table: &str,
        stream_id: &str,
        slot: Slot,
        op: &WireOp,
        claimed: bool,
    ) -> StoreResult<PutOutcome> {
        let claim = claim_seq(&op.op_id);
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
        let counter_code = reasons.first().and_then(|r| r.code());
        let claim_code = if claimed {
            reasons.get(1).and_then(|r| r.code())
        } else {
            None
        };
        let op_code = reasons
            .get(if claimed { 2 } else { 1 })
            .and_then(|r| r.code());
        // The counter as the failed check found it, when DynamoDB returned it; absent item or
        // attribute is a stream without a counter.
        let counter_seen = || -> StoreResult<Option<i64>> {
            match reasons.first().and_then(|r| r.item()) {
                Some(item) if item.contains_key("next_seq") => Ok(Some(parse_n(item, "next_seq")?)),
                Some(_) => Ok(Some(0)),
                None => Ok(None),
            }
        };
        match classify_append_cancellation(counter_code, claim_code, op_code) {
            Ok(AppendCancellation::Claimed) => {
                match Self::read_claim(client, table, stream_id, claim).await? {
                    Some((claimed_for, stored)) if claimed_for == op.op_id => {
                        Ok(PutOutcome::Existing(stored))
                    }
                    Some((claimed_for, _)) => {
                        eprintln!(
                            "nxf-relay: op {:?} shares its claim key {claim} with op \
                             {claimed_for:?} in stream {stream_id:?}; stored without a claim, \
                             so a re-push of it is stored again",
                            op.op_id
                        );
                        Ok(PutOutcome::ClaimForeign)
                    }
                    // Reported taken, read back absent: nothing deletes a claim today, so this is
                    // a race with something that does. Try again rather than fail the push.
                    None => Ok(PutOutcome::ClaimVanished),
                }
            }
            Ok(AppendCancellation::CounterMoved) => Ok(PutOutcome::Lost(counter_seen()?)),
            // Another transaction holds the counter (or the claim, for a racing re-push of this
            // very op): it is committing. The next round reads the counter and, should that
            // transaction have been this op's own, finds the claim taken.
            Ok(AppendCancellation::Conflict) => Ok(PutOutcome::Lost(None)),
            Ok(AppendCancellation::SeqTaken) => Ok(PutOutcome::SeqTaken(slot.seq)),
            Ok(AppendCancellation::Throttled) => Ok(PutOutcome::Throttled),
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
                Err(StoreError(message))
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
}

impl OpStore for DynamoDbOpStore {
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        let client = self.client.clone();
        let table = self.table.clone();
        let stream_id = stream.as_str().to_string();
        let op = op.clone();
        let before_write = self.before_write.clone();
        block_on(async move {
            // Each round reads the counter (unless the round before learned it), takes the seq
            // after it, and writes counter, claim and op in one transaction conditioned on the
            // counter not having moved (see `try_append_op` and the module doc). Uncontended, it
            // runs once: one read and one transaction.
            append_with_retry(&stream_id, |round| {
                // Re-borrowing inside the closure (rather than capturing it) keeps the
                // returned future borrowing the enclosing scope, not the closure itself —
                // the shape `FnMut(Round) -> impl Future` needs to type-check.
                let (client, table, stream_id) = (&client, table.as_str(), stream_id.as_str());
                let (op, before_write) = (&op, before_write.clone());
                async move {
                    let counter = match round.counter {
                        Some(counter) => counter,
                        None => Self::read_counter(client, table, stream_id).await?,
                    };
                    let slot = Slot::after(counter, round.floor);
                    if let Some(hook) = &before_write {
                        tokio::task::block_in_place(|| hook(&op.op_id));
                    }
                    Self::try_append_op(client, table, stream_id, slot, op, round.claimed).await
                }
            })
            .await
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
        assert_eq!(
            Slot::after(0, 0),
            Slot {
                expected: 0,
                seq: 1
            },
            "a stream without a counter starts at 1"
        );
        assert_eq!(
            Slot::after(9, 0),
            Slot {
                expected: 9,
                seq: 10
            }
        );
        assert_eq!(
            Slot::after(0, 1),
            Slot {
                expected: 0,
                seq: 2
            },
            "an op stored before the counter existed is stepped over, the counter still expected"
        );
        assert_eq!(
            Slot::after(12, 3),
            Slot {
                expected: 12,
                seq: 13
            },
            "a floor below the counter changes nothing"
        );
    }

    /// The rounds a stand-in was asked for, and an answer per round from `script`.
    fn scripted(
        script: Vec<PutOutcome>,
    ) -> (
        std::rc::Rc<std::cell::RefCell<Vec<Round>>>,
        impl FnMut(Round) -> std::future::Ready<StoreResult<PutOutcome>>,
    ) {
        let rounds = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&rounds);
        let mut script = script.into_iter();
        let round = move |r: Round| {
            seen.borrow_mut().push(r);
            std::future::ready(Ok(script.next().expect("the script covers every round")))
        };
        (rounds, round)
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
        let cursor = drive(append_with_retry("s", round)).expect("the third round lands");
        assert_eq!(cursor, Cursor(12));
        let counters: Vec<Option<i64>> = rounds.borrow().iter().map(|r| r.counter).collect();
        assert_eq!(
            counters,
            [None, Some(10), None],
            "read first; then the counter the failed check saw; read again when none came back"
        );
    }

    #[test]
    fn a_taken_seq_raises_the_floor_and_a_foreign_claim_drops_the_claim() {
        let (rounds, round) = scripted(vec![
            PutOutcome::SeqTaken(1),
            PutOutcome::ClaimForeign,
            PutOutcome::Written(2),
        ]);
        assert_eq!(drive(append_with_retry("s", round)).unwrap(), Cursor(2));
        let rounds = rounds.borrow();
        assert_eq!(
            rounds[1],
            Round {
                counter: None,
                floor: 1,
                claimed: true
            }
        );
        assert_eq!(
            rounds[2],
            Round {
                counter: None,
                floor: 1,
                claimed: false
            },
            "an op whose claim key is another op id's goes in unclaimed, as before the claims"
        );
    }

    #[test]
    fn append_gives_up_loudly_after_append_attempts_stalled_rounds() {
        // Never an unbounded spin: rounds that keep failing with no other writer committing are
        // structurally broken or saturated, and the caller must be told so.
        let (rounds, round) = scripted(
            (1..=i64::from(APPEND_ATTEMPTS))
                .map(PutOutcome::SeqTaken)
                .collect(),
        );
        let err = drive(append_with_retry("s", round)).expect_err("a stall must not loop forever");
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
        assert_eq!(drive(append_with_retry("s", round)).unwrap(), Cursor(99));

        // ...but they are bounded too, and the error says what happened.
        let (rounds, round) = scripted(
            (0..CONTENTION_ROUNDS)
                .map(|_| PutOutcome::Lost(None))
                .collect(),
        );
        let err = drive(append_with_retry("s", round)).expect_err("contention is bounded");
        assert_eq!(rounds.borrow().len() as u32, CONTENTION_ROUNDS);
        let err = err.to_string();
        assert!(
            err.contains("concurrent appends") && err.contains(&CONTENTION_ROUNDS.to_string()),
            "{err}"
        );
    }

    #[test]
    fn append_propagates_a_hard_error_from_the_round_immediately() {
        let calls = std::cell::Cell::new(0u32);
        let err = drive(append_with_retry("s", |_| {
            calls.set(calls.get() + 1);
            async { Err(StoreError("table gone".into())) }
        }))
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
        assert_eq!(drive(append_with_retry("s", round)).unwrap(), Cursor(3));
        assert!(
            started.elapsed() >= claim_backoff(0) + claim_backoff(1),
            "both throttled rounds waited at least their backoff"
        );
    }

    #[test]
    fn append_answers_an_already_stored_op_with_its_existing_seq() {
        let (_, round) = scripted(vec![PutOutcome::Existing(3)]);
        assert_eq!(drive(append_with_retry("s", round)).unwrap(), Cursor(3));
    }

    #[test]
    fn the_jitter_stays_inside_its_cap() {
        for lost in 0..40 {
            assert!(contention_jitter(lost) <= Duration::from_millis(16));
        }
        assert!(jitter(Duration::ZERO) <= Duration::from_micros(1));
        assert!(jitter(Duration::from_millis(5)) <= Duration::from_millis(5));
    }

    // ----- claim_seq + classify_append_cancellation --------------------------------------

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

    #[test]
    fn a_taken_claim_outranks_a_moved_counter_which_outranks_a_taken_seq() {
        let ccf = Some("ConditionalCheckFailed");
        let none = Some("None");
        assert_eq!(
            classify_append_cancellation(ccf, ccf, ccf),
            Ok(AppendCancellation::Claimed),
            "a re-push that lost the counter race still answers with its stored seq"
        );
        assert_eq!(
            classify_append_cancellation(Some("TransactionConflict"), ccf, none),
            Ok(AppendCancellation::Claimed)
        );
        assert_eq!(
            classify_append_cancellation(ccf, none, ccf),
            Ok(AppendCancellation::CounterMoved)
        );
        assert_eq!(
            classify_append_cancellation(ccf, None, none),
            Ok(AppendCancellation::CounterMoved),
            "an unclaimed round"
        );
        assert_eq!(
            classify_append_cancellation(none, none, ccf),
            Ok(AppendCancellation::SeqTaken)
        );
        assert_eq!(
            classify_append_cancellation(Some("TransactionConflict"), none, none),
            Ok(AppendCancellation::Conflict)
        );
        assert_eq!(
            classify_append_cancellation(
                none,
                Some("ThrottlingError"),
                Some("TransactionConflict")
            ),
            Ok(AppendCancellation::Conflict),
            "a conflict means another append is committing: contention, not a stall"
        );
        assert_eq!(
            classify_append_cancellation(None, None, Some("ProvisionedThroughputExceeded")),
            Ok(AppendCancellation::Throttled)
        );
    }

    #[test]
    fn any_other_append_cancellation_is_a_hard_error() {
        let err = classify_append_cancellation(Some("None"), Some("None"), Some("ValidationError"))
            .expect_err("a validation failure is a fault, never a retry");
        assert!(err.contains("ValidationError"), "{err}");
        assert!(classify_append_cancellation(None, None, None).is_err());
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
