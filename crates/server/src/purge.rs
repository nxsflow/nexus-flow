//! The whole-stream purge (6j6v.f6yb): remove everything the relay holds for ONE stream, on SQLite,
//! Postgres and DynamoDB, with a dry run that shows exactly what would go, a delete bound to that
//! dry run, a confirm run that proves nothing is left, a JSON report and fixed exit codes.
//!
//! It is the relay half of an account deletion (nexflow.it's published promise: deletion within
//! 30 days). The account orchestration — which streams, the fence, the order across the relay and
//! board-fold — is foundations' (kwv2) and follows consumer contract v1.3 (design §10 on the
//! ticket); this module is the tool it drives, through `nxf-relay purge` or [`purge_stream`].
//!
//! What a purge must never do (owner rule: correctness of a deletion over its date): touch anything
//! outside the one stream; say "deleted" when it deleted nothing, for example against the wrong
//! store; leave something behind and say it is complete. Each rule below exists for one of those.
//!
//! # The three invocations
//!
//! | Invocation | Deletes | Exit 0 means |
//! |---|---|---|
//! | `nxf-relay purge <id> --dry-run [--expect-identity <hex>]` | no | the plan was printed |
//! | `nxf-relay purge <id> --dry-run --expect-empty [--expect-identity <hex>]` (confirm) | no | nothing found, no lease item, on a store of the expected identity |
//! | `nxf-relay purge <id> --yes --plan <digest> [--expect-identity <hex>]` | yes | the plan still had that digest, it was deleted, a fresh read found nothing, and (SQLite) every checkpoint completed |
//!
//! Configuration is the environment only, with **no defaults** (a wrong default is a purge against
//! the wrong store): `NXF_RELAY_BACKEND`; SQLite `NXF_RELAY_DB` and `NXF_RELAY_REGISTRY_DB`;
//! Postgres `NXF_RELAY_PG_URL` (`DATABASE_URL` is NOT read); DynamoDB `NXF_RELAY_DDB_TABLE`,
//! `NXF_RELAY_DDB_REGISTRY_TABLE`, `NXF_RELAY_DDB_REGION`, optionally `NXF_RELAY_DDB_ENDPOINT`;
//! optionally `NXS_LOG_HMAC_KEY` (log-id v1). One JSON report on stdout; progress, with the stream
//! only as its `sid` token, on stderr.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | dry run printed; confirm found nothing; or delete ran, the verification found the stream empty and (SQLite) the checkpoint completed |
//! | 1 | a store error stopped the run; `deleted` says what went; a fresh dry run and a bound `--yes` finish |
//! | 2 | DynamoDB: the stream lease is held by another holder (nothing deleted) or was lost mid-run (`deleted` says what went) |
//! | 3 | delete: a writer added something after the plan (residue); confirm: something was found, a lease item included |
//! | 4 | delete: the plan differs from `--plan`; nothing deleted; take a fresh dry run |
//! | 5 | delete, SQLite: logically complete, but a WAL checkpoint stayed busy (`physical: "pending"`) |
//! | 64 | usage: the mode flags, `--keys`, a refused stream id (`refused.rule`), an unknown subcommand |
//! | 78 | configuration: a variable missing or malformed, a backend not in this build, not a relay store, `IdentityMismatch`, `EmptyStore`, `UnknownKind` |
//!
//! The table lives in one place, [`exit_code`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nxs_sync::protocol::StreamId;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::log_id::{Kind, LogKey};
use crate::registry::{LeaseItem, LeaseRenewed, LeaseTaken, StreamLease};
use crate::store::StoreError;

pub mod cli;
mod seams;
pub mod sqlite;
mod time;

#[cfg(feature = "dynamodb")]
pub mod ddb;
#[cfg(feature = "postgres")]
pub mod pg;

pub use seams::{InjectWrite, PurgeSeams, SeamAction};
pub use time::{format_utc, parse_rfc3339, Instant};

/// The report's format version.
pub const REPORT_FORMAT: u32 = 1;

/// The longest stream id a purge accepts, in bytes (MF-R3-4: ONE limit, driven by DynamoDB's
/// 1,024-byte sort key, which `stream#<id>` — the stream lease — must fit with room to spare).
pub const MAX_PURGE_ID_BYTES: usize = 1_000;

/// The DynamoDB kinds this build deletes: the ops partition's three, the registry partition's
/// three sort-key prefixes, `#due` and the stream lease. A registry prefix outside it is
/// `UnknownKind` (78) before anything is deleted, and the report carries it as `covers`.
pub const PURGE_COVERS: &[&str] = &[
    "ops",
    "claims",
    "counter",
    "registry:prefix#",
    "registry:uuid#",
    "registry:machine#",
    "due",
    "lease",
];

/// Above this many ops of one stream a SQLite `--yes` holds the write lock long enough that the
/// relay should be stopped first (the dry run says `large: true`; the runbook gives the measured
/// lock times). An initial value per MF-11, kept while the measured lock time at it stays under
/// the relay's `busy_timeout`.
pub const SQLITE_LARGE_OPS: u64 = 100_000;
/// The same rule by the size of the stream's op values, bytes.
pub const SQLITE_LARGE_VALUE_BYTES: u64 = 64 * 1024 * 1024;

/// The SQL tables this build deletes a stream's rows from. A table with a `stream_id` column
/// outside it is `UnknownKind` (78) before anything is deleted.
pub const SQL_PURGE_TABLES: &[&str] = &["stream_ops", "prefix_claims", "machine_presence"];

/// The SQL tables that hold no stream data and are allowed beside [`SQL_PURGE_TABLES`] (the schema
/// test asserts that every table of a relay store is in one of the two lists).
pub const SQL_ALLOWED_TABLES: &[&str] = &["relay_store_identity"];

