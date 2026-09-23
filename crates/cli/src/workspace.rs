//! On-disk `.nxs/` workspace resolution + store opening — now owned by
//! `nexus-flow-facade` (E5 #9t7.1) so the long-lived engine handle and the CLI share one
//! workspace implementation. Re-exported so `crate::workspace::…` paths resolve unchanged.

pub use nexus_flow_facade::workspace::*;
