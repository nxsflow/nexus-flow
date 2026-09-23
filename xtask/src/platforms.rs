//! `cargo xtask platforms check` (nexus-flow-4sr): enforce a single source of truth for the
//! four shipped platform keys.
//!
//! `release/platforms` is canonical. The key set was duplicated across the installer, the
//! publish/promote scripts, the release-workflow build matrix, and the npx runner shim — each
//! independently maintained. This check reads the canonical file and asserts every copy agrees:
//! the literal-list consumers (bash `PLATFORMS=(…)` arrays, the workflow matrix + `for p in …`
//! loops) must equal the set exactly; the componentwise consumers (install.sh's `platform_for`,
//! the shim's `osFor`/`archFor`) must map every canonical os/arch token. Drift fails CI
//! fail-closed.
//!
//! ONE CONSUMER USED TO BE CHECKED HERE AND IS NOT ANY MORE (nexus-flow-6j6v.sgfr): the updater
//! Lambda's `normalizeOs`/`normalizeArch`, read out of `infra/lambda/updater/decide.ts`. That
//! tree left this repo with the delivery move, and the copy this check was reading had not been
//! the SHIPPED one since — so the assertion had become a green tick on a file nobody deploys,
//! which is worse than no assertion: it read on every pull request as if the shipped platform
//! set were guarded. The shipped updater now lives in `nxsflow-landing-page`, outside anything
//! this repo can see. `release/platforms`' own header says so.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// Parse the canonical `release/platforms` body: non-blank, non-comment lines, trimmed.
pub fn parse_keys(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Split a `<os>-<arch>` key on its first `-` (os never contains `-`; arch may contain `_`).
pub fn split_os_arch(key: &str) -> Option<(String, String)> {
    key.split_once('-')
        .filter(|(os, arch)| !os.is_empty() && !arch.is_empty())
        .map(|(os, arch)| (os.to_string(), arch.to_string()))
}

/// Extract a bash `NAME=(a b c)` array's elements (single-line form, as used by the scripts).
pub fn bash_array(contents: &str, name: &str) -> Option<Vec<String>> {
    let needle = format!("{name}=(");
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(&needle) {
            let inner = rest.split(')').next().unwrap_or("");
            return Some(inner.split_whitespace().map(str::to_string).collect());
        }
    }
    None
}

/// All `platform: <key>` values from a YAML build matrix (the values, in document order).
pub fn yaml_matrix_platforms(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("platform:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Every `for p in <space-separated keys>; do` loop's key list found in a shell/YAML body. A
/// loop iterating an array indirection (`"${ARR[@]}"`) carries no literal list and is skipped.
pub fn for_in_lists(contents: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("for ") else {
            continue;
        };
        // `for <var> in <list>; do`
        let Some((_var, after_in)) = rest.split_once(" in ") else {
            continue;
        };
        let list = after_in
            .split(';')
            .next()
            .unwrap_or("")
            .split("do")
            .next()
            .unwrap_or("");
        if list.contains('$') || list.contains('"') {
            continue; // an indirection / quoted expansion, not a literal key list
        }
        let keys: Vec<String> = list.split_whitespace().map(str::to_string).collect();
        if !keys.is_empty() {
            out.push(keys);
        }
    }
    out
}

// ---- orchestration (reads the real repo files) --------------------------------------------

/// The canonical key set + the os/arch token sets derived from it.
struct Canonical {
    keys: BTreeSet<String>,
    oses: BTreeSet<String>,
    arches: BTreeSet<String>,
}

fn load_canonical(root: &Path) -> Result<Canonical> {
    let path = root.join("release/platforms");
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("reading canonical platform list {}", path.display()))?;
    let keys: Vec<String> = parse_keys(&body);
    if keys.is_empty() {
        bail!("{} lists no platforms", path.display());
    }
    let mut oses = BTreeSet::new();
    let mut arches = BTreeSet::new();
    for k in &keys {
        let (os, arch) =
            split_os_arch(k).ok_or_else(|| anyhow!("canonical key '{k}' is not <os>-<arch>"))?;
        oses.insert(os);
        arches.insert(arch);
    }
    Ok(Canonical {
        keys: keys.into_iter().collect(),
        oses,
        arches,
    })
}

