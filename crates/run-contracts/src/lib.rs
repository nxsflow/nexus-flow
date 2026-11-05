//! The contracts of the nxc run engine, as plain types: serde, no I/O, no clock, and no dependency
//! on any other workspace crate.
//!
//! - (a) run events: [`events`]
//! - (b) storage trait: [`store`]
//! - (c) session start spec and facts: [`session`]
//! - (d) run status: [`status`]
//!
//! The liveness policy pinned on `commissioned` is [`policy`]. These are frozen as v1
//! ([`CONTRACTS_STATUS`]), after the external reviews on board item 6j6v.p71m, and the wire as it was
//! frozen is `tests/golden/v1/`. Specification: the description of epic 6j6v.zjyv.
//!
//! Evolution is additive. A reader accepts unknown fields and keeps them on a round trip, and an
//! unknown kind or enum value is kept as `Unknown` rather than refused; a known kind with a bad field,
//! or a value of the wrong type, is refused. A new field on a logged kind is an `Option`, because a
//! known kind missing a required field is refused, which would make every written run unreadable.
//!
//! What a reader cannot fold past raises [`CONTRACT_V`]: a new decision kind, and a new value of an enum
//! only decisions carry ([`StepOutcome`], [`Outcome`], [`TimerPurpose`]). A reducer that folds past
//! either diverges silently (§13), so a reader that meets one refuses ownership and projects status
//! read-only. An unknown value in an input folds by the rule on its enum instead, and refuses nothing.

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
    LOCAL_TENANT,
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
    ClaimStatus, FlowStatus, InFlight, LocalStatus, MemberStatus, Needs, NeedsKind, Owner, Refusal,
    RunState, RunStatus, SlotLiveness, StepStatus, TreeStatus, Verdict,
};
pub use store::{
    Appended, Cas, Enqueued, RunRow, RunStore, StoreError, StoreLimits, StoreResult, Timer,
    TimerChange, TimerChangeKind,
};

/// Frozen at the end of both external reviews (6j6v.cck2). A breaking change from here on raises
/// [`CONTRACT_V`], and that version's reader still reads v1.
pub const CONTRACTS_STATUS: &str = "frozen-v1";

/// The version every wire object carries as `v`. Adding a [`Decision`] kind or a value of an enum only
/// decisions carry raises it, because a reader cannot safely fold past either; `tests/evolution.rs`
/// holds the rule.
pub const CONTRACT_V: u32 = 1;
