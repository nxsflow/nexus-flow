//! Contract (d): run status, read-only with no subscription. `finished` is the single machine field,
//! and `outcome` is set exactly when it is true. The engine projects most of this purely from state;
//! the host adds `owner` and `tree` around that projection. Status carries ids and codes; the one text
//! is a tree refusal's scrubbed `text`, read from its kv row and gone when the run finishes.

use serde::{Deserialize, Serialize};

use crate::codes::{closed_code, tolerant_tagged, ReasonCode};
use crate::events::{Base, Outcome, ParkOutcome, TreeRefusal};
use crate::ids::{Handle, MessageId, RunId, Seq, SessionKey, StepId, Timestamp};

/// The fixed values of `waiting_for`. The others name a party: the reply of a persona, a decision of the
/// requester, or the holder of the directory.
pub const WAITING_FOR_FIXED: &[&str] = &[
    "a newer nxs",
    "a fold that does not panic",
    "the park",
    "the chat store",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunStatus {
    pub v: u32,
    pub run: RunId,
    pub flow: FlowStatus,
    pub seq: Seq,
    pub finished: bool,
    pub outcome: Option<Outcome>,
    pub state: RunState,
    pub claim: ClaimStatus,
    pub owner: Owner,
    pub tree: TreeStatus,
    pub steps: Vec<StepStatus>,
    /// Inputs in the inbox the owner has not folded yet: status never claims a reply arrived before that.
    pub pending_inputs: u32,
    pub waiting_for: Option<String>,
    pub needs: Option<Needs>,
    pub reason: Option<ReasonCode>,
    pub next_action: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowStatus {
    pub name: String,
    pub hash: String,
}

tolerant_tagged! {
    /// The run's state. `NeedsDecision`, `Held` and `Parked` are projections of slot facts. An unknown one
    /// is rendered as it reads.
    RunState, tag = "name" {
        Created = "created",
        /// `behind` is a v2 run of this workspace, never a v1 thread.
        Queued = "queued" { behind: Option<RunId> },
        Running = "running",
        NeedsDecision = "needs_decision",
        Held = "held" { until: Timestamp },
        Parked = "parked",
        Finishing = "finishing",
        Finished = "finished",
    }
}

/// The run's claim on its directory. It has no time limit and ends with the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimStatus {
    pub workspace_id: String,
    pub held: bool,
    pub since: Option<Timestamp>,
}

/// Whether any service attends the workspace: seen within two lease expiries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub attended: bool,
    pub last_seen: Option<Timestamp>,
}

/// The v2 working directory. `path` is derived at status time and never stored. `commit`, `branch` and
/// `dirty` are read live, and `null` when the directory is missing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeStatus {
    pub path: String,
    pub base: Option<Base>,
    pub found: Option<ParkOutcome>,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
    pub in_flight: Option<InFlight>,
    pub parked: Option<ParkOutcome>,
    pub refusal: Option<Refusal>,
}

/// The tree operation the worker is running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlight {
    pub op: String,
    pub step: String,
    pub since: Timestamp,
}

/// The standing refusal, from `kv['tree_refusal/<run>']`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub reason: TreeRefusal,
    pub text: String,
    pub at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepStatus {
    pub id: StepId,
    pub visit: u32,
    pub pass: u32,
    pub state: String,
    pub outcome: Option<crate::events::StepOutcome>,
    pub started: Option<Timestamp>,
    pub ended: Option<Timestamp>,
    pub members: Vec<MemberStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStatus {
    pub handle: Handle,
    pub session_key: SessionKey,
    pub attempt: u32,
    pub state: String,
    pub liveness: SlotLiveness,
    /// The processes a stop could not end, so a person can end them.
    pub pids: Vec<u32>,
    /// `<rule>: <program>`, when the session reported a refusal and carried on.
    pub refused: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotLiveness {
    pub last_fact: Option<Timestamp>,
    pub quiet_for_s: u64,
    pub verdict: Verdict,
}

closed_code! {
    /// `working`: a fact within `quiet_after`. `quiet`: a declared wait, or no fact yet inside the window.
    /// `suspect`: nudged, waiting for confirmation. `hung`: the ladder is exhausted. An unknown one is
    /// rendered as it reads.
    Verdict {
        Working = "working",
        Quiet = "quiet",
        Suspect = "suspect",
        Hung = "hung",
    }
}

/// What the run needs from a person. A hold or a park has no question id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Needs {
    pub kind: NeedsKind,
    pub question: Option<MessageId>,
    pub since: Timestamp,
}

closed_code! {
    /// An unknown one is rendered as it reads.
    NeedsKind {
        Decision = "decision",
        Person = "person",
    }
}
