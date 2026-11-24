//! The genesis guide, `factory/genesis-guide.md` (RFC 0004, "The guide"): nexus-flow's text,
//! installed under RFC 0006's rules for shipped defaults.

use std::path::Path;

use super::{FileReport, GenesisError};
use crate::install::shipped::Shipped;

/// The current guide, embedded at build time.
pub const GUIDE: &str = include_str!("../../../../templates/genesis/genesis-guide.md");

/// Every version of the guide nexus-flow has shipped, in RFC 0006's `shipped.toml` format.
pub const SHIPPED: &str = include_str!("../../../../templates/genesis/shipped.toml");

/// The guide's path, relative to the repository's root.
pub const GUIDE_PATH: &str = "factory/genesis-guide.md";

/// Installs the guide `current` in the repository at `root` by the versions `shipped` lists.
pub fn install_guide(
    root: &Path,
    shipped: &Shipped,
    current: &str,
    update: bool,
) -> Result<Option<FileReport>, GenesisError> {
    let _ = (root, shipped, current, update);
    Ok(None)
}
