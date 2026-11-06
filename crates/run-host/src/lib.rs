//! The host of the nxc run engine: it owns runs by a per-run lease, folds their inboxes through the pure
//! reducer, executes what they owe, and never waits on a session or on git. `Host::tick` is called once per
//! iteration of the service loop. Specification: §5, §6.6, §7, §8 and §13 of epic 6j6v.zjyv.
//!
//! The rule that makes a crash uninteresting: every effect is derived from state and recorded by an input,
//! so what a restart must do is what a tick does anyway.

pub mod boundary;
pub mod bridge;
mod compose;
mod effects;
mod fold;
mod host;
mod lease;
pub mod ports;
pub mod session_stub;
pub mod start;
#[cfg(feature = "test-support")]
pub mod test_support;
pub(crate) mod tree_effects;
pub mod tree_ledger;
pub mod tree_worker;

pub use boundary::Boundary;
pub use host::{mint_owner_id, Host, Layout, LogSink, Ports, TickReport, TICK_BUDGET};
pub use tree_ledger::{LedgerEntry, TreeLedger, SPAWN_GRACE};
pub use tree_worker::{
    tree_state, InFlight, LeaseView, StepOutput, TreeBinding, TreeCx, TreeOp, TreeResult,
    TreeRunner, TreeState, TreeStateError, TreeStop, TreeWorker, WorkTree, OPERATION_DEADLINE,
};
