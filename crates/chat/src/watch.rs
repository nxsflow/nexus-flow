//! chat's change-notification re-export — the foundation's `PRAGMA data_version` watcher (no daemon,
//! no tokio). An app holds one [`Engine`](crate::engine::Engine), calls `subscribe()` once, and
//! re-reads `channels()`/`inbox()` on each coalesced [`Change`]. The tick is domain-blind (whole-db
//! "someone wrote"); per-channel routing is the app layer's re-read filter (nexus-chat-M1 spec, the
//! app-layer note). Mirrors flow's/memory's `watch` re-export.

pub use nxs_foundation::watch::{Change, POLL_INTERVAL};
