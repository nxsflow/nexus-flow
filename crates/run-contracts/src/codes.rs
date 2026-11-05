//! The closed code lists of the run log (§6.7): `ReasonCode`, and `GitReason` beside it. One
//! definition produces the enum, its wire form and `ALL`, so the published list cannot drift from the
//! enum. A writer only ever uses the named codes. A code from a newer version reads as `Unknown(raw)`
//! and survives a round trip; it is in no list.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! closed_code {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $wire:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
            /// A code this version does not know, kept as it was read.
            Unknown(String),
        }

        impl $name {
            /// Every code a writer of this version may use, in declaration order.
            pub const ALL: &'static [&'static str] = &[$($wire),+];

            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Unknown(raw) => raw,
                }
            }

            pub fn parse(text: &str) -> Self {
                match text {
                    $($wire => Self::$variant,)+
                    other => Self::Unknown(other.to_owned()),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self::parse(&String::deserialize(d)?))
            }
        }
    };
}

closed_code! {
    /// Why a decision stopped or skipped something (ST4), and how a session ended. `tree_refused` is the
    /// only code a refused tree gets here; WHY git refused is [`GitReason`], and what kind of refusal
    /// is `TreeRefusal`, both beside it and never in this list.
    ReasonCode {
        StaleTimer = "stale_timer",
        DuplicateReply = "duplicate_reply",
        LateReplyAfterSkip = "late_reply_after_skip",
        UnknownSession = "unknown_session",
        FinalState = "final_state",
        AlreadyFinishing = "already_finishing",
        /// An input that is valid in general and has no use in the run's present state.
        OutOfState = "out_of_state",
        Quiet = "quiet",
        Hung = "hung",
        StartFailed = "start_failed",
        Lost = "lost",
        Exit = "exit",
        TurnComplete = "turn_complete",
        StoppedByHost = "stopped_by_host",
        WouldNotStop = "would_not_stop",
        RootFailed = "root_failed",
        TreeRefused = "tree_refused",
        PoisonInput = "poison_input",
        PermissionRefused = "permission_refused",
        EndedWithoutReply = "ended_without_reply",
        FanOutIncomplete = "fan_out_incomplete",
        UsageLimit = "usage_limit",
        ProviderUnavailable = "provider_unavailable",
        HoldLimit = "hold_limit",
        Ceiling = "ceiling",
        AcceptedByRequester = "accepted_by_requester",
        WithdrawnByRequester = "withdrawn_by_requester",
        WithdrawnByOwner = "withdrawn_by_owner",
        Delivered = "delivered",
    }
}

closed_code! {
    /// Why a git call was refused, as a code and never as git's text; the scrubbed text lives in
    /// `kv['tree_refusal/<run>']`. A hook cannot fail a call because every call runs with hooks off, and
    /// a commit always has an identity because the runner falls back to one, so neither is a reason.
    GitReason {
        BoundExceeded = "bound_exceeded",
        FetchFailed = "fetch_failed",
        LockHeld = "lock_held",
        ExitNonzero = "exit_nonzero",
    }
}

/// The non-final rows of the §6.5 table that end in a park or a hold, with the code each one records.
/// The code column is typed, so a row cannot name a code the enum does not have; the golden pins the
/// table beside the list.
pub const REMEDY_TABLE: &[(&str, ReasonCode)] = &[
    ("run Created: no root_opened", ReasonCode::RootFailed),
    (
        "run Running, tree Unprepared: refused",
        ReasonCode::TreeRefused,
    ),
    ("slot Starting: no started fact", ReasonCode::StartFailed),
    (
        "slot Quiet or Nudged: no fact after the nudge",
        ReasonCode::Hung,
    ),
    (
        "slot Stopping: alive after the second SIGTERM",
        ReasonCode::WouldNotStop,
    ),
    (
        "slot Replied, process live: alive after the second SIGTERM",
        ReasonCode::WouldNotStop,
    ),
    (
        "slot Ended turn_complete without a reply",
        ReasonCode::EndedWithoutReply,
    ),
    ("slot Ended exit without a reply", ReasonCode::StartFailed),
    ("slot Ended lost without a reply", ReasonCode::Lost),
    ("slot Ended usage_limit", ReasonCode::UsageLimit),
    (
        "slot Ended provider_unavailable",
        ReasonCode::ProviderUnavailable,
    ),
    (
        "slot Ended usage_limit or provider_unavailable: over max_holds",
        ReasonCode::HoldLimit,
    ),
    (
        "slot Ended permission_refused",
        ReasonCode::PermissionRefused,
    ),
    (
        "fan-out member out of attempts",
        ReasonCode::FanOutIncomplete,
    ),
    ("inbox row that will not fold", ReasonCode::PoisonInput),
];
