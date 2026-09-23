//! Facade SemVer gate (nexus-flow-aye.14): make the §4.3 facade-API contract MACHINE-enforced.
//! `cargo-semver-checks` diffs the public API of the CONSUMED LIBRARY SURFACES against a baseline
//! release tag and classifies any change; on a PATCH bump a minor/major-worthy change fails the
//! build (fail-closed) — exactly the "a patch never breaks the facade" promise manufakt
//! fast-tracks on.
//!
//! Division of labour: `cargo-semver-checks` owns the API diff + the 0.x/≥1.0 adequacy verdict
//! (it knows minor is the break axis in 0.x). What lives HERE — and is unit-tested — is the
//! policy glue: pick the baseline tag and decide whether the run is fail-closed (a real forward
//! bump, so the tool's adequacy verdict is authoritative) or report-only (no version bump yet, so
//! the API may legitimately be ahead of the last tag — surface drift without failing the PR).
//!
//! **Baseline (documented):** the highest release tag `vX.Y.Z` whose version is ≤ the current
//! workspace version — i.e. the last release we promised compatibility against. The bump axis is
//! that tag → the current version. See `select_baseline`.
//!
//! **Trigger:** runs on PRs, `main`, and `v*` tags. On an ordinary feature PR the workspace
//! version equals the last tag (axis `None`) → report-only; the fail-closed gate fires on the
//! version-bump (release-prep) PR and on the tag, where the bump axis is known — i.e. it is
//! coupled to the bump axis, the changelog-side mirror being
//! `changelog::enforce_facade_breaking_axis` (nexus-flow-aye.15).
//!
//! **musl / no-network:** the baseline is a LOCAL git tag (`--baseline-rev`), so no network and
//! no crates.io are needed at check time (`publish = false` / §14 TB-8); CI only needs network to
//! install the tool itself.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::version::{semver_components, workspace_version};

/// One consumed library surface: the cargo package the SemVer gate diffs, and the source directory
/// a PR diff touches when it changes that surface. The two live together because they are two
/// spellings of one fact — which code is a shipped library API — and the two gates that need them
/// (`semver_check` here, `changelog::check`'s explicit-verdict rule) must never disagree about the
/// membership of that set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumedSurface {
    /// Cargo package name, as passed to `cargo semver-checks --package`.
    pub package: &'static str,
    /// Repo-relative source directory, WITH a trailing slash so `crates/chat/` cannot match a
    /// sibling like `crates/chat-extra/…`.
    pub dir: &'static str,
}

/// Every library surface a consumer pins by git tag and compiles against (spec §4.4) — the ONE
/// list, so the gate's reach and the changelog rule's reach are the same set by construction.
///
/// All three are pinned in lockstep and all three are public library API:
///
/// ```text
/// app-foundations/Cargo.toml
///   nexus-flow-facade = { …, tag = "v0.42.0" }
///   nexus-chat        = { …, tag = "v0.42.0", default-features = false }
///   nexus-memory      = { …, tag = "v0.42.0" }
/// ```
///
/// Guarding only `crates/facade` was the gap `6j6v.a7ce` closes: on 2026-08-03 the COMPILER (not
/// the release notes) found that `nexus_chat::orchestration::WorkflowStartRequest` had gained
/// fields in v0.41.0 — a break for anyone constructing it — because no gate was watching that
/// package.
pub const CONSUMED_SURFACES: &[ConsumedSurface] = &[
    ConsumedSurface {
        package: "nexus-flow-facade",
        dir: "crates/facade/",
    },
    ConsumedSurface {
        package: "nexus-chat",
        dir: "crates/chat/",
    },
    ConsumedSurface {
        package: "nexus-memory",
        dir: "crates/memory/",
    },
];

