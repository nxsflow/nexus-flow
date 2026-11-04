//! Contract (a): run events. The log is the only durable truth of a run. An event is an INPUT
//! (something happened outside the reducer) or a DECISION (the reducer's own conclusion). Both share
//! one envelope, and a replay folds all of them in order.
//!
//! The log carries ids, handles and codes, never bodies and never paths (PV1, M6).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::codes::{GitReason, ReasonCode};
use crate::ids::{Handle, Key, MessageId, RunId, Seq, SessionKey, Slot, StepId, Timestamp};
use crate::policy::LivenessPolicy;
use crate::session::SessionFact;
use crate::CONTRACT_V;

/// Who appended an event, set by the host from where the input really came and never taken from a
/// payload (M6). It carries no person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Cli,
    Host,
    Adapter,
    Reducer,
    Timer,
}

/// One event of a run's log.
///
/// `extra` holds whatever a newer writer put on the event that this version does not know, at any
/// depth of the envelope or body, so it survives a round trip. It is written back merged, and the
/// output's keys are sorted, not in insertion order.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub run: RunId,
    pub seq: Seq,
    pub at: Timestamp,
    pub v: u32,
    pub key: Key,
    pub origin: Origin,
    pub body: EventBody,
    pub extra: Map<String, Value>,
}

impl Event {
    /// An event of this version with nothing unknown on it.
    pub fn new(
        run: RunId,
        seq: Seq,
        at: Timestamp,
        key: impl Into<Key>,
        origin: Origin,
        body: EventBody,
    ) -> Self {
        Self {
            run,
            seq,
            at,
            v: CONTRACT_V,
            key: key.into(),
            origin,
            body,
            extra: Map::new(),
        }
    }

    /// A kind this version does not know, appended by the reducer of a newer one. Folding past it would
    /// diverge silently, so a reader refuses ownership of the run and projects status read-only (§13).
    pub fn is_unknown_decision(&self) -> bool {
        matches!(self.body, EventBody::Unknown(_)) && self.origin == Origin::Reducer
    }

    /// A kind this version does not know from anywhere else: a claim about the world, which folds to
    /// nothing (§6.2).
    pub fn is_unknown_input(&self) -> bool {
        matches!(self.body, EventBody::Unknown(_)) && self.origin != Origin::Reducer
    }
}

/// The envelope's known fields, in the shape the derive can read and write.
#[derive(Serialize, Deserialize)]
struct KnownEvent {
    run: RunId,
    seq: Seq,
    at: Timestamp,
    v: u32,
    key: Key,
    origin: Origin,
    body: EventBody,
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let known = KnownEvent {
            run: self.run.clone(),
            seq: self.seq,
            at: self.at,
            v: self.v,
            key: self.key.clone(),
            origin: self.origin,
            body: self.body.clone(),
        };
        let Value::Object(mut wire) =
            serde_json::to_value(known).map_err(serde::ser::Error::custom)?
        else {
            return Err(serde::ser::Error::custom("an event is an object"));
        };
        merge_unknown(&mut wire, &self.extra);
        wire.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let Value::Object(raw) = Value::deserialize(d)? else {
            return Err(serde::de::Error::custom("an event is an object"));
        };
        let known: KnownEvent =
            serde_json::from_value(Value::Object(raw.clone())).map_err(serde::de::Error::custom)?;
        let Value::Object(written) =
            serde_json::to_value(&known).map_err(serde::de::Error::custom)?
        else {
            return Err(serde::de::Error::custom("an event is an object"));
        };
        Ok(Self {
            run: known.run,
            seq: known.seq,
            at: known.at,
            v: known.v,
            key: known.key,
            origin: known.origin,
            body: known.body,
            extra: unknown_fields(&raw, &written),
        })
    }
}

/// What `raw` holds that `written` does not, keeping the objects on the way down.
fn unknown_fields(raw: &Map<String, Value>, written: &Map<String, Value>) -> Map<String, Value> {
    let mut unknown = Map::new();
    for (key, value) in raw {
        match (value, written.get(key)) {
            (_, None) => {
                unknown.insert(key.clone(), value.clone());
            }
            (Value::Object(raw_inner), Some(Value::Object(written_inner))) => {
                let inner = unknown_fields(raw_inner, written_inner);
                if !inner.is_empty() {
                    unknown.insert(key.clone(), Value::Object(inner));
                }
            }
            _ => {}
        }
    }
    unknown
}

