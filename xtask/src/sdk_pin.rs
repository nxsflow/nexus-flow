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
    let doc: toml_edit::DocumentMut = text.parse().context("Cargo.toml is not valid TOML")?;
    let Some(entry) = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.get(SDK))
    else {
        bail!("`[workspace.dependencies]` names no `{SDK}`: the SDK is pinned there");
    };
    let Some(table) = entry.as_table_like() else {
        bail!("`{SDK}` is a bare version: pin it as `{{ git = \"…\", rev = \"<full commit hash>\" }}`");
    };
    if table.get("git").is_none() {
        bail!("`{SDK}` has no `git` source: pin it as `{{ git = \"…\", rev = \"<full commit hash>\" }}`");
    }
    for key in ["branch", "tag", "version", "path"] {
        if table.get(key).is_some() {
            bail!("`{SDK}` names a `{key}`: the pin is a `rev` and nothing else");
        }
    }
    let Some(rev) = table.get("rev").and_then(|r| r.as_str()) else {
        bail!("`{SDK}` has no `rev`: pin it at a full commit hash");
    };
    let full = rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !full {
        bail!("`{SDK}` pins `rev = \"{rev}\"`: a full commit hash is 40 lowercase hex digits");
    }
    Ok(rev.to_string())
}

#[cfg(test)]
#[path = "sdk_pin_tests.rs"]
mod tests;
