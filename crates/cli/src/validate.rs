//! Shared input validation for the write commands — now owned by `nexus-flow-facade` so every
//! write seam (CLI, MCP, embedding app) validates identically through the one shared write layer
//! (E5 #9t7.5). Re-exported so `crate::validate::…` paths throughout the CLI resolve unchanged.

pub use nexus_flow_facade::validate::*;