fn merge_unknown(wire: &mut Map<String, Value>, unknown: &Map<String, Value>) {
    for (key, value) in unknown {
        match (wire.get_mut(key), value) {
            (Some(Value::Object(inner)), Value::Object(unknown_inner)) => {
                merge_unknown(inner, unknown_inner);
            }
            _ => {
                wire.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Declares a `kind`-tagged enum and its `KINDS` list from one source, so the list cannot leave a
/// variant out. `$($tail)*` are the variants that carry no kind of their own.
macro_rules! kinded_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $kind:literal $({ $($field:tt)* })?,)+
        }
        $($tail:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "kind")]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $kind)] $variant $({ $($field)* })?,)+
            $($tail)*
        }

        impl $name {
            /// Every kind of this enum a writer of this version may use, in declaration order.
            pub const KINDS: &'static [&'static str] = &[$($kind),+];
        }
    };
}

kinded_enum! {
    /// What an event says. The inputs are this enum's own variants; every decision is one
    /// [`Decision`], on the wire flat beside them, under its own `kind`.
    EventBody {
        /// The first event of every run. Only this one creates a run. It carries no return address, no
        /// body and no refs: those wait in `pending/<run>`.
        Commissioned = "commissioned" {
            run: RunId,
            workspace_id: String,
            flow: FlowRef,
            policy: LivenessPolicy,
            root_thread: MessageId,
            requester: Handle,
        },
        /// The root thread exists and holds the root message, so the first step's turn can name it.
        RootOpened = "root_opened" { thread: MessageId, message: MessageId },
        /// The host's CAS of `claim/<workspace_id>` succeeded.
        ClaimTaken = "claim_taken",
        /// The claim is held by another v2 run. Keyed `claim_busy/<run>/<changes>`, so a repeat is a
        /// duplicate and an unchanged holder writes nothing.
        ClaimBusy = "claim_busy" { holder: RunId },
        ClaimReleased = "claim_released",
        /// `found` is what prepare rescued from a tree it found dirty.
        TreePrepared = "tree_prepared" {
            base: Base,
            #[serde(default)]
            found: Option<ParkOutcome>,
        },
        /// A closed reason only, never git's text.
        TreeRefused = "tree_refused" { reason: TreeRefusal },
        TreeParked = "tree_parked" { outcome: ParkOutcome },
        /// A message owed to the commissioner was posted.
        Delivered = "delivered" { message: MessageId },
        /// The nudge the slot owed was sent.
        Nudged = "nudged" { session_key: SessionKey },
        /// The `nth` stop signal was sent, so a restart never owes one the log records.
        StopSignalled = "stop_signalled" { session_key: SessionKey, nth: u32, signal: StopKind },
        /// A fact the session runtime reported; `started` and `ended` among them.
        SessionFact = "session_fact" { session_key: SessionKey, fact: SessionFact },
        /// A step's party answered in its thread. The body is the message.
        Reply = "reply" {
            session_key: SessionKey,
            message: MessageId,
            escalate: bool,
            needs_rework: bool,
        },
        /// The commissioner's answer. `question` is the key of the `escalated` or `ceiling` event that
        /// asked. `accept` is valid at a ceiling only.
        Answer = "answer" { question: Key, disposition: Disposition, message: MessageId },
        /// A root reply while the run is `Held` or `Parked`: a resumable condition has no question id.
        Resume = "resume" { slot: Option<Slot> },
        /// The human exit, valid in every non-final state. `message` is absent while no root thread exists.
        Withdrawn = "withdrawn" { message: Option<MessageId>, by: WithdrawnBy },
        /// A due timer, keyed `timer/<id>/<fire_at>`, so a re-armed timer's firing is a distinct input.
        TimerFired = "timer_fired" { timer: String, fire_at: Timestamp },
    }
    /// A decision of the reducer.
    #[serde(untagged)]
    Decision(Decision),
    /// A kind this version does not know. See [`Event::is_unknown_decision`].
    #[serde(untagged)]
    Unknown(UnknownBody),
}