/// Refuse a stream id the purge will not handle (exit 64), with the rule it broke: `empty`,
/// `leading_hash` (the `#due`/`#sweep` partitions), `slash` (an S3 prefix boundary), `control`
/// or `too_long` (over [`MAX_PURGE_ID_BYTES`]). The rule list is part of the consumer contract;
/// board-fold's purge keeps a copy held equal by a parity test.
pub fn validate_purge_id(stream: &StreamId) -> Result<(), PurgeError> {
    let id = stream.as_str();
    let rule = if id.is_empty() {
        "empty"
    } else if id.starts_with('#') {
        "leading_hash"
    } else if id.contains('/') {
        "slash"
    } else if id.chars().any(char::is_control) {
        "control"
    } else if id.len() > MAX_PURGE_ID_BYTES {
        "too_long"
    } else {
        return Ok(());
    };
    Err(PurgeError::InvalidStreamId { rule })
}

/// Why a purge stopped. Every variant maps to one exit code in [`exit_code`].
#[non_exhaustive]
#[derive(Debug)]
pub enum PurgeError {
    /// 64: [`validate_purge_id`] refused the id.
    InvalidStreamId { rule: &'static str },
    /// 64: the command line is not one of the three invocations.
    Usage(String),
    /// 78: a required variable is missing or malformed, or names a backend this build lacks.
    Config(String),
    /// 78: missing table, wrong key schema, swapped or identical tables, no identity row.
    NotARelayStore(String),
    /// 78: `--expect-identity` differs from the store's identity digest.
    IdentityMismatch,
    /// 78: the plan found nothing and the store holds nothing (waived by `--expect-identity`).
    EmptyStore,
    /// 78: a registry prefix or a SQL table with a `stream_id` column this build does not cover.
    UnknownKind(Vec<String>),
    /// 4: the plan is not the one `--plan` names; nothing deleted.
    PlanChanged,
    /// 2: the stream lease is held by another holder; nothing deleted.
    LeaseHeld,
    /// 2: the stream lease was lost mid-run; `deleted` counts what went.
    LeaseLost,
    /// 78: the store has no purge.
    Unsupported,
    /// 1: a store error.
    Store(StoreError),
}

impl PurgeError {
    /// The variant's name, as the report's `error.kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            PurgeError::InvalidStreamId { .. } => "InvalidStreamId",
            PurgeError::Usage(_) => "Usage",
            PurgeError::Config(_) => "Config",
            PurgeError::NotARelayStore(_) => "NotARelayStore",
            PurgeError::IdentityMismatch => "IdentityMismatch",
            PurgeError::EmptyStore => "EmptyStore",
            PurgeError::UnknownKind(_) => "UnknownKind",
            PurgeError::PlanChanged => "PlanChanged",
            PurgeError::LeaseHeld => "LeaseHeld",
            PurgeError::LeaseLost => "LeaseLost",
            PurgeError::Unsupported => "Unsupported",
            PurgeError::Store(_) => "Store",
        }
    }
}

impl fmt::Display for PurgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PurgeError::InvalidStreamId { rule } => {
                write!(f, "the stream id is refused (rule {rule})")
            }
            PurgeError::Usage(m) | PurgeError::Config(m) => f.write_str(m),
            PurgeError::NotARelayStore(m) => write!(f, "not a relay store: {m}"),
            PurgeError::IdentityMismatch => f.write_str(
                "the store's identity digest differs from --expect-identity; this is not the \
                 store the pin names",
            ),
            PurgeError::EmptyStore => f.write_str(
                "the plan found nothing and the store holds nothing; pass --expect-identity to \
                 confirm this is the right store",
            ),
            PurgeError::UnknownKind(names) => write!(
                f,
                "this build does not cover {}; nothing is deleted — run a build whose covers \
                 include it",
                names.join(", ")
            ),
            PurgeError::PlanChanged => f.write_str(
                "the plan is not the one --plan names; nothing was deleted; take a fresh dry run",
            ),
            PurgeError::LeaseHeld => {
                f.write_str("the stream lease is held by another holder; nothing was deleted")
            }
            PurgeError::LeaseLost => f.write_str("the stream lease was lost during the run"),
            PurgeError::Unsupported => f.write_str("this store has no purge"),
            PurgeError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PurgeError {}

impl From<StoreError> for PurgeError {
    fn from(e: StoreError) -> PurgeError {
        PurgeError::Store(e)
    }
}

/// A SHA-256 digest, written and read as 64 lowercase hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256Hex(pub [u8; 32]);

impl Sha256Hex {
    fn of(bytes: &[u8]) -> Sha256Hex {
        Sha256Hex(Sha256::digest(bytes).into())
    }

    /// Parse 64 hex characters (either case); anything else is `None`.
    pub fn parse(text: &str) -> Option<Sha256Hex> {
        if text.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, pair) in text.as_bytes().chunks(2).enumerate() {
            let pair = std::str::from_utf8(pair).ok()?;
            out[i] = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Sha256Hex(out))
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The digest a dry run prints and `--yes --plan` must repeat.
pub type PlanDigest = Sha256Hex;
/// SHA-256 of the canonical `store.identity`; what `--expect-identity` pins.
pub type IdentityDigest = Sha256Hex;

/// The identity digest of a store: SHA-256 over the canonical JSON of its identity document.
pub fn identity_digest(identity: &Value) -> IdentityDigest {
    Sha256Hex::of(canonical_json(identity).as_bytes())
}

/// JSON with every object's keys sorted, whatever map type `serde_json` was built with — so a
/// digest never depends on insertion order.
pub fn canonical_json(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), sorted(&map[k]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    // With serde_json's default map the keys are already sorted; with `preserve_order` they are
    // inserted in sorted order above. Either way the text is canonical.
    serde_json::to_string(&sorted(value)).expect("a JSON value serializes")
}

// ----- the plan ---------------------------------------------------------------------------------

/// One key a plan found. The digest covers every key; the report prints none of them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanKey {
    /// An op: its seq, and its op id (MF-R4-1: the digest is bound to the stream's contents, so
    /// two streams with equal counts and equal clocks never share a digest).
    Op { seq: i64, op_id: String },
    /// A DynamoDB range key that is a number: a claim, the counter.
    Seq(i64),
    /// A DynamoDB range key that is a string: a registry `sk`, or the `#due` item's `sk`.
    Sk(String),
    /// A SQL row outside `stream_ops`: its table and its key columns other than `stream_id`.
    Row {
        table: &'static str,
        key: String,
        extra: String,
    },
}

