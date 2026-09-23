//! nxs-sync — the sync seam.
//!
//! Two responsibilities, deliberately split so the relay can depend on the wire
//! types ALONE (no `engine` feature → no core fold code in its graph, spec §3):
//!
//! - [`wire`] / [`protocol`] (always): the versioned, server-opaque envelope and the
//!   HTTP request/response shapes the relay and client both speak.
//! - [`presence`] (always): which machines sync a stream and the ONE rule for whether one is
//!   online (6j6v.f0b5) — pure, so the relay, the CLI and an embedding host share it.
//! - [`redact`] (always): the one rule that keeps a relay URL's credentials out of a message
//!   (6j6v.q3kk) — dependency-free, so a host linking only the wire types can use it too.
//! - [`engine`] (feature `engine`): the client anti-entropy algorithm that reads the
//!   local core store, pushes/pulls over a [`engine::Transport`], and folds foreign ops.
//! - [`snapshot`] (feature `engine`): a replica's folded state with the relay position it reaches
//!   (6j6v.mxt2) — export from one replica, import into a fresh one, then pull only the rest.

pub mod presence;
pub mod protocol;
pub mod redact;
pub mod wire;

#[cfg(feature = "engine")]
pub mod engine;
#[cfg(feature = "engine")]
pub mod snapshot;