kinded_enum! {
    /// The reducer's conclusions, appended with the input that caused them. Every kind is part of the
    /// contract's version: adding one raises [`CONTRACT_V`].
    Decision {
        /// A run in `Created` owes `OpenRoot`.
        RootRequested = "root_requested",
        /// `Queued { behind: None }`: a run owes `TakeClaim`.
        ClaimRequested = "claim_requested",
        /// The working copy is held by another v2 run, and this run waits for it visibly.
        RunQueued = "run_queued" { behind: Option<RunId> },
        /// `visit` rises per step per run; `attempt` per slot. One session per member is now owed.
        StepStarted = "step_started" {
            step: StepId,
            visit: u32,
            attempt: u32,
            members: Vec<Handle>,
        },
        /// The step's verdict, counted from each slot's latest reply, never with fewer than its members.
        StepFinished = "step_finished" { step: StepId, visit: u32, outcome: StepOutcome },
        /// A back edge. `pass` counts per `declaring` step, which holds the ceiling.
        Rework = "rework" { from: StepId, to: StepId, declaring: StepId, pass: u32 },
        /// Emitted when the escalating reply folds, once per attempt. `question` is this event's own key.
        Escalated = "escalated" { question: Key, slot: Slot, message: MessageId },
        /// The pass a rework would start is over the declaring step's `max_passes`: the run asks.
        Ceiling = "ceiling" { question: Key, step: StepId, pass: u32, last_verdict: MessageId },
        /// Arms a timer, or re-arms an armed id.
        TimerSet = "timer_set" { timer: String, fire_at: Timestamp, purpose: TimerPurpose },
        /// Only ever a cancellation.
        TimerCleared = "timer_cleared" { timer: String },
        /// The ladder's conclusion about a session: `quiet` or `hung`.
        Liveness = "liveness" { slot: Slot, reason: ReasonCode, since: Timestamp },
        /// On hold, not failed.
        Held = "held" { slot: Slot, reason: ReasonCode, until: Timestamp },
        /// Needs a person. `slot` is absent for a run-level park (`root_failed`, `tree_refused`).
        Parked = "parked" { slot: Option<Slot>, reason: ReasonCode },
        /// Every outcome passes through here: stop, park, deliver, release.
        Finishing = "finishing" { outcome: Outcome, reason: Option<ReasonCode> },
        /// A withdrawal met a run already `Finishing`: the outcome stays, and open stops are forced.
        WithdrawForced = "withdraw_forced",
        /// The single final event.
        Finished = "finished" { outcome: Outcome, reason: Option<ReasonCode> },
        /// An input that changed nothing, with its reason.
        Ignored = "ignored" { input_seq: Seq, reason: ReasonCode },
    }
}

/// A body whose `kind` this version does not know, kept whole so it survives a round trip.
#[derive(Clone, Debug, PartialEq)]
pub struct UnknownBody {
    pub kind: String,
    pub raw: Value,
}

impl Serialize for UnknownBody {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(s)
    }
}

impl<'de> Deserialize<'de> for UnknownBody {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(d)?;
        let kind = raw
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| serde::de::Error::custom("an event body carries a kind"))?
            .to_owned();
        Ok(Self { kind, raw })
    }
}

/// The pinned flow definition: `hash` is the SHA-256 of its canonical JSON, and the definition itself
/// is the value of `flow/<hash>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowRef {
    pub hash: String,
}

impl FlowRef {
    /// The reference to a definition with these canonical bytes.
    pub fn of_canonical_json(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Self {
            hash: format!("sha256:{hex}"),
        }
    }
}

/// The remote's default branch and the commit `refs/remotes/origin/<branch>` holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub branch: String,
    pub commit: String,
}

/// What a park left behind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParkOutcome {
    /// Committed on a fresh `nxs/park/run-<run>/<instant>` branch.
    Committed {
        branch: String,
        commit: String,
    },
    /// HEAD was already on the run's own branch.
    OnOwnBranch {
        branch: String,
        commit: String,
    },
    Clean,
    /// The directory is gone; nothing is left to save, which releases the claim.
    Missing,
    /// The claim key names another run, so no git ran.
    NotHeld,
}

/// Why a tree operation was refused: closed, and without text. The first two are permanent, the others
/// transient. `tree_refused` in [`ReasonCode`] is the park this leads to, never this detail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum TreeRefusal {
    NotARepository,
    ForeignDirectory,
    MidSequence { what: SequenceKind },
    Git { reason: GitReason },
}

impl TreeRefusal {
    /// The codes of this type. The golden pins them beside `ReasonCode::ALL` and `GitReason::ALL`.
    pub const CODES: &'static [&'static str] = &[
        "not_a_repository",
        "foreign_directory",
        "mid_sequence",
        "git",
    ];
}

/// The sequence a tree is in the middle of. A code, because the log carries no text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

/// `continue` is exactly one more pass; `accept` takes the verdict as it stands, at a ceiling only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Continue,
    Accept,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawnBy {
    Requester,
    Owner,
}

/// SIGTERM or SIGKILL. A kill is a person's act, and only a withdrawal's `forced` reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    Term,
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Pass,
    NeedsRework,
    Escalated,
}

/// How a run ended. There is no `rework_limit`: the ceiling asks, it never stops quietly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Delivered,
    Withdrawn,
    Accepted,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerPurpose {
    RootDeadline,
    StartDeadline,
    Quiet,
    Confirm,
    CallBound,
    SubprocessIdle,
    StopGrace,
    WindDown,
    HoldRetry,
    TreeWindow,
}
