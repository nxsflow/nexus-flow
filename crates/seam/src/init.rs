//! `init`, the seam's counterpart of `nxs init` (RFC 0001, "Creating and finding a space"):
//! gives the repository at or above a folder its space, creating the space and `nexus-flow.toml`
//! when the repository has none, and otherwise naming the space the file already holds.
//!
//! As with `Engine::open`, `init` takes the data directory from the environment and keeps the
//! device's keys in the platform's key store, and `init_with` names both.

use std::path::Path;

use nexus_flow_store::space::{self, InitReport};

use crate::engine::{Error, Options};

/// Gives the repository at or above `dir` its space. When no `nexus-flow.toml` is found from
/// `dir` upward, `dir` becomes the root.
pub async fn init(dir: &Path) -> Result<InitReport, Error> {
    init_with(dir, Options::from_env()?).await
}

/// [`init`], with the data directory and keys `options` names.
pub async fn init_with(dir: &Path, options: Options) -> Result<InitReport, Error> {
    Ok(space::init(dir, &options.data, options.keys).await?)
}
