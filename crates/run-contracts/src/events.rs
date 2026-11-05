//! Contract (a): run events. The log is the only durable truth of a run. An event is an INPUT
//! (something happened outside the reducer) or a DECISION (the reducer's own conclusion). Both share
//! one envelope, and a replay folds all of them in order.
//!
//! The log carries ids, handles and codes, never bodies and never paths (PV1, M6).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::codes::{closed_code, tolerant_tagged, GitReason, ReasonCode};
use crate::ids::{Handle, Key, MessageId, RunId, Seq, SessionKey, Slot, StepId, Timestamp};
use crate::policy::LivenessPolicy;
use crate::session::SessionFact;
use crate::CONTRACT_V;

/// The tenant of a run in slice 1: "the single tenant of this store", never a real binding.
pub const LOCAL_TENANT: &str = "local";

closed_code! {
    /// Who appended an event, set by the host from where the input really came and never taken from a
    /// payload (M6). It carries no person. An event folds by its kind, and an unknown origin is never
    /// `reducer`.
    Origin {
        Cli = "cli",
        Host = "host",
        Adapter = "adapter",
        Reducer = "reducer",
        Timer = "timer",
    }
}

/// One event of a run's log.
///
/// `extra` holds whatever a newer writer put on the event that this version does not know, in the
/// envelope, the body and the objects nested in them, not inside an array, so it survives a round trip.
/// It is written back merged, and the output's keys are sorted, not in insertion order.
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

    /// A decision this version cannot fold: a kind it does not know, appended by the reducer of a newer
    /// one, or a known kind holding a value it does not know. Folding past either would diverge silently,
    /// so a reader refuses ownership of the run and projects status read-only (§13).
    pub fn is_unknown_decision(&self) -> bool {
        match &self.body {
            EventBody::Unknown(_) => self.origin == Origin::Reducer,
            EventBody::Decision(decision) => decision.holds_unknown_value(),
            _ => false,
        }
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
            origin: self.origin.clone(),
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
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        $(#[$meta])*
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
pub(crate) use kinded_enum;

kinded_enum! {
    /// What an event says. The inputs are this enum's own variants; every decision is one
    /// [`Decision`], on the wire flat beside them, under its own `kind`.
    ///
    /// `remote = "Self"` makes the derives inherent functions, so the trait impls below can choose by
    /// `kind` first: a known kind goes to its typed reader alone, and its field error reaches the caller.
    /// The two `skip` variants carry their own `kind`, and those impls read and write them.
    #[serde(remote = "Self")]
    EventBody {
        /// The first event of every run. Only this one creates a run. It carries no return address, no
        /// body and no refs: those wait in `pending/<run>`.
        Commissioned = "commissioned" {
            run: RunId,
            workspace_id: String,
            /// Who this run belongs to: required, opaque, immutable, and no person. In slice 1 it is
            /// [`LOCAL_TENANT`]. A later real value is never a stream id or a person id in clear (PV1).
            /// A run with tenant `local` does not leave its machine: component 9 maps it at the boundary
            /// or refuses to sync it, and a mapping binds the synced copy and rewrites no log.
            tenant: String,
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
        /// A root reply while the RUN itself is parked (`root_failed`, `tree_refused`, `poison_input`):
        /// no slot holds that condition, so `resume` cannot name it. Added by 6j6v.r426; an input, so it
        /// does not raise the version.
        ResumeRun = "resume_run",
        /// The human exit, valid in every non-final state. `message` is absent while no root thread exists.
        Withdrawn = "withdrawn" { message: Option<MessageId>, by: WithdrawnBy },
        /// A due timer, keyed `timer/<id>/<fire_at>`, so a re-armed timer's firing is a distinct input.
        TimerFired = "timer_fired" { timer: String, fire_at: Timestamp },
    }
    /// A decision of the reducer.
    #[serde(skip)]
    Decision(Decision),
    /// A kind in neither list of this version. See [`Event::is_unknown_decision`]. A known kind with a
    /// bad field is never this: it is refused, or a damaged input would fold to nothing.
    #[serde(skip)]
    Unknown(UnknownBody),
}

impl Serialize for EventBody {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Decision(decision) => decision.serialize(s),
            Self::Unknown(unknown) => unknown.serialize(s),
            input => Self::serialize(input, s),
        }
    }
}

impl<'de> Deserialize<'de> for EventBody {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, raw) = read_tag(d, "kind")?;
        if Decision::KINDS.contains(&kind.as_str()) {
            Decision::deserialize(raw)
                .map(Self::Decision)
                .map_err(serde::de::Error::custom)
        } else if Self::KINDS.contains(&kind.as_str()) {
            Self::deserialize(raw).map_err(serde::de::Error::custom)
        } else {
            Ok(Self::Unknown(UnknownBody { kind, raw }))
        }
    }
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
        /// On hold, not failed. `until` is the retry, clamped to the policy's window; `resets_at` is what
        /// the backend named, which alone says the hold spends nothing of `max_holds`.
        Held = "held" { slot: Slot, reason: ReasonCode, until: Timestamp, resets_at: Option<Timestamp> },
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

