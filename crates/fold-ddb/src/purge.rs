//! The board-fold purge (6j6v.dbmd): delete ONE stream's partition of the board-fold table, with a
//! dry run that shows what would go, a delete bound to that dry run, a confirm run that proves
//! nothing is left, a JSON report and fixed exit codes — the sibling of the relay's
//! `nxf-relay purge` (6j6v.f6yb), for the derived copy of a board this crate folds.
//!
//! It is the board-fold half of an account deletion. The account sequence — which streams, the
//! fence, the order across the relay and board-fold — is the consuming stack's (foundations' kwv2)
//! and follows consumer contract v2.1 (the crate doc's "Purge" section, and the relay operations
//! runbook `docs/runbooks/relay-purge.md`); this module is the tool it drives, through
//! `nxf-fold-purge` or [`purge_stream`].
//!
//! What a purge must never do (owner rule: correctness of a deletion over its date): (a) touch
//! another stream's partition, or a table that is not a board-fold table; (b) report "deleted" when
//! it deleted nothing, for example against the wrong table; (c) leave rows behind and say it is
//! complete; (d) say complete while something can still re-create the partition. Each rule below
//! exists for one of those.
//!
//! # The three invocations
//!
//! | Invocation | Deletes | Exit 0 means |
//! |---|---|---|
//! | `nxf-fold-purge <id> --dry-run [--expect-identity <hex>]` | no | the plan was printed |
//! | `nxf-fold-purge <id> --dry-run --expect-empty [--expect-identity <hex>]` (confirm) | no | the account is on the deny list and the partition is empty, on a populated table of the expected identity |
//! | `nxf-fold-purge <id> --yes --plan <digest> [--expect-identity <hex>]` | yes | the account is on the deny list, the plan still had that digest, it was deleted, and a fresh read found nothing |
//!
//! # One delete run
//!
//! 1. The id ([`validate_purge_id`], 64), the table gate (`ddb::check_fold_table`, 78), the pin
//!    (`--expect-identity`, 78), the deny table (`ddb::check_deny_table`, 78), and ONE
//!    [`crate::deny::admission`] of the target: in the delete and confirm modes anything but
//!    `Denied` is [`PurgeError::NotFenced`] (78), a read error is [`PurgeError::Table`] (1) — all
//!    before the partition is read. The dry run reports the answer as `fence` and goes on.
//! 2. **The plan:** the partition's keys page by page ([`crate::table::Table::sort_keys_after`]),
//!    every key classified; a key of no kind this build knows, or a watermark of a newer index
//!    revision, is [`PurgeError::UnknownKind`] (78) and nothing is deleted. Its digest must be
//!    `--plan`'s, or 4.
//! 3. **F1:** the PLANNED keys other than the watermark, 25 a request, in plan order — no Query in
//!    between, so a page DynamoDB cut at 1 MB cannot end the delete early. Before each batch every
//!    key's partition is checked to be the target's. `UnprocessedItems` are sent again, 5 calls a
//!    batch in all, then exit 1 with `deleted` counting exactly what went.
//! 4. **F2:** the watermark, last: while a row remains the partition still says "folded, partly
//!    purged", never "never folded".
//! 5. **The verification:** a fresh plan; anything it finds is the `residue`, exit 3.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | dry run printed; confirm found nothing; or the delete ran and the verification found the partition empty |
//! | 1 | a table error stopped the run (or the plan did not converge); `deleted` says what went; a fresh dry run and a bound `--yes` finish |
//! | 3 | delete: the verification found rows (a writer is active); confirm: something was found |
//! | 4 | delete: the plan is not the one `--plan` names; nothing deleted |
//! | 64 | usage: the mode flags, `--keys`, a refused id (`refused.rule`), an unknown argument |
//! | 78 | configuration: a variable missing or malformed, not a board-fold table, not a deny table, `IdentityMismatch`, `EmptyStore`, `UnknownKind`, `NotFenced` |
//!
//! The table lives in one place, [`exit_code`].
//!
//! # Ids in output
//!
//! Log-id v1 (6j6v.01wt): the stream id appears only as its `sid` token (`h1:<kid>:<hex24>`, `-`
//! without `NXS_LOG_HMAC_KEY`), in the report and on every progress line. No sort key, no attribute
//! value and no account hash is printed in any form — counts only; a plan digest hashes every key
//! and carries no stream id; an AWS error is its code only.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::deny::{admission, Admission, DenyList};
use crate::fold::{platform_view_tables, INDEX_REVISION};
use crate::layout::WATERMARK_KEY;
use crate::table::{KeyRow, Table, BATCH_WRITE_KEYS};