/// Directories OUTSIDE the consumed surfaces whose BEHAVIOUR is nonetheless part of what those
/// surfaces promise, so a diff touching one obliges an explicit facade verdict just as a surface
/// does (`6j6v.gmjd`). Trailing slash, matched as a prefix — same rule as [`ConsumedSurface::dir`].
///
/// These are deliberately NOT [`CONSUMED_SURFACES`]: nobody pins `nxs-foundation`, so
/// `cargo-semver-checks` has no business gating its API. What flows outward is its behaviour.
/// `crates/foundation/src/model.rs` holds `validate_author`/`is_attributable`, which every one of
/// the 26 public write entry points across the three surfaces delegates to; tightening it is, BY
/// CONSTRUCTION, a behavioural break at all 26 — and it happened twice in one day (PR #311, PR
/// #314) with every gate green.
///
/// The whole crate, not just that one file (PR #315 review, Code Quality #2): `store.rs` carries
/// the append-time attributability assert, `error.rs` the error kinds those entry points return,
/// `schema.rs`/`workspace.rs` the substrate they all run on. Naming a single file would repeat in
/// miniature the very mistake `6j6v.a7ce` exists to correct — a guard built for the first case and
/// never pulled through to its siblings. It costs one line of frontmatter on the ~4% of commits
/// that touch this crate.
pub const CONTRACT_CRITICAL_DIRS: &[&str] = &["crates/foundation/"];

/// Whether a repo-relative changed path belongs to a consumed library surface (or to one of the
/// [`CONTRACT_CRITICAL_DIRS`] outside them) — the membership test the changelog gate's
/// explicit-verdict rule keys on.
pub fn touches_consumed_surface(path: &str) -> bool {
    CONSUMED_SURFACES.iter().any(|s| path.starts_with(s.dir))
        || CONTRACT_CRITICAL_DIRS.iter().any(|d| path.starts_with(d))
}

/// The full `cargo …` argv for the gate run: one `--package` per consumed surface against the
/// baseline tag. Pure and unit-tested, so the fan-out over [`CONSUMED_SURFACES`] cannot quietly
/// lose a package in a refactor — the whole point of the widened reach is that it is a SET (PR
/// #315 review, Test Quality #1).
///
/// `--baseline-rev` compares against a LOCAL git rev — no network / crates.io needed
/// (`publish = false`, §14 TB-8). cargo-semver-checks owns the API diff + 0.x/≥1.0 adequacy.
fn semver_checks_args(baseline_tag: &str) -> Vec<String> {
    let mut args = vec!["semver-checks".to_string(), "check-release".to_string()];
    for surface in CONSUMED_SURFACES {
        args.push("--package".to_string());
        args.push(surface.package.to_string());
    }
    args.push("--baseline-rev".to_string());
    args.push(baseline_tag.to_string());
    args
}

/// The guarded packages as one human-readable list, for log lines and error messages.
fn package_list() -> String {
    CONSUMED_SURFACES
        .iter()
        .map(|s| s.package)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The bump from the baseline tag to the current workspace version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpAxis {
    /// Current version equals the baseline tag (no forward bump yet) — report-only.
    None,
    Patch,
    Minor,
    Major,
}

impl BumpAxis {
    /// Whether the gate fails the build on a `cargo-semver-checks` failure. Only a PATCH bump is
    /// fail-closed — that is the "a patch never breaks the facade" guarantee (spec §4.3/§4.4,
    /// nexus-flow-aye.14). A Minor/Major bump is the break axis (breaks allowed, mirrored into the
    /// changelog marker, nexus-flow-aye.15); `None` means no version bump yet. All three run
    /// report-only — visible, never red.
    pub fn is_fail_closed(self) -> bool {
        matches!(self, BumpAxis::Patch)
    }

    fn label(self) -> &'static str {
        match self {
            BumpAxis::None => "none",
            BumpAxis::Patch => "patch",
            BumpAxis::Minor => "minor",
            BumpAxis::Major => "major",
        }
    }
}

/// The chosen comparison point: a release tag plus the bump axis from it to the current version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub tag: String,
    pub axis: BumpAxis,
}

