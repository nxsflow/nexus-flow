//! Changelog engine: PR fragments (`changes/<slug>.md`, EN+DE) folded into a single-source
//! `release-notes.json` feed that drives release bodies, self-update notes and the website
//! changelog. Semantics, fragment format and feed shape are ported from the reference
//! project (`scripts/changelog.mjs`, `changes/README.md`). Spec §4.2.
//!
//! Fragment format:
//! ```text
//! ---
//! type: added            # added | changed | fixed | removed
//! ---
//! [en]
//! English, user-facing.
//! [de]
//! Deutsch, nutzerseitig.
//! ```
//! Both languages are mandatory; malformed fragments are rejected loudly.
//!
//! A fragment MAY additionally carry an OPTIONAL migration note describing what happens
//! AUTOMATICALLY vs MANUALLY on upgrade, via `[migration.en]` / `[migration.de]` markers after
//! the body sections:
//! ```text
//! [migration.en]
//! What's automatic / what's manual, EN.
//! [migration.de]
//! Automatik / Handarbeit, DE.
//! ```
//! The migration block is optional, but if EITHER language marker is present BOTH must be
//! present and non-empty — a half-translated migration note is rejected loudly. On a real
//! major bump (new major ≥ 1 and increased vs the current workspace version) at least one
//! fragment in the release window MUST carry a migration note; in 0.x no migration note is
//! required (0.x may break freely). See `requires_migration_note`.
//!
//! Feed shape (`release-notes.json`, repo root):
//! ```json
//! { "channels": { "alpha": [], "beta": [ {
//!     "version": "0.2.0", "date": "2026-06-10",
//!     "items": [ { "type": "added", "en": "...", "de": "..." } ],
//!     "notes": { "en": "### Added\n- ...", "de": "### Neu\n- ..." }
//! } ], "stable": [] } }
//! ```
//! Beta carries incremental entries; stable aggregates since the last stable; a channel with
//! no entry inherits from the more-stable ring. EN headings Added/Changed/Fixed/Removed, DE
//! Neu/Geändert/Behoben/Entfernt (TB-12).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::version::{parse_plain_semver, semver_components, write_atomic};

// ---------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Added,
    Changed,
    Fixed,
    Removed,
}

/// Render order + the only valid `type:` values.
const TYPE_ORDER: [ChangeType; 4] = [
    ChangeType::Added,
    ChangeType::Changed,
    ChangeType::Fixed,
    ChangeType::Removed,
];

impl ChangeType {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "added" => ChangeType::Added,
            "changed" => ChangeType::Changed,
            "fixed" => ChangeType::Fixed,
            "removed" => ChangeType::Removed,
            other => bail!("invalid change type {other:?} (expected added|changed|fixed|removed)"),
        })
    }

    fn heading(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (ChangeType::Added, Lang::En) => "Added",
            (ChangeType::Changed, Lang::En) => "Changed",
            (ChangeType::Fixed, Lang::En) => "Fixed",
            (ChangeType::Removed, Lang::En) => "Removed",
            (ChangeType::Added, Lang::De) => "Neu",
            (ChangeType::Changed, Lang::De) => "Geändert",
            (ChangeType::Fixed, Lang::De) => "Behoben",
            (ChangeType::Removed, Lang::De) => "Entfernt",
        }
    }
}

/// Facade-contract impact of a change, a frontmatter flag orthogonal to `type`
/// (nexus-flow-aye.15) — optional in general, but MANDATORY on a PR whose diff touches a consumed
/// library surface (see [`enforce_explicit_facade_verdict`], which is the whole point of the
/// `none` value below). `changed` = the facade public API moved but stayed backward-compatible;
/// `breaking` = a backward-incompatible facade change. By the §4.3 contract a `breaking` facade
/// change may ship ONLY in a minor (the break axis) — `enforce_facade_breaking_axis` fail-closes
/// a patch bump that carries one, the changelog-side mirror of the `cargo-semver-checks` gate
/// (nexus-flow-aye.14).
///
/// [`Unaffected`](FacadeImpact::Unaffected) (`facade: none`) is the third, load-bearing value
/// (`6j6v.gmjd`): "I looked at the consumed surfaces and this change does not move their
/// contract". It exists so that a MISSING marker stops being indistinguishable from a checked
/// one — see [`enforce_explicit_facade_verdict`]. It is an assertion about the PR, not a property
/// of the release, so it never reaches the feed (see [`read_fragments`]).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum FacadeImpact {
    /// `facade: none` — checked, no contract impact. PR-gate only; never folded into the feed.
    #[serde(rename = "none")]
    Unaffected,
    Changed,
    Breaking,
}

impl FacadeImpact {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "none" => FacadeImpact::Unaffected,
            "changed" => FacadeImpact::Changed,
            "breaking" => FacadeImpact::Breaking,
            other => bail!("invalid facade impact {other:?} (expected none|changed|breaking)"),
        })
    }

    /// The language-neutral, grep-able label rendered in the dedicated feed section.
    fn label(self) -> &'static str {
        match self {
            FacadeImpact::Unaffected => "none",
            FacadeImpact::Changed => "changed",
            FacadeImpact::Breaking => "breaking",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "en" => Lang::En,
            "de" => Lang::De,
            other => bail!("invalid language {other:?} (expected en|de)"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Alpha,
    Beta,
    Stable,
}

impl Channel {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "alpha" => Channel::Alpha,
            "beta" => Channel::Beta,
            "stable" => Channel::Stable,
            other => bail!("invalid channel {other:?} (expected alpha|beta|stable)"),
        })
    }

    /// Lookup order for `show`: the channel itself, then the more-stable rings it inherits
    /// from when it has no entry of its own.
    fn inherit_chain(self) -> &'static [Channel] {
        match self {
            Channel::Alpha => &[Channel::Alpha, Channel::Beta, Channel::Stable],
            Channel::Beta => &[Channel::Beta, Channel::Stable],
            Channel::Stable => &[Channel::Stable],
        }
    }
}

/// An optional migration note carried by a fragment and retained through the feed: what
/// happens automatically vs manually on upgrade, both languages mandatory.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    pub en: String,
    pub de: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Item {
    #[serde(rename = "type")]
    pub change_type: ChangeType,
    pub en: String,
    pub de: String,
    /// Optional migration note. Absent in old feeds (`#[serde(default)]`) and omitted from
    /// output when `None`, so existing `release-notes.json` round-trips unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration: Option<Migration>,
    /// Optional facade-contract impact (nexus-flow-aye.15). `None` for a non-facade change;
    /// absent in old feeds and omitted from output when `None`, so existing
    /// `release-notes.json` round-trips unchanged. The machine-readable signal an embedding
    /// consumer (manufakt) decides re-review-vs-fast-track on (spec §4.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facade: Option<FacadeImpact>,
    /// Whether this change is a security fix. Defaults to `false` (absent in old feeds) and is
    /// omitted from output when `false`, so existing `release-notes.json` round-trips unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub security: bool,
    /// Whether this change is beta-only churn to omit from the STABLE aggregate (decision 8a2c):
    /// a fix/revert of something that was itself introduced and resolved between two stable
    /// promotes, so a stable user never saw it. The beta ring keeps it (append-only truth); only
    /// [`aggregate_items`] drops it. Defaults to `false` (absent in old feeds) and is omitted from
    /// output when `false`, so existing `release-notes.json` round-trips unchanged. Deserializable
    /// so the promoter can set it retroactively on a beta item in `release-notes.json`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unreleased: bool,
}

/// `skip_serializing_if` predicate: omit a `bool` field when it is `false`, so the feed only
/// carries the flag when it is set (and old feeds without the field round-trip unchanged).
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Notes {
    pub en: String,
    pub de: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub version: String,
    pub date: String,
    pub items: Vec<Item>,
    pub notes: Notes,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Channels {
    pub alpha: Vec<Entry>,
    pub beta: Vec<Entry>,
    pub stable: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Feed {
    pub channels: Channels,
}

/// A parsed fragment (frontmatter type + both language bodies + optional migration note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub change_type: ChangeType,
    pub en: String,
    pub de: String,
    /// Optional migration note: `Some` iff the fragment carried a well-formed
    /// `[migration.en]` + `[migration.de]` pair.
    pub migration: Option<Migration>,
    /// Optional `facade:` frontmatter flag (nexus-flow-aye.15): `changed` | `breaking`.
    pub facade: Option<FacadeImpact>,
    /// `security:` frontmatter flag (nexus-flow-aye.15); defaults to `false` when absent.
    pub security: bool,
    /// `unreleased:` frontmatter flag (decision 8a2c); defaults to `false`. Beta-only churn the
    /// stable aggregate omits (see [`Item::unreleased`]).
    pub unreleased: bool,
}

// ---------------------------------------------------------------------------------------
// Fragment parsing
// ---------------------------------------------------------------------------------------

/// Parse a single fragment file's text. Both `[en]` and `[de]` sections are mandatory and
/// non-empty; a missing frontmatter, unknown type, or missing language is a loud error.
pub fn parse_fragment(text: &str) -> Result<Fragment> {
    let mut lines = text.lines().peekable();

    // Skip leading blank lines, then require the opening `---`.
    while lines.peek().is_some_and(|l| l.trim().is_empty()) {
        lines.next();
    }
    match lines.next() {
        Some(l) if l.trim() == "---" => {}
        _ => bail!("fragment missing frontmatter (must start with a `---` line)"),
    }

    // Frontmatter: `key: value` lines until the closing `---`. `type` is mandatory; `facade`
    // and `security` (nexus-flow-aye.15) are optional flags.
    let mut change_type = None;
    let mut facade = None;
    let mut security = false;
    let mut unreleased = false;
    loop {
        let line = lines
            .next()
            .context("fragment frontmatter not closed with `---`")?;
        if line.trim() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            match key.trim() {
                "type" => change_type = Some(ChangeType::parse(value)?),
                "facade" => facade = Some(FacadeImpact::parse(value)?),
                "security" => security = parse_bool_flag(value)?,
                "unreleased" => unreleased = parse_bool_flag(value)?,
                _ => {} // unknown frontmatter keys are ignored (forward-compatible)
            }
        }
    }
    let change_type = change_type.context("fragment frontmatter missing `type:`")?;

    // Body: `[en]` / `[de]` section markers (either order), both mandatory and non-empty.
    // Optional `[migration.en]` / `[migration.de]` markers follow; if either is present both
    // must be present and non-empty.
    enum Sec {
        None,
        En,
        De,
        MigEn,
        MigDe,
    }
    let mut sec = Sec::None;
    let (mut en, mut de) = (Vec::new(), Vec::new());
    let (mut mig_en, mut mig_de) = (Vec::new(), Vec::new());
    let (mut seen_en, mut seen_de) = (false, false);
    let (mut seen_mig_en, mut seen_mig_de) = (false, false);
    for line in lines {
        match line.trim() {
            "[en]" => {
                sec = Sec::En;
                seen_en = true;
            }
            "[de]" => {
                sec = Sec::De;
                seen_de = true;
            }
            "[migration.en]" => {
                sec = Sec::MigEn;
                seen_mig_en = true;
            }
            "[migration.de]" => {
                sec = Sec::MigDe;
                seen_mig_de = true;
            }
            _ => match sec {
                Sec::En => en.push(line),
                Sec::De => de.push(line),
                Sec::MigEn => mig_en.push(line),
                Sec::MigDe => mig_de.push(line),
                Sec::None => {} // stray text between frontmatter and first marker is ignored
            },
        }
    }
    if !seen_en {
        bail!("fragment missing the mandatory `[en]` section");
    }
    if !seen_de {
        bail!("fragment missing the mandatory `[de]` section");
    }
    let en = en.join("\n").trim().to_string();
    let de = de.join("\n").trim().to_string();
    if en.is_empty() {
        bail!("fragment `[en]` section is empty");
    }
    if de.is_empty() {
        bail!("fragment `[de]` section is empty");
    }

    // Optional migration block: both languages or neither, each non-empty.
    let migration = parse_migration(seen_mig_en, seen_mig_de, &mig_en, &mig_de)?;

    Ok(Fragment {
        change_type,
        en,
        de,
        migration,
        facade,
        security,
        unreleased,
    })
}

/// Parse a strict boolean frontmatter flag (`true` | `false`). Loud on anything else, so a
/// typo (`security: yes`) fails the gate instead of silently reading as `false`.
fn parse_bool_flag(value: &str) -> Result<bool> {
    Ok(match value.trim() {
        "true" => true,
        "false" => false,
        other => bail!("invalid boolean flag {other:?} (expected true|false)"),
    })
}

/// Fold the optional migration markers into `Some(Migration)` / `None`, enforcing the
/// both-or-neither + non-empty invariant. Loud on a half-present or empty migration block.
fn parse_migration(
    seen_en: bool,
    seen_de: bool,
    en_lines: &[&str],
    de_lines: &[&str],
) -> Result<Option<Migration>> {
    if !seen_en && !seen_de {
        return Ok(None);
    }
    if seen_en && !seen_de {
        bail!("fragment has `[migration.en]` but is missing `[migration.de]` (both required)");
    }
    if seen_de && !seen_en {
        bail!("fragment has `[migration.de]` but is missing `[migration.en]` (both required)");
    }
    let en = en_lines.join("\n").trim().to_string();
    let de = de_lines.join("\n").trim().to_string();
    if en.is_empty() {
        bail!("fragment `[migration.en]` section is empty");
    }
    if de.is_empty() {
        bail!("fragment `[migration.de]` section is empty");
    }
    Ok(Some(Migration { en, de }))
}

// ---------------------------------------------------------------------------------------
// Rendering + feed I/O
// ---------------------------------------------------------------------------------------

/// Render grouped, ordered markdown notes for one language from a flat item list.
fn render_notes(items: &[Item], lang: Lang) -> String {
    let mut blocks = Vec::new();
    for ct in TYPE_ORDER {
        let group: Vec<&Item> = items.iter().filter(|i| i.change_type == ct).collect();
        if group.is_empty() {
            continue;
        }
        let mut block = format!("### {}", ct.heading(lang));
        for item in group {
            let text = match lang {
                Lang::En => &item.en,
                Lang::De => &item.de,
            };
            block.push_str("\n- ");
            block.push_str(text.trim());
        }
        blocks.push(block);
    }
    // Dedicated, machine-distinguishable facade-contract section (nexus-flow-aye.15), appended
    // after the standard groups. The `breaking`/`changed` label is language-neutral + grep-able;
    // the security marker is localized. Items keep their place in the normal type group too —
    // this section is the at-a-glance contract callout an embedding consumer (manufakt) scans.
    if let Some(block) = render_facade_section(items, lang) {
        blocks.push(block);
    }
    blocks.join("\n\n")
}

