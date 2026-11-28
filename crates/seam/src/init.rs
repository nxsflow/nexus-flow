//! `init`, the seam's counterpart of `nxs init` (RFC 0001, "Creating and finding a space"):
//! gives the repository at or above a folder its space, creating the space and `nexus-flow.toml`
//! when the repository has none, and otherwise naming the space the file already holds. It ends
//! with the factory's install (RFC 0006, "Installing the shipped defaults") and then GENESIS's
//! create (RFC 0004, "Installing the template"), both without `update`, so it never replaces a
//! file that is there.
//!
//! As with `Engine::open`, `init` takes the data directory from the environment and keeps the
//! device's keys in the platform's key store, and `init_with` names both.

use std::path::Path;

use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::space::{self, InitError, InitReport, SyncMode};

use crate::engine::{Error, Options};
use crate::genesis;
use crate::install::factory::{self, Defaults};

/// What `init` did: the space's part, then the factory's install and GENESIS's create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Init {
    /// The repository space's id, as `nexus-flow.toml` names it.
    pub space: String,
    /// Whether this call created the space; `false` when the file already named one.
    pub created: bool,
    /// How the space syncs.
    pub sync: SyncMode,
    /// Whether the root holds a v1 workspace, `.nxs/`, which was left as it was.
    pub v1_workspace: bool,
    /// What the factory's install did, as `Engine::factory_install(false)` reports it.
    pub factory: factory::Report,
    /// What GENESIS's create did, as `Engine::genesis_create(_, false)` reports it. When it
    /// [refuses](genesis::Report::is_refused), GENESIS is tracked in git but missing, and nothing
    /// was written there.
    pub genesis: genesis::Report,
}

/// Gives the repository at or above `dir` its space, then installs the factory and GENESIS.
/// When no `nexus-flow.toml` is found from `dir` upward, `dir` becomes the root.
pub async fn init(dir: &Path) -> Result<Init, Error> {
    init_with(dir, Options::from_env()?).await
}

/// [`init`], with the data directory and keys `options` names.
pub async fn init_with(dir: &Path, options: Options) -> Result<Init, Error> {
    let InitReport {
        space,
        created,
        sync,
        v1_workspace,
    } = space::init(dir, &options.data, options.keys).await?;
    let root = RepoRoot::find(dir).map_err(InitError::Root)?;
    let factory =
        factory::install(root.path(), &Defaults::embedded(), false).map_err(Error::Install)?;
    let genesis = genesis::create(root.path(), false).map_err(Error::Genesis)?;
    Ok(Init {
        space,
        created,
        sync,
        v1_workspace,
        factory,
        genesis,
    })
}
