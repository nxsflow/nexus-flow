//! `nxm guide [topic]` — memory's half of the ONE embedded-guide mechanism (nexus-flow-6j6v.9e3r).
//!
//! The suite is three building blocks, and the docs layer says so: each block serves its own
//! guides, and `nxs guide` fans out over them. The machinery — listing, lookup, the frozen `--json`
//! contract, the terminal renderer, the `TOPICS`↔directory parity rule — lives once in `nxs-guide`;
//! what memory owns is the embedded directory (the `include_dir!` macro resolves
//! `$CARGO_MANIFEST_DIR`, so it can only be expanded here) and the ordered [`TOPICS`] list.
//!
//! **`TOPICS` is memory's five guides** (`6j6v.h4k0`), written against the surface that actually
//! ships. The order is the reading order and the agent contract for `nxm guide --json`: what memory
//! IS (`getting-started`), the model underneath it (`core-concepts`), what you type (`commands`),
//! and the two seams a reader arrives from — an MCP host (`agents-and-mcp`) and an existing memory
//! store to bring in (`import-and-migration`).
//!
//! **`getting-started` is an INTRODUCTION, not a third copy of the suite's setup** (`6j6v.4g1b`,
//! from an owner review of 2026-08-22): what this module is for, what it presupposes, and its first
//! own command. Installing the suite and creating a workspace belong to the `nxs` bracket
//! (`6j6v.0fvt`) and are deliberately NOT repeated here. The cross-link to that bracket is left
//! unwritten on purpose — `nxs-getting-started` does not exist yet, and `checkCrossLinks` in
//! `content/lib/docs.mjs` makes an unknown target a BUILD failure in both locales. The cut is
//! already the one `4g1b` needs, so it can add the link with nothing to tear out.
//!
//! **The EN guides' ` ```console ` blocks are executed**, exactly as flow's and chat's are:
//! `tests/guide_examples.rs` holds every one of them against the golden trycmd corpus
//! (`tests/golden/*.trycmd`), so a guide cannot show a command or an output that is not real. The
//! DE guides carry the same blocks — the same commands, producing the same bytes — and are held to
//! the same corpus by the same test.

use crate::error::Result;
use include_dir::{include_dir, Dir};
use nxs_guide::Catalog;

/// The embedded EN guide content. Held against [`TOPICS`] by the parity test below, exactly as
/// flow's is, so a topic added without being listed (or vice versa) reddens immediately. The DE
/// tree beside it is not embedded — the CLI is EN-only, like flow's and chat's — and feeds the
/// website's federated docs through `content/lib/docs.mjs`.
static GUIDE_EN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/docs/guide/en");

/// The single ordered source of truth for `(topic, one-line summary)`. The order is the agent
/// contract for `nxm guide --json`; do not reorder without intent.
///
/// Topic names stay unqualified here — on the CLI the binary carries the namespace (`nxm guide
/// <topic>`). The `nxm-<topic>` qualification exists only on the website, where all three blocks'
/// guides land in one document; `content/lib/docs.mjs` derives it from this list.
const TOPICS: &[(&str, &str)] = &[
    (
        "getting-started",
        "What memory is for, what it needs, and your first remembered fact.",
    ),
    (
        "core-concepts",
        "Keys and auto-keys, the three registers, the retrieval rule, reading order, and the tombstone.",
    ),
    (
        "commands",
        "Every shipped verb, with its `--json` shape and the errors it can answer with.",
    ),
    (
        "agents-and-mcp",
        "The `memory_*` MCP tools, what `nxs prime` contributes, and where an agent writes.",
    ),
    (
        "import-and-migration",
        "Take a Claude-host memory store in, and file what a workspace already accumulated.",
    ),
];

/// memory's guide catalog — what `nxs guide`'s fan-out reaches by shelling out to `nxm guide`.
pub const CATALOG: Catalog = Catalog {
    binary: "nxm",
    topics: TOPICS,
    embedded: &GUIDE_EN,
};