impl PlanKey {
    fn canonical(&self) -> Value {
        match self {
            PlanKey::Op { seq, op_id } => json!([seq, op_id]),
            PlanKey::Seq(seq) => json!(seq),
            PlanKey::Sk(sk) => json!(sk),
            PlanKey::Row { table, key, extra } => json!([table, key, extra]),
        }
    }
}

/// What the plan read about the ops it found: never a value, only what the report says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpsFacts {
    /// The latest `wall_clock` that parses as RFC 3339, normalised to UTC.
    pub newest_wall_clock: Option<Instant>,
    /// How many `wall_clock`s did not parse.
    pub wall_clock_unparseable: u64,
    /// SQL: the sum of `LENGTH(CAST(value AS BLOB))`, bytes; `None` on DynamoDB.
    pub value_bytes: Option<u64>,
}

/// One kind's part of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedKind {
    /// `ops`, `claims`, `counter`, `registry` or `due`.
    pub kind: &'static str,
    /// The keys inside the digest, sorted.
    pub keys: Vec<PlanKey>,
    /// Registry items whose `expires_at` lies before the plan's start: deleted, counted as
    /// `expired`, outside the digest (DynamoDB's TTL removes them on its own schedule).
    pub expired: Vec<PlanKey>,
    /// Registry: items per sort-key prefix (SQL: per table), expired ones included.
    pub by_prefix: BTreeMap<String, u64>,
}

impl PlannedKind {
    pub fn new(kind: &'static str) -> PlannedKind {
        PlannedKind {
            kind,
            keys: Vec::new(),
            expired: Vec::new(),
            by_prefix: BTreeMap::new(),
        }
    }

    /// Every key this kind would delete: the digest's and the expired ones.
    pub fn all_keys(&self) -> impl Iterator<Item = &PlanKey> {
        self.keys.iter().chain(self.expired.iter())
    }

    pub fn found(&self) -> u64 {
        (self.keys.len() + self.expired.len()) as u64
    }
}

/// Everything a purge read about one stream: what a delete removes, and the facts the report
/// carries. A delete deletes exactly the keys of the plan it was bound to (DynamoDB), or reads the
/// plan again inside its transaction and refuses on any difference (SQL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// In the backend's report order.
    pub kinds: Vec<PlannedKind>,
    pub ops: OpsFacts,
    /// Registry prefixes or SQL tables this build does not cover, already named for output.
    pub unknown: Vec<String>,
    /// DynamoDB: the stream lease item as the plan found it (outside the digest).
    pub lease: Option<LeaseItem>,
    /// The latest `last_seen` among the stream's presence rows/items (the relay's clock, Unix
    /// seconds).
    pub newest_last_seen: Option<i64>,
}

impl Plan {
    pub fn kind(&self, kind: &str) -> Option<&PlannedKind> {
        self.kinds.iter().find(|k| k.kind == kind)
    }

    /// The highest op seq found.
    pub fn max_seq(&self) -> Option<i64> {
        self.kind("ops")?
            .all_keys()
            .filter_map(|k| match k {
                PlanKey::Op { seq, .. } | PlanKey::Seq(seq) => Some(*seq),
                _ => None,
            })
            .max()
    }

    /// Nothing found in any kind (the lease is not a kind).
    pub fn is_empty(&self) -> bool {
        self.kinds.iter().all(|k| k.found() == 0)
    }
}

/// The plan's digest: SHA-256 over the canonical JSON of `{format, identity, kinds[] (kind, found,
/// seq_ranges, every key), max_seq, newest_wall_clock}`. No stream id and no partition key in it
/// (design round 4, Integrity 2), no lease, no expired registry item, no `last_seen`.
pub fn plan_digest(identity: &Value, plan: &Plan) -> PlanDigest {
    let kinds: Vec<Value> = plan
        .kinds
        .iter()
        .map(|k| {
            let mut keys = k.keys.clone();
            keys.sort();
            let mut doc = json!({
                "kind": k.kind,
                "found": keys.len(),
                "keys": keys.iter().map(PlanKey::canonical).collect::<Vec<_>>(),
            });
            if k.kind == "ops" {
                let seqs = keys.iter().filter_map(|key| match key {
                    PlanKey::Op { seq, .. } | PlanKey::Seq(seq) => Some(*seq),
                    _ => None,
                });
                doc["seq_ranges"] = json!(seq_ranges(seqs));
            }
            doc
        })
        .collect();
    let doc = json!({
        "format": REPORT_FORMAT,
        "identity": identity,
        "kinds": kinds,
        "max_seq": plan.max_seq(),
        "newest_wall_clock": plan.ops.newest_wall_clock.map(format_utc),
    });
    Sha256Hex::of(canonical_json(&doc).as_bytes())
}

/// Compress sorted seqs into inclusive runs: `[1, 2, 3, 7]` → `[[1, 3], [7, 7]]`.
pub fn seq_ranges(seqs: impl IntoIterator<Item = i64>) -> Vec<[i64; 2]> {
    let mut seqs: Vec<i64> = seqs.into_iter().collect();
    seqs.sort_unstable();
    seqs.dedup();
    let mut ranges: Vec<[i64; 2]> = Vec::new();
    for seq in seqs {
        match ranges.last_mut() {
            Some(last) if last[1].checked_add(1) == Some(seq) => last[1] = seq,
            _ => ranges.push([seq, seq]),
        }
    }
    ranges
}

