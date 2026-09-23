//! The canonical JSON record — now owned by `nexus-flow-facade` as the `record` module
//! (E5 #9t7.1 / E9 #49g), the presentation-independent agent contract every consumer shares.
//! Re-exported under the CLI's historical `output` name so `output::item_value` (and friends)
//! keep resolving unchanged in the command renderers.

pub use nexus_flow_facade::record::*;