impl Decision {
    /// Whether this decision holds a value of an enum only decisions carry that this version does not
    /// know. `ReasonCode` is not among them: it only explains, and folds the same whatever it says, so
    /// it stays open and adding a code raises no version.
    pub fn holds_unknown_value(&self) -> bool {
        match self {
            Self::StepFinished { outcome, .. } => matches!(outcome, StepOutcome::Unknown(_)),
            Self::Finishing { outcome, .. } | Self::Finished { outcome, .. } => {
                matches!(outcome, Outcome::Unknown(_))
            }
            Self::TimerSet { purpose, .. } => matches!(purpose, TimerPurpose::Unknown(_)),
            _ => false,
        }
    }
}

impl EventBody {
    /// The `kind` this body has on the wire.
    pub fn kind(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|wire| wire.get("kind").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default()
    }
}

/// An object whose tag this version does not know, kept whole so it survives a round trip. `kind` is the
/// tag's value, whatever the field is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownBody {
    pub kind: String,
    pub raw: Value,
}

impl Serialize for UnknownBody {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(s)
    }
}

/// An object read whole, with the value of the tag that decides what reads it next.
pub(crate) fn read_tag<'de, D: Deserializer<'de>>(
    d: D,
    tag: &str,
) -> Result<(String, Value), D::Error> {
    let raw = Value::deserialize(d)?;
    let kind = raw
        .get(tag)
        .and_then(Value::as_str)
        .ok_or_else(|| serde::de::Error::custom(format!("this object carries a {tag}")))?
        .to_owned();
    Ok((kind, raw))
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

tolerant_tagged! {
    /// What a park left behind. An unknown one is recorded, the run goes on, and no park notice is owed.
    ParkOutcome, tag = "kind" {
        /// Committed on a fresh `nxs/park/run-<run>/<instant>` branch.
        Committed = "committed" { branch: String, commit: String },
        /// HEAD was already on the run's own branch, and a commit was made or local commits lie ahead of
        /// `origin/<branch>`: a clean branch with nothing ahead is `Clean`.
        OnOwnBranch = "on_own_branch" { branch: String, commit: String },
        /// Nothing to save, and nothing local ahead of the remote, so no notice is owed. `branch` is where
        /// HEAD stood; absent on a detached HEAD at the base. Absent on the wire when `None`, so a writer
        /// from before this field reads the same.
        Clean = "clean" {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            branch: Option<String>,
        },
        /// The directory is gone; nothing is left to save, which releases the claim.
        Missing = "missing",
        /// The claim key names another run, so no git ran.
        NotHeld = "not_held",
    }
}

tolerant_tagged! {
    /// Why a tree operation was refused: closed, and without text. The first two are permanent, the others
    /// transient, and an unknown one is permanent, so it parks. `tree_refused` in [`ReasonCode`] is the
    /// park this leads to, never this detail.
    TreeRefusal, tag = "code" {
        NotARepository = "not_a_repository",
        ForeignDirectory = "foreign_directory",
        MidSequence = "mid_sequence" { what: SequenceKind },
        Git = "git" { reason: GitReason },
    }
}

closed_code! {
    /// The sequence a tree is in the middle of. A code, because the log carries no text. An unknown one
    /// is still `mid_sequence`.
    SequenceKind {
        Merge = "merge",
        Rebase = "rebase",
        CherryPick = "cherry_pick",
        Revert = "revert",
        Bisect = "bisect",
    }
}

closed_code! {
    /// `continue` is exactly one more pass; `accept` takes the verdict as it stands, at a ceiling only. An
    /// unknown one folds to `ignored`, and the question stays open.
    Disposition {
        Continue = "continue",
        Accept = "accept",
    }
}

closed_code! {
    /// An unknown one still withdraws, because the human exit never closes, with the raw value as the
    /// reason.
    WithdrawnBy {
        Requester = "requester",
        Owner = "owner",
    }
}

closed_code! {
    /// SIGTERM or SIGKILL. A kill is a person's act, and only a withdrawal's `forced` reaches it. An
    /// unknown one counts as a signal sent, and is never assumed a kill.
    StopKind {
        Term = "term",
        Kill = "kill",
    }
}

closed_code! {
    /// Carried by a decision: adding a value raises [`CONTRACT_V`].
    StepOutcome {
        Pass = "pass",
        NeedsRework = "needs_rework",
        Escalated = "escalated",
    }
}

closed_code! {
    /// How a run ended. There is no `rework_limit`: the ceiling asks, it never stops quietly. Carried by a
    /// decision: adding a value raises [`CONTRACT_V`].
    Outcome {
        Delivered = "delivered",
        Withdrawn = "withdrawn",
        Accepted = "accepted",
        Failed = "failed",
    }
}

closed_code! {
    /// Carried by a decision: adding a value raises [`CONTRACT_V`].
    TimerPurpose {
        RootDeadline = "root_deadline",
        StartDeadline = "start_deadline",
        Quiet = "quiet",
        Confirm = "confirm",
        CallBound = "call_bound",
        SubprocessIdle = "subprocess_idle",
        StopGrace = "stop_grace",
        WindDown = "wind_down",
        HoldRetry = "hold_retry",
        TreeWindow = "tree_window",
    }
}