/// The `### Facade Contract` / `### Facade-Kontrakt` block, or `None` when no item carries a
/// facade flag. Items render in their given (already deterministic) order.
fn render_facade_section(items: &[Item], lang: Lang) -> Option<String> {
    let flagged: Vec<&Item> = items.iter().filter(|i| i.facade.is_some()).collect();
    if flagged.is_empty() {
        return None;
    }
    let heading = match lang {
        Lang::En => "Facade Contract",
        Lang::De => "Facade-Kontrakt",
    };
    let security_tag = match lang {
        Lang::En => "security",
        Lang::De => "Sicherheit",
    };
    let mut block = format!("### {heading}");
    for item in flagged {
        let label = item.facade.expect("filtered to Some").label();
        let text = match lang {
            Lang::En => &item.en,
            Lang::De => &item.de,
        };
        block.push_str(&format!("\n- `{label}` · "));
        if item.security {
            block.push_str(security_tag);
            block.push_str(" · ");
        }
        block.push_str(text.trim());
    }
    Some(block)
}

fn build_entry(version: &str, date: &str, items: Vec<Item>) -> Entry {
    let notes = Notes {
        en: render_notes(&items, Lang::En),
        de: render_notes(&items, Lang::De),
    };
    Entry {
        version: version.to_owned(),
        date: date.to_owned(),
        items,
        notes,
    }
}

/// Recompute every entry's stored `notes` from its own `items`, restoring the derivation
/// [`build_entry`] establishes. Returns the entries that did not match, newest ring first, as
/// `"<channel> <version>"`.
///
/// WHY THIS EXISTS: `notes` is derived from `items` but **stored**, and nothing recomputes it on
/// read — `show` (and through it the GitHub release body in `release.yml`) serves the stored copy
/// verbatim, while `aggregate` and the website's inline entries read `items`. Correcting an item's
/// prose therefore fixes the promotion path and leaves the per-version path shipping the old text.
/// This is the reproducible repair for that, and the opposite of the hand-edit
/// `docs/specs/release-management.md` §5.5 warns about: it *re-derives* the rendered copy instead
/// of typing over it, so a later re-promote reproduces the same result from the same `items`.
///
/// With `check_only`, nothing is written — the returned list is the finding.
pub fn rerender(root: &Path, check_only: bool) -> Result<Vec<String>> {
    let mut feed = load_feed(root)?;
    let mut drifted = Vec::new();
    for (channel, entries) in [
        ("stable", &mut feed.channels.stable),
        ("beta", &mut feed.channels.beta),
        ("alpha", &mut feed.channels.alpha),
    ] {
        for entry in entries.iter_mut() {
            let en = render_notes(&entry.items, Lang::En);
            let de = render_notes(&entry.items, Lang::De);
            if entry.notes.en != en || entry.notes.de != de {
                drifted.push(format!("{channel} {}", entry.version));
                entry.notes.en = en;
                entry.notes.de = de;
            }
        }
    }
    if !check_only && !drifted.is_empty() {
        save_feed(root, &feed)?;
    }
    Ok(drifted)
}

