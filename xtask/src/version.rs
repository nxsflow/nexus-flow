//! `version set` / `version check`: one workspace version, the tag is the truth.
//!
//! The shipped version lives once in the root `[workspace.package] version`; product
//! crates inherit it via `version.workspace = true`. This module bumps that source and the
//! matching `Cargo.lock` entries atomically (`set`), and verifies they agree fail-closed —
//! plus `tag == version` on release tags in CI (`check`). Spec §4.1–§4.3.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Validate a plain SemVer `X.Y.Z`. Suffixes (`-beta.1`, `+build`, `v` prefix) are rejected
/// loudly: the release channel is a ring, not a part of the version (spec §4.3).
pub fn parse_plain_semver(s: &str) -> Result<()> {
    semver_components(s).map(|_| ())
}

/// Parse a plain SemVer `X.Y.Z` into its numeric `(major, minor, patch)`. This is the single
/// validation path: exactly three components, each a non-empty run of digits with no leading
/// zero (rejecting `-beta`, `+build`, `x`, empty, `01`) **and** in `u64` range (so a
/// pathological 20-digit component is a loud error here, never a panic in an ordering path).
pub(crate) fn semver_components(s: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        bail!("not plain SemVer X.Y.Z: {s:?} (channel suffixes like -beta.1 are rejected)");
    }
    let mut nums = [0u64; 3];
    for (slot, part) in nums.iter_mut().zip(parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            bail!("not plain SemVer X.Y.Z: {s:?}");
        }
        if part.len() > 1 && part.starts_with('0') {
            bail!("SemVer component has a leading zero: {s:?}");
        }
        *slot = part
            .parse::<u64>()
            .with_context(|| format!("SemVer component out of u64 range in {s:?}"))?;
    }
    Ok((nums[0], nums[1], nums[2]))
}

/// Options for [`check`].
#[derive(Debug, Default)]
pub struct CheckOpts {
    /// When set (CI release-tag mode), enforce `tag == v<workspace-version>`. A malformed
    /// tag is an error, never a skip (fail-closed).
    pub tag: Option<String>,
}

/// `version set <v>`: write the workspace version + every inheriting crate's `Cargo.lock`
/// entry atomically. Rejects non-plain-SemVer input before touching any file.
pub fn set(root: &Path, new_version: &str) -> Result<()> {
    parse_plain_semver(new_version)
        .with_context(|| format!("refusing to set version {new_version:?}"))?;

    // Edit both documents in memory and write them only after both parse, so a parse error
    // can't leave a half-bump. A crash strictly *between* the two atomic renames can still
    // leave Cargo.toml ahead of Cargo.lock — that residual window is the reason `version
    // check` exists and runs fail-closed in CI: it detects exactly this drift.
    let root_path = root.join("Cargo.toml");
    let lock_path = root.join("Cargo.lock");
    let mut root_doc = read_doc(&root_path)?;
    root_doc["workspace"]["package"]["version"] = toml_edit::value(new_version);

    let inheriting = inheriting_member_names(root)?;
    let mut lock_doc = read_doc(&lock_path)?;
    let packages = lock_doc
        .get_mut("package")
        .and_then(|p| p.as_array_of_tables_mut())
        .context("Cargo.lock has no [[package]] entries")?;
    for pkg in packages.iter_mut() {
        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        if inheriting.iter().any(|m| m == name) {
            pkg["version"] = toml_edit::value(new_version);
        }
    }

    write_atomic(&root_path, &root_doc.to_string())?;
    write_atomic(&lock_path, &lock_doc.to_string())?;
    Ok(())
}

/// `version check`: workspace version is plain SemVer, and every inheriting crate's
/// `Cargo.lock` entry equals it. In tag mode additionally enforce `tag == v<version>`.
pub fn check(root: &Path, opts: &CheckOpts) -> Result<()> {
    let version = workspace_version(root)?;
    parse_plain_semver(&version)
        .with_context(|| format!("workspace version {version:?} is not plain SemVer"))?;

    let inheriting = inheriting_member_names(root)?;
    let lock_versions = lock_versions(root)?;
    for name in &inheriting {
        match lock_versions.get(name) {
            Some(locked) if locked == &version => {}
            Some(locked) => bail!(
                "version drift: {name} is {locked} in Cargo.lock but the workspace version is \
                 {version} — run `cargo xtask version set {version}`"
            ),
            None => bail!("{name} inherits the workspace version but has no Cargo.lock entry"),
        }
    }

    if let Some(tag) = &opts.tag {
        let expected = format!("v{version}");
        let tag_version = tag
            .strip_prefix('v')
            .with_context(|| format!("release tag {tag:?} is malformed (expected v<version>)"))?;
        parse_plain_semver(tag_version)
            .with_context(|| format!("release tag {tag:?} is not v<plain-SemVer>"))?;
        if tag != &expected {
            bail!("tag {tag} does not match workspace version (expected {expected})");
        }
    }
    Ok(())
}

