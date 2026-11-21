//! The SDK boundary (NEX-89, RFC 0001 "The dependency on NoCentralDB"): only the storage crate,
//! `nexus-flow-store`, depends on NoCentralDB's SDK, so that a change in NoCentralDB's API at a pin
//! move touches one crate.

use anyhow::{bail, Result};

use crate::layering::Metadata;

/// Fail when a workspace crate other than the storage crate lists the SDK.
pub fn check(_metadata: &Metadata) -> Result<()> {
    bail!("the SDK boundary check is not written yet")
}

#[cfg(test)]
#[path = "sdk_boundary_tests.rs"]
mod tests;