/// `nxm guide [topic]`: list topics (no arg) or print one topic's embedded markdown.
pub fn guide(json: bool, topic: Option<&str>) -> Result<()> {
    nxs_guide::run(&CATALOG, json, topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: the embedded dir and the `TOPICS` list must agree exactly — the same rule flow
    /// and chat assert, applied to memory's catalog. A `.md` added without listing it (or a topic
    /// listed with no file) is what goes red.
    #[test]
    fn embedded_dir_and_topics_list_are_in_sync() {
        nxs_guide::assert_catalog_parity(&CATALOG);
    }

    /// Every topic must be listed with a real summary and resolve to markdown with a top heading —
    /// the listing's agent contract, and the H1 the docs assembler is fail-closed on.
    #[test]
    fn every_topic_resolves_to_real_content() {
        for (topic, summary) in TOPICS {
            assert!(!summary.is_empty(), "{topic} has a summary");
            let content = CATALOG.lookup(topic).expect("topic resolves");
            assert!(content.contains("# "), "{topic} has a top heading");
        }
    }

    /// The listing's shape, pinned where it is observable: an array of `{summary, topic}` in
    /// declared order. An agent that fans out over the suite parses this, and `nxs guide` composes
    /// it — the reason the order is a contract rather than a preference.
    #[test]
    fn the_listing_is_the_declared_order_as_json() {
        let listed: Vec<String> = TOPICS.iter().map(|(t, _)| (*t).to_string()).collect();
        assert_eq!(
            listed,
            [
                "getting-started",
                "core-concepts",
                "commands",
                "agents-and-mcp",
                "import-and-migration",
            ]
        );
        assert!(guide(true, None).is_ok(), "`nxm guide --json` succeeds");
        assert!(guide(false, None).is_ok(), "`nxm guide` succeeds");
    }

    /// An unknown topic is a legible `validation` error that lists what IS on offer — never a
    /// silent empty answer, and no longer the "none yet" wording, which would now be a lie.
    #[test]
    fn an_unknown_topic_lists_the_topics_that_exist() {
        let err = CATALOG.lookup("mcp").unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        assert!(err.msg.contains("agents-and-mcp"), "{}", err.msg);
    }

    /// The DE tree is held to the EN one by hand here, because `Catalog` embeds only EN: a topic
    /// written in one locale and forgotten in the other would otherwise reach the website's
    /// assembler, which fails the whole build for it. Failing HERE names the missing file.
    #[test]
    fn every_topic_exists_in_both_locales() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guide");
        for (topic, _) in TOPICS {
            for locale in ["en", "de"] {
                let path = dir.join(locale).join(format!("{topic}.md"));
                let body = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(
                    body.starts_with("# "),
                    "{} must open with an H1 (the docs assembler is fail-closed on it)",
                    path.display()
                );
            }
        }
    }

    /// Every `[text](target)` target in a guide body, outside fenced code blocks — a fence may hold
    /// a literal `](…)` in sample output, which is text, not a link. The Rust mirror of
    /// `linkTargets` in `content/lib/docs.mjs`, and deliberately only as much of it as the check
    /// below needs.
    fn link_targets(md: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut in_fence = false;
        for line in md.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let mut rest = line;
            while let Some(i) = rest.find("](") {
                rest = &rest[i + 2..];
                match rest.find(')') {
                    Some(end) => {
                        out.push(rest[..end].trim().to_string());
                        rest = &rest[end + 1..];
                    }
                    None => break,
                }
            }
        }
        out
    }

    /// What is wrong with one cross-link target, or `None` if this crate has no business judging
    /// it. Pulled out of the walk below so the carve-outs can be pinned on SYNTHETIC input (PR #356
    /// post-hoc review, Test Quality #1) — walking today's real guides exercises only the shapes
    /// they happen to contain, which is how the anchor gap below survived.
    ///
    /// **The carve-outs mirror `checkCrossLinks` in `content/lib/docs.mjs` target for target**, and
    /// that parity is the whole claim this function makes:
    ///
    /// - an EMPTY target, a URL and a path are skipped there (`safeUrl`'s business) and here;
    /// - a `#anchor` is an in-page link. The website resolves it against the headings of the page
    ///   it is written in — a set this crate would have to re-derive, including the landing's own
    ///   heading-slug folding — so it is skipped here and left to the authority. Skipping it is not
    ///   a detail: `[text](#anchor)` is established usage in flow's and chat's guides, and the
    ///   first version of this check PANICKED on it;
    /// - a trailing `.md` and a `#fragment` are stripped before the slug is read, exactly as that
    ///   file's `bare` regex does — `nxm-commands#a-heading` is a valid link to a heading of
    ///   another topic, not a topic called `commands#a-heading`;
    /// - a target that is not slug-shaped at all is skipped, because there it means "not a slug,
    ///   therefore a URL" rather than "a broken slug".
    ///
    /// What is left is the one class this crate CAN judge without the other blocks' registries: the
    /// `<binary>-<topic>` prefix, and — for its own block — the topic itself.
    fn cross_link_problem(target: &str, topics: &[&str]) -> Option<String> {
        let t = target.trim();
        if t.is_empty() || t.starts_with('#') || t.contains(':') || t.contains('/') {
            return None;
        }
        let slug = t.split('#').next().unwrap_or(t);
        let slug = slug.strip_suffix(".md").unwrap_or(slug);
        let shaped = slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !shaped {
            return None;
        }
        let Some((block, own)) = slug.split_once('-') else {
            return Some(format!(
                "cross-link '{target}' is not a '<binary>-<topic>' slug — the bare topic name \
                 was the PRE-rename form and resolves to nothing since 6j6v.9e3r"
            ));
        };
        if !["nxf", "nxm", "nxc"].contains(&block) {
            return Some(format!(
                "cross-link '{target}' names '{block}', which carries no guides — slugs are \
                 '<binary>-<topic>' over nxf | nxm | nxc. A bracket topic (`nxs-…`) has no slug \
                 until 6j6v.0fvt writes one, and linking one is a content BUILD failure, not a \
                 broken link somebody notices later."
            ));
        }
        if block == "nxm" && !topics.contains(&own) {
            return Some(format!(
                "cross-link '{target}' names no topic of this block: {topics:?}"
            ));
        }
        None
    }

    /// Half one of the `6j6v.4g1b` cut, and the half that can be checked STRUCTURALLY: every
    /// cross-link in these guides names a building block that actually carries guides.
    ///
    /// The published slug is `<binary>-<topic>` and only `nxf`/`nxm`/`nxc` have a guide tree — so a
    /// link to the `nxs` bracket (`nxs-getting-started`, which `6j6v.0fvt` has not written yet) is
    /// caught HERE, and so is any other bracket topic somebody links before its slug exists. That
    /// generality is the point (PR #356 review, Test Quality #1): pinning the one literal
    /// `(nxs-getting-started)` would pass a differently-named not-yet-existing slug silently.
    ///
    /// `checkCrossLinks` in `content/lib/docs.mjs` remains the authority — it holds every target
    /// against the REAL slug registry of all three blocks, which this crate cannot see (it depends
    /// on neither `nexus-flow-cli` nor `nexus-chat`), and it is the only side that can resolve an
    /// in-page anchor. This is the early red in `cargo test` rather than a second opinion; what it
    /// judges and what it deliberately leaves alone is [`cross_link_problem`].
    #[test]
    fn every_cross_link_names_a_block_that_carries_guides() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guide");
        let topics: Vec<&str> = TOPICS.iter().map(|(t, _)| *t).collect();
        for locale in ["en", "de"] {
            for (topic, _) in TOPICS {
                let where_ = format!("{locale}/{topic}.md");
                let body = std::fs::read_to_string(dir.join(locale).join(format!("{topic}.md")))
                    .unwrap_or_else(|e| panic!("{where_}: {e}"));
                for target in link_targets(&body) {
                    if let Some(problem) = cross_link_problem(&target, &topics) {
                        panic!("{where_}: {problem}");
                    }
                }
            }
        }
    }

    /// The carve-outs, on input the real guides do not contain today — which is exactly why the
    /// anchor gap survived the first version of this check (PR #356 post-hoc review).
    #[test]
    fn the_cross_link_carve_outs_match_the_website_check_shape_for_shape() {
        let topics = ["getting-started", "commands"];
        // Left to the authority: nothing to judge here.
        for ok in [
            "",
            "#the-nxs-umbrella", // in-page anchor — the website resolves it against headings
            "https://nxsflow.com/x", // a URL
            "../elsewhere.md",   // a path
            "not_a_slug",        // not slug-shaped ⇒ "not a slug, therefore a URL"
            "nxm-commands",
            "nxm-commands.md",        // the `.md` form the website strips
            "nxm-commands#a-heading", // a heading of ANOTHER topic, not a topic named `commands#…`
            "nxf-mcp",                // a foreign block: prefix only, its registry is not ours
            "nxc-limits-and-safety",
        ] {
            assert_eq!(
                cross_link_problem(ok, &topics),
                None,
                "{ok} must be left alone"
            );
        }
        // Judged, and rejected.
        for (bad, needle) in [
            ("nxs-getting-started", "carries no guides"),
            ("nxs-anything-at-all", "carries no guides"),
            ("nxm-does-not-exist", "names no topic of this block"),
            ("nxm-does-not-exist#anchor", "names no topic of this block"),
            ("commands", "is not a '<binary>-<topic>' slug"),
        ] {
            let problem = cross_link_problem(bad, &topics)
                .unwrap_or_else(|| panic!("{bad} must be rejected"));
            assert!(problem.contains(needle), "{bad}: {problem}");
        }
    }

    /// Half two of the `6j6v.4g1b` cut: memory's introduction does not re-document the SUITE's
    /// installation, because that belongs to the `nxs` bracket and a third copy is what the owner
    /// review of 2026-08-22 asked us to stop producing.
    ///
    /// **This is a narrow tripwire, not a proof, and it is written down as one** (PR #356 review,
    /// Test Quality #1). It pins the one shape the suite install actually has — the `install.sh`
    /// one-liner every other getting-started guide carries — so a copy-paste of that paragraph
    /// reddens here. Re-explaining installation in fresh prose would pass it, and nothing short of
    /// asserting the truth of English would catch that; the three gate shapes considered for this
    /// class are recorded in `crates/chat/tests/guide_examples.rs`. What holds the rest of the cut
    /// is the structural check above and a human reading the diff.
    ///
    /// Note what it does NOT forbid: `nxs init --module memory` and `nxm init` are memory's OWN
    /// activation, not the suite's setup, and the introduction is supposed to show them.
    #[test]
    fn the_introduction_does_not_re_document_the_suite_install() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guide");
        for locale in ["en", "de"] {
            let body =
                std::fs::read_to_string(dir.join(locale).join("getting-started.md")).unwrap();
            assert!(
                !body.contains("install.sh"),
                "{locale}/getting-started.md re-documents the suite install (6j6v.4g1b): that \
                 belongs in the `nxs` bracket, and memory keeps an INTRODUCTION"
            );
        }
    }
}
