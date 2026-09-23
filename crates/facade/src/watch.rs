//! File-based change notification is foundation-owned (it polls `PRAGMA data_version`, a substrate
//! property, and knows no product vocabulary). This re-exports the consumer-facing [`Change`] so
//! existing `nexus_flow_facade::watch::Change` paths resolve; the [`Engine`](crate::engine::Engine)
//! drives the watcher through the foundation handle.

pub use nxs_foundation::watch::{Change, POLL_INTERVAL};
