//! `nxc guide [topic]` — chat's half of the ONE embedded-guide mechanism (nexus-flow-6j6v.9e3r).
//!
//! The suite is three building blocks, and the docs layer now says so: each block serves its own
//! guides, and `nxs guide` fans out over them. The machinery — listing, lookup, the frozen `--json`
//! contract, the terminal renderer, the `TOPICS`↔directory parity rule — lives once in `nxs-guide`;
//! what chat owns is the embedded directory (the `include_dir!` macro resolves
//! `$CARGO_MANIFEST_DIR`, so it can only be expanded here) and the ordered [`TOPICS`] list.
//!
//! **`TOPICS` is chat's seven guides** (`6j6v.t6vd`, plus `6j6v.9w08`), written against the
//! surface that actually ships. The order is the reading order and the agent contract for
//! `nxc guide --json`: what chat IS (`getting-started`, `core-concepts`), what you type
//! (`commands`), what you declare (`personas`, `channels`) and how to declare it well
//! (`writing-declarations`), and what constrains all of it (`limits-and-safety`).
//!
//! **`writing-declarations` sits AFTER `channels` and before `limits-and-safety`** because writing
//! quality is the last stage of "what you declare", not a fourth thing beside it: it is one topic
//! rather than additions to `personas.md` and `channels.md` because six of its seven error classes
//! hold for BOTH declaration kinds, and splitting it over two guides would have produced two copies
//! of the same rules — the very mistake the topic forbids. The topic the
//! item's own proposal called "workflow" is `channels`: `6j6v.dvyq` §3 removed the `workflow` verb
//! group outright and an ordered channel (`flow: sequential`) is what replaced it, so documenting a
//! `workflow` topic would have documented a surface that is not there.
//!
//! **The EN guides' ` ```console ` blocks are executed**, exactly as flow's are:
//! `tests/guide_examples.rs` holds every one of them against the golden corpus
//! (`tests/golden/*.trycmd`), so a guide cannot show a command or an output that is not real. The
//! DE guides carry the same blocks — the same commands, producing the same bytes — and are held to
//! the same corpus by the same test.
//!
//! CLI-surface only (`cli` feature), like [`crate::cli`] itself: an embedding consumer that links
//! only `Engine` has no `nxc` to explain and ships its own documentation.

use crate::error::Result;
use include_dir::{include_dir, Dir};
use nxs_guide::Catalog;

/// The embedded EN guide content. Held against [`TOPICS`] by the parity test below, exactly as
/// flow's is, so a topic added without being listed (or vice versa) reddens immediately. The DE
/// tree beside it is not embedded — the CLI is EN-only, like flow's — and feeds the website's
/// federated docs through `content/lib/docs.mjs`.
static GUIDE_EN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/docs/guide/en");

/// The single ordered source of truth for `(topic, one-line summary)`. The order is the agent
/// contract for `nxc guide --json`; do not reorder without intent.
///
/// Topic names stay unqualified here — on the CLI the binary carries the namespace (`nxc guide
/// <topic>`). The `nxc-<topic>` qualification exists only on the website, where all three blocks'
/// guides land in one document; `content/lib/docs.mjs` derives it from this list.
const TOPICS: &[(&str, &str)] = &[
    (
        "getting-started",
        "Activate chat, declare a persona, and get your first answer back.",
    ),
    (
        "core-concepts",
        "The shared op-log, qualified handles, threads and their derived quorum, and how a message is delivered.",
    ),
    (
        "commands",
        "Every shipped verb, with its `--json` shape — and what was removed, with what to do instead.",
    ),
    (
        "personas",
        "Declaring one agent: identity, prompt layers, model band, tools, and who may address it.",
    ),
    (
        "channels",
        "Declaring a group — and the ordered channel (`flow: sequential`) that IS the workflow.",
    ),
    (
        "writing-declarations",
        "From an intention to a declaration that answers usefully: seven measured error classes, and a checklist.",
    ),
    (
        "limits-and-safety",
        "The hop cap, the human gate, the working-copy lease, and what an agent does NOT decide.",
    ),
];

/// chat's guide catalog — what `nxs guide`'s fan-out reaches by shelling out to `nxc guide`.
pub const CATALOG: Catalog = Catalog {
    binary: "nxc",
    topics: TOPICS,
    embedded: &GUIDE_EN,
};

/// `nxc guide [topic]`: list topics (no arg) or print one topic's embedded markdown.
pub fn guide(json: bool, topic: Option<&str>) -> Result<()> {
    nxs_guide::run(&CATALOG, json, topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: the embedded dir and the `TOPICS` list must agree exactly — the same rule flow
    /// and memory assert, applied to chat's catalog. A `.md` added without listing it (or a topic
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
                "personas",
                "channels",
                "writing-declarations",
                "limits-and-safety",
            ]
        );
        assert!(guide(true, None).is_ok(), "`nxc guide --json` succeeds");
        assert!(guide(false, None).is_ok(), "`nxc guide` succeeds");
    }

    /// An unknown topic is a legible `validation` error that lists what IS on offer — never a
    /// silent empty answer, and no longer the "none yet" wording, which would now be a lie.
    #[test]
    fn an_unknown_topic_lists_the_topics_that_exist() {
        let err = CATALOG.lookup("workflow").unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        assert!(err.msg.contains("channels"), "{}", err.msg);
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
}
