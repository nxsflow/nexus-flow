//! The liveness policy. It is DATA: the host pins one value on `commissioned.policy`, and `decide`
//! reads it from the run's state, so a replay folds with the values the run started under and nothing
//! reads the environment.

use serde::{Deserialize, Serialize};

use crate::ids::Secs;

/// The durations and budgets of §6.5, each a field on the run. `Default` is the §6.5 table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessPolicy {
    /// How long a run in `Created` may go with no `root_opened`, per period.
    pub root_deadline: Secs,
    /// How long a slot in `Starting` may go with no `started` fact.
    pub start_deadline: Secs,
    /// No model output for this long makes a session with no open call `Quiet`.
    pub quiet_after: Secs,
    /// How long a nudged session has to show any fact.
    pub confirm_within: Secs,
    /// The bound on an open call, on a host that cannot sample its subprocesses only.
    pub call_bound: Secs,
    /// Subprocesses of an open call that have not moved for this long make the slot `Quiet`.
    pub subprocess_idle: Secs,
    /// The grace after each stop signal, and the wind-down of a replied slot.
    pub stop_grace: Secs,
    /// How long a transient tree refusal is retried before the run parks.
    pub tree_window: Secs,
    /// A clock jump above this defers destructive timers until the next adapter sample.
    pub wake_jump: Secs,
    /// The retry of a hold whose backend named no `resets_at`.
    pub hold_retry: Secs,
    /// The floor a backend's `resets_at` is clamped to.
    pub hold_retry_min: Secs,
    /// The ceiling a backend's `resets_at` is clamped to.
    pub hold_retry_max: Secs,
    pub max_attempts: u32,
    pub max_holds: u32,
}

impl Default for LivenessPolicy {
    fn default() -> Self {
        Self {
            root_deadline: Secs::minutes(1),
            start_deadline: Secs::minutes(3),
            quiet_after: Secs::minutes(10),
            confirm_within: Secs::minutes(5),
            call_bound: Secs::hours(3),
            subprocess_idle: Secs::minutes(30),
            stop_grace: Secs::minutes(5),
            tree_window: Secs::minutes(10),
            // Two adapter samples: one late sample is not a wake. §6.5 names the value, not the number.
            wake_jump: Secs::minutes(2),
            hold_retry: Secs::minutes(30),
            hold_retry_min: Secs::minutes(1),
            hold_retry_max: Secs::hours(24),
            max_attempts: 3,
            max_holds: 5,
        }
    }
}
