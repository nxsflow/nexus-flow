//! Contract (b): the storage trait. Anyone puts inputs into a run's inbox with `enqueue`; only the
//! owner moves them into the log, through `append`. So the log has one writer, a CLI never races the
//! owner for a sequence number, and inputs are accepted while no owner is alive.
//!
//! The named keys, all strings: `owner/<run>` (the per-run lease, compact serde_json of
//! `{owner_id, epoch, expires_at}`), `claim/<workspace_id>` (the run holding the directory),
//! `flow/<hash>` (written once, never deleted while an open run references it), `pending/<run>`
//! (the commission body, deleted by the append that folds `root_opened` or `finished`), `thread/<id>`,
//! `session/<session_key>`, `persona_session/<run>/<handle>`, `effect/<run>/<key>`,
//! `host-seen/<workspace_id>`, `tree_refusal/<run>`, `poison/<run>/<key>`. Values are opaque bytes in
//! a canonical encoding: compact serde_json of the named struct.
//!
//! The inbox keys: `input/<run>/<ulid>`, `fact/<session_key>/<byte offset>`,
//! `claim_busy/<run>/<changes>`, `timer/<id>/<fire_at>`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::events::{Event, TimerPurpose};
use crate::ids::{Epoch, Key, RunId, Seq, Timestamp};

pub type StoreResult<T> = Result<T, StoreError>;

/// One log per run behind this trait, SQLite in slice 1.
///
/// Rules every backend keeps: CAS keys in one call are distinct; no `append` carries a CAS on an
/// `owner/*` key, because the lease changes only through `compare_and_set`; an `owner/*` key is never
/// deleted and its epoch never restarts (`Protected`); an `effect/<run>/*` value records the epoch that
/// wrote it, and a write under any other epoch than the run's current one is `Fenced`; only
/// `commissioned` creates a run (`UnknownRun` otherwise). The store assigns `seq`, whatever an event
/// carries, and `finished` leaves no timer of its run armed. Errors follow [`StoreError::PRECEDENCE`],
/// so every backend reports the same error for the same state.
pub trait RunStore: Send + Sync {
    /// Puts an INPUT into the run's inbox with no fence and no sequence, atomically with `cas`. A key
    /// already in the inbox OR the log is `Enqueued::Duplicate` and writes nothing.
    fn enqueue(&self, run: &RunId, event: &Event, cas: &[Cas]) -> StoreResult<Enqueued>;
    /// Appends to the log as one transaction: compares `fence` with this run's owner epoch and
    /// `expected_seq` with its last seq, inserts `events`, deletes the `consume` inbox rows, applies the
    /// timer changes the events imply, applies `cas`. All of it, or none.
    fn append(
        &self,
        run: &RunId,
        expected_seq: Seq,
        fence: Epoch,
        events: &[Event],
        consume: &[Key],
        cas: &[Cas],
    ) -> StoreResult<Appended>;
    /// The oldest inbox entry of a run.
    fn next_input(&self, run: &RunId) -> StoreResult<Option<Event>>;
    /// Timers armed with `fire_at <= now`. A backend with external timers keeps this as the reconciler.
    fn due_timers(&self, now: Timestamp) -> StoreResult<Vec<Timer>>;
    /// Compare-and-set on a named value: `from: None` means "must not exist", `to: None` deletes. A
    /// failed comparison is `CasFailed`.
    fn compare_and_set(&self, key: &str, from: Option<&[u8]>, to: Option<&[u8]>)
        -> StoreResult<()>;
    /// A named value.
    fn get(&self, key: &str) -> StoreResult<Option<Vec<u8>>>;
    /// Runs not finished, a run with only an inbox row included.
    fn open_runs(&self) -> StoreResult<Vec<RunRow>>;
    /// The run's events after `from_seq`, in order, gap-free.
    fn load(&self, run: &RunId, from_seq: Seq) -> StoreResult<Vec<Event>>;
    /// The bounds this backend declares.
    fn limits(&self) -> StoreLimits;
}

/// One compare-and-set of an `enqueue` or `append` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cas {
    pub key: Key,
    pub from: Option<Vec<u8>>,
    pub to: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enqueued {
    Accepted,
    Duplicate,
}