pub mod log_id;
mod seams;

#[cfg(feature = "dynamodb")]
pub mod cli;

pub use log_id::{KeyError, Kind, LogKey, KEY_ENV};
pub use seams::{PurgeSeams, SeamAction, SHIPPED_PAGE};

/// The report's format version.
pub const REPORT_FORMAT: u32 = 1;

/// The longest stream id a purge accepts, in bytes — the relay purge's limit (`validate_purge_id`
/// is held equal to `nxs_server::purge::validate_purge_id` by a parity test).
pub const MAX_PURGE_ID_BYTES: usize = 1_000;

/// The sort-key prefixes this build deletes beside the view tables' rows: the board's entries and
/// the watermark. A key outside them and outside every view table is `UnknownKind` (78).
pub const PURGE_COVERS_PREFIXES: &[&str] = &[
    ".adj#",
    ".ladj#",
    ".tadj#",
    ".cadj#",
    ".radj#",
    ".rsrc#",
    WATERMARK_KEY,
];

/// A plan that reads more pages than this has not converged: a writer outpaces the read, which the
/// fence should make impossible (exit 1).
pub const PLAN_MAX_PAGES: u64 = 100_000;

/// Calls per batch, the first included, before `UnprocessedItems` is a failure (exit 1).
pub const BATCH_ATTEMPTS: u32 = 5;

/// Refuse a stream id the purge will not handle (exit 64), with the rule it broke: `empty`,
/// `leading_hash`, `slash`, `control` or `too_long` (over [`MAX_PURGE_ID_BYTES`]). A copy of the
/// relay purge's rule, held equal to it by a parity test (design MF-1), so one id is refused by
/// both tools or by neither.
pub fn validate_purge_id(stream: &str) -> Result<(), PurgeError> {
    let rule = if stream.is_empty() {
        "empty"
    } else if stream.starts_with('#') {
        "leading_hash"
    } else if stream.contains('/') {
        "slash"
    } else if stream.chars().any(char::is_control) {
        "control"
    } else if stream.len() > MAX_PURGE_ID_BYTES {
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
    /// 78: a required variable is missing or malformed (`NXS_LOG_HMAC_KEY` included, C1).
    Config(String),
    /// 78: the table does not exist or is not a board-fold table; the message names the rule.
    NotAFoldTable(String),
    /// 78: `--expect-identity` differs from the table's identity digest.
    IdentityMismatch,
    /// 78: the plan found nothing and the table holds nothing (waived by `--expect-identity`).
    EmptyStore,
    /// 78: a sort-key kind this build does not cover, or a watermark of a newer index revision.
    UnknownKind(Vec<String>),
    /// 78: `NXS_DENY_TABLE` is unset, or names no table, or a table that is not a deny table.
    NotADenyTable(String),
    /// 78: `--yes` or the confirm while the target's account is not on the deny list.
    NotFenced,
    /// 4: the plan is not the one `--plan` names; nothing deleted.
    PlanChanged,
    /// 1: a table error — its kind and AWS error code, never the SDK's message.
    Table(String),
}

impl PurgeError {
    /// The variant's name, as the report's `error.kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            PurgeError::InvalidStreamId { .. } => "InvalidStreamId",
            PurgeError::Usage(_) => "Usage",
            PurgeError::Config(_) => "Config",
            PurgeError::NotAFoldTable(_) => "NotAFoldTable",
            PurgeError::IdentityMismatch => "IdentityMismatch",
            PurgeError::EmptyStore => "EmptyStore",
            PurgeError::UnknownKind(_) => "UnknownKind",
            PurgeError::NotADenyTable(_) => "NotADenyTable",
            PurgeError::NotFenced => "NotFenced",
            PurgeError::PlanChanged => "PlanChanged",
            PurgeError::Table(_) => "Table",
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
            PurgeError::NotAFoldTable(m) => write!(f, "not a board-fold table: {m}"),
            PurgeError::IdentityMismatch => f.write_str(
                "the table's identity digest differs from --expect-identity; this is not the \
                 table the pin names",
            ),
            PurgeError::EmptyStore => f.write_str(
                "the plan found nothing and the table holds nothing; pass --expect-identity to \
                 confirm this is the right table",
            ),
            PurgeError::UnknownKind(names) => write!(
                f,
                "this build does not cover {}; nothing is deleted — run a build whose covers \
                 include it",
                names.join(", ")
            ),
            PurgeError::NotADenyTable(m) => write!(f, "no deny table: {m}"),
            PurgeError::NotFenced => f.write_str(
                "the target's account is not on the deny list (fence is not \"denied\"); nothing \
                 is deleted or confirmed before step 0's fence is in place",
            ),
            PurgeError::PlanChanged => f.write_str(
                "the plan is not the one --plan names; nothing was deleted; take a fresh dry run",
            ),
            PurgeError::Table(m) => write!(f, "a table request failed: {m}"),
        }
    }
}