/// An unknown registry kind, named for output: its prefix (up to and including the first `#`)
/// when it matches `^[a-z_]{1,32}#$`, else `<nonconforming, <n> bytes>` — so a key can never
/// carry an id into a log by way of the error that refuses it.
pub fn name_unknown_prefix(sk: &str) -> String {
    let prefix = sk.find('#').map(|i| &sk[..=i]);
    match prefix {
        Some(p)
            if (2..=33).contains(&p.len())
                && p[..p.len() - 1]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_') =>
        {
            p.to_string()
        }
        _ => format!("<nonconforming, {} bytes>", sk.len()),
    }
}

/// An unknown SQL table, named for output when it matches `^[a-z_][a-z0-9_]{0,62}$`, else
/// `<nonconforming, <n> bytes>`.
pub fn name_unknown_table(table: &str) -> String {
    let b = table.as_bytes();
    let conforms = (1..=63).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
    if conforms {
        table.to_string()
    } else {
        format!("<nonconforming, {} bytes>", table.len())
    }
}

// ----- the store side ---------------------------------------------------------------------------

/// Where a purge runs, and the identity that pins it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreIdentity {
    /// `sqlite`, `postgres` or `dynamodb`.
    pub backend: &'static str,
    /// Where it runs, for the report only: paths, host, tables, region. Never digested.
    pub locator: serde_json::Map<String, Value>,
    /// The canonical identity; [`identity_digest`] is its SHA-256.
    pub identity: Value,
    /// Whether the store holds any op at all (SQL: a `stream_ops` row; DynamoDB: `ItemCount > 0`).
    pub populated: bool,
}

/// The counts a delete has removed so far, per kind. Updated as each batch or statement
/// completes, so an error mid-run still reports exactly what went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deleted(pub BTreeMap<&'static str, u64>);

impl Deleted {
    pub fn add(&mut self, kind: &'static str, n: u64) {
        *self.0.entry(kind).or_insert(0) += n;
    }

    pub fn get(&self, kind: &str) -> u64 {
        self.0.get(kind).copied().unwrap_or(0)
    }
}

/// What a backend says about physical erasure after a delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Physical {
    /// SQLite: `secure_delete` was on for this run's deletes and every WAL checkpoint completed.
    /// Says nothing about pages freed earlier without `secure_delete`, nor about blocks a
    /// copy-on-write file system or a backup keeps.
    FileLevel,
    /// SQLite: a checkpoint stayed busy (exit 5).
    Pending,
    /// Postgres: dead tuples, TOAST and WAL until VACUUM, page reuse and recycling — the operator's.
    NotAttempted,
    /// DynamoDB: the provider's.
    Provider,
}

/// One relay store, opened for a purge. Implemented per backend in [`sqlite`], `pg` and `ddb`.
///
/// Not methods on `OpStore`/`PrefixRegistry` (design §2 A as first written): on SQL the delete is
/// ONE transaction over the op log's and the registry's tables, which two independently locked
/// store objects cannot give, and on Postgres it is one connection of the purge's own rather than
/// the server's pools. So one value holds the whole store. The DynamoDB stream lease stays where
/// the sweep reaches it too: [`crate::registry::PrefixRegistry::stream_lease`].
pub trait PurgeStore: Send + Sync {
    /// `sqlite`, `postgres` or `dynamodb`.
    fn backend(&self) -> &'static str;

    /// The kinds this store's report lists, in order.
    fn kinds(&self) -> &'static [&'static str];

    /// What this build deletes on this backend (the report's `covers`).
    fn covers(&self) -> Vec<String>;

    /// Where the store is and what pins it; refuses a store that is not a relay store.
    fn identity(&self) -> Result<StoreIdentity, PurgeError>;

    /// Read everything the stream holds (keys only; never an op value), including the
    /// unknown-kind guard. Calls `ctx.plan_page()` after every page it reads.
    fn plan(&self, stream: &StreamId, ctx: &mut RunCtx<'_>) -> Result<Plan, PurgeError>;

    /// Delete the plan. DynamoDB deletes exactly its keys; SQL reads the plan again inside its
    /// transaction and answers [`PurgeError::PlanChanged`] on any difference from `expected`.
    /// Counts what went into `deleted` as it goes.
    fn delete(
        &self,
        stream: &StreamId,
        plan: &Plan,
        expected: &PlanDigest,
        identity: &Value,
        ctx: &mut RunCtx<'_>,
        deleted: &mut Deleted,
    ) -> Result<Physical, PurgeError>;

    /// The DynamoDB stream lease; `None` on SQL (design §7a.7).
    fn stream_lease(&self) -> Option<&dyn StreamLease> {
        None
    }

    /// Test seam: one write of the target through the store's own path, with a new key.
    #[cfg(feature = "test-seams")]
    fn inject(&self, stream: &StreamId, write: InjectWrite) -> Result<(), StoreError>;
}

/// What a purge needs: the store. (5vxp-a's sweep holds one of these.)
#[derive(Clone)]
pub struct PurgeDeps {
    pub store: Arc<dyn PurgeStore>,
}

/// Which of the three invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeMode {
    /// `--dry-run`, or with `expect_empty` the confirm run `--dry-run --expect-empty`.
    DryRun { expect_empty: bool },
    /// `--yes --plan <digest>`.
    Delete { plan: PlanDigest },
}

impl PurgeMode {
    fn label(self) -> &'static str {
        match self {
            PurgeMode::DryRun {
                expect_empty: false,
            } => "dry_run",
            PurgeMode::DryRun { expect_empty: true } => "confirm",
            PurgeMode::Delete { .. } => "delete",
        }
    }
}

/// What every mode may carry.
#[derive(Debug, Clone, Default)]
pub struct PurgeOptions {
    /// `--expect-identity`: refuse another store (78) and waive `EmptyStore`.
    pub expect_identity: Option<IdentityDigest>,
    /// `NXS_LOG_HMAC_KEY` (log-id v1 C1); `None` writes `sid` as `"-"`.
    pub log_key: Option<LogKey>,
}

/// A purge's result: the report as far as the run got, and the error that stopped it, if any. The
/// report travels with every error, so `deleted` says what went before an exit 1 or 2.
#[derive(Debug)]
pub struct PurgeOutcome {
    pub report: PurgeReport,
    pub error: Option<PurgeError>,
}

