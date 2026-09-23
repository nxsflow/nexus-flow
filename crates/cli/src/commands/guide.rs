//! `nxf guide [topic]` (E7-S3): embedded, offline, agent-native narrative docs.
//!
//! Content is compiled into the binary via `include_dir!`, so the command works with no
//! filesystem dependency and is identical in CI, a container, or a developer's laptop.
//! CLI content is **EN-only**; the DE tree beside it feeds the website /docs portal.
//!
//! **This file is now flow's HALF of the mechanism, not the mechanism** (nexus-flow-6j6v.9e3r).
//! The suite documents three building blocks, each serving its own topics, so the listing/lookup/
//! render/`--json` machinery moved to `nxs-guide` where `nxm` and `nxc` share the same copy — and
//! the same frozen `--json` contract. What stays here is exactly what is flow's own: the embedded
//! directory (the `include_dir!` macro can only be expanded from the crate that owns the path) and
//! the ordered `TOPICS` list.

use crate::error::Result;
use include_dir::{include_dir, Dir};
use nxs_guide::Catalog;

/// The embedded EN guide content. The parity test below asserts this stays in lockstep
/// with `TOPICS`: every topic has a `<topic>.md` here, and every `.md` here is a topic.
static GUIDE_EN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/docs/guide/en");

/// The single ordered source of truth for `(topic, one-line summary)`. The order is the
/// agent contract for `nxf guide --json`; do not reorder without intent.
///
/// The topic NAMES are unqualified (`getting-started`, not `nxf-getting-started`) and stay that
/// way: on the CLI the binary already carries the namespace — you type `nxf guide getting-started`
/// — so a prefix here would only stutter. The `<tool>-<topic>` qualification exists on the WEBSITE,
/// where all three blocks' guides land in one document and their slugs must not collide
/// (6j6v.9e3r); `content/lib/docs.mjs` derives it from this list rather than duplicating it.
const TOPICS: &[(&str, &str)] = &[
    (
        "getting-started",
        "Install, init a workspace, and create your first task.",
    ),
    (
        "mcp",
        "Connect an MCP host (Claude Desktop, Cursor, …) — register the server and bootstrap.",
    ),
    (
        "core-concepts",
        "Items, dependencies, and how `next` and `blocked` are derived rather than stored.",
    ),
    ("commands", "A tour of the most-used nxf commands."),
    (
        "plugins",
        "How vocabulary, ranking, and presentation are configured.",
    ),
    (
        "migration",
        "Bring an existing tracker in and bind a sync stream.",
    ),
    (
        "deferring-and-waiting",
        "Defer only for real calendar dates; model waiting on a delivery as a WAIT chore.",
    ),
    (
        "running-a-relay",
        "Run your own sync server for backup or collaboration — SQLite, or a Postgres such as Supabase.",
    ),
];

/// flow's guide catalog — the value `nxs guide`'s fan-out reaches by shelling out to `nxf guide`,
/// and the one this crate's own verb serves.
pub const CATALOG: Catalog = Catalog {
    binary: "nxf",
    topics: TOPICS,
    embedded: &GUIDE_EN,
};

/// `nxf guide [topic]`: list topics (no arg) or print one topic's embedded markdown.
pub fn guide(json: bool, topic: Option<&str>) -> Result<()> {
    nxs_guide::run(&CATALOG, json, topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: the embedded dir and the `TOPICS` list must agree exactly. This is the
    /// "drift = build/test failure" principle — adding a `.md` without listing it (or vice
    /// versa) fails here rather than shipping a half-wired guide. The RULE lives in `nxs-guide`
    /// so all three blocks assert the same one; this is flow's application of it.
    #[test]
    fn embedded_dir_and_topics_list_are_in_sync() {
        nxs_guide::assert_catalog_parity(&CATALOG);
    }

    /// Every listed topic must resolve to non-empty embedded markdown with a top heading,
    /// and every summary must be non-empty (the listing's agent contract).
    #[test]
    fn every_topic_resolves_to_real_content() {
        for (topic, summary) in TOPICS {
            assert!(!summary.is_empty(), "{topic} has a summary");
            let content = CATALOG.lookup(topic).expect("topic resolves");
            assert!(content.contains("# "), "{topic} has a top heading");
        }
    }

    #[test]
    fn deferring_and_waiting_topic_teaches_defer_and_the_wait_chore() {
        // oxmu: the working convention is reachable as its own guide topic — defer is for a real
        // calendar date only, and waiting on a delivery is an open WAIT chore the dependents `dep`
        // onto, released by closing the chore with the delivered version.
        let content = CATALOG
            .lookup("deferring-and-waiting")
            .expect("topic resolves");
        let lower = content.to_lowercase();
        assert!(lower.contains("defer"), "covers defer: {content}");
        assert!(
            content.contains("WAIT"),
            "names the WAIT chore convention: {content}"
        );
        assert!(
            lower.contains("chore"),
            "models waiting as a chore: {content}"
        );
        assert!(
            lower.contains("dep add") || lower.contains("dependency"),
            "dependents block on the chore via a dependency: {content}"
        );
    }

    #[test]
    fn unknown_topic_is_validation_error_listing_topics() {
        let err = CATALOG.lookup("does-not-exist").unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        for (topic, _) in TOPICS {
            assert!(err.msg.contains(topic), "error lists {topic}: {}", err.msg);
        }
    }
}