/// Classify the bump from `prev` to `next` (both plain SemVer). Equal → `None`; otherwise the
/// highest-significance component that increased. A `next` strictly below `prev` is treated as
/// `None` (no forward bump — the gate does not police a downgrade).
pub fn classify(prev: &str, next: &str) -> Result<BumpAxis> {
    let (pmaj, pmin, ppatch) =
        semver_components(prev).with_context(|| format!("version {prev:?} is not plain SemVer"))?;
    let (nmaj, nmin, npatch) =
        semver_components(next).with_context(|| format!("version {next:?} is not plain SemVer"))?;
    Ok(if nmaj > pmaj {
        BumpAxis::Major
    } else if nmaj == pmaj && nmin > pmin {
        BumpAxis::Minor
    } else if nmaj == pmaj && nmin == pmin && npatch > ppatch {
        BumpAxis::Patch
    } else {
        BumpAxis::None
    })
}

/// Pick the baseline from a list of git tags and the current version: the highest tag
/// `v<plain-SemVer>` whose version is ≤ `current`, paired with the bump axis from it to `current`.
/// Tags that are not `v<plain-SemVer>` (e.g. a suffixed pre-release) are ignored. `None` when no
/// release tag is ≤ `current` (nothing to compare against yet).
pub fn select_baseline(tags: &[String], current: &str) -> Result<Option<Baseline>> {
    let cur_key = semver_components(current)
        .with_context(|| format!("current version {current:?} is not plain SemVer"))?;
    // Among tags `v<plain-SemVer>` with version <= current, pick the highest.
    let mut best: Option<(String, (u64, u64, u64))> = None;
    for tag in tags {
        let Some(rest) = tag.strip_prefix('v') else {
            continue;
        };
        let Ok(key) = semver_components(rest) else {
            continue; // non-plain-SemVer tag (e.g. a suffixed pre-release) — ignore
        };
        if key <= cur_key && best.as_ref().is_none_or(|(_, b)| key > *b) {
            best = Some((tag.clone(), key));
        }
    }
    match best {
        Some((tag, key)) => {
            let (maj, min, patch) = key;
            let axis = classify(&format!("{maj}.{min}.{patch}"), current)?;
            Ok(Some(Baseline { tag, axis }))
        }
        None => Ok(None),
    }
}

/// Decide the gate outcome from the bump axis and whether `cargo-semver-checks` passed. On a
/// fail-closed axis (a forward bump) the tool's verdict is authoritative: a patch with a break
/// (`tool_succeeded == false`) fails the build; a no-op patch passes. On a report-only axis (no
/// bump yet) a tool failure is surfaced but does not fail the build. This is the gate's core
/// patch-never-breaks decision (nexus-flow-aye.14 acceptance: green on a no-op patch, red on a
/// constructed `pub` break).
fn gate_outcome(axis: BumpAxis, tool_succeeded: bool) -> Result<()> {
    if axis.is_fail_closed() && !tool_succeeded {
        bail!(
            "cargo-semver-checks reported a public-API change in one of the consumed library \
             surfaces ({}) that the {} bump does not allow — on a patch these APIs may not break \
             (spec §4.3). Ship the change in a minor (the break axis) or revert the breaking edit.",
            package_list(),
            axis.label()
        );
    }
    Ok(())
}

