//! nexus-flow repo automation. Invoked as `cargo xtask <...>` (alias in
//! `.cargo/config.toml`). Houses the version + changelog tooling so the core workflow
//! stays single-toolchain (Rust only). Spec §4.

mod cache;
mod changelog;
mod channels;
mod docs;
mod facade;
mod macos_tests;
mod platforms;
mod runners;
mod truth;
mod version;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "xtask",
    about = "nexus-flow repo automation (version + changelog)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Version source-of-truth tooling (workspace version + Cargo.lock).
    #[command(subcommand)]
    Version(VersionCmd),
    /// Changelog fragments + release-notes.json feed.
    #[command(subcommand)]
    Changelog(ChangelogCmd),
    /// Shipped-platform tooling (single source of truth for the 4 platform keys).
    #[command(subcommand)]
    Platforms(PlatformsCmd),
    /// Promotion-channel tooling (single source of truth for the stable|beta|alpha ring).
    #[command(subcommand)]
    Channels(ChannelsCmd),
    /// Living-documentation pipeline: generate the command reference from the clap tree,
    /// or check the committed artifacts for drift (fail-closed CI gate). Epic E7.
    #[command(subcommand)]
    Docs(DocsCmd),
    /// Facade-contract tooling: the cargo-semver-checks gate on crates/facade (nexus-flow-aye.14).
    #[command(subcommand)]
    Facade(FacadeCmd),
    /// Runner-routing guard: keep the org's self-hosted Mac off every job a pull request can
    /// reach (nexus-flow-6j6v.2sb1).
    #[command(subcommand)]
    Runners(RunnersCmd),
    /// macOS test-coverage guard: keep every `cfg(target_os = "macos")` test inside a package the
    /// macOS lane runs (nexus-flow-6j6v.fb1d).
    #[command(subcommand, name = "macos-tests")]
    MacosTests(MacosTestsCmd),
    /// Actions-cache housekeeping: decide which cache entries no future run can reach
    /// (nexus-flow-6j6v.gapx).
    #[command(subcommand)]
    Cache(CacheCmd),
}

