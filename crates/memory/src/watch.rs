//! File-based change notification is foundation-owned (it polls `PRAGMA data_version`, a substrate
//! property, and knows no product vocabulary). This re-exports the consumer-facing [`Change`] so
//! `nexus_memory::watch::Change` resolves; the [`Engine`](crate::engine::Engine) drives the watcher
//! through the foundation handle. Mirror of flow's `nexus_flow_facade::watch`.

pub use nxs_foundation::watch::{Change, POLL_INTERVAL};
