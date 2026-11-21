//! The pin of NoCentralDB's Rust SDK (NEX-88, RFC 0001 "The dependency on NoCentralDB", as the
//! owner answered NEX-211): the workspace `Cargo.toml` names the SDK, and any other NoCentralDB
//! crate it names (`ncdb-*`), from crates.io at an exact version, `"=<major>.<minor>.<patch>"`,
//! never a range, a git source, a path or another registry. The pin moves one pull request at a
//! time, and this check fails on any other form.

use anyhow::{bail, Context, Result};

/// The SDK's package name, as the workspace `Cargo.toml` names it.
pub const SDK: &str = "nocentraldb";

/// Check the NoCentralDB entries in `[workspace.dependencies]` of the workspace manifest `text`,
/// and return the SDK's version requirement.
pub fn check(text: &str) -> Result<String> {
    let _ = text;
    todo!()
}

#[cfg(test)]
#[path = "sdk_pin_tests.rs"]
mod tests;