fn read(root: &Path, rel: &str) -> Result<String> {
    let path = root.join(rel);
    std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

fn set_of(items: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    items.into_iter().collect()
}

/// Run the full single-source check against the repo at `root`. Returns an error naming the
/// first consumer that disagrees with `release/platforms`.
pub fn check(root: &Path) -> Result<()> {
    let canon = load_canonical(root)?;

    // 1. Literal bash arrays in the publish/promote scripts.
    for script in [
        ".github/scripts/publish-release.sh",
        ".github/scripts/promote-release.sh",
    ] {
        let body = read(root, script)?;
        let arr = bash_array(&body, "PLATFORMS")
            .ok_or_else(|| anyhow!("{script}: no `PLATFORMS=(…)` array found"))?;
        if set_of(arr) != canon.keys {
            bail!("{script}: PLATFORMS array disagrees with release/platforms");
        }
    }

    // 2. The release workflow: build matrix `platform:` values AND every `for p in …` loop.
    let release_yml = read(root, ".github/workflows/release.yml")?;
    let matrix = yaml_matrix_platforms(&release_yml);
    if matrix.is_empty() {
        bail!(".github/workflows/release.yml: no `platform:` matrix entries found");
    }
    if set_of(matrix) != canon.keys {
        bail!(
            ".github/workflows/release.yml: build-matrix platforms disagree with release/platforms"
        );
    }
    let mut saw_platform_loop = false;
    for list in for_in_lists(&release_yml) {
        let set = set_of(list);
        // Only loops that reference platform keys are platform loops; `for doc in LICENSE …`
        // and the like carry their own literals and are not our concern.
        if set.is_disjoint(&canon.keys) {
            continue;
        }
        saw_platform_loop = true;
        if set != canon.keys {
            bail!(".github/workflows/release.yml: a `for p in …` platform loop disagrees with release/platforms");
        }
    }
    if !saw_platform_loop {
        bail!(".github/workflows/release.yml: expected a literal `for p in …` platform loop");
    }

    // 3. Componentwise consumers must map every canonical os/arch token.
    let install_sh = read(root, "install.sh")?;
    for os in &canon.oses {
        if !install_sh.contains(&format!("_pf_os=\"{os}\"")) {
            bail!("install.sh: platform_for does not map the canonical os '{os}'");
        }
    }
    for arch in &canon.arches {
        if !install_sh.contains(&format!("_pf_arch=\"{arch}\"")) {
            bail!("install.sh: platform_for does not map the canonical arch '{arch}'");
        }
    }

    // The npx runner shim (npm/mcp) maps the same os/arch tokens in lib/platform.js's osFor/archFor
    // (mirrors install.sh's platform_for) — a componentwise consumer, guarded like the others.
    let shim = read(root, "npm/mcp/lib/platform.js")?;
    for os in &canon.oses {
        if !shim.contains(&format!("return '{os}';")) {
            bail!("npm/mcp/lib/platform.js: osFor does not map the canonical os '{os}'");
        }
    }
    for arch in &canon.arches {
        if !shim.contains(&format!("return '{arch}';")) {
            bail!("npm/mcp/lib/platform.js: archFor does not map the canonical arch '{arch}'");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, derived from this crate's manifest dir (`<root>/xtask`).
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// Write a minimal but COMPLETE fixture repo where every consumer lists exactly `keys`.
    fn write_fixture(root: &Path, keys: &[&str]) {
        let w = |rel: &str, body: String| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let arr = keys.join(" ");
        w(
            "release/platforms",
            format!("# canon\n{}\n", keys.join("\n")),
        );
        w(
            ".github/scripts/publish-release.sh",
            format!("#!/bin/sh\nPLATFORMS=({arr})\n"),
        );
        w(
            ".github/scripts/promote-release.sh",
            format!("#!/bin/sh\nPLATFORMS=({arr})\n"),
        );
        let matrix: String = keys
            .iter()
            .map(|k| format!("          - target: t\n            platform: {k}\n"))
            .collect();
        w(
            ".github/workflows/release.yml",
            format!("jobs:\n    strategy:\n      matrix:\n        include:\n{matrix}      steps:\n        - run: |\n            for p in {arr}; do echo $p; done\n"),
        );
        // Componentwise consumers map every os/arch token the keys decompose into.
        let oses: BTreeSet<_> = keys.iter().map(|k| split_os_arch(k).unwrap().0).collect();
        let arches: BTreeSet<_> = keys.iter().map(|k| split_os_arch(k).unwrap().1).collect();
        let install: String = oses
            .iter()
            .map(|o| format!("    _pf_os=\"{o}\"\n"))
            .chain(arches.iter().map(|a| format!("    _pf_arch=\"{a}\"\n")))
            .collect();
        w("install.sh", format!("#!/bin/sh\n{install}"));
        // The npx shim's componentwise consumer (osFor/archFor use single-quoted returns).
        let shim: String = oses
            .iter()
            .map(|o| format!("      return '{o}';\n"))
            .chain(arches.iter().map(|a| format!("      return '{a}';\n")))
            .collect();
        w("npm/mcp/lib/platform.js", shim);
    }

    #[test]
    fn drift_in_any_consumer_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let four = [
            "darwin-aarch64",
            "darwin-x86_64",
            "linux-x86_64",
            "linux-aarch64",
        ];
        // A fully consistent fixture passes.
        write_fixture(dir.path(), &four);
        check(dir.path()).expect("consistent fixture passes");

        // Add a 5th platform to ONLY the canonical file ⇒ every literal-list consumer now drifts.
        let mut five = four.to_vec();
        five.push("linux-riscv64");
        std::fs::write(
            dir.path().join("release/platforms"),
            format!("{}\n", five.join("\n")),
        )
        .unwrap();
        assert!(
            check(dir.path()).is_err(),
            "a platform present only in release/platforms must fail the check"
        );
    }

    #[test]
    fn every_consumer_agrees_with_the_canonical_list() {
        // The live single-source assertion: install.sh, the publish/promote scripts, the release
        // workflow matrix + loops, and the npx shim all match release/platforms. If this fails, a
        // platform was added/renamed without updating every consumer — exactly what 4sr prevents.
        check(&repo_root()).expect("all consumers agree with release/platforms");
    }

    #[test]
    fn parse_keys_drops_comments_and_blanks() {
        let body = "# header\n\ndarwin-aarch64\n  linux-x86_64  \n\n# trailing\n";
        assert_eq!(parse_keys(body), vec!["darwin-aarch64", "linux-x86_64"]);
    }

    #[test]
    fn split_os_arch_splits_on_the_first_dash() {
        assert_eq!(
            split_os_arch("linux-x86_64"),
            Some(("linux".to_string(), "x86_64".to_string()))
        );
        assert_eq!(
            split_os_arch("darwin-aarch64"),
            Some(("darwin".to_string(), "aarch64".to_string()))
        );
        assert_eq!(split_os_arch("nodash"), None);
    }

    #[test]
    fn bash_array_extracts_single_line_elements() {
        let body =
            "X=1\nPLATFORMS=(darwin-aarch64 darwin-x86_64 linux-x86_64 linux-aarch64)\nY=2\n";
        assert_eq!(
            bash_array(body, "PLATFORMS"),
            Some(vec![
                "darwin-aarch64".to_string(),
                "darwin-x86_64".to_string(),
                "linux-x86_64".to_string(),
                "linux-aarch64".to_string(),
            ])
        );
        assert_eq!(bash_array(body, "MISSING"), None);
    }

    #[test]
    fn yaml_matrix_platforms_collects_values() {
        let body = "        - target: a\n          platform: darwin-aarch64\n        - target: b\n          platform: linux-x86_64\n";
        assert_eq!(
            yaml_matrix_platforms(body),
            vec!["darwin-aarch64".to_string(), "linux-x86_64".to_string()]
        );
    }

    #[test]
    fn for_in_lists_extracts_each_loop_list() {
        let body = "          for p in darwin-aarch64 darwin-x86_64 linux-x86_64 linux-aarch64; do\n            echo $p\n          done\n";
        assert_eq!(
            for_in_lists(body),
            vec![vec![
                "darwin-aarch64".to_string(),
                "darwin-x86_64".to_string(),
                "linux-x86_64".to_string(),
                "linux-aarch64".to_string(),
            ]]
        );
        // A `for x in "${ARR[@]}"` indirection is not a literal list ⇒ ignored.
        assert!(for_in_lists("for p in \"${PLATFORMS[@]}\"; do\n").is_empty());
    }
}
