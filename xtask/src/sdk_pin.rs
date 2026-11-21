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
    let doc: toml_edit::DocumentMut = text.parse().context("Cargo.toml is not valid TOML")?;
    let deps = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table_like());
    let Some(deps) = deps else {
        bail!("Cargo.toml has no `[workspace.dependencies]`: the SDK `{SDK}` is pinned there");
    };
    let mut sdk = None;
    for (name, entry) in deps.iter() {
        if name == SDK || name.starts_with("ncdb-") {
            let req = exact_version(name, entry)?;
            if name == SDK {
                sdk = Some(req);
            }
        }
    }
    sdk.with_context(|| {
        format!("`[workspace.dependencies]` names no `{SDK}`: the SDK is pinned there")
    })
}

/// The version requirement of one NoCentralDB entry, when it is an exact crates.io version.
fn exact_version(name: &str, entry: &toml_edit::Item) -> Result<String> {
    let req = match entry.as_table_like() {
        None => entry.as_str(),
        Some(table) => {
            for key in [
                "git",
                "branch",
                "tag",
                "rev",
                "path",
                "registry",
                "registry-index",
            ] {
                if table.contains_key(key) {
                    bail!("`{name}` names a `{key}`: it is pinned from crates.io, at an exact version");
                }
            }
            table.get("version").and_then(|v| v.as_str())
        }
    };
    let Some(req) = req else {
        bail!("`{name}` has no version: pin it as `\"=<major>.<minor>.<patch>\"`");
    };
    if !is_exact(req) {
        bail!("`{name}` requires `{req}`: pin it at one exact version, `\"=<major>.<minor>.<patch>\"`");
    }
    Ok(req.to_string())
}

/// `=MAJOR.MINOR.PATCH`, with an optional `-pre` and `+build`, and nothing else.
fn is_exact(req: &str) -> bool {
    let Some(version) = req.strip_prefix('=') else {
        return false;
    };
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let numbers: Vec<&str> = core.split('.').collect();
    let suffix = &version[core.len()..];
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        && suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'+' | b'.'))
}

#[cfg(test)]
#[path = "sdk_pin_tests.rs"]
mod tests;