/// All `v*` git tags (newline-separated, in git's default order; ordering is irrelevant —
/// `select_baseline` picks the maximum itself).
fn git_release_tags(root: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["tag", "--list", "v*"])
        .output()
        .context("running git tag --list")?;
    if !out.status.success() {
        bail!(
            "git tag --list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Whether `cargo-semver-checks` is installed (a clear, actionable error beats a cryptic
/// "no such command" mid-run).
fn tool_available() -> bool {
    Command::new("cargo")
        .args(["semver-checks", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `facade baseline`: print the resolved baseline tag + bump axis (the documented comparison
/// point), or `none` when there is no prior release tag. Used in CI logs and by humans.
pub fn print_baseline(root: &Path) -> Result<()> {
    let current = workspace_version(root)?;
    match select_baseline(&git_release_tags(root)?, &current)? {
        Some(b) => println!(
            "baseline {} · axis {} · current {current}",
            b.tag,
            b.axis.label()
        ),
        None => println!("baseline none · current {current} (no prior release tag at or below)"),
    }
    Ok(())
}

/// `facade semver-check`: the gate. Resolve the baseline release tag, run `cargo-semver-checks`
/// on every [`CONSUMED_SURFACES`] package against it, and apply the patch-never-breaks decision
/// (`gate_outcome`). Report-only when there is no version bump yet (the bump axis is undecided on
/// a feature PR). One invocation for all packages: they share the baseline checkout and the
/// rustdoc build, and a failure in any of them is the same verdict.
pub fn semver_check(root: &Path) -> Result<()> {
    let current = workspace_version(root)?;
    let Some(baseline) = select_baseline(&git_release_tags(root)?, &current)? else {
        println!(
            "facade semver-check: no prior release tag at or below {current} — nothing to diff \
             (first release on this line). Skipping."
        );
        return Ok(());
    };

    if !tool_available() {
        bail!(
            "cargo-semver-checks is not installed — install it with \
             `cargo install cargo-semver-checks` (CI uses taiki-e/install-action). It diffs the \
             public API of {} against the baseline tag.",
            package_list()
        );
    }

    let mode = if baseline.axis.is_fail_closed() {
        "fail-closed"
    } else {
        "report-only"
    };
    println!(
        "facade semver-check: {} vs {} ({} bump → {mode})",
        package_list(),
        baseline.tag,
        baseline.axis.label()
    );

    let status = Command::new("cargo")
        .current_dir(root)
        .args(semver_checks_args(&baseline.tag))
        .status()
        .context("running cargo semver-checks")?;

    if !baseline.axis.is_fail_closed() && !status.success() {
        let why = match baseline.axis {
            BumpAxis::None => {
                "the workspace version is not bumped past the baseline yet (the \
                               release-prep version-bump PR and the tag are where the \
                               patch-never-breaks gate fires fail-closed)"
            }
            _ => {
                "this is a minor/major bump — the break axis (§4.3), so the change is allowed \
                  here; flag it with a `facade: breaking` changelog fragment (nexus-flow-aye.15)"
            }
        };
        // Emit a GitHub Actions `::warning::` (not a bare line) so a report-only detection
        // SURFACES in the PR checks UI — a green check on a feature PR must not be mistaken for
        // "no facade break". The fail-closed gate still fires on the patch-prep PR / tag.
        println!(
            "::warning::facade semver-check (report-only): cargo-semver-checks found an API \
             change in one of {}, but {why}.",
            package_list()
        );
    }
    gate_outcome(baseline.axis, status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// The repo root (`xtask/`'s parent), for the tests that assert the surface list still points
    /// at real code.
    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent")
    }

    // ---- CONSUMED_SURFACES (the gate's reach, 6j6v.a7ce) ----

    #[test]
    fn the_gate_covers_all_three_pinned_library_surfaces() {
        // The reach IS the contract: a consumer pins these three in lockstep (spec §4.4), so a
        // surface missing here is a surface nothing watches — the 2026-08-03 WorkflowStartRequest
        // break got through exactly that way.
        let packages: Vec<&str> = CONSUMED_SURFACES.iter().map(|s| s.package).collect();
        assert_eq!(
            packages,
            vec!["nexus-flow-facade", "nexus-chat", "nexus-memory"]
        );
    }

    #[test]
    fn every_surface_dir_holds_the_package_it_claims() {
        // Guards against silent rot: a crate rename (dir or package) would otherwise leave the
        // gate pointed at nothing and green forever.
        for surface in CONSUMED_SURFACES {
            let manifest = repo_root().join(surface.dir).join("Cargo.toml");
            let text = std::fs::read_to_string(&manifest)
                .unwrap_or_else(|e| panic!("reading {}: {e}", manifest.display()));
            assert!(
                text.contains(&format!("name = \"{}\"", surface.package)),
                "{} does not declare package {}",
                manifest.display(),
                surface.package
            );
        }
    }

    #[test]
    fn every_contract_critical_dir_still_exists() {
        for dir in CONTRACT_CRITICAL_DIRS {
            assert!(
                repo_root().join(dir).is_dir(),
                "contract-critical dir {dir} no longer exists — the rule keyed on it is dead"
            );
        }
    }

    #[test]
    fn touches_consumed_surface_matches_each_surface_and_the_critical_dirs() {
        assert!(touches_consumed_surface("crates/facade/src/validate.rs"));
        assert!(touches_consumed_surface("crates/chat/src/facade.rs"));
        assert!(touches_consumed_surface("crates/memory/src/facade.rs"));
        assert!(touches_consumed_surface("crates/facade/Cargo.toml"));
        // The whole substrate crate, not only the one file the two incidents happened to touch
        // (PR #315 review, Code Quality #2): the append-time attributability assert, the error
        // kinds and the schema reach the surfaces' behaviour just as `validate_author` does.
        assert!(touches_consumed_surface("crates/foundation/src/model.rs"));
        assert!(touches_consumed_surface("crates/foundation/src/store.rs"));
        assert!(touches_consumed_surface("crates/foundation/src/error.rs"));
    }

    #[test]
    fn touches_consumed_surface_ignores_unguarded_paths() {
        // A sibling crate whose name merely PREFIXES a guarded dir must not match (the trailing
        // slash), and neither must the rest of the workspace.
        assert!(!touches_consumed_surface("crates/chat-extra/src/lib.rs"));
        assert!(!touches_consumed_surface("crates/foundation-x/src/lib.rs"));
        assert!(!touches_consumed_surface("crates/core/src/lib.rs"));
        assert!(!touches_consumed_surface("xtask/src/facade.rs"));
        assert!(!touches_consumed_surface(
            "docs/specs/release-management.md"
        ));
    }

    // ---- semver_checks_args (the fan-out over the set) ----

    #[test]
    fn semver_checks_args_pass_every_surface_package_and_the_baseline() {
        // A refactor that drops a package from the fan-out would leave the gate silently narrower
        // again — the exact failure 6j6v.a7ce is about — and CI would stay green while it did
        // (PR #315 review, Test Quality #1).
        assert_eq!(
            semver_checks_args("v0.57.0"),
            vec![
                "semver-checks",
                "check-release",
                "--package",
                "nexus-flow-facade",
                "--package",
                "nexus-chat",
                "--package",
                "nexus-memory",
                "--baseline-rev",
                "v0.57.0",
            ]
        );
    }

    // ---- gate_outcome (the patch-never-breaks decision) ----

    #[test]
    fn gate_red_on_break_during_patch() {
        assert!(gate_outcome(BumpAxis::Patch, false).is_err());
    }

    #[test]
    fn gate_green_on_noop_patch() {
        assert!(gate_outcome(BumpAxis::Patch, true).is_ok());
    }

    #[test]
    fn gate_green_on_break_during_minor() {
        // A minor is the break axis — the tool passes anyway, but even a failure is allowed there.
        assert!(gate_outcome(BumpAxis::Minor, false).is_ok());
        assert!(gate_outcome(BumpAxis::Major, false).is_ok());
    }

    #[test]
    fn gate_green_report_only_when_no_bump() {
        // No version bump yet (feature PR): surface drift, never fail the build.
        assert!(gate_outcome(BumpAxis::None, false).is_ok());
    }

    // ---- classify ----

    #[test]
    fn classify_equal_is_none() {
        assert_eq!(classify("0.7.1", "0.7.1").unwrap(), BumpAxis::None);
    }

    #[test]
    fn classify_patch_minor_major() {
        assert_eq!(classify("0.7.1", "0.7.2").unwrap(), BumpAxis::Patch);
        assert_eq!(classify("0.7.1", "0.8.0").unwrap(), BumpAxis::Minor);
        assert_eq!(classify("0.7.9", "1.0.0").unwrap(), BumpAxis::Major);
    }

    #[test]
    fn classify_downgrade_is_none() {
        // The gate does not police a downgrade — treat it as no forward bump.
        assert_eq!(classify("0.7.2", "0.7.1").unwrap(), BumpAxis::None);
    }

    // ---- is_fail_closed ----

    #[test]
    fn fail_closed_only_on_patch() {
        // Patch is the only fail-closed axis (a patch never breaks the facade). Minor/Major are
        // the break axis (report-only, mirrored to the changelog marker); None = no bump yet.
        assert!(BumpAxis::Patch.is_fail_closed());
        assert!(!BumpAxis::None.is_fail_closed());
        assert!(!BumpAxis::Minor.is_fail_closed());
        assert!(!BumpAxis::Major.is_fail_closed());
    }

    // ---- select_baseline ----

    #[test]
    fn baseline_is_the_current_tag_with_none_axis_on_a_released_version() {
        // current == latest tag (an ordinary feature PR): baseline is that tag, axis None
        // (report-only — the version is not bumped yet).
        let got = select_baseline(&tags(&["v0.6.0", "v0.7.0", "v0.7.1"]), "0.7.1").unwrap();
        assert_eq!(
            got,
            Some(Baseline {
                tag: "v0.7.1".into(),
                axis: BumpAxis::None
            })
        );
    }

    #[test]
    fn baseline_is_last_release_with_patch_axis_on_a_patch_bump() {
        let got = select_baseline(&tags(&["v0.6.0", "v0.7.0", "v0.7.1"]), "0.7.2").unwrap();
        assert_eq!(
            got,
            Some(Baseline {
                tag: "v0.7.1".into(),
                axis: BumpAxis::Patch
            })
        );
    }

    #[test]
    fn baseline_is_last_release_with_minor_axis_on_a_minor_bump() {
        let got = select_baseline(&tags(&["v0.6.0", "v0.7.0", "v0.7.1"]), "0.8.0").unwrap();
        assert_eq!(
            got,
            Some(Baseline {
                tag: "v0.7.1".into(),
                axis: BumpAxis::Minor
            })
        );
    }

    #[test]
    fn baseline_ignores_tags_above_current() {
        // A tag newer than the current version (e.g. on an older branch) is never the baseline.
        let got = select_baseline(&tags(&["v0.7.0", "v0.8.0"]), "0.7.1").unwrap();
        assert_eq!(got.unwrap().tag, "v0.7.0");
    }

    #[test]
    fn baseline_ignores_non_plain_semver_tags() {
        // Suffixed / non-matching tags are skipped (tags are plain SemVer, §4.4).
        let got = select_baseline(
            &tags(&["v0.7.0", "v0.8.0-beta.1", "nightly", "0.7.1"]),
            "0.7.1",
        )
        .unwrap();
        assert_eq!(got.unwrap().tag, "v0.7.0");
    }

    #[test]
    fn baseline_is_none_when_no_tag_at_or_below_current() {
        let got = select_baseline(&tags(&["v0.8.0", "v0.9.0"]), "0.7.0").unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn baseline_on_a_backport_line_ignores_newer_lines() {
        // Maintenance-line backport (nexus-flow-aye.18): on release/0.6 a patch 0.6.3 is gated
        // against its OWN line predecessor v0.6.2 — the newer 0.7.x tags are above 0.6.3 and
        // ignored — so the gate fail-closes the backport patch (the semver-gate/backport
        // interaction design question). This makes the facade contract hold per line.
        let got = select_baseline(&tags(&["v0.6.1", "v0.6.2", "v0.7.0", "v0.7.1"]), "0.6.3")
            .unwrap()
            .unwrap();
        assert_eq!(got.tag, "v0.6.2");
        assert_eq!(got.axis, BumpAxis::Patch);
        assert!(got.axis.is_fail_closed());
    }
}