impl std::error::Error for PurgeError {}

/// A SHA-256 digest, written and read as 64 lowercase hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256Hex(pub [u8; 32]);

impl Sha256Hex {
    fn of(bytes: &[u8]) -> Sha256Hex {
        Sha256Hex(Sha256::digest(bytes).into())
    }

    /// Parse 64 hex characters (either case); anything else is `None`.
    pub fn parse(text: &str) -> Option<Sha256Hex> {
        if text.len() != 64 || !text.is_ascii() {
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

/// JSON with every object's keys sorted, so a digest never depends on insertion order.
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
    serde_json::to_string(&sorted(value)).expect("a JSON value serializes")
}

// ----- the table's identity ---------------------------------------------------------------------

/// The board-fold table a purge runs against, and what pins it: `DescribeTable`'s `TableArn`,
/// `TableId` and `CreationDateTime` (a table re-created under the same name has a new `TableId`
/// and creation time), and whether it holds anything at all (`ItemCount > 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreIdentity {
    /// `dynamodb`, or `memory` for a [`crate::mem::MemTable`] in tests.
    pub backend: &'static str,
    /// The table's name, for the report only: never digested.
    pub fold_table: String,
    /// The region, for the report only: never digested.
    pub region: String,
    pub table_arn: String,
    pub table_id: Option<String>,
    /// `CreationDateTime`, RFC 3339 UTC.
    pub created: Option<String>,
    /// `ItemCount > 0`. DynamoDB refreshes it about every six hours, so it lags both ways; the pin
    /// is the answer to both.
    pub populated: bool,
}

impl StoreIdentity {
    /// The canonical identity document; [`StoreIdentity::digest`] is its SHA-256.
    pub fn identity(&self) -> Value {
        json!({
            "table_arn": self.table_arn,
            "table_id": self.table_id,
            "created": self.created,
        })
    }

    /// What `--expect-identity` pins.
    pub fn digest(&self) -> IdentityDigest {
        Sha256Hex::of(canonical_json(&self.identity()).as_bytes())
    }
}

// ----- the plan ---------------------------------------------------------------------------------

/// The kinds a plan sorts the partition's keys into, in report order.
pub const KINDS: [&str; 3] = ["rows", "entries", "watermark"];

/// Everything a plan read: the keys a delete removes, in plan (sort-key) order, and the counts the
/// report carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Every key found, in sort-key order, with the partition the table answered it under.
    pub keys: Vec<PlannedKey>,
    /// `rows` by view table.
    pub by_table: BTreeMap<String, u64>,
    /// `entries` by prefix.
    pub by_prefix: BTreeMap<String, u64>,
    /// The watermark's `folded_through`, when there is a watermark.
    pub folded_through: Option<i64>,
    /// Kinds this build does not cover, already named for output.
    pub unknown: Vec<String>,
}

/// One key of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedKey {
    pub stream: String,
    pub sk: String,
    pub kind: &'static str,
}

