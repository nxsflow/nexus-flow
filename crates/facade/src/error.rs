//! The structured error envelope shared by every consumer of the engine — owned by the
//! foundation (it is presentation-free, domain-agnostic data) and re-exported here so existing
//! `nexus_flow_facade::error::*` paths keep resolving.

pub use nxs_foundation::error::{ErrorKind, NxfError, Result};
