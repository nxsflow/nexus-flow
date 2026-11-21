//! nexus-flow's storage crate (RFC 0001, "The library seam", new: `store`). It is the only crate
//! that depends on NoCentralDB's Rust SDK; every other crate reaches NoCentralDB through it. It
//! will hold the model, opening member databases, members and role records, sync and derived
//! state. It is not public: the facade re-exports what an embedder needs, never the SDK whole.

use nocentraldb as _;

pub mod goals;
pub mod status;