impl Plan {
    /// How many keys of `kind` the plan found.
    pub fn found(&self, kind: &str) -> u64 {
        self.keys.iter().filter(|k| k.kind == kind).count() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// An unknown sort-key kind, named for output: its prefix (up to and including the first `#`) when
/// it matches `^[.a-z_]{1,32}#$`, else `<nonconforming, <n> bytes>` — so a key can never carry user
/// text into a report by way of the error that refuses it.
pub fn name_unknown_prefix(sk: &str) -> String {
    let prefix = sk.find('#').map(|i| &sk[..=i]);
    match prefix {
        Some(p)
            if (2..=33).contains(&p.len())
                && p[..p.len() - 1]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'.') =>
        {
            p.to_string()
        }
        _ => format!("<nonconforming, {} bytes>", sk.len()),
    }
}

/// Sort one key into its kind, or name it as unknown.
fn classify(key: &KeyRow, view_tables: &[&str]) -> Result<(&'static str, String), String> {
    if key.sk == WATERMARK_KEY {
        return match key.index_revision {
            Some(rev) if rev > INDEX_REVISION => Err(format!(
                "a watermark of index revision {rev} (this build: {INDEX_REVISION})"
            )),
            _ => Ok(("watermark", WATERMARK_KEY.to_string())),
        };
    }
    if let Some(prefix) = PURGE_COVERS_PREFIXES
        .iter()
        .find(|p| **p != WATERMARK_KEY && key.sk.starts_with(**p))
    {
        return Ok(("entries", prefix.to_string()));
    }
    if let Some(table) = key
        .sk
        .find('#')
        .map(|i| &key.sk[..i])
        .filter(|t| view_tables.contains(t))
    {
        return Ok(("rows", table.to_string()));
    }
    Err(name_unknown_prefix(&key.sk))
}

/// The plan's digest: SHA-256 over the canonical JSON of `{format, identity, kinds[] (kind, found,
/// by_table / by_prefix), folded_through, the sorted SHA-256 of every sort key}`. No stream id in
/// it (design round 4, Integrity 2): an empty plan's digest is the same for every stream of the
/// table.
pub fn plan_digest(identity: &StoreIdentity, plan: &Plan) -> PlanDigest {
    let mut keys: Vec<String> = plan
        .keys
        .iter()
        .map(|k| Sha256Hex::of(k.sk.as_bytes()).to_string())
        .collect();
    keys.sort();
    let doc = json!({
        "format": REPORT_FORMAT,
        "identity": identity.identity(),
        "kinds": [
            {"kind": "rows", "found": plan.found("rows"), "by_table": plan.by_table},
            {"kind": "entries", "found": plan.found("entries"), "by_prefix": plan.by_prefix},
            {"kind": "watermark", "found": plan.found("watermark")},
        ],
        "folded_through": plan.folded_through,
        "keys": keys,
    });
    Sha256Hex::of(canonical_json(&doc).as_bytes())
}

// ----- modes, options, outcome ------------------------------------------------------------------

/// Which of the three invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeMode {
    /// `--dry-run`, or with `expect_empty` the confirm run `--dry-run --expect-empty`.
    DryRun { expect_empty: bool },
    /// `--yes --plan <digest>`.
    Delete { plan: PlanDigest },
}

impl PurgeMode {
    pub fn label(self) -> &'static str {
        match self {
            PurgeMode::DryRun {
                expect_empty: false,
            } => "dry_run",
            PurgeMode::DryRun { expect_empty: true } => "confirm",
            PurgeMode::Delete { .. } => "delete",
        }
    }

    /// Whether the mode needs the fence: the delete and the confirm.
    fn fenced(self) -> bool {
        !matches!(
            self,
            PurgeMode::DryRun {
                expect_empty: false
            }
        )
    }
}

/// What every mode may carry.
#[derive(Debug, Clone, Default)]
pub struct PurgeOptions {
    /// `--expect-identity`: refuse another table (78) and waive `EmptyStore`.
    pub expect_identity: Option<IdentityDigest>,
    /// `NXS_LOG_HMAC_KEY` (log-id v1 C1); `None` writes `sid` as `"-"`.
    pub log_key: Option<LogKey>,
}

/// A point of a delete run, where a test can stop or inject (design §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgePoint {
    /// After the k-th F1 batch.
    RowsBatch(u64),
    /// After F1.
    Rows,
    /// After F2.
    Watermark,
    /// Before the verification read.
    V0,
}

impl PurgePoint {
    pub fn label(self) -> String {
        match self {
            PurgePoint::RowsBatch(k) => format!("RowsBatch:{k}"),
            PurgePoint::Rows => "Rows".into(),
            PurgePoint::Watermark => "Watermark".into(),
            PurgePoint::V0 => "V0".into(),
        }
    }
}

/// A purge's result: the report as far as the run got, and the error that stopped it, if any. The
/// report travels with every error, so `deleted` says what went before an exit 1.
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
    pub covers: Covers,
    /// The stream id as its log-id v1 value, `h1:<kid>:<hex24>`, or `"-"` without a key.
    pub sid: String,
    pub mode: &'static str,
    pub store: Option<StoreReport>,
    /// The deny table the fence was read from (consumer contract v2.1: the runner compares its
    /// `table_arn` with the one the authorizer reports, and stops on a mismatch).
    pub deny: Option<DenyReport>,
    /// The one admission of the target at the start of the run: `denied`, `not_denied` or
    /// `unscoped`. Outside the plan digest: the digest binds the data, the fence is checked on
    /// every run.
    pub fence: Option<&'static str>,
    pub plan_digest: Option<String>,
    pub folded_through: Option<i64>,
    pub kinds: Vec<KindReport>,
    /// `"provider"` after a delete; `null` otherwise.
    pub physical: Option<&'static str>,
    /// What the verification read found after a delete, per kind; `null` when it did not run.
    pub residue: Option<Vec<ResidueEntry>>,
    pub refused: Option<RefusedReport>,
    pub complete: bool,
    pub error: Option<ErrorReport>,
    pub started_at: String,
    pub finished_at: String,
}

