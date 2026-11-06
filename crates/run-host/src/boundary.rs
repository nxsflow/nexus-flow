//! Every place the crash harness may abort the host (§12.4). `Boundary::ALL` is exhaustive by
//! construction: the list and the enum come from one macro, and nothing else names a boundary.
//!
//! `NXR_CRASH_AT=<boundary>:<nth>` makes the process `abort()` at the nth occurrence of a boundary. It is
//! read only under `test-support`, so the shipped service cannot reach it (§11): without the feature
//! [`crash_at`] is an empty function.

macro_rules! boundaries {
    ($($(#[$meta:meta])* $variant:ident = $name:literal,)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Boundary {
            $($(#[$meta])* $variant,)+
        }

        impl Boundary {
            pub const ALL: &'static [Boundary] = &[$(Boundary::$variant,)+];

            pub fn name(self) -> &'static str {
                match self {
                    $(Boundary::$variant => $name,)+
                }
            }
        }
    };
}

boundaries! {
    BeforeAppend = "before_append",
    AfterAppendBeforeEffect = "after_append_before_effect",
    AfterSpawnJsonBeforeSpawn = "after_spawn_json_before_spawn",
    AfterSpawnBeforeChildJson = "after_spawn_before_child_json",
    AfterThreadIdBeforeOpen = "after_thread_id_before_open",
    /// The turn message and the `thread/<id>` and `session/<key>` CASes are behind this one and the next.
    AfterOpenBeforeTurnPost = "after_open_before_turn_post",
    AfterTurnPostBeforeRoutes = "after_turn_post_before_routes",
    BetweenTheRoutes = "between_the_routes",
    AfterRoutesBeforeSpawn = "after_routes_before_spawn",
    /// The ledger entry says `done`; the backend has not been asked yet.
    BeforeSpawn = "before_spawn",
    /// A ledger entry says `pending`, and nothing is posted yet.
    AfterLedgerBeforePost = "after_ledger_before_post",
    AfterPostBeforeDone = "after_post_before_done",
    AfterClaimCasBeforeClaimTaken = "after_claim_cas_before_claim_taken",
    BeforeTreeLedger = "before_tree_ledger",
    AfterGitSpawnBeforeRecord = "after_git_spawn_before_record",
    /// The gate is open: the program runs, with its record in the ledger, and the host is about to wait on it.
    AfterGitReleasedBeforeResult = "after_git_released_before_result",
    DuringClone = "during_clone",
    AfterParkCommitBeforeTreeParked = "after_park_commit_before_tree_parked",
    AfterTreeResultBeforeInput = "after_tree_result_before_input",
    AfterReleaseCasBeforeClaimReleased = "after_release_cas_before_claim_released",
    AfterEffectBeforeResult = "after_effect_before_result",
    BetweenFactsReadAndOffset = "between_facts_read_and_offset",
    /// The stop ledger is written, the signal not sent yet.
    BeforeSignal = "before_signal",
    BetweenSignalAndRecord = "between_signal_and_record",
}

impl Boundary {
    pub fn parse(name: &str) -> Option<Boundary> {
        Boundary::ALL.iter().copied().find(|b| b.name() == name)
    }
}

/// Aborts here if the harness asked for this occurrence of this boundary.
#[cfg(feature = "test-support")]
pub fn crash_at(boundary: Boundary) {
    crate::test_support::crash::reached(boundary);
}

/// The shipped service has no way to abort at a boundary.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn crash_at(_boundary: Boundary) {}

/// Tells the journal a write happened, so the executor's test can check that every write it makes is
/// behind a listed boundary (§12.4). Nothing without the feature.
#[cfg(feature = "test-support")]
pub fn wrote(what: &str) {
    crate::test_support::crash::wrote(what);
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn wrote(_what: &str) {}