/// What an `append` did: the new last seq, and the timer changes it applied, so a host on a backend
/// with external timers can arm or cancel them in the same step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appended {
    pub last_seq: Seq,
    pub timers: Vec<TimerChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerChange {
    pub timer: String,
    pub change: TimerChangeKind,
    pub fire_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerChangeKind {
    Armed,
    Cleared,
    Fired,
}

/// An armed timer that is due.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub run: RunId,
    pub timer: String,
    pub fire_at: Timestamp,
    pub purpose: TimerPurpose,
}

/// A run the store knows. `workspace_id` is opaque and immutable, set by `commissioned`; `max_v` is
/// the highest `v` ever appended to the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub run: RunId,
    pub workspace_id: String,
    pub finished: bool,
    pub last_seq: Seq,
    pub max_v: u32,
    pub commissioned_at: Timestamp,
}

/// The bounds a backend declares. These are constraints of one backend and never engine requirements;
/// the host respects what is declared, and the SQLite store declares none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreLimits {
    pub max_events_per_append: Option<u32>,
    pub max_event_bytes: Option<u32>,
    pub max_events_per_run: Option<u32>,
    pub max_inbox_depth: Option<u32>,
}

/// Why a store call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The fence is not this run's current owner epoch.
    Fenced,
    /// `expected_seq` is not the run's last seq: another writer appended.
    SeqMismatch,
    /// The key is already in the log, at `seq`. The caller treats it as done.
    DuplicateKey { seq: Seq },
    /// A comparison failed, and the whole call rolled back.
    CasFailed { key: Key },
    /// The events' `v` is below the run's `max_v`: a newer version has written to this run.
    VersionTooOld { max_v: u32 },
    /// The store is locked. Retried next tick, never counted against the input.
    Busy,
    /// A delete of an `owner/*` key, a write that would lower its epoch, or one riding in a list.
    Protected { key: Key },
    /// An input other than `commissioned` for a run the store does not know.
    UnknownRun,
    /// An event the contract does not allow, such as a `commissioned` with a non-id ref.
    InvalidEvent { what: String },
    /// Anything else the backend reports.
    Backend { what: String },
    /// A stored row or value that does not decode: a known kind with a bad field among them, which is
    /// refused rather than read as unknown. `seq` is its position when it is in the log. It carries the
    /// row's name and nothing of its content, so an input row can still be named in `consume`.
    DamagedRow { key: Key, seq: Option<Seq> },
}

impl StoreError {
    /// The order `append` checks in, first wins, so every backend reports the same error for the same
    /// state.
    pub const PRECEDENCE: [&'static str; 4] =
        ["fenced", "seq_mismatch", "duplicate_key", "cas_failed"];

    /// The name the golden pins.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Fenced => "fenced",
            Self::SeqMismatch => "seq_mismatch",
            Self::DuplicateKey { .. } => "duplicate_key",
            Self::CasFailed { .. } => "cas_failed",
            Self::VersionTooOld { .. } => "version_too_old",
            Self::Busy => "busy",
            Self::Protected { .. } => "protected",
            Self::UnknownRun => "unknown_run",
            Self::InvalidEvent { .. } => "invalid_event",
            Self::Backend { .. } => "backend",
            Self::DamagedRow { .. } => "damaged_row",
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fenced => f.write_str("fenced: the lease of this run moved on"),
            Self::SeqMismatch => f.write_str("sequence mismatch: another writer appended"),
            Self::DuplicateKey { seq } => write!(f, "key already in the log at seq {seq}"),
            Self::CasFailed { key } => write!(f, "compare-and-set failed on {key}"),
            Self::VersionTooOld { max_v } => {
                write!(f, "a newer version has written to this run (v{max_v})")
            }
            Self::Busy => f.write_str("the store is locked"),
            Self::Protected { key } => write!(f, "{key} is never deleted"),
            Self::UnknownRun => f.write_str("no such run"),
            Self::InvalidEvent { what } => write!(f, "invalid event: {what}"),
            Self::Backend { what } => write!(f, "store backend: {what}"),
            Self::DamagedRow { key, seq: None } => write!(f, "{key} does not decode"),
            Self::DamagedRow {
                key,
                seq: Some(seq),
            } => write!(f, "{key} at seq {seq} does not decode"),
        }
    }
}

impl std::error::Error for StoreError {}
