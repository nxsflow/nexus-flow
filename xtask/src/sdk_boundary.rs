//! The SDK boundary (NEX-89, RFC 0001 "The dependency on NoCentralDB"): only the storage crate,
//! `nexus-flow-store`, depends on NoCentralDB's SDK, so that a change in NoCentralDB's API at a pin
//! move touches one crate. Every other workspace crate reaches NoCentralDB through it. The check
//! reads `cargo metadata --no-deps`, so it needs no build, and it covers every dependency kind
//! (normal, dev, build, and target-specific ones), under any local rename. NoCentralDB's other
//! crates (`ncdb-*`) count as the SDK here, as they do for the pin (`sdk_pin.rs`).

use std::path::Path;

use anyhow::{bail, Result};

use crate::layering::{read_metadata, Metadata};

/// The storage crate, the one workspace crate that may depend on the SDK (decided in NEX-88).
pub const STORE: &str = "nexus-flow-store";

/// The SDK's package name.
const SDK: &str = "nocentraldb";

/// Runs the check against the workspace at `root`.
pub fn run(root: &Path) -> Result<()> {
    check(&read_metadata(root)?)
}

/// Fail when a workspace crate other than the storage crate lists the SDK, naming every such edge.
pub fn check(metadata: &Metadata) -> Result<()> {
    let mut problems = Vec::new();
    for package in metadata.packages.iter().filter(|p| p.name != STORE) {
        for dep in package.dependencies.iter().filter(|d| is_sdk(&d.name)) {
            let kind = dep.kind.as_deref().unwrap_or("normal");
            problems.push(format!(
                "`{}` lists `{}` as a {kind} dependency: only `{STORE}` depends on the SDK",
                package.name, dep.name
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "the SDK boundary check found {} problem(s) (RFC 0001, \"The dependency on NoCentralDB\"):\n  {}",
        problems.len(),
        problems.join("\n  ")
    )
}

fn is_sdk(name: &str) -> bool {
    name == SDK || name.starts_with("ncdb-")
}

#[cfg(test)]
#[path = "sdk_boundary_tests.rs"]
mod tests;