/// Load `release-notes.json`, or `Feed::default()` if it does not exist yet. Every entry's
/// version must be plain SemVer — the feed is the single source of truth and may be
/// hand-edited, so a malformed version fails loudly here rather than behaving differently in
/// each downstream code path (sort vs aggregate vs promote).
fn load_feed(root: &Path) -> Result<Feed> {
    let path = root.join("release-notes.json");
    if !path.exists() {
        return Ok(Feed::default());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let feed: Feed =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    for channel in [
        &feed.channels.alpha,
        &feed.channels.beta,
        &feed.channels.stable,
    ] {
        for entry in channel {
            parse_plain_semver(&entry.version).with_context(|| {
                format!(
                    "{} has entry with non-plain-SemVer version {:?}",
                    path.display(),
                    entry.version
                )
            })?;
        }
    }
    Ok(feed)
}

fn save_feed(root: &Path, feed: &Feed) -> Result<()> {
    let path = root.join("release-notes.json");
    let mut json = serde_json::to_string_pretty(feed).context("serializing feed")?;
    json.push('\n');
    write_atomic(&path, &json)
}

/// Replace any same-version entry, insert, and keep the channel newest-first.
fn upsert(channel: &mut Vec<Entry>, entry: Entry) {
    channel.retain(|e| e.version != entry.version);
    channel.push(entry);
    channel.sort_by(|a, b| {
        semver_key(&b.version)
            .ok()
            .cmp(&semver_key(&a.version).ok())
    });
}

fn channel_list(feed: &Feed, channel: Channel) -> &Vec<Entry> {
    match channel {
        Channel::Alpha => &feed.channels.alpha,
        Channel::Beta => &feed.channels.beta,
        Channel::Stable => &feed.channels.stable,
    }
}

fn notes_for(entry: &Entry, lang: Lang) -> &str {
    match lang {
        Lang::En => &entry.notes.en,
        Lang::De => &entry.notes.de,
    }
}

/// Parsed `(major, minor, patch)` for ordering. Errors on non-plain-SemVer input (including a
/// component out of `u64` range — single validation path, no double-parse, no panic).
fn semver_key(s: &str) -> Result<(u64, u64, u64)> {
    semver_components(s).with_context(|| format!("version {s:?} is not plain SemVer"))
}

// ---------------------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------------------

/// `changelog check <base>`: the PR gate. Requires the PR to add/modify at least one
/// `changes/*.md` fragment vs `base`, every fragment in `changes/` to be well-formed, and — when
/// the diff touches a consumed library surface — every fragment the PR touches to carry an
/// explicit `facade:` verdict ([`enforce_explicit_facade_verdict`]).
pub fn check(root: &Path, base: &str) -> Result<()> {
    let changed = git_changed_paths(root, base)?;
    // A real fragment, not changes/README.md (which documents the format and is excluded from
    // the fragment scan too — keep the trigger and the well-formedness scan in agreement, so a
    // PR that only touches README cannot satisfy the requirement with nothing validated).
    let touched_fragment = changed.iter().any(|p| is_fragment_path(p));
    if !touched_fragment {
        // No fragment is an answer only when nothing the PR touches ships (6j6v.c37e): an outside
        // contributor cannot set the label, so the gate has to see a no-impact PR by itself.
        // Deletions and both halves of a move count — removing shipped code is a product change.
        let touched = git_touched_paths(root, base)?;
        if touched.is_empty() {
            // Red, not green: an empty diff is what a mis-wired base looks like, and "nothing
            // ships" would then be vacuously true.
            bail!(
                "the diff against {base} is empty — there is nothing to judge; check that the \
                 base ref is right"
            );
        }
        let shipping: Vec<&str> = touched
            .iter()
            .map(String::as_str)
            .filter(|p| !has_no_product_impact(p))
            .collect();
        if shipping.is_empty() {
            println!(
                "changelog: no fragment needed — none of the {} path(s) this PR touches ships",
                touched.len()
            );
            return Ok(());
        }
        const SHOWN: usize = 10;
        let mut named = shipping[..shipping.len().min(SHOWN)].join(", ");
        if shipping.len() > SHOWN {
            named.push_str(&format!(" and {} more", shipping.len() - SHOWN));
        }
        bail!(
            "this PR touches no changes/*.md fragment, and it touches what ships: {named} — add \
             a fragment (EN+DE, see changes/README.md) describing the user-facing change; a \
             maintainer can instead apply the `skip-changelog` label"
        );
    }
    // Well-formedness: every present fragment must parse (loud on the first malformed one).
    for path in fragment_paths(root)? {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        parse_fragment(&text).with_context(|| format!("malformed fragment {}", path.display()))?;
    }
    enforce_explicit_facade_verdict(root, &changed, &git_touched_paths(root, base)?)
}

/// Turn a SILENT omission into a deliberate statement (`6j6v.gmjd`): when a PR's diff touches a
/// consumed library surface, every `changes/*.md` fragment that PR adds or modifies must carry an
/// explicit `facade:` value — `none`, `changed` or `breaking`.
///
/// **Why this gate has to exist at all.** The two machine gates around it both key on the PRESENCE
/// of a signal. `cargo-semver-checks` diffs the API *shape*, so a behavioural break behind
/// unchanged signatures is structurally invisible to it. `enforce_facade_breaking_axis` only fires
/// on a fragment that CARRIES `facade: breaking`. A missing marker was therefore identical to
/// "checked, no impact" — both are `None` — and a behavioural break could ship as a patch onto the
/// auto-adopt lane (spec §4.4), the one lane designed to run without a human. That is not
/// hypothetical: it happened twice on 2026-08-09 (PR #311 and PR #314, both tightening
/// `validate_author` at 26 public write entry points), both times with every gate green, both times
/// caught only by hand at the release question.
///
/// **What it is and is not.** It is process assurance, not a detector: it cannot tell whether the
/// behaviour actually changed, only that somebody was made to answer the question. The stronger
/// variant (fail-closed on a patch bump unless a marker says otherwise) was considered and NOT
/// taken first — replayed against both real incidents it would have caught nothing this does not,
/// because a minor was being cut anyway.
///
/// **Reach.** Every fragment the PR touches, not merely one of them: a rule satisfied by marking
/// some *other* fragment would let the breaking one stay silent, which is the exact failure mode.
/// The `skip-changelog` label bypasses this gate along with the rest of `check` — deliberately: a
/// label is itself a human act on the record, so it is not the silent omission this closes.
///
/// **Two path sets, on purpose.** `fragments` is the add/modify set — those are files this gate
/// opens and parses, so a deleted one must not be in it. `touched` is every path the diff mentions
/// on either side, DELETIONS AND MOVES INCLUDED, because removing a file from a consumed surface is
/// a contract change of the most obvious kind and asking about it is the entire job (PR #315
/// review, Integrity & Robustness #1: a deprecate-now-delete-later cleanup used to slip through,
/// and on a minor bump `cargo-semver-checks` is report-only, so nothing else would have spoken).
fn enforce_explicit_facade_verdict(
    root: &Path,
    fragments: &[String],
    touched: &[String],
) -> Result<()> {
    let triggers: Vec<&str> = touched
        .iter()
        .map(String::as_str)
        .filter(|p| crate::facade::touches_consumed_surface(p))
        .collect();
    if triggers.is_empty() {
        return Ok(());
    }
    for rel in fragments.iter().filter(|p| is_fragment_path(p)) {
        let path = root.join(rel);
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let frag = parse_fragment(&text)
            .with_context(|| format!("malformed fragment {}", path.display()))?;
        if frag.facade.is_none() {
            bail!(
                "this PR touches a consumed library surface ({}) but {rel} carries no `facade:` \
                 value — an omitted marker is indistinguishable from `checked, no impact`, so say \
                 which it is: `facade: none` (checked, the consumed contract does not move), \
                 `facade: changed` (the API moved, backward-compatible) or `facade: breaking` (a \
                 backward-incompatible change — minor axis only, spec §4.3). A behavioural break \
                 behind unchanged signatures is invisible to cargo-semver-checks; this marker is \
                 the only thing that carries it.",
                triggers.join(", ")
            );
        }
    }
    Ok(())
}

/// Paths whose change is no change to what a user gets (6j6v.c37e). A PR that touches ONLY these
/// needs no fragment.
///
/// **The boundary is what ships, not the file type.** "`*.md` needs no changelog" would be wrong:
/// `crates/*/docs/guide/**` is compiled into `nxs guide` and published as the docs on
/// nxsflow.com, `docs/architecture/**` carries the diagrams those pages embed, and `npm/mcp`'s
/// README is in the published package. So this is an ALLOWLIST, and everything absent from it —
/// every crate's `src/`, the guide trees, `npm/`, `agent-sidecar/src`, `content/`, `install.sh`,
/// `release/`, the manifests, `LICENSE`, the feed — needs a fragment or the label. A new
/// top-level directory is therefore red until someone decides what it is, and a new workflow that
/// can publish is red until it is named in [`SHIPS_THOUGH_BELOW_AN_EXEMPT_DIR`] (a test holds
/// that).
///
/// **Exempt means "not what a user gets", not "safe to run".** `npm/mcp/test/**` runs inside the
/// `publish-npm` job and `content/test/**` inside `publish-content`, both with an OIDC token. The
/// label used to force a maintainer's look at such a PR; now review does, and fork PRs still wait
/// for a maintainer's approval before any workflow runs (org setting, 6j6v.w07d).
///
/// A `/`-terminated entry covers everything below that directory; any other entry is one file.
/// Entries under `crates/` would be unreachable — that tree is decided by the one shape in
/// [`has_no_product_impact`].
const NO_PRODUCT_IMPACT: &[&str] = &[
    // Specs, runbooks, reviews, vision: the repo's own working documents. `docs/architecture/` and
    // `docs/generated/` are carved out in `SHIPS_THOUGH_BELOW_AN_EXEMPT_DIR`.
    "docs/",
    // Build and release tooling: never linked into the binary. It is not inert — `changelog
    // show|aggregate|promote` render the release body and the stable notes — but a change to it is
    // a change to how notes are produced, not to the product; `changelog rerender --check` and
    // `docs check` backstop the rendered output. The files that DEFINE this gate are carved out.
    "xtask/",
    "tests/",
    // CI. What can build, sign or publish is carved out below, by name.
    ".github/workflows/",
    // Tests of the npm shim, the agent sidecar and the content assembler; none of the three
    // directories is in what is published (see the exempt-means-not-safe note above).
    "npm/mcp/test/",
    "agent-sidecar/test/",
    "content/test/",
    // A sample embedding host, built by example-check.yml and never shipped. The published
    // developer docs point readers at it, so a change there can matter to a developer — but the
    // pages that say so are `content/docs-blocks/**`, which is not exempt.
    "examples/",
    // Repository prose. `README.md` also rides in every release tarball (release.yml copies it
    // next to LICENSE), so it IS delivered — as prose about the product, not as product
    // behaviour. A README typo is the archetypal first contribution; it keeps its exemption for
    // that reason, not because it never ships. `CONTRIBUTING.md` is listed ahead of its arrival.
    "README.md",
    "CONTRIBUTING.md",
    "AGENTS.md",
    "CLAUDE.md",
    "NEXUS_MEMORY.md",
    "changes/README.md",
    // Local tooling configuration.
    ".claude/",
    ".gitignore",
    ".envrc",
];

/// Below an exempt directory, but a change to what a user gets all the same.
const SHIPS_THOUGH_BELOW_AN_EXEMPT_DIR: &[&str] = &[
    // The diagrams the published docs embed (`DOCS_ASSETS` in content/lib/docs.mjs).
    "docs/architecture/",
    // Rendered from the CLI's definitions and not consumed by anything shipped; `docs check`
    // already reds a hand edit. A safety margin, not a boundary derived from shipping: a change
    // here without a change in `crates/` should not happen, and if it does it deserves a look.
    "docs/generated/",
    // Every workflow that can publish — `contents`, `id-token` or `packages: write`, or callable
    // by one. A change can change the bytes a user installs or the docs they read.
    // `every_publishing_workflow_is_carved_out` keeps this in step with .github/workflows/.
    ".github/workflows/release.yml",
    ".github/workflows/promote.yml",
    ".github/workflows/publish-content.yml",
    // The gate's own definition. Moving the boundary is a decision about what counts as product
    // impact, so it takes a fragment or a maintainer's label, not a silent green.
    ".github/workflows/changelog-check.yml",
    "xtask/src/changelog.rs",
    "xtask/src/facade.rs",
];

/// Whether a change to a repo-relative path is no change to what a user gets — see
/// [`NO_PRODUCT_IMPACT`]. Beside the list, one shape: a crate's integration tests
/// (`crates/<name>/tests/**`), which only ever compile into test binaries — except for a consumed
/// surface, whose tests pin a contract other repos build on (the silent case 6j6v.gmjd closed).
fn has_no_product_impact(path: &str) -> bool {
    let covers = |entry: &&str| match entry.strip_suffix('/') {
        Some(_) => path.starts_with(entry),
        None => path == *entry,
    };
    if SHIPS_THOUGH_BELOW_AN_EXEMPT_DIR.iter().any(covers)
        || crate::facade::touches_consumed_surface(path)
    {
        return false;
    }
    if let Some(rest) = path.strip_prefix("crates/") {
        let mut segments = rest.splitn(3, '/');
        let (_crate, dir, below) = (segments.next(), segments.next(), segments.next());
        return dir == Some("tests") && below.is_some_and(|b| !b.is_empty());
    }
    NO_PRODUCT_IMPACT.iter().any(covers)
}

/// Whether a repo-relative path is a real `changes/` fragment. `changes/README.md` documents the
/// format and is not a fragment — the trigger, the well-formedness scan and the facade-verdict
/// rule all agree on that through this one predicate.
fn is_fragment_path(path: &str) -> bool {
    path.starts_with("changes/") && path.ends_with(".md") && !path.ends_with("changes/README.md")
}

/// `changelog show <v> <channel> <lang>`: render the stored notes for a version, inheriting
/// from the more-stable ring when the channel has no entry of its own.
pub fn show(root: &Path, version: &str, channel: Channel, lang: Lang) -> Result<String> {
    let feed = load_feed(root)?;
    for ring in channel.inherit_chain() {
        if let Some(entry) = channel_list(&feed, *ring)
            .iter()
            .find(|e| e.version == version)
        {
            return Ok(notes_for(entry, lang).to_string());
        }
    }
    bail!("no changelog entry for {version} in channel {channel:?} (nor any more-stable ring)")
}

/// `changelog aggregate <prev> <v> <lang>`: render the stable aggregate — all beta items
/// strictly after `prev` and up to and including `v`, grouped and rendered for `lang`.
pub fn aggregate(root: &Path, prev: &str, version: &str, lang: Lang) -> Result<String> {
    // A curated rollup WINS, and it wins here as well as in `promote` so both paths say the same
    // thing — `rerender`'s own doc records what it costs when they drift.
    let curated = curated_stable_items(root, version)?;
    if !curated.is_empty() {
        return Ok(render_notes(&curated, lang));
    }
    let prev_key = semver_key(prev)?;
    let ver_key = semver_key(version)?;
    let feed = load_feed(root)?;
    let items = aggregate_items(&feed, prev_key, ver_key)?;
    Ok(render_notes(&items, lang))
}

/// `changelog migration-guide <major> <lang>`: fold every migration note for the given
/// `major` version into a single guide (markdown, one language). Walks the stable ring (the
/// durable, aggregated truth) for entries whose `major` matches, in ascending version order,
/// and emits one `## <version>` block per version that has migration notes, each note as a
/// bullet. Deterministic and ordered; empty string when there are no migration notes.
pub fn migration_guide(root: &Path, major: u64, lang: Lang) -> Result<String> {
    let feed = load_feed(root)?;
    let mut entries: Vec<&Entry> = feed
        .channels
        .stable
        .iter()
        .filter(|e| {
            semver_key(&e.version)
                .map(|(m, _, _)| m == major)
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| semver_key(&e.version).expect("validated by load_feed"));

    let mut blocks = Vec::new();
    for entry in entries {
        let notes: Vec<&Migration> = entry
            .items
            .iter()
            .filter_map(|i| i.migration.as_ref())
            .collect();
        if notes.is_empty() {
            continue;
        }
        let mut block = format!("## {}", entry.version);
        for note in notes {
            let text = match lang {
                Lang::En => &note.en,
                Lang::De => &note.de,
            };
            block.push_str("\n- ");
            block.push_str(text.trim());
        }
        blocks.push(block);
    }
    Ok(blocks.join("\n\n"))
}

/// `changelog prev-stable <v>`: the most recent stable version strictly below `v`, if any.
///
/// **Line-correctness (nexus-flow-aye.18).** This is a GLOBAL `max{stable s : s < v}`, and that
/// is exactly right for BOTH a current-line forward minor AND a maintenance-line backport — by
/// SemVer total order, not by luck. For a backport `x.Y.z` the line predecessor is the answer:
/// any higher line `x.Y'.*` (Y' > Y) has versions ≥ `x.(Y+1).0 > x.Y.z`, so it is never `< v`;
/// the max below `v` is therefore always the highest `x.Y.*` below it. For a forward minor
/// `x.(Y+1).0` the answer is the previous line `x.Y.*` — which is what the main "aggregate since
/// the last stable" promotion needs. A `major.minor` filter would BREAK that forward-minor case
/// (it would return `None` → aggregate from `0.0.0`), so the scope is deliberately global. The
/// `..._with_interleaved_entries` test locks both directions.
pub fn prev_stable(root: &Path, version: &str) -> Result<Option<String>> {
    let key = semver_key(version)?;
    let feed = load_feed(root)?;
    let mut best: Option<(u64, u64, u64)> = None;
    for entry in &feed.channels.stable {
        let k = semver_key(&entry.version)?;
        if k < key && best.is_none_or(|b| k > b) {
            best = Some(k);
        }
    }
    Ok(best.map(|(a, b, c)| format!("{a}.{b}.{c}")))
}

/// `changelog promote <v>`: lift `v` into the stable ring as an aggregate since the previous
/// stable, reusing the beta entry's date.
pub fn promote(root: &Path, version: &str) -> Result<()> {
    let ver_key = semver_key(version)?;
    let mut feed = load_feed(root)?;
    let date = feed
        .channels
        .beta
        .iter()
        .find(|e| e.version == version)
        .map(|e| e.date.clone())
        .with_context(|| format!("no beta entry for {version} to promote"))?;

    let prev_key = match prev_stable(root, version)? {
        Some(p) => semver_key(&p)?,
        None => (0, 0, 0),
    };
    let curated = curated_stable_items(root, version)?;
    let items = if curated.is_empty() {
        aggregate_items(&feed, prev_key, ver_key)?
    } else {
        curated
    };
    upsert(
        &mut feed.channels.stable,
        build_entry(version, &date, items),
    );
    save_feed(root, &feed)
}

/// Parse every `changes/*.md` fragment (loud on the first malformed one) without touching
/// anything. Lets `version set` reject a bad fragment *before* it rewrites the manifests, so
/// a failed run can never leave a half-applied bump.
pub fn validate_fragments(root: &Path) -> Result<()> {
    read_fragments(root).map(|_| ())
}

/// True iff a bump from `prev` to `next` is a real MAJOR bump that must carry a migration
/// note: `next.major >= 1` AND `next.major > prev.major`. A bump that stays in 0.x (or a
/// minor/patch bump within the same major) imposes no requirement — 0.x may break freely and
/// a same-major release is non-breaking by SemVer contract.
pub fn requires_migration_note(prev: &str, next: &str) -> Result<bool> {
    let (prev_major, _, _) = semver_components(prev)
        .with_context(|| format!("previous version {prev:?} is not plain SemVer"))?;
    let (next_major, _, _) = semver_components(next)
        .with_context(|| format!("next version {next:?} is not plain SemVer"))?;
    Ok(next_major >= 1 && next_major > prev_major)
}

/// Fail-closed MAJOR-bump gate. On a real major bump (see [`requires_migration_note`]) at
/// least one `changes/*.md` fragment being consumed must carry a `migration:` section; if none
/// does, fail loudly BEFORE any manifest is mutated. Inactive in 0.x. Reads the current
/// workspace version from the root `Cargo.toml` via [`crate::version::workspace_version`].
pub fn enforce_major_bump_migration(root: &Path, next_version: &str) -> Result<()> {
    let prev_version = crate::version::workspace_version(root)?;
    if !requires_migration_note(&prev_version, next_version)? {
        return Ok(());
    }
    let frags = read_fragments(root)?;
    let has_migration = frags.iter().any(|(_, item)| item.migration.is_some());
    if !has_migration {
        bail!(
            "major bump {prev_version} → {next_version} requires at least one migration note, \
             but none of the changes/*.md fragments carry a `[migration.en]`/`[migration.de]` \
             section — add one describing what upgrades automatically vs manually (spec §4.2)"
        );
    }
    Ok(())
}

/// Fail-closed facade-axis gate (nexus-flow-aye.15), the changelog-side mirror of the
/// `cargo-semver-checks` gate (nexus-flow-aye.14): on a PATCH bump (`x.Y.z → x.Y.z+1`) no
/// `changes/*.md` fragment may be marked `facade: breaking` — a facade break may ship only in a
/// minor (the break axis, spec §4.3/§4.4). Inactive on a minor/major bump (breaks allowed there)
/// and when there is no patch-forward bump. Checked before any manifest mutation. Reads the
/// current workspace version via [`crate::version::workspace_version`].
pub fn enforce_facade_breaking_axis(root: &Path, next_version: &str) -> Result<()> {
    let prev_version = crate::version::workspace_version(root)?;
    if !is_patch_bump(&prev_version, next_version)? {
        return Ok(());
    }
    let frags = read_fragments(root)?;
    let offender = frags
        .iter()
        .find(|(_, item)| item.facade == Some(FacadeImpact::Breaking));
    if let Some((path, _)) = offender {
        bail!(
            "patch bump {prev_version} → {next_version} but {} is marked `facade: breaking` — a \
             facade break may ship only in a MINOR (the break axis, spec §4.3/§4.4); bump the \
             minor instead, or downgrade the fragment to `facade: changed`",
            path.display()
        );
    }
    Ok(())
}

/// True iff `next` is a patch-forward bump of `prev`: same major and minor, `next.patch >
/// prev.patch`. A minor/major bump (or no forward patch) is not a patch bump.
fn is_patch_bump(prev: &str, next: &str) -> Result<bool> {
    let (pmaj, pmin, ppatch) = semver_components(prev)
        .with_context(|| format!("previous version {prev:?} is not plain SemVer"))?;
    let (nmaj, nmin, npatch) = semver_components(next)
        .with_context(|| format!("next version {next:?} is not plain SemVer"))?;
    Ok(nmaj == pmaj && nmin == pmin && npatch > ppatch)
}

/// `changes/*.md` parsed into `(path, item)` pairs (sorted, empty if the dir is absent).
///
/// `facade: none` is dropped here — this is the ONE place a fragment becomes a feed item, so the
/// invariant "an [`Item`] never carries [`FacadeImpact::Unaffected`]" holds by construction. The
/// value is a statement about the PR ("checked, no contract impact"), not about the release, and
/// the feed's `facade` field is a contract a consumer already parses as `changed|breaking`:
/// emitting a third value there would be exactly the kind of unannounced widening this gate
/// exists to prevent (`6j6v.gmjd`).
/// The CURATED stable rollup for `version`, if its author wrote one: every fragment in
/// `changes/stable/<version>/`, in filename order. Empty when the directory does not exist.
///
/// # Why this exists (nexus-flow, promoting v0.63.0)
///
/// `docs/specs/release-management.md` §5.5 step 3 tells the promoter to condense the stable
/// rollup — "multiple beta refinements of one feature into a single stable line", phrased for a
/// stable audience. **There was no way to do that durably.** [`promote`] `upsert`s the stable
/// entry from the mechanical union of beta items on every run, and `promote.yml` re-derives the
/// manifest notes from [`aggregate`] besides — so a hand-curated entry was overwritten by the very
/// promotion it was written for. The practice was specified and structurally impossible, which is
/// why it had never been done: promoting 0.35.0 → 0.63.0 produced a 201 KiB mechanical union of 74
/// fragments across 33 versions.
///
/// **A curated rollup is just the ITEMS the stable entry should have**, authored by hand instead of
/// unioned. Same fragment format, same parser, same [`render_notes`] — so the rendered `notes` stay
/// DERIVED from `items` and `changelog rerender --check` keeps holding that derivation. Nothing
/// here is a second notes path; it is a different source for the same one.
///
/// It lives under `changes/` because that is where changelog copy lives, and in a SUBDIRECTORY
/// because [`fragment_paths`] reads `changes/*.md` non-recursively — a curated rollup can never be
/// mistaken for a pending fragment, and `version set` cannot consume it by accident.
///
/// Absent is the ordinary case: a release whose stable jump is one or two versions needs no
/// curation, and the mechanical union is the honest answer for it.
fn curated_stable_items(root: &Path, version: &str) -> Result<Vec<Item>> {
    let dir = root.join("changes").join("stable").join(version);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let frag = parse_fragment(&text)
            .with_context(|| format!("malformed curated rollup item {}", path.display()))?;
        out.push(Item {
            change_type: frag.change_type,
            en: frag.en,
            de: frag.de,
            migration: frag.migration,
            facade: frag.facade.filter(|f| *f != FacadeImpact::Unaffected),
            security: frag.security,
            // Meaningless on a curated item: the whole point of writing one is that the churn is
            // already gone. Carried through rather than forced, so a author who flags one is not
            // silently overridden.
            unreleased: frag.unreleased,
        });
    }
    Ok(out)
}

fn read_fragments(root: &Path) -> Result<Vec<(PathBuf, Item)>> {
    let mut out = Vec::new();
    for path in fragment_paths(root)? {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let frag = parse_fragment(&text)
            .with_context(|| format!("malformed fragment {}", path.display()))?;
        out.push((
            path,
            Item {
                change_type: frag.change_type,
                en: frag.en,
                de: frag.de,
                migration: frag.migration,
                facade: frag.facade.filter(|f| *f != FacadeImpact::Unaffected),
                security: frag.security,
                unreleased: frag.unreleased,
            },
        ));
    }
    Ok(out)
}

/// Consume every `changes/*.md` fragment into a beta feed entry for `version` (deleting the
/// fragments). ALWAYS writes the entry: with no fragments it upserts an empty `items: []` beta
/// entry (tnce), so "every released version is in the feed" holds by construction and
/// [`promote`]/[`assert_beta_entry`] can never fail on a missing entry. Shared with `version set`.
pub fn consume(root: &Path, version: &str, date: &str) -> Result<()> {
    parse_plain_semver(version)?;
    let frags = read_fragments(root)?;
    // NB: no early return when `frags` is empty (tnce). A release with no fragments — a re-ship, a
    // pure infra/CI patch — must STILL get a beta feed entry (an empty `items: []` one), so that
    // `changelog promote` always finds it and can never fail late in promote.yml (the half-promoted
    // state). "Every released version is in the feed" is the invariant, established here at
    // `version set` time (before any GitHub release flip). With no fragments the merge below keeps
    // any existing entry's items and simply upserts the version marker.
    let new_items: Vec<Item> = frags.iter().map(|(_, item)| item.clone()).collect();

    let mut feed = load_feed(root)?;
    // Merge (don't replace) into any existing same-version beta entry: start from its already-
    // folded items and append only newly-read items not already present. This makes `consume`
    // idempotent — a re-run reading a leftover fragment whose item is already folded neither
    // duplicates it (dedup) nor drops the items whose fragments were deleted last run (the
    // double-fold / data-loss bug, 85y.24). `promote`'s replace-on-recompute stays correct.
    //
    // Dedup is by VALUE equality of (type, en, de): two byte-identical fragments collapse to one
    // entry. That is the intended trade-off — identical changelog lines are redundant, and the
    // win (idempotent re-fold of a leftover fragment) outweighs the vanishingly rare case of two
    // deliberately-identical fragments. An EDITED fragment (same type, changed text) is a NEW
    // value, so both the old and the edited line surface in beta until the next `consume` for a
    // fresh version or `promote` recomputes the aggregate — the loss-averse direction by design.
    let mut merged = feed
        .channels
        .beta
        .iter()
        .find(|e| e.version == version)
        .map(|e| e.items.clone())
        .unwrap_or_default();
    for item in new_items {
        if !merged.contains(&item) {
            merged.push(item);
        }
    }

    upsert(&mut feed.channels.beta, build_entry(version, date, merged));
    save_feed(root, &feed)?;
    // Only delete once the feed is durably written.
    for (path, _) in &frags {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// tnce (fail-fast guard): assert the beta ring carries an entry for `version`. `promote.yml` runs
/// this as its FIRST real step — before the aggregate/promote step and before any AWS copy or feed
/// commit — so if the invariant `consume` upholds ("every released version is in the feed") is ever
/// violated, the promotion fails CLEANLY and early instead of half-way, never leaving GitHub-says-
/// stable / S3-stable-empty. Read-only: it never mutates the feed.
pub fn assert_beta_entry(root: &Path, version: &str) -> Result<()> {
    parse_plain_semver(version)?;
    let feed = load_feed(root)?;
    if feed.channels.beta.iter().any(|e| e.version == version) {
        return Ok(());
    }
    bail!(
        "no beta feed entry for {version} in release-notes.json — refusing to promote. A released \
         version must carry a beta entry; `cargo xtask version set {version}` writes one even with no \
         changelog fragments. Add the entry (and re-run `version set`) before firing the promotion."
    )
}

/// Beta items with key in `(prev_key, ver_key]`, oldest version first. With `prev_key` set to the
/// line predecessor (from [`prev_stable`]) and `ver_key` the version being promoted, the upper
/// bound `<= ver_key` excludes every higher-line beta (their versions are `> ver_key`) and the
/// lower bound `> prev_key` excludes everything at/below the predecessor — so a maintenance-line
/// backport aggregates ONLY its own line's window, by SemVer ordering (nexus-flow-aye.18).
fn aggregate_items(
    feed: &Feed,
    prev_key: (u64, u64, u64),
    ver_key: (u64, u64, u64),
) -> Result<Vec<Item>> {
    let mut entries: Vec<&Entry> = Vec::new();
    for entry in &feed.channels.beta {
        let k = semver_key(&entry.version)?;
        if k > prev_key && k <= ver_key {
            entries.push(entry);
        }
    }
    entries.sort_by_key(|e| semver_key(&e.version).expect("validated above"));
    // Stable = net-diff, not union (decision 8a2c): drop `unreleased`-flagged items — beta-only
    // churn a stable user never saw. The beta ring keeps them (append-only truth); only this
    // stable rollup omits them.
    Ok(entries
        .iter()
        .flat_map(|e| e.items.iter().filter(|i| !i.unreleased).cloned())
        .collect())
}

/// Sorted `changes/*.md` fragment paths (empty if the directory is absent). `README.md` documents
/// the fragment format and is NOT a fragment — it is excluded so the scan never tries to parse it.
fn fragment_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join("changes");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("README.md"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Paths added/copied/modified/renamed vs `base` (three-dot: the PR's own changes since the
/// merge base) — the files that EXIST in the working tree afterwards, so a caller may open them.
fn git_changed_paths(root: &Path, base: &str) -> Result<Vec<String>> {
    git_diff_names(root, base, &["--diff-filter=ACMR"])
}

/// Every path the diff mentions on EITHER side vs `base`: additions, modifications, deletions, and
/// both halves of a move (`--no-renames` splits a rename into a delete plus an add).
///
/// This is the membership set for "did this PR touch a consumed surface", where
/// [`git_changed_paths`]'s add/modify view is wrong twice over: deleting a file from a surface is a
/// contract change, and moving one OUT of a surface would otherwise be seen only at its new,
/// unguarded home (PR #315 review, Integrity & Robustness #1). Never use it to read files — half
/// these paths are gone.
fn git_touched_paths(root: &Path, base: &str) -> Result<Vec<String>> {
    git_diff_names(root, base, &["--no-renames"])
}

/// `git diff --name-only <extra…> <base>...HEAD`. Tag/branch data flows as an argv element, never
/// through a shell.
fn git_diff_names(root: &Path, base: &str, extra: &[&str]) -> Result<Vec<String>> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["diff", "--name-only"])
        .args(extra)
        .arg(format!("{base}...HEAD"))
        .output()
        .context("running git diff")?;
    if !out.status.success() {
        bail!(
            "git diff against {base} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

/// Today's date as `YYYY-MM-DD` (UTC). Overridable via `$XTASK_DATE` for reproducible runs.
pub fn today() -> String {
    if let Ok(d) = std::env::var("XTASK_DATE") {
        if !d.is_empty() {
            return d;
        }
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days-since-Unix-epoch → civil `(year, month, day)` (Howard Hinnant's algorithm). Keeps
/// date formatting dependency-free and deterministic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str =
        "---\ntype: added\n---\n[en]\nAdded a thing.\n[de]\nEine Sache hinzugefügt.\n";

    fn write_feed(root: &Path, feed: &Feed) {
        fs::write(
            root.join("release-notes.json"),
            serde_json::to_string_pretty(feed).unwrap(),
        )
        .unwrap();
    }

    fn entry(version: &str, date: &str, ct: ChangeType, en: &str, de: &str) -> Entry {
        let items = vec![Item {
            change_type: ct,
            en: en.into(),
            de: de.into(),
            migration: None,
            facade: None,
            security: false,
            unreleased: false,
        }];
        let notes = Notes {
            en: render_notes(&items, Lang::En),
            de: render_notes(&items, Lang::De),
        };
        Entry {
            version: version.into(),
            date: date.into(),
            items,
            notes,
        }
    }

    // ---- fragment parsing ----

    #[test]
    fn parses_well_formed_fragment() {
        let f = parse_fragment(GOOD).unwrap();
        assert_eq!(f.change_type, ChangeType::Added);
        assert_eq!(f.en, "Added a thing.");
        assert_eq!(f.de, "Eine Sache hinzugefügt.");
    }

    #[test]
    fn fragment_preserves_multiline_body() {
        let txt =
            "---\ntype: fixed\n---\n[en]\nLine one.\nLine two.\n[de]\nZeile eins.\nZeile zwei.\n";
        let f = parse_fragment(txt).unwrap();
        assert_eq!(f.en, "Line one.\nLine two.");
        assert_eq!(f.de, "Zeile eins.\nZeile zwei.");
    }

    #[test]
    fn rejects_missing_frontmatter() {
        assert!(parse_fragment("[en]\nx\n[de]\ny\n").is_err());
    }

    #[test]
    fn rejects_unknown_type() {
        assert!(parse_fragment("---\ntype: bogus\n---\n[en]\nx\n[de]\ny\n").is_err());
    }

    #[test]
    fn rejects_missing_german_section() {
        assert!(parse_fragment("---\ntype: added\n---\n[en]\nx\n").is_err());
    }

    #[test]
    fn rejects_empty_language_body() {
        assert!(parse_fragment("---\ntype: added\n---\n[en]\n\n[de]\ny\n").is_err());
    }

    // ---- migration block parsing ----

    const GOOD_MIG: &str = "---\ntype: changed\n---\n[en]\nChanged a thing.\n[de]\nEine Sache \
        geändert.\n[migration.en]\nAuto: schema migrates. Manual: re-auth.\n[migration.de]\n\
        Automatik: Schema migriert. Manuell: neu anmelden.\n";

    #[test]
    fn parses_fragment_with_migration() {
        let f = parse_fragment(GOOD_MIG).unwrap();
        assert_eq!(f.change_type, ChangeType::Changed);
        let mig = f.migration.expect("migration present");
        assert_eq!(mig.en, "Auto: schema migrates. Manual: re-auth.");
        assert_eq!(mig.de, "Automatik: Schema migriert. Manuell: neu anmelden.");
    }

    #[test]
    fn fragment_without_migration_yields_none() {
        let f = parse_fragment(GOOD).unwrap();
        assert!(f.migration.is_none());
    }

    #[test]
    fn migration_preserves_multiline_body() {
        let txt = "---\ntype: changed\n---\n[en]\nx\n[de]\ny\n[migration.en]\nLine one.\n\
            Line two.\n[migration.de]\nZeile eins.\nZeile zwei.\n";
        let mig = parse_fragment(txt).unwrap().migration.unwrap();
        assert_eq!(mig.en, "Line one.\nLine two.");
        assert_eq!(mig.de, "Zeile eins.\nZeile zwei.");
    }

    #[test]
    fn rejects_migration_en_only() {
        let txt = "---\ntype: changed\n---\n[en]\nx\n[de]\ny\n[migration.en]\nonly english\n";
        assert!(parse_fragment(txt).is_err());
    }

    #[test]
    fn rejects_migration_de_only() {
        let txt = "---\ntype: changed\n---\n[en]\nx\n[de]\ny\n[migration.de]\nnur deutsch\n";
        assert!(parse_fragment(txt).is_err());
    }

    #[test]
    fn rejects_empty_migration_body() {
        let txt =
            "---\ntype: changed\n---\n[en]\nx\n[de]\ny\n[migration.en]\n\n[migration.de]\nde\n";
        assert!(parse_fragment(txt).is_err());
    }

    // ---- facade / security flags (nexus-flow-aye.15) ----

    #[test]
    fn parses_facade_breaking_flag() {
        let txt = "---\ntype: changed\nfacade: breaking\n---\n[en]\nx\n[de]\ny\n";
        let f = parse_fragment(txt).unwrap();
        assert_eq!(f.facade, Some(FacadeImpact::Breaking));
        assert!(!f.security);
    }

    #[test]
    fn parses_facade_changed_flag() {
        let txt = "---\ntype: changed\nfacade: changed\n---\n[en]\nx\n[de]\ny\n";
        let f = parse_fragment(txt).unwrap();
        assert_eq!(f.facade, Some(FacadeImpact::Changed));
    }

    #[test]
    fn parses_facade_none_as_a_checked_verdict() {
        // `facade: none` is a VALUE, not the absence of one (6j6v.gmjd) — that distinction is the
        // whole point: `Some(Unaffected)` means somebody looked, `None` means nobody said.
        let txt = "---\ntype: changed\nfacade: none\n---\n[en]\nx\n[de]\ny\n";
        let f = parse_fragment(txt).unwrap();
        assert_eq!(f.facade, Some(FacadeImpact::Unaffected));
    }

    #[test]
    fn parses_security_flag() {
        let txt = "---\ntype: fixed\nsecurity: true\n---\n[en]\nx\n[de]\ny\n";
        let f = parse_fragment(txt).unwrap();
        assert!(f.security);
        assert_eq!(f.facade, None);
    }

    #[test]
    fn parses_facade_and_security_together() {
        let txt = "---\ntype: changed\nfacade: breaking\nsecurity: true\n---\n[en]\nx\n[de]\ny\n";
        let f = parse_fragment(txt).unwrap();
        assert_eq!(f.facade, Some(FacadeImpact::Breaking));
        assert!(f.security);
    }

    #[test]
    fn fragment_without_flags_yields_none_and_not_security() {
        let f = parse_fragment(GOOD).unwrap();
        assert_eq!(f.facade, None);
        assert!(!f.security);
    }

    #[test]
    fn rejects_invalid_facade_value() {
        let txt = "---\ntype: changed\nfacade: bogus\n---\n[en]\nx\n[de]\ny\n";
        assert!(parse_fragment(txt).is_err());
    }

    #[test]
    fn rejects_invalid_security_value() {
        let txt = "---\ntype: fixed\nsecurity: yes\n---\n[en]\nx\n[de]\ny\n";
        assert!(parse_fragment(txt).is_err());
    }

    #[test]
    fn parses_unreleased_flag_and_defaults_off() {
        // decision 8a2c: `unreleased: true` is a strict bool flag, default off when absent.
        let f =
            parse_fragment("---\ntype: fixed\nunreleased: true\n---\n[en]\nx\n[de]\ny\n").unwrap();
        assert!(f.unreleased, "unreleased: true parses to true");
        assert!(
            !parse_fragment(GOOD).unwrap().unreleased,
            "absent ⇒ default false"
        );
    }

    #[test]
    fn rejects_invalid_unreleased_value() {
        let txt = "---\ntype: fixed\nunreleased: maybe\n---\n[en]\nx\n[de]\ny\n";
        assert!(
            parse_fragment(txt).is_err(),
            "a non-bool unreleased value is a loud error"
        );
    }

    // ---- rendering ----

    #[test]
    fn renders_grouped_headings_in_order() {
        let items = vec![
            Item {
                change_type: ChangeType::Fixed,
                en: "Fixed B".into(),
                de: "B behoben".into(),
                migration: None,
                facade: None,
                security: false,
                unreleased: false,
            },
            Item {
                change_type: ChangeType::Added,
                en: "Added A".into(),
                de: "A neu".into(),
                migration: None,
                facade: None,
                security: false,
                unreleased: false,
            },
        ];
        let en = render_notes(&items, Lang::En);
        assert_eq!(en, "### Added\n- Added A\n\n### Fixed\n- Fixed B");
        let de = render_notes(&items, Lang::De);
        assert_eq!(de, "### Neu\n- A neu\n\n### Behoben\n- B behoben");
    }

    #[test]
    fn renders_dedicated_facade_section_en_de() {
        let items = vec![
            Item {
                change_type: ChangeType::Changed,
                en: "Engine::foo signature changed".into(),
                de: "Signatur von Engine::foo geändert".into(),
                migration: None,
                facade: Some(FacadeImpact::Breaking),
                security: false,
                unreleased: false,
            },
            Item {
                change_type: ChangeType::Fixed,
                en: "Hardened input validation".into(),
                de: "Eingabevalidierung gehärtet".into(),
                migration: None,
                facade: Some(FacadeImpact::Changed),
                security: true,
                unreleased: false,
            },
        ];
        let en = render_notes(&items, Lang::En);
        assert_eq!(
            en,
            "### Changed\n- Engine::foo signature changed\n\n\
             ### Fixed\n- Hardened input validation\n\n\
             ### Facade Contract\n\
             - `breaking` · Engine::foo signature changed\n\
             - `changed` · security · Hardened input validation"
        );
        let de = render_notes(&items, Lang::De);
        assert_eq!(
            de,
            "### Geändert\n- Signatur von Engine::foo geändert\n\n\
             ### Behoben\n- Eingabevalidierung gehärtet\n\n\
             ### Facade-Kontrakt\n\
             - `breaking` · Signatur von Engine::foo geändert\n\
             - `changed` · Sicherheit · Eingabevalidierung gehärtet"
        );
    }

    /// `notes` is defined as a deterministic render of `items` ([`build_entry`]), but it is STORED,
    /// not recomputed on read — `changelog show` (and therefore the GitHub release body) serves the
    /// stored copy verbatim. So a corrected `items[]` with a stale `notes` ships the OLD text while
    /// looking fixed in the feed. Nothing detected that before; `rerender` is the reproducible
    /// repair, and its `--check` mode is the guard.
    #[test]
    fn rerender_check_spots_notes_that_no_longer_match_their_items() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(root.join("changes/a.md"), GOOD).unwrap();
        consume(root, "0.1.0", "2026-06-10").unwrap();

        // A consumed feed is consistent by construction.
        assert!(
            rerender(root, true).unwrap().is_empty(),
            "freshly consumed feed is consistent"
        );

        // Correct the item's prose the way a read-through would, leaving `notes` behind.
        let mut feed = load_feed(root).unwrap();
        feed.channels.beta[0].items[0].en = "Corrected English prose.".into();
        save_feed(root, &feed).unwrap();

        let drifted = rerender(root, true).unwrap();
        assert_eq!(
            drifted,
            vec!["beta 0.1.0".to_string()],
            "check names the drifted entry"
        );
        // `--check` must not write.
        assert!(!load_feed(root).unwrap().channels.beta[0]
            .notes
            .en
            .contains("Corrected"));

        // The repair is a pure re-derivation from `items`, and is idempotent.
        assert_eq!(
            rerender(root, false).unwrap(),
            vec!["beta 0.1.0".to_string()]
        );
        let fixed = load_feed(root).unwrap();
        assert!(fixed.channels.beta[0]
            .notes
            .en
            .contains("Corrected English prose."));
        assert_eq!(
            fixed.channels.beta[0].notes.en,
            render_notes(&fixed.channels.beta[0].items, Lang::En)
        );
        assert!(rerender(root, true).unwrap().is_empty(), "idempotent");
    }

    #[test]
    fn render_notes_without_facade_has_no_facade_section() {
        let items = vec![Item {
            change_type: ChangeType::Added,
            en: "A".into(),
            de: "A neu".into(),
            migration: None,
            facade: None,
            security: false,
            unreleased: false,
        }];
        assert!(!render_notes(&items, Lang::En).contains("Facade"));
        assert!(!render_notes(&items, Lang::De).contains("Facade"));
    }

    // ---- consume ----

    #[test]
    fn consume_folds_fragments_and_deletes_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap();
        fs::write(
            changes.join("b.md"),
            "---\ntype: fixed\n---\n[en]\nFixed it.\n[de]\nBehoben.\n",
        )
        .unwrap();

        consume(root, "0.2.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.beta.len(), 1);
        let e = &feed.channels.beta[0];
        assert_eq!(e.version, "0.2.0");
        assert_eq!(e.date, "2026-06-10");
        assert_eq!(e.items.len(), 2);
        assert!(e.notes.en.contains("### Added"));
        assert!(e.notes.de.contains("### Behoben"));
        // Fragments consumed (deleted).
        assert!(!changes.join("a.md").exists());
        assert!(!changes.join("b.md").exists());
    }

    #[test]
    fn consume_merges_into_existing_same_version_entry_no_data_loss() {
        // Regression for the double-fold / data-loss bug (85y.24): a second consume of the
        // same version must accumulate items, never replace (and so drop) the already-folded
        // ones from a prior run whose fragments were deleted.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap();
        fs::write(
            changes.join("b.md"),
            "---\ntype: fixed\n---\n[en]\nFixed it.\n[de]\nBehoben.\n",
        )
        .unwrap();

        consume(root, "0.2.0", "2026-06-10").unwrap();
        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.beta.len(), 1);
        assert_eq!(feed.channels.beta[0].items.len(), 2);

        // A genuinely new same-version fragment after the first run's fragments are gone.
        fs::write(
            changes.join("c.md"),
            "---\ntype: changed\n---\n[en]\nChanged it.\n[de]\nGeändert.\n",
        )
        .unwrap();
        consume(root, "0.2.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        // Exactly one 0.2.0 beta entry, and the prior two items are NOT lost.
        assert_eq!(feed.channels.beta.len(), 1);
        let e = &feed.channels.beta[0];
        assert_eq!(e.version, "0.2.0");
        assert_eq!(e.items.len(), 3);
    }

    #[test]
    fn consume_is_idempotent_against_leftover_fragment() {
        // A fragment whose delete failed last run (or a re-run for the same version) must not
        // duplicate its already-folded item.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap();

        consume(root, "0.2.0", "2026-06-10").unwrap();
        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.beta.len(), 1);
        assert_eq!(feed.channels.beta[0].items.len(), 1);
        assert!(!changes.join("a.md").exists());

        // Identical fragment reappears (simulating a failed delete) and is consumed again.
        fs::write(changes.join("a.md"), GOOD).unwrap();
        consume(root, "0.2.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.beta.len(), 1);
        // Deduped: still exactly one item, not two.
        assert_eq!(feed.channels.beta[0].items.len(), 1);
    }

    #[test]
    fn consume_then_promote_then_consume_keeps_both_rings_intact() {
        // Locks the consume→promote→consume interaction: a re-consume of an already-promoted
        // version must MERGE into beta (no loss) and must NOT disturb the promoted stable entry.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap(); // added
        fs::write(
            changes.join("b.md"),
            "---\ntype: fixed\n---\n[en]\nFixed it.\n[de]\nBehoben.\n",
        )
        .unwrap();

        consume(root, "0.2.0", "2026-06-10").unwrap();
        promote(root, "0.2.0").unwrap();

        // Stable now carries the promoted aggregate (2 items); beta still has its 2.
        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.stable.len(), 1);
        assert_eq!(feed.channels.stable[0].version, "0.2.0");
        assert_eq!(feed.channels.stable[0].items.len(), 2);

        // A late same-version fragment arrives and is consumed again.
        fs::write(
            changes.join("c.md"),
            "---\ntype: changed\n---\n[en]\nChanged it.\n[de]\nGeändert.\n",
        )
        .unwrap();
        consume(root, "0.2.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        // Beta merged the new item (3, no loss); stable is untouched by consume (still 2).
        assert_eq!(feed.channels.beta.len(), 1);
        assert_eq!(feed.channels.beta[0].items.len(), 3);
        assert_eq!(feed.channels.stable.len(), 1);
        assert_eq!(feed.channels.stable[0].items.len(), 2);
    }

    #[test]
    fn consume_rejects_malformed_fragment_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("bad.md"), "no frontmatter here").unwrap();

        assert!(consume(root, "0.2.0", "2026-06-10").is_err());
        assert!(!root.join("release-notes.json").exists());
        assert!(
            changes.join("bad.md").exists(),
            "fragment left intact on error"
        );
    }

    #[test]
    fn consume_writes_an_empty_beta_entry_when_no_fragments() {
        // tnce: a release with no `changes/*.md` fragments (a re-ship, a pure infra/CI patch) must
        // STILL get a beta feed entry — an empty `items: []` one — so `changelog promote` always
        // finds it and can never fail late in promote.yml (the half-promoted-state bug). "Every
        // released version is in the feed" becomes an invariant established at `version set` time,
        // BEFORE any GitHub release flip.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        consume(root, "0.2.0", "2026-06-10").unwrap();
        let feed = load_feed(root).unwrap();
        let entry = feed
            .channels
            .beta
            .iter()
            .find(|e| e.version == "0.2.0")
            .expect("a beta entry exists for the released version even with no fragments");
        assert!(entry.items.is_empty(), "no fragments → empty items");
        assert_eq!(entry.date, "2026-06-10");
        // Empty items render to empty notes — no dangling headings.
        assert_eq!(entry.notes.en, "");
        assert_eq!(entry.notes.de, "");
        // The promote-side guard now finds the entry the empty consume wrote — no late failure.
        assert_beta_entry(root, "0.2.0").expect("guard passes: the empty entry exists");
    }

    #[test]
    fn assert_beta_entry_fails_closed_when_the_version_has_no_feed_entry() {
        // tnce fail-fast guard: promote.yml runs this before any GitHub/AWS/feed mutation. A missing
        // beta entry must error (never a silent pass that lets a half-promotion proceed).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A feed exists, but only for a DIFFERENT version — the released one is absent.
        consume(root, "0.1.0", "2026-06-01").unwrap();
        let err = assert_beta_entry(root, "0.2.0").unwrap_err().to_string();
        assert!(
            err.contains("no beta feed entry for 0.2.0"),
            "message: {err}"
        );
        // The version that IS in the feed passes.
        assert_beta_entry(root, "0.1.0").expect("0.1.0 has an entry");
    }

    #[test]
    fn promote_succeeds_on_a_zero_item_beta_entry() {
        // tnce end-to-end (PR-review Test Quality #2): the fix's NAMED bug — "a no-fragment release
        // must not half-promote". A no-fragment `consume` writes an EMPTY beta entry; `promote` must
        // then lift it into stable WITHOUT the "no beta entry to promote" error that previously
        // stranded promote.yml half-way. This exercises the promote SEAM, not just consume's write.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        consume(root, "0.2.0", "2026-06-10").unwrap(); // no fragments -> empty beta entry
        promote(root, "0.2.0")
            .expect("promote lifts the zero-item entry into stable, no late failure");
        let feed = load_feed(root).unwrap();
        let st = feed
            .channels
            .stable
            .iter()
            .find(|e| e.version == "0.2.0")
            .expect("0.2.0 is now in the stable ring");
        assert!(
            st.items.is_empty(),
            "a zero-item beta promotes to a zero-item stable aggregate"
        );
    }

    #[test]
    fn consume_retains_migration_note_and_round_trips() {
        // A folded fragment's migration note must survive into the feed and round-trip through
        // serialize/deserialize (load_feed).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("m.md"), GOOD_MIG).unwrap();

        consume(root, "0.2.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        let item = &feed.channels.beta[0].items[0];
        let mig = item.migration.as_ref().expect("migration retained in feed");
        assert_eq!(mig.en, "Auto: schema migrates. Manual: re-auth.");
        assert_eq!(mig.de, "Automatik: Schema migriert. Manuell: neu anmelden.");

        // The serialized JSON carries the migration object (not skipped when Some).
        let json = fs::read_to_string(root.join("release-notes.json")).unwrap();
        assert!(json.contains("\"migration\""));
        assert!(json.contains("schema migrates"));
    }

    #[test]
    fn consume_retains_facade_and_security_and_round_trips() {
        // A facade-flagged, security fragment must fold into the feed carrying both fields, and
        // round-trip through serialize/deserialize (load_feed).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("f.md"),
            "---\ntype: changed\nfacade: breaking\nsecurity: true\n---\n[en]\nx\n[de]\ny\n",
        )
        .unwrap();

        consume(root, "0.8.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        let item = &feed.channels.beta[0].items[0];
        assert_eq!(item.facade, Some(FacadeImpact::Breaking));
        assert!(item.security);

        // Serialized JSON carries both flags (machine-readable signal for the consumer).
        let json = fs::read_to_string(root.join("release-notes.json")).unwrap();
        assert!(json.contains("\"facade\": \"breaking\""), "got: {json}");
        assert!(json.contains("\"security\": true"), "got: {json}");
        // And the dedicated rendered section is present in the stored notes.
        assert!(feed.channels.beta[0]
            .notes
            .en
            .contains("### Facade Contract"));
    }

    #[test]
    fn consume_drops_facade_none_so_the_feed_contract_stays_two_valued() {
        // `facade: none` is an assertion about the PR ("checked, no impact"), not a property of
        // the release (6j6v.gmjd). A consumer parses the feed's `facade` field as
        // `changed|breaking`; widening it to a third value would be the very kind of unannounced
        // contract change this ticket exists to stop. So the value must survive the PR gate and
        // die at the fold — including in the rendered notes, where `- none · …` would be noise.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("f.md"),
            "---\ntype: changed\nfacade: none\n---\n[en]\nx\n[de]\ny\n",
        )
        .unwrap();

        consume(root, "0.8.0", "2026-06-10").unwrap();

        let feed = load_feed(root).unwrap();
        assert_eq!(feed.channels.beta[0].items[0].facade, None);
        let json = fs::read_to_string(root.join("release-notes.json")).unwrap();
        assert!(!json.contains("facade"), "got: {json}");
        assert!(!feed.channels.beta[0]
            .notes
            .en
            .contains("### Facade Contract"));
    }

    #[test]
    fn consume_carries_unreleased_from_fragment_into_beta_and_stable_drops_it() {
        // decision 8a2c, the AUTHORING path (the retro path is asserted separately, further down):
        // a fragment flagged `unreleased: true` folds into the beta entry like any other — the ring
        // is the append-only truth — and is exactly the item the stable rollup leaves out. Both
        // halves are asserted here because "beta keeps it, stable drops it" is one claim; testing
        // only the second half would also pass on a build that never wrote the item at all.
        //
        // MUTATION-CHECKED, recorded rather than claimed — one perturbation per change site of
        // 8a2c, each seen red here and reverted: removing the `"unreleased"` frontmatter arm from
        // `parse_fragment`, replacing `unreleased: frag.unreleased` with `false` in
        // `read_fragments`, and dropping `.filter(|i| !i.unreleased)` from `aggregate_items` each
        // fail THIS test; dropping `skip_serializing_if` from `Item::unreleased` fails the
        // byte-identity test below. No site is covered only on paper.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("a-real-change.md"),
            "---\ntype: added\n---\n[en]\nreal\n[de]\nrichtig\n",
        )
        .unwrap();
        fs::write(
            changes.join("b-beta-only-fix.md"),
            "---\ntype: fixed\nunreleased: true\n---\n[en]\nbeta-only churn\n[de]\nnur beta\n",
        )
        .unwrap();

        consume(root, "0.2.0", "2026-02-01").unwrap();

        // Beta ring, read back through the feed (so the flag is proven to survive serialization).
        let feed = load_feed(root).unwrap();
        let beta = &feed.channels.beta[0];
        assert_eq!(beta.items.len(), 2, "the beta entry keeps both items");
        let flagged = beta
            .items
            .iter()
            .find(|i| i.en == "beta-only churn")
            .expect("the flagged item is in the beta entry");
        assert!(
            flagged.unreleased,
            "the fragment flag reached the feed item"
        );
        assert!(
            beta.notes.en.contains("beta-only churn"),
            "beta notes still render it: {}",
            beta.notes.en
        );

        // Stable rollup: the flagged item is gone, the real one stays.
        let out = aggregate(root, "0.1.0", "0.2.0", Lang::En).unwrap();
        assert_eq!(
            out, "### Added\n- real",
            "the `unreleased` fragment is omitted from the stable aggregate"
        );
    }

    #[test]
    fn item_without_facade_omits_fields_in_json() {
        // A non-facade, non-security item must serialize without the facade/security keys, so an
        // existing release-notes.json shape is unchanged for ordinary items.
        let item = Item {
            change_type: ChangeType::Added,
            en: "A".into(),
            de: "A-de".into(),
            migration: None,
            facade: None,
            security: false,
            unreleased: false,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("facade"), "got: {json}");
        assert!(!json.contains("security"), "got: {json}");
    }

    #[test]
    fn old_feed_without_migration_field_deserializes() {
        // Backward compatibility: an existing release-notes.json whose items predate the
        // migration field must still load (`#[serde(default)]`), with migration == None.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("release-notes.json"),
            r#"{"channels":{"alpha":[],"beta":[{"version":"0.2.0","date":"2026-01-01","items":[{"type":"added","en":"A","de":"A-de"}],"notes":{"en":"","de":""}}],"stable":[]}}"#,
        )
        .unwrap();
        let feed = load_feed(root).unwrap();
        let item = &feed.channels.beta[0].items[0];
        assert_eq!(item.migration, None);
        // The same backward-compat guarantee for the aye.15 fields: absent → default.
        assert_eq!(item.facade, None);
        assert!(!item.security);
    }

    #[test]
    fn feed_predating_the_unreleased_field_round_trips_byte_identically() {
        // decision 8a2c: the field is purely additive. The committed `release-notes.json` predates
        // it, and every `version set` / `promote` rewrites the WHOLE file — so "old entries are
        // unaffected" only holds if load→save is a byte-for-byte identity: the field absent on the
        // way in, absent (not `false`) on the way out. A `contains` check would not state that;
        // equality against a frozen pre-feature sample does. The sample is written in `save_feed`'s
        // own shape (2-space pretty JSON, trailing newline) because that is what is on disk.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // `r####"…"####`: the rendered notes carry `"###` (a markdown heading right after a JSON
        // quote), which would terminate a raw string delimited by three `#` or fewer.
        let before = r####"{
  "channels": {
    "alpha": [],
    "beta": [
      {
        "version": "0.2.0",
        "date": "2026-02-01",
        "items": [
          {
            "type": "added",
            "en": "A",
            "de": "A-de"
          }
        ],
        "notes": {
          "en": "### Added\n- A",
          "de": "### Neu\n- A"
        }
      }
    ],
    "stable": []
  }
}
"####;
        let path = root.join("release-notes.json");
        fs::write(&path, before).unwrap();

        let feed = load_feed(root).unwrap();
        assert!(
            !feed.channels.beta[0].items[0].unreleased,
            "an absent field reads as false"
        );
        save_feed(root, &feed).unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "a feed without the field must come back byte-identical"
        );
    }

    #[test]
    fn item_without_migration_omits_field_in_json() {
        // A None migration must not appear in serialized output (skip_serializing_if), so the
        // feed shape for migration-less items is unchanged from before this feature.
        let item = Item {
            change_type: ChangeType::Added,
            en: "A".into(),
            de: "A-de".into(),
            migration: None,
            facade: None,
            security: false,
            unreleased: false,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("migration"), "got: {json}");
    }

    // ---- requires_migration_note (predicate truth table) ----

    #[test]
    fn requires_migration_note_truth_table() {
        // 0.x bumps never require a note (0.x may break freely).
        assert!(!requires_migration_note("0.1.0", "0.2.0").unwrap());
        assert!(!requires_migration_note("0.1.0", "0.1.1").unwrap());
        assert!(!requires_migration_note("0.9.9", "0.10.0").unwrap());
        // Reaching 1.0.0 from 0.x IS a real major bump (major increased to ≥1).
        assert!(requires_migration_note("0.9.0", "1.0.0").unwrap());
        // Same-major ≥1 bumps (minor/patch) do not require a note.
        assert!(!requires_migration_note("1.2.0", "1.3.0").unwrap());
        assert!(!requires_migration_note("1.2.0", "1.2.1").unwrap());
        // A real major bump within ≥1 requires a note.
        assert!(requires_migration_note("1.5.0", "2.0.0").unwrap());
        assert!(requires_migration_note("1.0.0", "3.0.0").unwrap());
        // A major DECREASE is not a major bump (predicate requires next.major > prev.major).
        assert!(!requires_migration_note("2.0.0", "1.0.0").unwrap());
        assert!(!requires_migration_note("2.3.1", "1.0.0").unwrap());
        // Non-plain-SemVer is a loud error, not a silent false.
        assert!(requires_migration_note("0.1", "1.0.0").is_err());
        assert!(requires_migration_note("0.1.0", "v1.0.0").is_err());
    }

    // ---- enforce_major_bump_migration (the gate) ----

    /// Minimal workspace with a `[workspace.package] version`, so `workspace_version` reads it.
    fn scaffold_workspace(root: &Path, version: &str) {
        fs::write(
            root.join("Cargo.toml"),
            format!("[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\n"),
        )
        .unwrap();
    }

    #[test]
    fn gate_passes_for_minor_bump_without_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.1.0");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        // No migration note, but 0.1.0 → 0.2.0 imposes no requirement (0.x may break freely).
        fs::write(changes.join("a.md"), GOOD).unwrap();
        enforce_major_bump_migration(root, "0.2.0").unwrap();
    }

    #[test]
    fn gate_passes_reaching_1_0_0_with_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.9.0");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("m.md"), GOOD_MIG).unwrap();
        enforce_major_bump_migration(root, "1.0.0").unwrap();
    }

    #[test]
    fn gate_fails_reaching_1_0_0_without_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.9.0");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap(); // no migration
        let err = enforce_major_bump_migration(root, "1.0.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("migration note"), "got: {err}");
    }

    #[test]
    fn gate_fails_major_bump_within_stable_without_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "1.5.0");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap();
        assert!(enforce_major_bump_migration(root, "2.0.0").is_err());
    }

    #[test]
    fn gate_passes_major_bump_within_stable_with_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "1.5.0");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("a.md"), GOOD).unwrap(); // no migration
        fs::write(changes.join("m.md"), GOOD_MIG).unwrap(); // one WITH migration → passes
        enforce_major_bump_migration(root, "2.0.0").unwrap();
    }

    // ---- enforce_facade_breaking_axis (the facade-side gate, nexus-flow-aye.15) ----

    const FACADE_BREAKING: &str = "---\ntype: changed\nfacade: breaking\n---\n[en]\nx\n[de]\ny\n";

    #[test]
    fn facade_axis_gate_fails_patch_bump_with_breaking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.7.1");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("f.md"), FACADE_BREAKING).unwrap();
        let err = enforce_facade_breaking_axis(root, "0.7.2")
            .unwrap_err()
            .to_string();
        assert!(err.contains("facade"), "got: {err}");
    }

    #[test]
    fn facade_axis_gate_passes_patch_bump_with_changed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.7.1");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("f.md"),
            "---\ntype: changed\nfacade: changed\n---\n[en]\nx\n[de]\ny\n",
        )
        .unwrap();
        enforce_facade_breaking_axis(root, "0.7.2").unwrap();
    }

    #[test]
    fn facade_axis_gate_passes_minor_bump_with_breaking() {
        // A minor IS the break axis (§4.3) — a facade break is allowed there.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.7.1");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(changes.join("f.md"), FACADE_BREAKING).unwrap();
        enforce_facade_breaking_axis(root, "0.8.0").unwrap();
    }

    #[test]
    fn facade_axis_gate_passes_patch_bump_security_only() {
        // A security patch (no facade break) is exactly the patch-lane use case — must pass.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.7.1");
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("f.md"),
            "---\ntype: fixed\nsecurity: true\n---\n[en]\nx\n[de]\ny\n",
        )
        .unwrap();
        enforce_facade_breaking_axis(root, "0.7.2").unwrap();
    }

    #[test]
    fn facade_axis_gate_passes_patch_bump_without_fragments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scaffold_workspace(root, "0.7.1");
        enforce_facade_breaking_axis(root, "0.7.2").unwrap();
    }

    // ---- migration-guide aggregation ----

    /// Build an entry whose single item carries a migration note.
    fn entry_with_migration(version: &str, date: &str, mig_en: &str, mig_de: &str) -> Entry {
        let items = vec![Item {
            change_type: ChangeType::Changed,
            en: "c".into(),
            de: "c-de".into(),
            migration: Some(Migration {
                en: mig_en.into(),
                de: mig_de.into(),
            }),
            facade: None,
            security: false,
            unreleased: false,
        }];
        build_entry(version, date, items)
    }

    #[test]
    fn migration_guide_folds_one_major_ordered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        // Out-of-order insertion; guide must emit ascending by version.
        feed.channels.stable.push(entry_with_migration(
            "1.2.0",
            "2026-03-01",
            "Step C",
            "Schritt C",
        ));
        feed.channels.stable.push(entry_with_migration(
            "1.0.0",
            "2026-01-01",
            "Step A",
            "Schritt A",
        ));
        // A 2.x entry must NOT appear in the 1.x guide.
        feed.channels.stable.push(entry_with_migration(
            "2.0.0",
            "2026-05-01",
            "Step Z",
            "Schritt Z",
        ));
        // A 1.x entry with no migration note is skipped.
        feed.channels
            .stable
            .push(entry("1.1.0", "2026-02-01", ChangeType::Fixed, "f", "f-de"));
        write_feed(root, &feed);

        let en = migration_guide(root, 1, Lang::En).unwrap();
        assert_eq!(en, "## 1.0.0\n- Step A\n\n## 1.2.0\n- Step C");
        let de = migration_guide(root, 1, Lang::De).unwrap();
        assert_eq!(de, "## 1.0.0\n- Schritt A\n\n## 1.2.0\n- Schritt C");
    }

    #[test]
    fn migration_guide_empty_when_no_notes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        feed.channels
            .stable
            .push(entry("1.0.0", "2026-01-01", ChangeType::Added, "a", "a-de"));
        write_feed(root, &feed);
        assert_eq!(migration_guide(root, 1, Lang::En).unwrap(), "");
    }

    #[test]
    fn channel_and_lang_parse_reject_unknown() {
        assert!(Channel::parse("beta").is_ok());
        assert!(Lang::parse("de").is_ok());
        assert!(Channel::parse("nightly").is_err());
        assert!(Channel::parse("").is_err());
        assert!(Lang::parse("fr").is_err());
        assert!(Lang::parse("EN").is_err());
    }

    #[test]
    fn validate_fragments_accepts_good_and_rejects_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        // No fragments yet → vacuously valid (a no-op release is allowed).
        validate_fragments(root).unwrap();

        fs::write(changes.join("good.md"), GOOD).unwrap();
        validate_fragments(root).unwrap();

        fs::write(changes.join("bad.md"), "no frontmatter").unwrap();
        assert!(validate_fragments(root).is_err());
    }

    #[test]
    fn fragment_scan_ignores_readme() {
        // changes/README.md documents the format; it is not a fragment and must never be parsed
        // (it has no frontmatter) or folded into the feed.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("README.md"),
            "# Changelog fragments\n\nDocs, not a fragment.\n",
        )
        .unwrap();
        fs::write(changes.join("good.md"), GOOD).unwrap();

        // The README is skipped, so validation passes and only the real fragment is scanned.
        validate_fragments(root).unwrap();
        let paths = fragment_paths(root).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("good.md"));
    }

    #[test]
    fn load_feed_rejects_non_plain_semver_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A hand-edited feed with a channel suffix must fail loudly on load, so every code
        // path (sort, aggregate, promote) sees the same strict policy.
        fs::write(
            root.join("release-notes.json"),
            r#"{"channels":{"alpha":[],"beta":[{"version":"0.2.0-rc1","date":"2026-01-01","items":[],"notes":{"en":"","de":""}}],"stable":[]}}"#,
        )
        .unwrap();
        assert!(load_feed(root).is_err());
    }

    // ---- show / inheritance ----

    // ---- the curated stable rollup (promoting v0.63.0) ----------------------------------------

    /// A workspace whose beta ring carries one 0.2.0 entry with a MECHANICAL item — the union a
    /// curated rollup has to replace. Built through `consume`, so the fixture is the real path.
    fn seed_feed_with_beta(root: &Path) {
        let changes = root.join("changes");
        fs::create_dir_all(&changes).unwrap();
        fs::write(
            changes.join("a.md"),
            "---\ntype: added\n---\n[en]\nA mechanical line.\n[de]\nEine mechanische Zeile.\n",
        )
        .unwrap();
        consume(root, "0.2.0", "2026-06-10").unwrap();
    }

    /// Write a curated rollup item for `version` into `changes/stable/<version>/`.
    fn write_curated(root: &Path, version: &str, name: &str, body: &str) {
        let dir = root.join("changes").join("stable").join(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.md")), body).unwrap();
    }

    const CURATED: &str =
        "---\ntype: added\n---\n[en]\nOne stable line.\n[de]\nEine stabile Zeile.\n";

    #[test]
    fn a_curated_rollup_replaces_the_mechanical_union_in_both_paths() {
        // The whole point: `aggregate` feeds the MANIFEST and `promote` feeds the FEED, and before
        // this they were two independent re-derivations from the beta ring. A rollup that won only
        // one of them would ship a curated website and a mechanical `self-update`.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        seed_feed_with_beta(root);
        write_curated(root, "0.2.0", "01-one", CURATED);

        let rendered = aggregate(root, "0.1.0", "0.2.0", Lang::En).unwrap();
        assert!(rendered.contains("One stable line."), "{rendered}");
        assert!(
            !rendered.contains("mechanical"),
            "the union must not survive beside it: {rendered}"
        );

        promote(root, "0.2.0").unwrap();
        let feed = load_feed(root).unwrap();
        let entry = feed
            .channels
            .stable
            .iter()
            .find(|e| e.version == "0.2.0")
            .expect("promoted");
        assert_eq!(entry.items.len(), 1, "the curated items ARE the entry's");
        assert!(entry.notes.en.contains("One stable line."));
        assert!(entry.notes.de.contains("Eine stabile Zeile."));
    }

    #[test]
    fn a_curated_entry_still_satisfies_the_notes_derivation_gate() {
        // `notes` stays DERIVED from `items` — that is why the rollup is authored as items and not
        // as rendered copy. If this ever fails, `changelog rerender --check` would rewrite a
        // curated release back into the mechanical union on the next CI run.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        seed_feed_with_beta(root);
        write_curated(root, "0.2.0", "01-one", CURATED);
        promote(root, "0.2.0").unwrap();

        assert!(
            rerender(root, true).unwrap().is_empty(),
            "a curated entry must not read as drift"
        );
    }

    #[test]
    fn no_curated_directory_means_the_mechanical_union_exactly_as_before() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        seed_feed_with_beta(root);

        let rendered = aggregate(root, "0.1.0", "0.2.0", Lang::En).unwrap();
        assert!(rendered.contains("mechanical"), "{rendered}");
    }

    #[test]
    fn a_curated_rollup_is_never_mistaken_for_a_pending_fragment() {
        // `changes/*.md` is read non-recursively, which is the whole reason the rollup lives in a
        // subdirectory: `version set` must not consume it into the next beta entry.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("changes")).unwrap();
        write_curated(root, "0.2.0", "01-one", CURATED);

        assert!(
            read_fragments(root).unwrap().is_empty(),
            "the fragment scan must not see it"
        );
    }

    #[test]
    fn show_renders_channel_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        feed.channels
            .beta
            .push(entry("0.2.0", "2026-06-10", ChangeType::Added, "A", "A-de"));
        write_feed(root, &feed);
        let out = show(root, "0.2.0", Channel::Beta, Lang::En).unwrap();
        assert_eq!(out, "### Added\n- A");
    }

    #[test]
    fn show_inherits_from_more_stable_ring() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        // Only stable has the entry; beta should inherit it.
        feed.channels
            .stable
            .push(entry("0.2.0", "2026-06-10", ChangeType::Fixed, "F", "F-de"));
        write_feed(root, &feed);
        let out = show(root, "0.2.0", Channel::Beta, Lang::De).unwrap();
        assert_eq!(out, "### Behoben\n- F-de");
    }

    #[test]
    fn show_errors_when_version_absent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_feed(root, &Feed::default());
        assert!(show(root, "9.9.9", Channel::Beta, Lang::En).is_err());
    }

    // ---- prev-stable / aggregate / promote ----

    #[test]
    fn prev_stable_finds_latest_below() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        feed.channels
            .stable
            .push(entry("0.1.0", "2026-01-01", ChangeType::Added, "x", "x"));
        feed.channels
            .stable
            .push(entry("0.3.0", "2026-03-01", ChangeType::Added, "y", "y"));
        write_feed(root, &feed);
        assert_eq!(prev_stable(root, "0.4.0").unwrap(), Some("0.3.0".into()));
        assert_eq!(prev_stable(root, "0.2.0").unwrap(), Some("0.1.0".into()));
        assert_eq!(prev_stable(root, "0.1.0").unwrap(), None);
    }

    #[test]
    fn aggregate_collects_beta_in_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        feed.channels.beta.push(entry(
            "0.2.0",
            "2026-02-01",
            ChangeType::Added,
            "two",
            "zwei",
        ));
        feed.channels.beta.push(entry(
            "0.3.0",
            "2026-03-01",
            ChangeType::Fixed,
            "three",
            "drei",
        ));
        feed.channels.beta.push(entry(
            "0.1.0",
            "2026-01-01",
            ChangeType::Added,
            "one",
            "eins",
        ));
        write_feed(root, &feed);
        // since 0.1.0 up to 0.3.0 => includes 0.2.0 and 0.3.0, not 0.1.0.
        let out = aggregate(root, "0.1.0", "0.3.0", Lang::En).unwrap();
        assert_eq!(out, "### Added\n- two\n\n### Fixed\n- three");
    }

    #[test]
    fn aggregate_omits_unreleased_items_but_the_beta_ring_keeps_them() {
        // decision 8a2c, the PROMOTER-RETRO path: intra-window churn often only becomes apparent at
        // promote time, so the flag is set straight on the beta `Item` in the feed — no fragment
        // involved (they are deleted by `version set` long before). That must filter exactly like a
        // fragment-authored flag: the STABLE rollup drops it (net-diff), the beta ring keeps the
        // full append-only truth.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        let items = vec![
            Item {
                change_type: ChangeType::Added,
                en: "real".into(),
                de: "echt".into(),
                migration: None,
                facade: None,
                security: false,
                unreleased: false,
            },
            Item {
                change_type: ChangeType::Fixed,
                en: "beta-only churn".into(),
                de: "nur beta".into(),
                migration: None,
                facade: None,
                security: false,
                unreleased: true,
            },
        ];
        let notes = Notes {
            en: render_notes(&items, Lang::En),
            de: render_notes(&items, Lang::De),
        };
        feed.channels.beta.push(Entry {
            version: "0.2.0".into(),
            date: "2026-02-01".into(),
            items,
            notes,
        });
        write_feed(root, &feed);

        // Stable aggregate keeps the real item, drops the `unreleased` one.
        let out = aggregate(root, "0.1.0", "0.2.0", Lang::En).unwrap();
        assert_eq!(
            out, "### Added\n- real",
            "unreleased item omitted from the stable aggregate"
        );
        assert!(!out.contains("beta-only churn"));
        // The beta ring still carries both — read back off disk, not off the `feed` we just built,
        // so this asserts the stored truth rather than restating the fixture.
        let stored = load_feed(root).unwrap();
        assert_eq!(
            stored.channels.beta[0].items.len(),
            2,
            "the beta entry keeps both items"
        );
        assert!(
            stored.channels.beta[0].items[1].unreleased,
            "the retro flag survives the feed round-trip"
        );
    }

    #[test]
    fn backport_promotion_is_line_scoped_across_parallel_lines() {
        // Maintenance-line release mechanics (nexus-flow-aye.18): two active lines coexist in the
        // feed (0.6.x and the newer 0.7.x). Promoting a backport 0.6.3 must aggregate WITHIN the
        // 0.6 line — its predecessor is 0.6.2 (not the globally-higher 0.7.1), and the aggregate
        // must not pull 0.7.x items. This is the line-awareness promote.yml relies on.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        for (v, en) in [
            ("0.6.2", "six-two"),
            ("0.6.3", "six-three (backport)"),
            ("0.7.0", "seven-zero"),
            ("0.7.1", "seven-one"),
        ] {
            feed.channels
                .beta
                .push(entry(v, "2026-06-01", ChangeType::Fixed, en, en));
        }
        for v in ["0.6.2", "0.7.0", "0.7.1"] {
            feed.channels
                .stable
                .push(entry(v, "2026-06-01", ChangeType::Fixed, v, v));
        }
        write_feed(root, &feed);

        // The predecessor of the backport is the 0.6-line stable, not the higher 0.7.1.
        assert_eq!(
            prev_stable(root, "0.6.3").unwrap().as_deref(),
            Some("0.6.2")
        );

        // Aggregate the backport against its line predecessor: only the 0.6.3 item, no 0.7.x.
        let prev = prev_stable(root, "0.6.3").unwrap().unwrap();
        let notes = aggregate(root, &prev, "0.6.3", Lang::En).unwrap();
        assert!(notes.contains("six-three (backport)"), "got: {notes}");
        assert!(
            !notes.contains("seven"),
            "leaked a newer-line item: {notes}"
        );

        // Promote the backport: a stable 0.6.3 appears, the existing 0.7.x stable entries survive.
        promote(root, "0.6.3").unwrap();
        let feed = load_feed(root).unwrap();
        let stable: Vec<&str> = feed
            .channels
            .stable
            .iter()
            .map(|e| e.version.as_str())
            .collect();
        assert!(
            stable.contains(&"0.6.3"),
            "backport not promoted: {stable:?}"
        );
        assert!(
            stable.contains(&"0.7.1"),
            "newer line clobbered: {stable:?}"
        );
    }

    #[test]
    fn line_scoping_holds_with_interleaved_entries_and_a_gap() {
        // Stronger than the happy path (PR review #114): stable entries from THREE lines
        // interleaved in non-sorted insertion order, a multi-patch gap within the old line, and a
        // second backport. prev_stable + aggregate must still pick the per-line predecessor and
        // never reach across a line boundary — the SemVer-ordering invariant documented on
        // prev_stable/aggregate_items, not merely "backports sort below the next minor".
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        // Deliberately out of order, across 0.6 / 0.7 / 0.8, with a gap in the 0.6 line.
        for (v, en) in [
            ("0.8.0", "eight-zero"),
            ("0.6.1", "six-one"),
            ("0.7.1", "seven-one"),
            ("0.6.2", "six-two (backport)"),
            ("0.6.0", "six-zero"),
            ("0.7.0", "seven-zero"),
        ] {
            feed.channels
                .beta
                .push(entry(v, "2026-06-01", ChangeType::Fixed, en, en));
        }
        // Stable already carries 0.6.0/0.6.1 on the old line plus the newer lines — but NOT 0.6.2.
        for v in ["0.6.0", "0.6.1", "0.7.0", "0.7.1", "0.8.0"] {
            feed.channels
                .stable
                .push(entry(v, "2026-06-01", ChangeType::Fixed, v, v));
        }
        write_feed(root, &feed);

        // Predecessor of the 0.6.2 backport is 0.6.1 — never 0.7.x/0.8.x, even though they are
        // numerically higher and interleaved in the array.
        assert_eq!(
            prev_stable(root, "0.6.2").unwrap().as_deref(),
            Some("0.6.1")
        );
        let prev = prev_stable(root, "0.6.2").unwrap().unwrap();
        let notes = aggregate(root, &prev, "0.6.2", Lang::En).unwrap();
        assert!(notes.contains("six-two (backport)"), "got: {notes}");
        assert!(!notes.contains("seven"), "leaked a 0.7 item: {notes}");
        assert!(!notes.contains("eight"), "leaked a 0.8 item: {notes}");

        // And the forward-minor direction still spans lines: 0.8.0's predecessor is 0.7.1, so its
        // stable aggregate is "everything since the last stable" (the cross-line semantics a
        // major.minor filter would have broken).
        assert_eq!(
            prev_stable(root, "0.8.0").unwrap().as_deref(),
            Some("0.7.1")
        );
    }

    #[test]
    fn promote_lifts_beta_into_stable_as_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut feed = Feed::default();
        feed.channels.stable.push(entry(
            "0.1.0",
            "2026-01-01",
            ChangeType::Added,
            "old",
            "alt",
        ));
        feed.channels.beta.push(entry(
            "0.2.0",
            "2026-02-01",
            ChangeType::Added,
            "two",
            "zwei",
        ));
        feed.channels.beta.push(entry(
            "0.3.0",
            "2026-03-01",
            ChangeType::Fixed,
            "three",
            "drei",
        ));
        write_feed(root, &feed);

        promote(root, "0.3.0").unwrap();

        let feed = load_feed(root).unwrap();
        // newest stable first
        let s = &feed.channels.stable[0];
        assert_eq!(s.version, "0.3.0");
        assert_eq!(s.date, "2026-03-01");
        // aggregates 0.2.0 + 0.3.0 (since prev stable 0.1.0)
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.notes.en, "### Added\n- two\n\n### Fixed\n- three");
    }

    #[test]
    fn promote_errors_without_beta_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_feed(root, &Feed::default());
        assert!(promote(root, "0.3.0").is_err());
    }

    // ---- today / env override ----

    #[test]
    fn today_honors_env_override() {
        // Safety: single-threaded within this test; we set then read immediately.
        std::env::set_var("XTASK_DATE", "2030-12-31");
        assert_eq!(today(), "2030-12-31");
        std::env::remove_var("XTASK_DATE");
    }

    // ---- check (real git) ----

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("README"), "base\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn check_passes_with_added_fragment() {
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(root.join("changes").join("x.md"), GOOD).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "add fragment"]);
        check(root, "main").unwrap();
    }

    #[test]
    fn check_fails_without_fragment() {
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::write(root.join("src.txt"), "code\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "no fragment"]);
        assert!(check(root, "main").is_err());
    }

    #[test]
    fn check_fails_when_readme_is_the_only_changelog_file_touched() {
        // changes/README.md is docs, not a fragment — touching it must NOT satisfy the
        // real-fragment requirement (the trigger agrees with the fragment scan that excludes it).
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(root.join("changes").join("README.md"), "# docs\n").unwrap();
        // Paired with a shipping change: README alone ships nothing and passes as no product
        // impact (6j6v.c37e), so only this pairing still asks whether README counts as a fragment.
        write_at(root, "crates/core/src/lib.rs", "// code\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "touch readme only"]);
        assert!(check(root, "main").is_err());
    }

    #[test]
    fn check_fails_on_malformed_fragment() {
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(root.join("changes").join("bad.md"), "garbage").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "bad fragment"]);
        assert!(check(root, "main").is_err());
    }

    #[test]
    fn check_passes_with_well_formed_migration() {
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(root.join("changes").join("m.md"), GOOD_MIG).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "add migration fragment"]);
        check(root, "main").unwrap();
    }

    // ---- check: the explicit-facade-verdict rule (6j6v.gmjd, real git) ----

    /// Write `rel` (creating parent dirs) inside the temp repo.
    fn write_at(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A PR replaying the shape of the two real incidents: unchanged signatures, changed
    /// behaviour, across all three consumed surfaces — plus whatever fragments the caller passes
    /// as `(name, body)`.
    fn behavioural_break_pr(root: &Path, fragments: &[(&str, &str)]) {
        git(root, &["checkout", "-q", "-b", "feature"]);
        // Bodies stand in for PR #311/#314's `validate_author` tightening: same signature, a value
        // that used to be accepted now rejected.
        write_at(
            root,
            "crates/facade/src/validate.rs",
            "pub fn author(a: &str) -> Result<&str, ()> { if a.trim().is_empty() { Err(()) } \
             else { Ok(a) } }\n",
        );
        write_at(
            root,
            "crates/memory/src/facade.rs",
            "// delegates to author()\n",
        );
        write_at(
            root,
            "crates/chat/src/facade.rs",
            "// delegates to author()\n",
        );
        for (name, body) in fragments {
            write_at(root, &format!("changes/{name}"), body);
        }
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "tighten actor validation"]);
    }

    #[test]
    fn check_fails_when_a_surface_is_touched_and_the_fragment_says_nothing() {
        // THE regression this ticket is about (6j6v.gmjd DoD): PR #311/#314 replayed. Both shipped
        // a behavioural break at 26 public write entry points behind unchanged signatures with a
        // marker-less fragment, and every gate stayed green — cargo-semver-checks because no
        // signature moved, enforce_facade_breaking_axis because it only fires on a marker that IS
        // there. This case must now be red.
        let dir = init_repo();
        let root = dir.path();
        behavioural_break_pr(root, &[("actor.md", GOOD)]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("facade:"), "got: {err}");
        assert!(err.contains("changes/actor.md"), "got: {err}");
        // The message names what triggered it, so the author does not have to guess which of their
        // paths is a consumed surface.
        assert!(err.contains("crates/facade/src/validate.rs"), "got: {err}");
    }

    #[test]
    fn check_passes_when_the_fragment_answers_none() {
        // The cheap, honest answer: "I looked, the consumed contract does not move."
        let dir = init_repo();
        let root = dir.path();
        let frag = "---\ntype: changed\nfacade: none\n---\n[en]\nx\n[de]\ny\n";
        behavioural_break_pr(root, &[("actor.md", frag)]);
        check(root, "main").unwrap();
    }

    #[test]
    fn check_passes_when_the_fragment_answers_breaking() {
        // Any EXPLICIT verdict satisfies the rule — the gate asks the question, §4.3's axis rule
        // (enforce_facade_breaking_axis, at `version set`) decides what the answer costs.
        let dir = init_repo();
        let root = dir.path();
        let frag = "---\ntype: changed\nfacade: breaking\n---\n[en]\nx\n[de]\ny\n";
        behavioural_break_pr(root, &[("actor.md", frag)]);
        check(root, "main").unwrap();
    }

    #[test]
    fn check_demands_a_verdict_from_every_fragment_the_pr_touches() {
        // Not "at least one fragment says something": PR #312 had to split this very change into
        // several fragments by hand. A rule satisfied by marking the innocent one would leave the
        // breaking one exactly as silent as before.
        let dir = init_repo();
        let root = dir.path();
        let marked = "---\ntype: changed\nfacade: none\n---\n[en]\nx\n[de]\ny\n";
        behavioural_break_pr(root, &[("marked.md", marked), ("silent.md", GOOD)]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("changes/silent.md"), "got: {err}");
    }

    #[test]
    fn check_demands_a_verdict_for_the_contract_critical_file_outside_the_surfaces() {
        // The package heuristic alone would miss this: `validate_author` lives in
        // crates/foundation, and tightening it is by construction a behavioural break at all 26
        // write entry points of the three surfaces without touching any of their directories.
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        write_at(
            root,
            "crates/foundation/src/model.rs",
            "pub fn validate_author(a: &str) -> bool { !a.trim().is_empty() }\n",
        );
        write_at(root, "changes/actor.md", GOOD);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "tighten validate_author"]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("crates/foundation/src/model.rs"), "got: {err}");
    }

    #[test]
    fn check_demands_a_verdict_when_the_pr_only_deletes_from_a_consumed_surface() {
        // PR #315 review, Integrity & Robustness #1. The add/modify view (`--diff-filter=ACMR`)
        // cannot see a deletion, so a deprecate-now-delete-later cleanup used to satisfy nothing
        // and be asked nothing — while removing a `pub` item is a break of the most obvious kind.
        // On a minor bump cargo-semver-checks is report-only, so no other gate would have spoken.
        let dir = init_repo();
        let root = dir.path();
        write_at(root, "crates/facade/src/legacy.rs", "pub fn gone() {}\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base with a facade file"]);

        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::remove_file(root.join("crates/facade/src/legacy.rs")).unwrap();
        write_at(root, "changes/cleanup.md", GOOD);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "drop the deprecated seam"]);

        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("crates/facade/src/legacy.rs"), "got: {err}");
    }

    #[test]
    fn check_demands_a_verdict_when_a_file_is_moved_out_of_a_consumed_surface() {
        // Same finding, its other half: git reports a rename by its NEW path only, so a file moved
        // out of a surface would be seen at its unguarded destination and nowhere else. The gate's
        // membership query passes `--no-renames` so the vacated path still shows up.
        let dir = init_repo();
        let root = dir.path();
        write_at(root, "crates/memory/src/leaving.rs", "pub fn m() {}\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base with a memory file"]);

        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::remove_file(root.join("crates/memory/src/leaving.rs")).unwrap();
        write_at(root, "crates/core/src/leaving.rs", "pub fn m() {}\n");
        write_at(root, "changes/move.md", GOOD);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "move it out of memory"]);

        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("crates/memory/src/leaving.rs"), "got: {err}");
    }

    #[test]
    fn check_asks_for_no_verdict_when_the_pr_touches_no_consumed_surface() {
        // The rule must stay narrow — an unrelated PR keeps writing plain fragments, or the marker
        // degrades into a reflex nobody reads.
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        write_at(root, "crates/core/src/lib.rs", "// internal\n");
        write_at(root, "changes/x.md", GOOD);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "internal change"]);
        check(root, "main").unwrap();
    }

    #[test]
    fn check_fails_on_half_translated_migration() {
        // A fragment with `[migration.en]` but no `[migration.de]` must fail the PR gate, since
        // `check` parses every present fragment and `parse_fragment` rejects a half-present
        // migration block.
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("changes")).unwrap();
        fs::write(
            root.join("changes").join("bad-mig.md"),
            "---\ntype: changed\n---\n[en]\nx\n[de]\ny\n[migration.en]\nonly english\n",
        )
        .unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "half migration"]);
        assert!(check(root, "main").is_err());
    }

    // ---- no product impact (6j6v.c37e) ----

    /// A PR on its own branch that touches exactly `paths` and carries no fragment.
    fn pr_touching(paths: &[&str]) -> tempfile::TempDir {
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        for rel in paths {
            write_at(root, rel, "changed\n");
        }
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "no fragment"]);
        dir
    }

    #[test]
    fn check_passes_without_a_fragment_when_nothing_the_pr_touches_ships() {
        // The outside contributor's first PR: a typo in a spec, a CI tweak, a test. They cannot set
        // the `skip-changelog` label (a fork PR has no write access), so the gate must see it
        // itself — the owner's decision on 6j6v.c37e.
        for paths in [
            &["docs/specs/release-management.md"][..],
            &[".github/workflows/ci.yml"],
            &["xtask/src/runners.rs", "tests/install.sh.test.sh"],
            &["crates/core/tests/graph.rs"],
            &["README.md", "CONTRIBUTING.md", "AGENTS.md"],
            &[
                "npm/mcp/test/verify.test.js",
                "agent-sidecar/test/x.test.ts",
            ],
            // One case per remaining entry: dropping any of them would silently bring back the
            // maintainer dependency this ticket removes (review of PR #494, Test Quality #3).
            &["content/test/docs.test.mjs"],
            &["examples/tauri-board/src/main.rs"],
            &["CLAUDE.md", "NEXUS_MEMORY.md"],
            &["changes/README.md"],
            &[".claude/settings.json", ".gitignore", ".envrc"],
        ] {
            let dir = pr_touching(paths);
            check(dir.path(), "main").unwrap_or_else(|e| panic!("{paths:?}: {e}"));
        }
    }

    #[test]
    fn check_still_fails_without_a_fragment_when_the_pr_touches_what_ships() {
        // The second trap the owner named: the relaxation must not become a way for a real product
        // change to slip through without a changelog entry. Each of these reaches a user — the
        // binary, the guide it embeds, the published docs, the npm shim, the installer, the
        // release pipeline that builds the bytes.
        for shipping in [
            "crates/core/src/lib.rs",
            "crates/cli/docs/guide/core-concepts.md",
            "crates/nxs/docs/develop/index.md",
            "docs/architecture/overview.svg",
            "npm/mcp/lib/verify.js",
            "agent-sidecar/src/index.ts",
            "install.sh",
            "Cargo.toml",
            "Cargo.lock",
            "LICENSE",
            "content/data/x.json",
            ".github/workflows/release.yml",
            ".github/scripts/publish-release.sh",
            // Every carve-out as a literal, not by iterating the constant — deleting an entry must
            // not delete its own test (review of PR #494, Test Quality #1).
            ".github/workflows/promote.yml",
            ".github/workflows/publish-content.yml",
            "docs/generated/commands.json",
            // The gate's own definition: moving the boundary needs a maintainer.
            ".github/workflows/changelog-check.yml",
            "xtask/src/changelog.rs",
            "xtask/src/facade.rs",
            // Delivery config and the release gate script, which stay off the allowlist.
            ".github/scripts/require-green-ci.sh",
            "release/platforms",
            "release-notes.json",
            ".cargo/config.toml",
        ] {
            let dir = pr_touching(&[shipping]);
            let err = check(dir.path(), "main").expect_err(shipping).to_string();
            assert!(err.contains(shipping), "{shipping}: {err}");
        }
    }

    #[test]
    fn check_fails_without_a_fragment_when_one_shipping_path_hides_among_exempt_ones() {
        // ALL paths must be exempt, not any: a code change padded with a doc edit stays red.
        let dir = pr_touching(&["docs/specs/x.md", "crates/core/src/lib.rs"]);
        let err = check(dir.path(), "main").unwrap_err().to_string();
        assert!(err.contains("crates/core/src/lib.rs"), "{err}");
        assert!(!err.contains("docs/specs/x.md"), "{err}");
    }

    #[test]
    fn check_fails_without_a_fragment_when_the_pr_deletes_shipping_code() {
        // Deletions count: removing a source file is a product change even though no shipping
        // file is added or modified.
        let dir = init_repo();
        let root = dir.path();
        write_at(root, "crates/core/src/old.rs", "fn f() {}\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base"]);
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::remove_file(root.join("crates/core/src/old.rs")).unwrap();
        write_at(root, "docs/specs/x.md", "why\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "delete"]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("crates/core/src/old.rs"), "{err}");
    }

    #[test]
    fn check_does_not_exempt_the_tests_of_a_consumed_surface() {
        // A consumed surface's own tests pin its contract; editing them without a word is the
        // silent case 6j6v.gmjd closed. The label stays the way for a maintainer to wave it on.
        // Every consumed surface and the contract-critical crate, not only the first.
        for path in [
            "crates/facade/tests/validate.rs",
            "crates/chat/tests/send.rs",
            "crates/memory/tests/recall.rs",
            "crates/foundation/tests/store.rs",
        ] {
            let dir = pr_touching(&[path]);
            let err = check(dir.path(), "main").expect_err(path).to_string();
            assert!(err.contains(path), "{path}: {err}");
        }
    }

    #[test]
    fn check_fails_on_an_empty_diff() {
        // An empty diff is what a mis-wired base looks like; "nothing ships" is then vacuously
        // true and must not read as green.
        let dir = init_repo();
        let root = dir.path();
        git(root, &["checkout", "-q", "-b", "feature"]);
        git(root, &["commit", "-q", "--allow-empty", "-m", "empty"]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn check_judges_every_path_not_only_the_first() {
        // git lists paths sorted; here the shipping one sorts LAST, so a gate that looked only at
        // the first path would pass it.
        let dir = pr_touching(&["docs/specs/a.md", "install.sh"]);
        let err = check(dir.path(), "main").unwrap_err().to_string();
        assert!(err.contains("install.sh"), "{err}");
    }

    #[test]
    fn check_fails_without_a_fragment_when_shipping_code_moves_into_an_exempt_dir() {
        // A move reports its vacated source path (`--no-renames`), so moving code out of the
        // product into `docs/` is still a product change.
        let dir = init_repo();
        let root = dir.path();
        write_at(root, "crates/core/src/gone.rs", "fn f() {}\n");
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base"]);
        git(root, &["checkout", "-q", "-b", "feature"]);
        fs::create_dir_all(root.join("docs/specs")).unwrap();
        git(
            root,
            &["mv", "crates/core/src/gone.rs", "docs/specs/gone.rs"],
        );
        git(root, &["commit", "-qm", "move"]);
        let err = check(root, "main").unwrap_err().to_string();
        assert!(err.contains("crates/core/src/gone.rs"), "{err}");
    }

    #[test]
    fn check_caps_the_listed_paths() {
        let paths: Vec<String> = (0..13)
            .map(|i| format!("crates/core/src/m{i:02}.rs"))
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let dir = pr_touching(&refs);
        let err = check(dir.path(), "main").unwrap_err().to_string();
        assert!(err.contains("and 3 more"), "{err}");
        assert!(!err.contains("m12.rs"), "{err}");
    }

    /// Whether a workflow file can publish: it holds a write scope that reaches outside the run,
    /// or other workflows can call it (and hand it their scopes).
    fn can_publish(workflow: &str) -> bool {
        workflow.lines().map(str::trim_start).any(|l| {
            l.starts_with("workflow_call:")
                || ["contents:", "id-token:", "packages:"].iter().any(|scope| {
                    l.strip_prefix(scope)
                        .is_some_and(|rest| rest.trim_start().starts_with("write"))
                })
        })
    }

    #[test]
    fn every_publishing_workflow_is_carved_out() {
        // The `.github/workflows/` exemption is only honest while every workflow that can publish
        // is named in the carve-out. A hand-kept list of names is not self-maintaining; this is
        // (review of PR #494, Code Quality #2 / Integrity #2). A new `publish-*.yml`, or a split
        // of release.yml into a `workflow_call` file, reds here until it is named.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows");
        let mut publishing = Vec::new();
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("yml") {
                continue;
            }
            let rel = format!(
                ".github/workflows/{}",
                path.file_name().unwrap().to_str().unwrap()
            );
            if can_publish(&fs::read_to_string(&path).unwrap()) {
                assert!(
                    !has_no_product_impact(&rel),
                    "{rel} can publish but is exempt from the changelog gate — name it in \
                     SHIPS_THOUGH_BELOW_AN_EXEMPT_DIR"
                );
                publishing.push(rel);
            }
        }
        // The detector itself must find something, or this test proves nothing.
        assert!(
            publishing.contains(&".github/workflows/release.yml".to_string()),
            "{publishing:?}"
        );
    }

    #[test]
    fn can_publish_reads_the_scopes_that_reach_outside_the_run() {
        assert!(can_publish("permissions:\n  id-token: write # oidc\n"));
        assert!(can_publish("    contents:   write\n"));
        assert!(can_publish("on:\n  workflow_call:\n"));
        assert!(!can_publish(
            "permissions:\n  contents: read\n  actions: read\n"
        ));
        assert!(!can_publish("# contents: write in a comment\n"));
    }

    #[test]
    fn no_product_impact_is_segment_exact() {
        // A prefix match must not leak into a sibling that merely shares the spelling.
        assert!(has_no_product_impact("docs/specs/a.md"));
        assert!(!has_no_product_impact("docs-site/a.md"));
        assert!(!has_no_product_impact("README.md.bak"));
        assert!(!has_no_product_impact("crates/core/testsuite/a.rs"));
        assert!(!has_no_product_impact("crates/core/tests"));
        assert!(!has_no_product_impact("xtask"));
    }
}
