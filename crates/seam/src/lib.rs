//! v2's library seam (RFC 0001, "The library seam"), built as `crates/seam` until the cut-over (NEX-194) names it `nexus-flow-facade`.

mod engine;
pub mod genesis;
mod init;
pub mod install;
pub mod merge_gate;

pub use engine::{set_relay, Engine, Error, Options};
pub use init::{init, init_with};
pub use nexus_flow_store::config::{ConfigError, Problem};
pub use nexus_flow_store::coordination::{JoinError, JoinReport};
pub use nexus_flow_store::data_dir::{DataDir, DataDirError, Platform};
pub use nexus_flow_store::decisions::{
    Decision, DecisionError, Linked, NewDecision, Subject, SubjectKind,
};
pub use nexus_flow_store::device::{NotARelayUrl, RelayUrl, SettingsError};
pub use nexus_flow_store::id::{NotASessionId, NotASpaceId, SessionId, SpaceId};
pub use nexus_flow_store::member_select::SessionVarError;
pub use nexus_flow_store::members::{Member, MemberError, MemberKind, TicketRef};
pub use nexus_flow_store::repo_root::RootError;
pub use nexus_flow_store::space::{DeviceKeys, InitError, InitReport, SyncMode};
pub use nexus_flow_store::sync::{Behind, SpaceBehind, SyncError, SyncStatus};

/// The store's test helpers, for the tests of the crates above the seam: invitations of members
/// nexus-flow cannot start yet.
#[cfg(feature = "test-support")]
pub use nexus_flow_store::testing;
