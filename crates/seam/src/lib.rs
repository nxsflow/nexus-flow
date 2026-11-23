//! v2's library seam (RFC 0001, "The library seam"), built as `crates/seam` until the cut-over (NEX-194) names it `nexus-flow-facade`.

mod engine;

pub use engine::{Engine, Error, Options};
pub use nexus_flow_store::data_dir::{DataDir, DataDirError};
pub use nexus_flow_store::id::{NotASpaceId, SpaceId};
pub use nexus_flow_store::space::{DeviceKeys, InitError};
