//! The seam between a nexus-flow **background service** and everything that wants it to attend a
//! workspace (6j6v.5zst).
//!
//! # The gap this closes
//!
//! The service runs one process per INSTANCE (nxf 6j6v.gd9p — the production one by default, and
//! for most machines the only one) and works through a registry —
//! `~/.nexusflow/workspaces.toml` — sweeping every workspace listed there. Until this crate, the
//! only thing that could write that registry was `nxs sync bind`, a verb in `crates/nxs`: the
//! umbrella CLI. An embedding app deliberately does not link it (see the crate manifest), so
//! "register this project with the service" was reachable only by shelling out to a binary the app
//! does not ship. The owner decision of 2026-08-25 makes the opposite true — deadlines run ALWAYS
//! through the background service, including for work an app started — and a service can only
//! attend a workspace it has been told about.
//!
//! # The shape
//!
//! Everything is addressed through a [`ServiceHome`], the directory the service keeps its state in.
//! [`ServiceHome::resolve`] is the real `~/.nexusflow`; [`ServiceHome::at`] points at any directory,
//! which is what makes every behaviour here testable without a `$HOME` to fake. That is not a
//! convenience: the registry this crate now owns was measured at 98 stale entries on one developer
//! machine, because in-process unit tests reaching a hardcoded `~/.nexusflow` had been writing into
//! it for months.
//!
//! Three verbs an app needs, and no more:
//!
//! - [`ServiceHome::register`] — attend this workspace. Idempotent.
//! - [`ServiceHome::deregister`] — stop attending it. The half that was missing everywhere: an app
//!   that registers every project it opens and never removes one rebuilds those 98 entries, only
//!   faster.
//! - [`ServiceHome::attendance`] — the reading. Is the service up, does it know this workspace,
//!   and what did it last do with it?
//!
//! And one identity (nxf 6j6v.f0b5): [`ServiceHome::machine`] / [`ServiceHome::rename_machine`] —
//! the name and id this service home is listed under at every relay it syncs to. A service home
//! IS a machine in that sense; [`machine`] says why.

pub mod atomic;
pub mod heartbeat;
pub mod home;
pub mod instance;
pub mod launchd;
pub mod lock;
pub mod machine;
pub mod orders;
pub mod program;
pub mod registry;
pub mod timers;
pub mod wake;

pub use heartbeat::{Heartbeat, ServiceState, WorkspaceHealth};
pub use home::ServiceHome;
pub use instance::{ignored_instance_env, instance_env_ignored_note, Instance, Origin};
pub use lock::ServiceLock;
pub use machine::Machine;
pub use program::ProgramState;
pub use registry::WorkspaceEntry;
pub use timers::{Deadline, Job};
pub use wake::{SystemWake, Wake, WakeChange, WakeKeeper};

mod attend;
pub use attend::{shared_workspace_note, Attendance, ServiceFault, SharedWorkspace};
