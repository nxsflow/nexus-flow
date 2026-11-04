//! The contracts of the nxc run engine, as plain types: serde, no I/O, no clock, and no dependency
//! on any other workspace crate.
//!
//! - (a) run events: [`events`]
//! - (b) storage trait: [`store`]
//! - (c) session start spec and facts: [`session`]
//! - (d) run status: [`status`]
//!
//! The liveness policy pinned on `commissioned` is [`policy`]. These are DRAFTS
//! ([`CONTRACTS_STATUS`]): the external reviews on board item 6j6v.p71m come first, and a later
//! ticket freezes them. Specification: the description of epic 6j6v.zjyv.
//!
//! Evolution is additive. A reader accepts unknown fields and keeps them on a round trip, and an
//! unknown kind is kept as [`UnknownBody`] rather than refused. Adding a decision kind is the one
//! addition that also raises [`CONTRACT_V`], because a reducer that folds past a decision it does
//! not know diverges silently (§13).

mod codes;
pub mod events;
mod ids;
pub mod policy;
pub mod session;
pub mod status;
pub mod store;

pub use codes::{GitReason, ReasonCode, REMEDY_TABLE};
pub use events::{
    Base, Decision, Disposition, Event, EventBody, FlowRef, Origin, Outcome, ParkOutcome,
    SequenceKind, StepOutcome, StopKind, TimerPurpose, TreeRefusal, UnknownBody, WithdrawnBy,
};
pub use ids::{
    Epoch, Handle, Key, MessageId, RunId, Secs, Seq, SessionKey, Slot, StepId, Timestamp,
};
pub use policy::LivenessPolicy;
pub use session::{
    BackendError, Capabilities, Capability, DrainedFact, EndCause, EnvVar, HeartbeatKind, Liveness,
    Prompt, SessionBackend, SessionFact, StartOutcome, StartSpec, Turn, ENV_ALLOWLIST,
};
pub use status::{
    ClaimStatus, FlowStatus, InFlight, MemberStatus, Needs, NeedsKind, Owner, Refusal, RunState,
    RunStatus, SlotLiveness, StepStatus, TreeStatus, Verdict,
};
pub use store::{
    Appended, Cas, Enqueued, RunRow, RunStore, StoreError, StoreLimits, StoreResult, Timer,
    TimerChange, TimerChangeKind,
};

/// Drafts until the freeze ticket; manufakt.io's review is still outstanding.
pub const CONTRACTS_STATUS: &str = "draft";

/// The version every wire object carries as `v`. Adding a [`Decision`] kind raises it, because a
/// reader cannot safely fold past a decision it does not know; `tests/evolution.rs` holds the rule.
pub const CONTRACT_V: u32 = 1;
