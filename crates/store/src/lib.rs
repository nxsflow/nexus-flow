//! nexus-flow's storage crate (RFC 0001, "The library seam", new: `store`). It is the only crate
//! that depends on NoCentralDB's Rust SDK; every other crate reaches NoCentralDB through it. It
//! will hold the model, opening member databases, members and role records, sync and derived
//! state. It is not public: the facade re-exports what an embedder needs, never the SDK whole.

use nocentraldb as _;

pub mod config;
pub mod coordination;
pub mod data_dir;
pub mod decisions;
pub mod device;
pub mod goals;
pub mod id;
pub mod member_select;
pub mod members;
pub mod migration;
pub mod model;
mod open;
pub mod opened;
mod read;
pub mod repo_root;
pub mod space;
pub mod space_kind;
pub mod space_marker;
pub mod status;
pub mod sync;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod write;
pub mod write_once;