/// What this build deletes: the report's `covers`.
#[derive(Debug, Clone, Serialize)]
pub struct Covers {
    pub view_tables: Vec<&'static str>,
    pub prefixes: Vec<&'static str>,
    pub index_revision: i64,
}

impl Covers {
    pub fn this_build() -> Covers {
        let mut view_tables = platform_view_tables();
        view_tables.sort_unstable();
        Covers {
            view_tables,
            prefixes: PURGE_COVERS_PREFIXES.to_vec(),
            index_revision: INDEX_REVISION,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreReport {
    pub backend: &'static str,
    pub fold_table: String,
    pub region: String,
    pub identity: Value,
    pub identity_digest: String,
    pub populated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DenyReport {
    pub table: String,
    pub table_arn: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindReport {
    pub kind: &'static str,
    pub found: u64,
    /// `null` outside a delete.
    pub deleted: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_table: Option<BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_prefix: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResidueEntry {
    pub kind: &'static str,
    pub count: u64,
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

/// The exit code of an outcome — design §5's table, the one place it lives.
pub fn exit_code(outcome: &PurgeOutcome) -> u8 {
    if let Some(error) = &outcome.error {
        return match error {
            PurgeError::InvalidStreamId { .. } | PurgeError::Usage(_) => 64,
            PurgeError::PlanChanged => 4,
            PurgeError::Table(_) => 1,
            PurgeError::Config(_)
            | PurgeError::NotAFoldTable(_)
            | PurgeError::IdentityMismatch
            | PurgeError::EmptyStore
            | PurgeError::UnknownKind(_)
            | PurgeError::NotADenyTable(_)
            | PurgeError::NotFenced => 78,
        };
    }
    let report = &outcome.report;
    match report.mode {
        "confirm" if report.kinds.iter().any(|k| k.found > 0) => 3,
        "delete" if report.residue.as_ref().is_some_and(|r| !r.is_empty()) => 3,
        _ => 0,
    }
}

// ----- the run ----------------------------------------------------------------------------------

/// The clock a run reads.
pub type Clock<'a> = &'a (dyn Fn() -> SystemTime + Sync);

/// `YYYY-MM-DDTHH:MM:SSZ` of `t`, whole seconds.
pub fn utc_text(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days, as the relay purge's own formatter.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The report's `sid`: the stream id's log-id v1 value, or `"-"` without a key.
pub fn sid_value(key: Option<&LogKey>, stream: &str) -> String {
    match key {
        Some(key) => key.tag(Kind::Sid, stream),
        None => "-".to_string(),
    }
}

/// A fresh report for `mode`, before anything was read.
pub fn new_report(mode: PurgeMode, sid: String, now: SystemTime) -> PurgeReport {
    PurgeReport {
        format: REPORT_FORMAT,
        tool_version: env!("CARGO_PKG_VERSION"),
        covers: Covers::this_build(),
        sid,
        mode: mode.label(),
        store: None,
        deny: None,
        fence: None,
        plan_digest: None,
        folded_through: None,
        kinds: Vec::new(),
        physical: None,
        residue: None,
        refused: None,
        complete: false,
        error: None,
        started_at: utc_text(now),
        finished_at: utc_text(now),
    }
}

/// The report's `store` for an identity.
pub fn store_report(identity: &StoreIdentity) -> StoreReport {
    StoreReport {
        backend: identity.backend,
        fold_table: identity.fold_table.clone(),
        region: identity.region.clone(),
        identity: identity.identity(),
        identity_digest: identity.digest().to_string(),
        populated: identity.populated,
    }
}

/// An outcome for a run that stopped before the purge itself started (the command line, the
/// environment, the gates): the report as far as it is known, and the error.
pub fn refusal(
    mode: PurgeMode,
    sid: &str,
    error: PurgeError,
    started: SystemTime,
    identity: Option<&StoreIdentity>,
) -> PurgeOutcome {
    let mut report = new_report(mode, sid.to_string(), started);
    report.store = identity.map(store_report);
    if let PurgeError::InvalidStreamId { rule } = &error {
        report.refused = Some(RefusedReport { rule });
    }
    report.error = Some(ErrorReport {
        kind: error.kind(),
        message: error.to_string(),
    });
    PurgeOutcome {
        report,
        error: Some(error),
    }
}

/// What a run is handed besides the table: the seams, the progress line, and (test builds) the
/// table an injection folds into.
struct RunCtx<'a, T: Table> {
    seams: &'a PurgeSeams,
    sid: String,
    progress: &'a mut (dyn FnMut(&str) + Send),
    #[cfg_attr(not(feature = "test-seams"), allow(dead_code))]
    inject_into: &'a T,
    #[cfg_attr(not(feature = "test-seams"), allow(dead_code))]
    injected: u64,
}

impl<T: Table> RunCtx<'_, T> {
    /// One progress line on stderr; the stream is always named by its `sid` token.
    fn progress(&mut self, message: &str) {
        let line = format!("nxf-fold-purge: sid={} {message}", self.sid);
        (self.progress)(&line);
    }

    /// The seam at `point`: inject, then stop, under `test-seams`; nothing otherwise.
    async fn reached(&mut self, point: PurgePoint) -> Result<(), PurgeError> {
        let label = point.label();
        for action in self.seams.actions_at(&label) {
            match action {
                SeamAction::Inject => self.inject(&label).await?,
                SeamAction::Stop => {
                    self.progress(&format!("stop at {label} (test seam)"));
                    std::process::abort()
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "test-seams")]
    async fn inject(&mut self, label: &str) -> Result<(), PurgeError> {
        self.injected += 1;
        let n = self.injected;
        self.progress(&format!("inject at {label} (test seam)"));
        let folder = crate::fold::Folder::platform()
            .map_err(|e| PurgeError::Table(format!("test seam: {e}")))?;
        let ops: Vec<(i64, nxs_foundation::model::Op)> = injected_ops(n)
            .into_iter()
            .map(|op| (INJECTED_SEQ + n as i64, op))
            .collect();
        folder
            .fold_batch(self.inject_into, &ops)
            .await
            .map_err(|e| PurgeError::Table(format!("test seam: {e}")))?;
        Ok(())
    }

    #[cfg(not(feature = "test-seams"))]
    async fn inject(&mut self, _label: &str) -> Result<(), PurgeError> {
        Ok(())
    }
}

/// The relay position an injected op is folded at: far past any fixture's, so the injection moves
/// an existing watermark forward and re-creates a deleted one.
#[cfg(feature = "test-seams")]
pub const INJECTED_SEQ: i64 = 1_000_000_000;

/// The ops the `n`-th injection of a run folds (test seam): the creation of item `inj0.<n>`, which
/// no fixture holds — so every row it folds to has a key no plan holds. The oracle of the injection
/// tests folds the same ops into an empty partition.
#[cfg(feature = "test-seams")]
pub fn injected_ops(n: u64) -> Vec<nxs_foundation::model::Op> {
    let mut store = nexus_flow_core::store::Store::open_in_memory(9_000 + n as i64);
    store.set_wall_clock("2026-10-23T00:00:00Z");
    store.create_item(
        &format!("inj0.{n:04}"),
        "task",
        "injected by the purge test seam",
        "purge-test-seam",
    );
    store.export()
}

/// Purge one stream's partition. `table` is bound to the stream; `identity` is what
/// `ddb::check_fold_table` returned for its table; `deny` is the deny list, already checked
/// (`ddb::DynamoDbDenyList::from_env`). See the module doc for the modes and [`exit_code`] for what
/// the outcome means. Progress goes to stderr.
pub async fn purge_stream<T: Table, D: DenyList>(
    table: &T,
    identity: &StoreIdentity,
    deny: &D,
    mode: PurgeMode,
    opts: &PurgeOptions,
    seams: &PurgeSeams,
) -> PurgeOutcome {
    let clock = SystemTime::now;
    let mut progress = |line: &str| eprintln!("{line}");
    purge_stream_with(
        table,
        table,
        identity,
        deny,
        mode,
        opts,
        seams,
        &clock,
        &mut progress,
    )
    .await
}

/// [`purge_stream`] with the table an injection folds into (test builds; the same table
/// elsewhere), an explicit clock and a progress sink.
#[allow(clippy::too_many_arguments)]
pub async fn purge_stream_with<T: Table, D: DenyList>(
    table: &T,
    inject_into: &T,
    identity: &StoreIdentity,
    deny: &D,
    mode: PurgeMode,
    opts: &PurgeOptions,
    seams: &PurgeSeams,
    clock: Clock<'_>,
    progress: &mut (dyn FnMut(&str) + Send),
) -> PurgeOutcome {
    let started = clock();
    let sid = sid_value(opts.log_key.as_ref(), table.stream());
    let mut report = new_report(mode, sid.clone(), started);
    let mut ctx = RunCtx {
        seams,
        sid,
        progress,
        inject_into,
        injected: 0,
    };
    if opts.log_key.is_none() {
        ctx.progress(
            "log ids are off: NXS_LOG_HMAC_KEY is not set, so the stream is written sid=-",
        );
    }
    ctx.progress(&format!("{} started", report.mode));
    let error = run(table, identity, deny, mode, opts, &mut ctx, &mut report)
        .await
        .err();
    report.finished_at = utc_text(clock());
    if let Some(e) = &error {
        if let PurgeError::InvalidStreamId { rule } = e {
            report.refused = Some(RefusedReport { rule });
        }
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

/// A table error, as the report may carry it: the table's own text, which for `ddb::DynamoDbTable`
/// is the operation and the AWS error code only.
fn table_err<E: fmt::Display>(e: E) -> PurgeError {
    PurgeError::Table(e.to_string())
}

async fn run<T: Table, D: DenyList>(
    table: &T,
    identity: &StoreIdentity,
    deny: &D,
    mode: PurgeMode,
    opts: &PurgeOptions,
    ctx: &mut RunCtx<'_, T>,
    report: &mut PurgeReport,
) -> Result<(), PurgeError> {
    let stream = table.stream().to_string();
    validate_purge_id(&stream)?;
    let digest = identity.digest();
    report.store = Some(store_report(identity));
    ctx.progress(&format!(
        "fold table identity_digest={digest} populated={}",
        identity.populated
    ));
    if let Some(expected) = opts.expect_identity {
        if expected != digest {
            return Err(PurgeError::IdentityMismatch);
        }
    }
    report.deny = deny.table().map(|t| DenyReport {
        table: t.name.clone(),
        table_arn: t.table_arn.clone(),
    });
    let fence = admission(deny, &stream).await.map_err(table_err)?;
    report.fence = Some(fence.fence());
    ctx.progress(&format!("fence={}", fence.fence()));
    if mode.fenced() && fence != Admission::Denied {
        return Err(PurgeError::NotFenced);
    }

    let plan = read_plan(table, identity, opts, ctx, report).await?;
    let PurgeMode::Delete { plan: expected } = mode else {
        return Ok(());
    };
    if plan_digest(identity, &plan) != expected {
        return Err(PurgeError::PlanChanged);
    }
    ctx.progress("the plan matches --plan; deleting");
    let mut deleted: BTreeMap<&'static str, u64> = KINDS.iter().map(|k| (*k, 0)).collect();
    let result = delete(table, &stream, &plan, ctx, &mut deleted).await;
    for kind in &mut report.kinds {
        kind.deleted = Some(deleted.get(kind.kind).copied().unwrap_or(0));
    }
    result?;

    ctx.reached(PurgePoint::V0).await?;
    let verification = plan_partition(table, ctx).await?;
    let mut residue: Vec<ResidueEntry> = KINDS
        .iter()
        .map(|kind| ResidueEntry {
            kind,
            count: verification.found(kind),
        })
        .filter(|r| r.count > 0)
        .collect();
    // A kind this build does not know that appeared during the run (a newer writer): it is still
    // there, so the run is not complete — residue, never a silent success.
    if !verification.unknown.is_empty() {
        residue.push(ResidueEntry {
            kind: "unknown",
            count: verification.unknown.len() as u64,
        });
    }
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
    report.physical = Some("provider");
    Ok(())
}

/// Read the partition's keys to the end and sort them into kinds. Unknown kinds are collected, not
/// refused: the caller decides (the plan refuses, the verification reports them as residue).
async fn plan_partition<T: Table>(table: &T, ctx: &mut RunCtx<'_, T>) -> Result<Plan, PurgeError> {
    let view_tables = platform_view_tables();
    let mut plan = Plan::default();
    let mut after: Option<String> = None;
    let mut pages = 0u64;
    loop {
        pages += 1;
        if pages > PLAN_MAX_PAGES {
            return Err(PurgeError::Table(
                "plan did not converge: more pages than the plan may read".into(),
            ));
        }
        let (keys, cursor) = table
            .sort_keys_after(after.as_deref(), ctx.seams.page())
            .await
            .map_err(table_err)?;
        for key in keys {
            match classify(&key, &view_tables) {
                Ok((kind, part)) => {
                    match kind {
                        "rows" => *plan.by_table.entry(part).or_insert(0) += 1,
                        "entries" => *plan.by_prefix.entry(part).or_insert(0) += 1,
                        _ => plan.folded_through = key.folded_through,
                    }
                    plan.keys.push(PlannedKey {
                        stream: key.stream,
                        sk: key.sk,
                        kind,
                    });
                }
                Err(name) => {
                    if !plan.unknown.contains(&name) {
                        plan.unknown.push(name);
                    }
                }
            }
        }
        match cursor {
            Some(c) => after = Some(c),
            None => return Ok(plan),
        }
    }
}

/// The plan, into the report: refuses an unknown kind and an empty table (without a pin).
async fn read_plan<T: Table>(
    table: &T,
    identity: &StoreIdentity,
    opts: &PurgeOptions,
    ctx: &mut RunCtx<'_, T>,
    report: &mut PurgeReport,
) -> Result<Plan, PurgeError> {
    let plan = plan_partition(table, ctx).await?;
    if !plan.unknown.is_empty() {
        return Err(PurgeError::UnknownKind(plan.unknown.clone()));
    }
    report.kinds = vec![
        KindReport {
            kind: "rows",
            found: plan.found("rows"),
            deleted: None,
            by_table: Some(plan.by_table.clone()),
            by_prefix: None,
        },
        KindReport {
            kind: "entries",
            found: plan.found("entries"),
            deleted: None,
            by_table: None,
            by_prefix: Some(plan.by_prefix.clone()),
        },
        KindReport {
            kind: "watermark",
            found: plan.found("watermark"),
            deleted: None,
            by_table: None,
            by_prefix: None,
        },
    ];
    report.folded_through = plan.folded_through;
    let digest = plan_digest(identity, &plan);
    report.plan_digest = Some(digest.to_string());
    ctx.progress(&format!(
        "plan rows={} entries={} watermark={} plan_digest={digest}",
        plan.found("rows"),
        plan.found("entries"),
        plan.found("watermark")
    ));
    if plan.is_empty() && !identity.populated && opts.expect_identity.is_none() {
        return Err(PurgeError::EmptyStore);
    }
    Ok(plan)
}

/// F1 and F2: delete exactly the plan's keys, the watermark last.
async fn delete<T: Table>(
    table: &T,
    stream: &str,
    plan: &Plan,
    ctx: &mut RunCtx<'_, T>,
    deleted: &mut BTreeMap<&'static str, u64>,
) -> Result<(), PurgeError> {
    let (watermark, rows): (Vec<&PlannedKey>, Vec<&PlannedKey>) =
        plan.keys.iter().partition(|k| k.kind == "watermark");
    for (i, batch) in rows.chunks(BATCH_WRITE_KEYS).enumerate() {
        delete_keys(table, stream, batch, deleted).await?;
        ctx.reached(PurgePoint::RowsBatch(i as u64 + 1)).await?;
    }
    ctx.reached(PurgePoint::Rows).await?;
    if !watermark.is_empty() {
        delete_keys(table, stream, &watermark, deleted).await?;
    }
    ctx.reached(PurgePoint::Watermark).await
}

/// One batch: the target check, then up to [`BATCH_ATTEMPTS`] calls while DynamoDB leaves keys
/// unprocessed, counting what went as it goes.
async fn delete_keys<T: Table>(
    table: &T,
    stream: &str,
    batch: &[&PlannedKey],
    deleted: &mut BTreeMap<&'static str, u64>,
) -> Result<(), PurgeError> {
    // Defence in depth: a key of another partition in a batch is a bug; stop before sending.
    if batch.iter().any(|k| k.stream != stream) {
        return Err(PurgeError::Table(
            "a key outside the target's partition was planned; nothing more is deleted".into(),
        ));
    }
    let kind_of: BTreeMap<&str, &'static str> =
        batch.iter().map(|k| (k.sk.as_str(), k.kind)).collect();
    let mut pending: Vec<String> = batch.iter().map(|k| k.sk.clone()).collect();
    let mut attempt = 0;
    while !pending.is_empty() {
        if attempt == BATCH_ATTEMPTS {
            return Err(PurgeError::Table(format!(
                "UnprocessedItems after {BATCH_ATTEMPTS} calls of one batch"
            )));
        }
        if attempt > 0 {
            table.backoff(attempt).await;
        }
        let left = table.delete_batch(&pending).await.map_err(table_err)?;
        for sk in pending.iter().filter(|sk| !left.contains(sk)) {
            let kind = kind_of.get(sk.as_str()).copied().unwrap_or("rows");
            *deleted.entry(kind).or_insert(0) += 1;
        }
        pending = left;
        attempt += 1;
    }
    Ok(())
}
