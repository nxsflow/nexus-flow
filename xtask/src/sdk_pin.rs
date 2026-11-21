//! The pin of NoCentralDB's Rust SDK (NEX-88, RFC 0001 "The dependency on NoCentralDB"): the
//! workspace `Cargo.toml` names the SDK as a git dependency at a full commit hash,
//! `rev = "<40 hex digits>"`, never a branch, a tag, a short hash or a registry version. The pin
//! moves one pull request at a time, and this check fails on any other form.

use anyhow::{bail, Context, Result};

/// The SDK's package name, as the workspace `Cargo.toml` names it.
pub const SDK: &str = "nocentraldb";

/// Check the SDK's entry in `[workspace.dependencies]` of the workspace manifest `text`, and
/// return the pinned commit hash.
pub fn check(text: &str) -> Result<String> {
    let _ = text;
    todo!()
}

#[cfg(test)]
#[path = "sdk_pin_tests.rs"]
mod tests;