// ----- the report -------------------------------------------------------------------------------

/// The one JSON document on stdout.
#[derive(Debug, Clone, Serialize)]
pub struct PurgeReport {
    pub format: u32,
    pub tool_version: &'static str,
    pub covers: Vec<String>,
    /// The stream id as its log-id v1 value, `h1:<kid>:<hex24>`, or `"-"` without a key.
    pub sid: String,
    pub mode: &'static str,
    pub store: Option<StoreReport>,
    pub plan_digest: Option<String>,
    pub kinds: Vec<KindReport>,
    pub machines: MachinesReport,
    /// SQLite: the plan is above [`SQLITE_LARGE_OPS`] ops or [`SQLITE_LARGE_VALUE_BYTES`] of
    /// values — stop the relay before the `--yes` (the runbook's rule, MF-11). `null` elsewhere.
    pub large: Option<bool>,
    pub lease: Option<LeaseReport>,
    pub present: Option<PresentReport>,
    pub physical: Option<Physical>,
    /// What the verification read found after a delete, per kind; `null` when it did not run.
    pub residue: Option<Vec<ResidueEntry>>,
    pub refused: Option<RefusedReport>,
    pub complete: bool,
    pub error: Option<ErrorReport>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreReport {
    pub backend: &'static str,
    #[serde(flatten)]
    pub locator: serde_json::Map<String, Value>,
    pub identity: Value,
    pub identity_digest: String,
    pub populated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindReport {
    pub kind: &'static str,
    pub found: u64,
    /// `null` outside a delete.
    pub deleted: Option<u64>,
    #[serde(flatten)]
    pub detail: KindDetail,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum KindDetail {
    Ops {
        seq_ranges: Vec<[i64; 2]>,
        max_seq: Option<i64>,
        newest_wall_clock: Option<String>,
        wall_clock_unparseable: u64,
        value_bytes: Option<u64>,
    },
    Registry {
        by_prefix: BTreeMap<String, u64>,
        expired: u64,
    },
    Plain {},
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MachinesReport {
    pub newest_last_seen: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LeaseReport {
    pub found: bool,
    pub expired: Option<bool>,
    pub taken: Option<bool>,
    pub renewals: Option<u32>,
    pub released: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresentReport {
    pub counter: bool,
    pub registry: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResidueEntry {
    pub kind: &'static str,
    pub count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_prefix: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefusedReport {
    pub rule: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorReport {
    pub kind: &'static str,
    pub message: String,
}

/// The exit code of an outcome — design §6's table, the one place it lives.
pub fn exit_code(outcome: &PurgeOutcome) -> u8 {
    if let Some(error) = &outcome.error {
        return match error {
            PurgeError::InvalidStreamId { .. } | PurgeError::Usage(_) => 64,
            PurgeError::PlanChanged => 4,
            PurgeError::LeaseHeld | PurgeError::LeaseLost => 2,
            PurgeError::Store(_) => 1,
            PurgeError::Config(_)
            | PurgeError::NotARelayStore(_)
            | PurgeError::IdentityMismatch
            | PurgeError::EmptyStore
            | PurgeError::UnknownKind(_)
            | PurgeError::Unsupported => 78,
        };
    }
    let report = &outcome.report;
    match report.mode {
        "confirm" => {
            let found = report.kinds.iter().any(|k| k.found > 0);
            let lease = report.lease.as_ref().is_some_and(|l| l.found);
            if found || lease {
                3
            } else {
                0
            }
        }
        "delete" => {
            if report.residue.as_ref().is_some_and(|r| !r.is_empty()) {
                3
            } else if report.physical == Some(Physical::Pending) {
                5
            } else {
                0
            }
        }
        _ => 0,
    }
}

// ----- the run ----------------------------------------------------------------------------------

/// The clock a run reads. The system's in production; a fake in the lease rule's tests.
pub type Clock<'a> = &'a (dyn Fn() -> SystemTime + Sync);

/// Unix seconds of `t`, saturating at 0 before the epoch.
pub fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn utc_text(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    format_utc(Instant {
        secs: d.as_secs() as i64,
        nanos: 0,
    })
}

/// The stream lease a delete run holds (DynamoDB), and the rule of design §7a.3 that keeps it.
struct HeldLease<'a> {
    lease: &'a dyn StreamLease,
    stream: StreamId,
    holder: String,
    /// When the lease expires, Unix seconds, as this run last wrote it.
    expires_at: i64,
    renewals: u32,
}

/// Plan pages between two looks at the lease (design §7a.3: the plan can outlast the lease).
pub const LEASE_CHECK_PAGES: u64 = 50;

/// A plan that reads more pages than this has not converged: a writer outpaces the read, which
/// the fence should make impossible (exit 1).
pub const PLAN_MAX_PAGES: u64 = 100_000;

/// What a backend's `plan` and `delete` are handed: the seams, the clock, the lease rule and the
/// progress line.
pub struct RunCtx<'a> {
    seams: &'a PurgeSeams,
    clock: Clock<'a>,
    lease: Option<HeldLease<'a>>,
    pages_since_check: u64,
    pages_total: u64,
    plan_started: SystemTime,
    sid: String,
    progress: &'a mut dyn FnMut(&str),
    #[cfg(feature = "test-seams")]
    injector: Option<(&'a dyn PurgeStore, StreamId)>,
}

impl<'a> RunCtx<'a> {
    /// A context outside a run, for a backend's own unit tests.
    #[cfg(test)]
    pub(crate) fn for_test(
        seams: &'a PurgeSeams,
        clock: Clock<'a>,
        progress: &'a mut dyn FnMut(&str),
    ) -> RunCtx<'a> {
        RunCtx {
            seams,
            clock,
            lease: None,
            pages_since_check: 0,
            pages_total: 0,
            plan_started: clock(),
            sid: "sid=-".into(),
            progress,
            #[cfg(feature = "test-seams")]
            injector: None,
        }
    }
}

impl RunCtx<'_> {
    /// The run's clock.
    pub fn now(&self) -> SystemTime {
        (self.clock)()
    }

    /// When the current plan started reading (registry items that expired before it are outside
    /// the digest).
    pub fn plan_started(&self) -> SystemTime {
        self.plan_started
    }

    /// The plan's page size.
    pub fn page(&self) -> usize {
        self.seams.page()
    }

    /// One progress line on stderr; the stream is always named by its `sid` token.
    pub fn progress(&mut self, message: &str) {
        let line = format!("nxf-relay purge: {} {message}", self.sid);
        (self.progress)(&line);
    }

    /// A plan read one more page: every [`LEASE_CHECK_PAGES`] look at the lease, and give up after
    /// [`PLAN_MAX_PAGES`].
    pub fn plan_page(&mut self) -> Result<(), PurgeError> {
        self.pages_total += 1;
        if self.pages_total > PLAN_MAX_PAGES {
            return Err(PurgeError::Store(StoreError(
                "plan did not converge: more pages than the plan may read".into(),
            )));
        }
        self.pages_since_check += 1;
        if self.pages_since_check >= LEASE_CHECK_PAGES {
            self.pages_since_check = 0;
            self.keep_lease()?;
        }
        Ok(())
    }

    /// Before every delete batch: the lease rule.
    pub fn before_batch(&mut self) -> Result<(), PurgeError> {
        self.keep_lease()
    }

    /// Design §7a.3. (i) Past `expires_at - margin` the lease may have expired and been taken
    /// meanwhile: `LeaseLost`, whether or not another holder is visible now. (ii) Less than half
    /// the ttl left: renew; a failed condition is `LeaseLost`, any other failure `Store`.
    fn keep_lease(&mut self) -> Result<(), PurgeError> {
        let Some(held) = self.lease.as_mut() else {
            return Ok(());
        };
        let now = (self.clock)();
        let ttl = self.seams.lease_ttl();
        let margin = self.seams.lease_margin();
        let now_ms = millis(now);
        let expires_ms = held.expires_at.saturating_mul(1000);
        if now_ms >= expires_ms - margin.as_millis() as i64 {
            return Err(PurgeError::LeaseLost);
        }
        if expires_ms - now_ms >= ttl.as_millis() as i64 / 2 {
            return Ok(());
        }
        if self.seams.action_at("renew") == SeamAction::Fail {
            return Err(PurgeError::Store(StoreError(
                "the lease renewal failed (test seam)".into(),
            )));
        }
        match held.lease.renew(&held.stream, &held.holder, ttl, now) {
            Ok(LeaseRenewed::Renewed { expires_at }) => {
                held.expires_at = expires_at;
                held.renewals += 1;
                Ok(())
            }
            Ok(LeaseRenewed::Lost) => Err(PurgeError::LeaseLost),
            Err(e) => Err(PurgeError::Store(e)),
        }
    }

    /// The seam at `point`: abort, inject, fail or pause under `test-seams`; nothing otherwise.
    pub fn reached(&mut self, point: &str) -> Result<(), PurgeError> {
        match self.seams.action_at(point) {
            SeamAction::None => Ok(()),
            SeamAction::Stop => {
                self.progress(&format!("stop at {point} (test seam)"));
                std::process::abort()
            }
            SeamAction::Fail => Err(PurgeError::Store(StoreError(format!(
                "failure injected at {point} (test seam)"
            )))),
            SeamAction::Pause(d) => {
                std::thread::sleep(d);
                Ok(())
            }
            SeamAction::Inject(write) => self.inject(point, write),
        }
    }

    #[cfg(feature = "test-seams")]
    fn inject(&mut self, point: &str, write: InjectWrite) -> Result<(), PurgeError> {
        self.progress(&format!("inject {write:?} at {point} (test seam)"));
        let Some((store, stream)) = &self.injector else {
            return Ok(());
        };
        store.inject(stream, write).map_err(PurgeError::Store)
    }

    #[cfg(not(feature = "test-seams"))]
    fn inject(&mut self, _point: &str, _write: InjectWrite) -> Result<(), PurgeError> {
        Ok(())
    }
}

/// Purge one stream. See the module doc for the modes and [`exit_code`] for what the outcome
/// means. Progress goes to stderr.
pub fn purge_stream(
    deps: &PurgeDeps,
    stream: &StreamId,
    mode: PurgeMode,
    opts: &PurgeOptions,
    seams: &PurgeSeams,
) -> PurgeOutcome {
    let clock = SystemTime::now;
    let mut progress = |line: &str| eprintln!("{line}");
    purge_stream_with(deps, stream, mode, opts, seams, &clock, &mut progress)
}

/// [`purge_stream`] with an explicit clock and progress sink (tests).
pub fn purge_stream_with(
    deps: &PurgeDeps,
    stream: &StreamId,
    mode: PurgeMode,
    opts: &PurgeOptions,
    seams: &PurgeSeams,
    clock: Clock<'_>,
    progress: &mut dyn FnMut(&str),
) -> PurgeOutcome {
    let started = clock();
    let sid = sid_value(opts.log_key.as_ref(), stream);
    let mut report = new_report(mode, sid.clone(), started);
    let mut ctx = RunCtx {
        seams,
        clock,
        lease: None,
        pages_since_check: 0,
        pages_total: 0,
        plan_started: started,
        sid: format!("sid={sid}"),
        progress,
        #[cfg(feature = "test-seams")]
        injector: Some((deps.store.as_ref(), stream.clone())),
    };
    if opts.log_key.is_none() {
        ctx.progress(
            "log ids are off: NXS_LOG_HMAC_KEY is not set, so the stream is written sid=-",
        );
    }
    ctx.progress(&format!("{} started", report.mode));
    let error = run(
        deps.store.as_ref(),
        stream,
        mode,
        opts,
        &mut ctx,
        &mut report,
    )
    .err();
    report.finished_at = utc_text(clock());
    if let Some(e) = &error {
        report.error = Some(ErrorReport {
            kind: e.kind(),
            message: e.to_string(),
        });
    }
    let outcome = PurgeOutcome { report, error };
    let code = exit_code(&outcome);
    match &outcome.error {
        Some(e) => ctx.progress(&format!("stopped: {} (exit {code})", e.kind())),
        None => ctx.progress(&format!("finished (exit {code})")),
    }
    outcome
}

fn millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 128 random bits as 32 lowercase hex characters: a lease holder, a store id. Drawn from the
/// standard library's randomly keyed hasher (seeded by the OS per process) over the time, the
/// process and a counter — unique, which is all either needs; neither is a secret.
pub(crate) fn random_hex32() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = String::with_capacity(32);
    for half in 0..2u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(half);
        h.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        h.write_u32(std::process::id());
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// The op an injection appends (test seam): a new op id, so no planned batch deletes it.
#[cfg(feature = "test-seams")]
pub(crate) fn injected_op() -> nxs_sync::wire::WireOp {
    nxs_sync::wire::WireOp {
        envelope_version: nxs_sync::wire::ENVELOPE_VERSION,
        op_id: format!("inject-{}", random_hex32()),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "inject.0001".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("injected by the purge test seam".into()),
        author: "purge-test-seam".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

/// The machine an injection announces (test seam): a new machine id.
#[cfg(feature = "test-seams")]
pub(crate) fn injected_hello() -> nxs_sync::protocol::MachineHello {
    nxs_sync::protocol::MachineHello {
        machine_id: format!("inject-{}", &random_hex32()[..16]),
        name: "Injected machine".into(),
        interval_secs: 300,
    }
}

/// The prefix and replica uuid an injection registers (test seam): both new.
#[cfg(feature = "test-seams")]
pub(crate) fn injected_claim() -> (String, String) {
    let id = random_hex32();
    // Four characters of the relay's prefix alphabet; collisions are reassigned, which is fine.
    (format!("z{}", &id[..3]), format!("inject-{id}"))
}

/// Releases the stream lease if the run unwinds before its explicit release (design §7a.4). A
/// process abort releases nothing; the lease then expires on its own after at most its ttl.
struct ReleaseOnUnwind<'a> {
    lease: &'a dyn StreamLease,
    stream: &'a StreamId,
    holder: &'a str,
    armed: bool,
}

impl Drop for ReleaseOnUnwind<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.lease.release(self.stream, self.holder);
        }
    }
}

fn run<'a>(
    store: &'a dyn PurgeStore,
    stream: &StreamId,
    mode: PurgeMode,
    opts: &PurgeOptions,
    ctx: &mut RunCtx<'a>,
    report: &mut PurgeReport,
) -> Result<(), PurgeError> {
    if let Err(e) = validate_purge_id(stream) {
        if let PurgeError::InvalidStreamId { rule } = &e {
            report.refused = Some(RefusedReport { rule });
        }
        return Err(e);
    }
    report.covers = store.covers();
    let identity = store.identity()?;
    let digest = identity_digest(&identity.identity);
    report.store = Some(StoreReport {
        backend: identity.backend,
        locator: identity.locator.clone(),
        identity: identity.identity.clone(),
        identity_digest: digest.to_string(),
        populated: identity.populated,
    });
    ctx.progress(&format!(
        "store {} identity_digest={digest} populated={}",
        identity.backend, identity.populated
    ));
    if let Some(expected) = opts.expect_identity {
        if expected != digest {
            return Err(PurgeError::IdentityMismatch);
        }
    }
    let expected = match mode {
        PurgeMode::DryRun { .. } => {
            read_plan(store, stream, &identity, opts, ctx, report)?;
            return Ok(());
        }
        PurgeMode::Delete { plan } => plan,
    };
    let Some(lease) = store.stream_lease() else {
        return delete(store, stream, &expected, &identity, opts, ctx, report);
    };

    // DynamoDB: take the stream lease (L0) before the plan, release it after whatever happened.
    let holder = random_hex32();
    let now = ctx.now();
    match lease.take(stream, &holder, ctx.seams.lease_ttl(), now)? {
        LeaseTaken::Held(_) => {
            report.lease = Some(LeaseReport {
                found: true,
                expired: Some(false),
                taken: Some(false),
                renewals: Some(0),
                released: None,
            });
            Err(PurgeError::LeaseHeld)
        }
        LeaseTaken::Taken {
            expires_at,
            previous,
        } => {
            report.lease = Some(LeaseReport {
                found: previous.is_some(),
                expired: previous.map(|p| p.expires_at < unix_secs(now)),
                taken: Some(true),
                renewals: Some(0),
                released: None,
            });
            let mut guard = ReleaseOnUnwind {
                lease,
                stream,
                holder: &holder,
                armed: true,
            };
            ctx.lease = Some(HeldLease {
                lease,
                stream: stream.clone(),
                holder: holder.clone(),
                expires_at,
                renewals: 0,
            });
            ctx.progress("lease taken");
            let result = ctx
                .reached("L0")
                .and_then(|()| delete(store, stream, &expected, &identity, opts, ctx, report));
            let held = ctx.lease.take();
            guard.armed = false;
            let released = lease.release(stream, &holder);
            if let Some(l) = report.lease.as_mut() {
                l.renewals = held.map(|h| h.renewals);
                l.released = Some(matches!(released, Ok(true)));
            }
            match &released {
                Ok(true) => ctx.progress("lease released"),
                Ok(false) => ctx.progress("lease not released: it was not ours any more"),
                Err(_) => ctx.progress("lease release failed; it expires on its own"),
            }
            result?;
            ctx.reached("L9")
        }
    }
}

/// Read the plan and put everything it found into the report. Refuses an unknown kind and an
/// empty store (without a pin).
fn read_plan(
    store: &dyn PurgeStore,
    stream: &StreamId,
    identity: &StoreIdentity,
    opts: &PurgeOptions,
    ctx: &mut RunCtx<'_>,
    report: &mut PurgeReport,
) -> Result<Plan, PurgeError> {
    ctx.plan_started = ctx.now();
    let plan = store.plan(stream, ctx)?;
    if !plan.unknown.is_empty() {
        return Err(PurgeError::UnknownKind(plan.unknown.clone()));
    }
    report.kinds = kind_reports(store.kinds(), &plan);
    if store.backend() == "sqlite" {
        let ops = plan.kind("ops").map_or(0, PlannedKind::found);
        let bytes = plan.ops.value_bytes.unwrap_or(0);
        report.large = Some(ops > SQLITE_LARGE_OPS || bytes > SQLITE_LARGE_VALUE_BYTES);
    }
    report.machines.newest_last_seen = plan
        .newest_last_seen
        .map(|secs| format_utc(Instant { secs, nanos: 0 }));
    let found = |kind: &str| plan.kind(kind).is_some_and(|k| k.found() > 0);
    report.present = Some(PresentReport {
        counter: if plan.kind("counter").is_some() {
            found("counter")
        } else {
            found("ops")
        },
        registry: found("registry"),
    });
    if store.stream_lease().is_some() && report.lease.is_none() {
        let now = unix_secs(ctx.now());
        report.lease = Some(LeaseReport {
            found: plan.lease.is_some(),
            expired: plan.lease.as_ref().map(|l| l.expires_at < now),
            ..LeaseReport::default()
        });
    }
    let digest = plan_digest(&identity.identity, &plan);
    report.plan_digest = Some(digest.to_string());
    let counts: Vec<String> = plan
        .kinds
        .iter()
        .map(|k| format!("{}={}", k.kind, k.found()))
        .collect();
    ctx.progress(&format!("plan {} plan_digest={digest}", counts.join(" ")));
    if plan.is_empty() && !identity.populated && opts.expect_identity.is_none() {
        return Err(PurgeError::EmptyStore);
    }
    Ok(plan)
}

fn kind_reports(order: &[&'static str], plan: &Plan) -> Vec<KindReport> {
    order
        .iter()
        .map(|&kind| {
            let planned = plan.kind(kind);
            let found = planned.map_or(0, PlannedKind::found);
            let detail = match kind {
                "ops" => KindDetail::Ops {
                    seq_ranges: seq_ranges(planned.into_iter().flat_map(|k| {
                        k.all_keys().filter_map(|key| match key {
                            PlanKey::Op { seq, .. } | PlanKey::Seq(seq) => Some(*seq),
                            _ => None,
                        })
                    })),
                    max_seq: plan.max_seq(),
                    newest_wall_clock: plan.ops.newest_wall_clock.map(format_utc),
                    wall_clock_unparseable: plan.ops.wall_clock_unparseable,
                    value_bytes: plan.ops.value_bytes,
                },
                "registry" => KindDetail::Registry {
                    by_prefix: planned.map(|k| k.by_prefix.clone()).unwrap_or_default(),
                    expired: planned.map_or(0, |k| k.expired.len() as u64),
                },
                _ => KindDetail::Plain {},
            };
            KindReport {
                kind,
                found,
                deleted: None,
                detail,
            }
        })
        .collect()
}

/// Plan, compare with `--plan`, delete, verify. Called with the lease held on DynamoDB.
fn delete(
    store: &dyn PurgeStore,
    stream: &StreamId,
    expected: &PlanDigest,
    identity: &StoreIdentity,
    opts: &PurgeOptions,
    ctx: &mut RunCtx<'_>,
    report: &mut PurgeReport,
) -> Result<(), PurgeError> {
    let plan = read_plan(store, stream, identity, opts, ctx, report)?;
    if plan_digest(&identity.identity, &plan) != *expected {
        return Err(PurgeError::PlanChanged);
    }
    ctx.progress("the plan matches --plan; deleting");
    for kind in &mut report.kinds {
        kind.deleted = Some(0);
    }
    let mut deleted = Deleted::default();
    let result = store.delete(
        stream,
        &plan,
        expected,
        &identity.identity,
        ctx,
        &mut deleted,
    );
    for kind in &mut report.kinds {
        kind.deleted = Some(deleted.get(kind.kind));
    }
    let physical = result?;
    ctx.reached("V0")?;
    ctx.plan_started = ctx.now();
    let verification = store.plan(stream, ctx)?;
    let residue: Vec<ResidueEntry> = verification
        .kinds
        .iter()
        .filter(|k| k.found() > 0)
        .map(|k| ResidueEntry {
            kind: k.kind,
            count: k.found(),
            by_prefix: (k.kind == "registry").then(|| k.by_prefix.clone()),
        })
        .collect();
    let left: Vec<String> = residue
        .iter()
        .map(|r| format!("{}={}", r.kind, r.count))
        .collect();
    ctx.progress(&format!(
        "deleted; verification found {}",
        if left.is_empty() {
            "nothing".to_string()
        } else {
            left.join(" ")
        }
    ));
    report.complete = residue.is_empty();
    report.residue = Some(residue);
    report.physical = Some(physical);
    Ok(())
}

/// A fresh report for `mode`, before anything was read.
pub fn new_report(mode: PurgeMode, sid: String, now: SystemTime) -> PurgeReport {
    PurgeReport {
        format: REPORT_FORMAT,
        tool_version: env!("CARGO_PKG_VERSION"),
        covers: Vec::new(),
        sid,
        mode: mode.label(),
        store: None,
        plan_digest: None,
        kinds: Vec::new(),
        machines: MachinesReport::default(),
        large: None,
        lease: None,
        present: None,
        physical: None,
        residue: None,
        refused: None,
        complete: false,
        error: None,
        started_at: utc_text(now),
        finished_at: utc_text(now),
    }
}

/// The report's `sid`: the stream id's log-id v1 value, or `"-"` without a key.
pub fn sid_value(key: Option<&LogKey>, stream: &StreamId) -> String {
    match key {
        Some(key) => key.tag(Kind::Sid, stream.as_str()),
        None => "-".to_string(),
    }
}

#[cfg(test)]
mod tests;