/// Read `[workspace.package] version` from the root `Cargo.toml`.
pub fn workspace_version(root: &Path) -> Result<String> {
    let doc = read_doc(&root.join("Cargo.toml"))?;
    doc["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_owned)
        .context("root Cargo.toml has no [workspace.package] version")
}

/// Names of workspace members whose `[package] version` is `version.workspace = true`
/// (i.e. they inherit the workspace version, so their `Cargo.lock` entry must match it).
fn inheriting_member_names(root: &Path) -> Result<Vec<String>> {
    let root_doc = read_doc(&root.join("Cargo.toml"))?;
    let members = root_doc["workspace"]["members"]
        .as_array()
        .context("root Cargo.toml has no [workspace] members array")?;

    let mut names = Vec::new();
    for member in members.iter() {
        let rel = member
            .as_str()
            .context("workspace member is not a string")?;
        let member_path = root.join(rel).join("Cargo.toml");
        let doc = read_doc(&member_path)?;
        let pkg = &doc["package"];
        // Inherited iff `version` is the inline `{ workspace = true }`, not a literal string.
        let inherits = pkg["version"].get("workspace").and_then(|w| w.as_bool()) == Some(true);
        if inherits {
            let name = pkg["name"]
                .as_str()
                .with_context(|| format!("{rel}/Cargo.toml has no package name"))?;
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// Map of package name → version from `Cargo.lock`.
fn lock_versions(root: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    let doc = read_doc(&root.join("Cargo.lock"))?;
    let mut map = std::collections::BTreeMap::new();
    if let Some(pkgs) = doc.get("package").and_then(|p| p.as_array_of_tables()) {
        for pkg in pkgs.iter() {
            if let (Some(name), Some(version)) = (
                pkg.get("name").and_then(|n| n.as_str()),
                pkg.get("version").and_then(|v| v.as_str()),
            ) {
                map.insert(name.to_owned(), version.to_owned());
            }
        }
    }
    Ok(map)
}

fn read_doc(path: &Path) -> Result<toml_edit::DocumentMut> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parsing {}", path.display()))
}

/// Write via a sibling temp file + atomic rename, so a crash never leaves a half-written
/// manifest. The temp lives in the same directory as the target so the rename stays on one
/// filesystem.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("toml")
    ));
    fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Build a throwaway workspace: root `Cargo.toml` with `version`, two inheriting member
    /// crates, one pinned (non-inheriting) member, and a matching `Cargo.lock`.
    fn scaffold(version: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "[workspace]\nresolver = \"2\"\nmembers = [\"crates/core\", \"crates/cli\", \"xtask\"]\n\n[workspace.package]\nversion = \"{version}\"\n"
            ),
        )
        .unwrap();
        for name in ["core", "cli"] {
            let cdir = root.join("crates").join(name);
            fs::create_dir_all(&cdir).unwrap();
            fs::write(
                cdir.join("Cargo.toml"),
                format!("[package]\nname = \"nexus-flow-{name}\"\nversion.workspace = true\nedition = \"2021\"\n"),
            )
            .unwrap();
        }
        // xtask pins its own version — must NOT be treated as inheriting.
        let xdir = root.join("xtask");
        fs::create_dir_all(&xdir).unwrap();
        fs::write(
            xdir.join("Cargo.toml"),
            "[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            format!(
                "version = 3\n\n[[package]]\nname = \"nexus-flow-core\"\nversion = \"{version}\"\n\n[[package]]\nname = \"nexus-flow-cli\"\nversion = \"{version}\"\n\n[[package]]\nname = \"xtask\"\nversion = \"0.0.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n"
            ),
        )
        .unwrap();
        (dir, root)
    }

    #[test]
    fn plain_semver_accepts_xyz() {
        assert!(parse_plain_semver("0.2.0").is_ok());
        assert!(parse_plain_semver("12.34.56").is_ok());
    }

    #[test]
    fn plain_semver_rejects_suffixes_and_garbage() {
        for bad in [
            "0.2.0-beta.1",
            "v0.2.0",
            "0.2",
            "1.2.3.4",
            "0.2.0+build",
            "1.2.x",
            "",
            "01.2.3",
        ] {
            assert!(parse_plain_semver(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn plain_semver_rejects_u64_overflow() {
        // 20 digits overflows u64: must be a loud error, never a panic in the ordering path.
        assert!(parse_plain_semver("99999999999999999999.0.0").is_err());
        // The component-parsing entry point agrees.
        assert!(semver_components("0.0.99999999999999999999").is_err());
        assert_eq!(semver_components("1.2.3").unwrap(), (1, 2, 3));
    }

    #[test]
    fn workspace_version_reads_root() {
        let (_d, root) = scaffold("0.1.0");
        assert_eq!(workspace_version(&root).unwrap(), "0.1.0");
    }

    #[test]
    fn inheriting_members_excludes_pinned_xtask() {
        let (_d, root) = scaffold("0.1.0");
        let mut names = inheriting_member_names(&root).unwrap();
        names.sort();
        assert_eq!(names, vec!["nexus-flow-cli", "nexus-flow-core"]);
    }

    #[test]
    fn check_passes_when_consistent() {
        let (_d, root) = scaffold("0.1.0");
        check(&root, &CheckOpts::default()).unwrap();
    }

    #[test]
    fn check_detects_lock_drift() {
        let (_d, root) = scaffold("0.1.0");
        // Bump root only — Cargo.lock still says 0.1.0.
        set_root_version_only(&root, "0.2.0");
        let err = check(&root, &CheckOpts::default()).unwrap_err().to_string();
        assert!(
            err.contains("nexus-flow-core"),
            "drift error names the crate: {err}"
        );
    }

    #[test]
    fn set_bumps_root_and_lock() {
        let (_d, root) = scaffold("0.1.0");
        set(&root, "0.2.0").unwrap();
        assert_eq!(workspace_version(&root).unwrap(), "0.2.0");
        let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"nexus-flow-core\"\nversion = \"0.2.0\""));
        assert!(lock.contains("name = \"nexus-flow-cli\"\nversion = \"0.2.0\""));
        // xtask + third-party deps untouched.
        assert!(lock.contains("name = \"xtask\"\nversion = \"0.0.0\""));
        assert!(lock.contains("name = \"serde\"\nversion = \"1.0.0\""));
        check(&root, &CheckOpts::default()).unwrap();
    }

    #[test]
    fn set_rejects_suffix_before_touching_files() {
        let (_d, root) = scaffold("0.1.0");
        assert!(set(&root, "0.2.0-beta.1").is_err());
        // Nothing changed.
        assert_eq!(workspace_version(&root).unwrap(), "0.1.0");
    }

    #[test]
    fn check_tag_mode_matches_version() {
        let (_d, root) = scaffold("0.2.0");
        check(
            &root,
            &CheckOpts {
                tag: Some("v0.2.0".into()),
            },
        )
        .unwrap();
    }

    #[test]
    fn check_tag_mode_rejects_mismatch() {
        let (_d, root) = scaffold("0.2.0");
        assert!(check(
            &root,
            &CheckOpts {
                tag: Some("v0.3.0".into())
            }
        )
        .is_err());
    }

    #[test]
    fn check_tag_mode_rejects_malformed_tag() {
        let (_d, root) = scaffold("0.2.0");
        // No `v` prefix, and a suffix — malformed must error, not skip.
        assert!(check(
            &root,
            &CheckOpts {
                tag: Some("0.2.0".into())
            }
        )
        .is_err());
        assert!(check(
            &root,
            &CheckOpts {
                tag: Some("v0.2.0-beta".into())
            }
        )
        .is_err());
    }

    /// Test helper: bump only the root workspace version (simulates drift vs Cargo.lock).
    fn set_root_version_only(root: &Path, v: &str) {
        let p = root.join("Cargo.toml");
        let txt = fs::read_to_string(&p).unwrap();
        let mut doc: toml_edit::DocumentMut = txt.parse().unwrap();
        doc["workspace"]["package"]["version"] = toml_edit::value(v);
        fs::write(&p, doc.to_string()).unwrap();
    }
}