#[derive(Subcommand, Debug)]
enum CacheCmd {
    /// Read the repository's cache entries and the refs it can still reach, and print the
    /// entries no run can read any more — one id per line on stdout, the reasoning on stderr.
    /// Decides only; `cache-sweep.yml` does the deleting.
    Sweep {
        /// NDJSON cache entries, one per line, as `gh api --paginate .../actions/caches
        /// -q '.actions_caches[]'` emits them. `-` reads stdin.
        #[arg(long)]
        caches: PathBuf,
        /// Newline-separated branch names that still exist. Must contain the default branch.
        #[arg(long)]
        branches: PathBuf,
        /// Newline-separated numbers of the pull requests still open. May legitimately be empty.
        #[arg(long)]
        open_prs: PathBuf,
        /// The newest release tag, whose cache is kept. Omitted, NO tag scope is swept.
        #[arg(long)]
        keep_tag: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum MacosTestsCmd {
    /// Assert every macOS-gated test lives in a package `ci.yml`'s macOS lane runs (CI gate,
    /// fail-closed).
    Check,
}

#[derive(Subcommand, Debug)]
enum RunnersCmd {
    /// Assert no job reachable from a `pull_request`/`pull_request_target` run routes to a runner
    /// this repo cannot prove is GitHub-hosted (CI gate, fail-closed).
    Check,
}

#[derive(Subcommand, Debug)]
enum PlatformsCmd {
    /// Assert every consumer agrees with `release/platforms` (CI gate, fail-closed).
    Check,
}

#[derive(Subcommand, Debug)]
enum ChannelsCmd {
    /// Assert every consumer agrees with `release/channels` (CI gate, fail-closed).
    Check,
}

#[derive(Subcommand, Debug)]
enum DocsCmd {
    /// Generate the command reference (Markdown + man-pages + JSON) into docs/generated/.
    Gen,
    /// Assert docs/generated/ matches the clap tree (CI gate, fail-closed on drift).
    Check,
    /// Assert every non-hidden command has a reference entry + golden example under both
    /// plugins (CI gate, fail-closed). Epic E7 / nexus-flow-4oa.2.
    Coverage,
}

#[derive(Subcommand, Debug)]
enum FacadeCmd {
    /// Diff the public API of crates/facade against the baseline release tag and fail-closed on a
    /// patch bump that breaks it (cargo-semver-checks; nexus-flow-aye.14). Report-only when the
    /// workspace version is not bumped yet.
    SemverCheck,
    /// Print the resolved baseline tag + bump axis (the documented comparison point).
    Baseline,
}

#[derive(Subcommand, Debug)]
enum VersionCmd {
    /// Bump the workspace version + Cargo.lock atomically (rejects non-plain-SemVer), then
    /// fold any `changes/*.md` fragments into the beta ring of the feed.
    Set { version: String },
    /// Verify version files agree; in CI tag mode enforce `tag == v<version>`.
    Check {
        /// Release tag to match against the workspace version (CI release-tag mode).
        /// Defaults to `$GITHUB_REF_NAME` when the ref type is a tag.
        #[arg(long)]
        tag: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ChangelogCmd {
    /// PR gate: require a well-formed `changes/*.md` fragment (EN+DE) vs `base`, unless every
    /// path the PR touches is one that never ships.
    Check {
        /// Base ref to diff against (e.g. `origin/main`).
        base: String,
    },
    /// Render the notes for a version from the feed (inherits from the more-stable ring).
    Show {
        version: String,
        channel: String,
        lang: String,
    },
    /// Render the stable aggregate of beta items in `(prev, version]` for `lang`.
    Aggregate {
        prev: String,
        version: String,
        lang: String,
    },
    /// Print the most recent stable version below `version` (empty if none).
    PrevStable { version: String },
    /// Lift `version` into the stable ring as an aggregate since the previous stable.
    Promote { version: String },
    /// Fail-closed guard (tnce): assert the beta ring has a feed entry for `version`. promote.yml's
    /// pre-flight, run before any GitHub/AWS/feed mutation so a missing entry fails early, never
    /// half-way. Read-only.
    AssertBeta { version: String },
    /// Render the consolidated migration guide for a major version in `lang` (markdown).
    MigrationGuide { major: u64, lang: String },
    /// Re-derive every entry's stored `notes` from its own `items`, and report which entries were
    /// out of step. `notes` is what `show` — and so the GitHub release body — serves verbatim, so
    /// an entry whose prose was corrected in `items` alone still ships the old text. `--check`
    /// reports without writing.
    Rerender {
        #[arg(long)]
        check: bool,
    },
}

/// The workspace root is the current working directory: `cargo xtask` runs from the repo
/// root, and CI checks out there too.
fn workspace_root() -> PathBuf {
    PathBuf::from(".")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root();
    match cli.command {
        Command::Version(VersionCmd::Set { version }) => {
            // Validate fragments BEFORE mutating any manifest, so a malformed fragment can't
            // leave the version half-bumped on disk. `set` then bumps, and `consume` folds the
            // (now known-good) fragments into the feed — bump and notes move together. §4.2.
            changelog::validate_fragments(&root)?;
            // Fail-closed MAJOR-bump gate (≥1.0.0 only, inactive in 0.x): a real major bump
            // must carry at least one migration note. Checked before mutation so a missing
            // note can never leave the version half-bumped. §4.2.
            changelog::enforce_major_bump_migration(&root, &version)?;
            // Fail-closed facade-axis gate (nexus-flow-aye.15): a PATCH bump may not carry a
            // `facade: breaking` fragment — a facade break belongs on the minor (break) axis
            // (§4.3/§4.4). The changelog-side mirror of the cargo-semver-checks gate (aye.14),
            // checked before mutation. §4.2.
            changelog::enforce_facade_breaking_axis(&root, &version)?;
            version::set(&root, &version)?;
            changelog::consume(&root, &version, &changelog::today())?;
            // A version bump changes the version stamp embedded in docs/generated (commands.json
            // + the man-pages), so regenerate the reference in the same step. Otherwise the
            // committed docs would be stale and the `docs check` drift gate would fail RED on the
            // release PR. Wired here in the dispatch (not inside version::set) so the version.rs
            // unit tests that call version::set directly stay unaffected.
            docs::gen(&root)?;
            println!("version set to {version}");
            println!("reference docs regenerated into docs/generated");
        }
        Command::Version(VersionCmd::Check { tag }) => {
            let tag = tag.or_else(env_release_tag);
            version::check(&root, &version::CheckOpts { tag })?;
            println!("version check ok");
        }
        Command::Changelog(cmd) => run_changelog(&root, cmd)?,
        Command::Platforms(PlatformsCmd::Check) => {
            platforms::check(&root)?;
            println!("platforms check ok");
        }
        Command::Channels(ChannelsCmd::Check) => {
            channels::check(&root)?;
            println!("channels check ok");
        }
        Command::Docs(DocsCmd::Gen) => {
            docs::gen(&root)?;
            println!("docs generated into docs/generated");
        }
        Command::Docs(DocsCmd::Check) => {
            docs::check(&root)?;
            println!("docs check ok");
        }
        Command::Docs(DocsCmd::Coverage) => {
            docs::coverage(&root)?;
            println!("docs coverage ok");
        }
        Command::Facade(FacadeCmd::SemverCheck) => {
            facade::semver_check(&root)?;
            println!("facade semver-check ok");
        }
        Command::Facade(FacadeCmd::Baseline) => {
            facade::print_baseline(&root)?;
        }
        Command::Runners(RunnersCmd::Check) => {
            runners::check(&root)?;
            println!("runners check ok");
        }
        Command::MacosTests(MacosTestsCmd::Check) => {
            let inventory = macos_tests::check(&root)?;
            println!("macos-tests check ok — {inventory}");
        }
        Command::Cache(CacheCmd::Sweep {
            caches,
            branches,
            open_prs,
            keep_tag,
        }) => cache::run_sweep(&caches, &branches, &open_prs, keep_tag)?,
    }
    Ok(())
}

fn run_changelog(root: &std::path::Path, cmd: ChangelogCmd) -> Result<()> {
    use changelog::{Channel, Lang};
    match cmd {
        ChangelogCmd::Check { base } => {
            changelog::check(root, &base)?;
            println!("changelog check ok");
        }
        ChangelogCmd::Show {
            version,
            channel,
            lang,
        } => {
            let out = changelog::show(
                root,
                &version,
                Channel::parse(&channel)?,
                Lang::parse(&lang)?,
            )?;
            println!("{out}");
        }
        ChangelogCmd::Aggregate {
            prev,
            version,
            lang,
        } => {
            let out = changelog::aggregate(root, &prev, &version, Lang::parse(&lang)?)?;
            println!("{out}");
        }
        ChangelogCmd::PrevStable { version } => {
            if let Some(prev) = changelog::prev_stable(root, &version)? {
                println!("{prev}");
            }
        }
        ChangelogCmd::Promote { version } => {
            changelog::promote(root, &version)?;
            println!("promoted {version} to stable");
        }
        ChangelogCmd::AssertBeta { version } => {
            changelog::assert_beta_entry(root, &version)?;
            println!("beta feed entry present for {version}");
        }
        ChangelogCmd::Rerender { check } => {
            let drifted = changelog::rerender(root, check)?;
            if drifted.is_empty() {
                println!("changelog rerender: every entry's notes match its items");
            } else if check {
                anyhow::bail!(
                    "these feed entries' `notes` no longer render from their `items`: {}. \
                     Run `cargo xtask changelog rerender` to re-derive them.",
                    drifted.join(", ")
                );
            } else {
                println!(
                    "changelog rerender: re-derived {} entr{} ({})",
                    drifted.len(),
                    if drifted.len() == 1 { "y" } else { "ies" },
                    drifted.join(", ")
                );
            }
        }
        ChangelogCmd::MigrationGuide { major, lang } => {
            let out = changelog::migration_guide(root, major, Lang::parse(&lang)?)?;
            println!("{out}");
        }
    }
    Ok(())
}

/// `$GITHUB_REF_NAME` when `$GITHUB_REF_TYPE == tag`, else `None`. Lets the workflow call
/// `cargo xtask version check` with no args and still get tag==version enforcement on
/// release tags (spec §9). Tag data is read from env, never interpolated into a shell.
fn env_release_tag() -> Option<String> {
    if std::env::var("GITHUB_REF_TYPE").ok().as_deref() == Some("tag") {
        std::env::var("GITHUB_REF_NAME")
            .ok()
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag-mode CI gate hinges on this: a tag ref yields the tag name, anything else
    /// yields `None` (so a branch push gets consistency-only checking, not tag==version).
    #[test]
    fn env_release_tag_only_fires_on_tag_ref_type() {
        // No other test reads GITHUB_REF_*, so mutating it here is isolated.
        std::env::set_var("GITHUB_REF_TYPE", "tag");
        std::env::set_var("GITHUB_REF_NAME", "v1.2.3");
        assert_eq!(env_release_tag(), Some("v1.2.3".to_string()));

        std::env::set_var("GITHUB_REF_TYPE", "branch");
        assert_eq!(env_release_tag(), None);

        // Tag ref type but empty name → None (no spurious empty tag).
        std::env::set_var("GITHUB_REF_TYPE", "tag");
        std::env::set_var("GITHUB_REF_NAME", "");
        assert_eq!(env_release_tag(), None);

        std::env::remove_var("GITHUB_REF_TYPE");
        std::env::remove_var("GITHUB_REF_NAME");
        assert_eq!(env_release_tag(), None);
    }
}
