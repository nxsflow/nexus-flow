//! The board-item half of the **retrieval rule** (6j6v.srpg): the memories `nxf show <id>` prints
//! in full and `nxf next` merely hints at.
//!
//! A memory carries a **reach**, and the reach decides where and how deep it surfaces:
//!
//! | reach     | `nxf next`                 | `nxf show <id>` | `nxs prime` |
//! |-----------|----------------------------|-----------------|-------------|
//! | `item`    | a hint that there are some | **full text**   | —           |
//! | `project` | —                          | —               | **full text** |
//! | `global`  | —                          | —               | **full text** |
//!
//! The bottom two rows are memory's own (`nxm prime`, fanned out by `nxs prime`); this module is
//! the top row. Both halves apply the SAME predicate from `nexus_memory::model`, so the rule has
//! one definition and two callers rather than two definitions.
//!
//! The table's top row assumes the memory NAMES board items. One that is `item`-scoped with empty
//! `refs` has no board item to read on, so the bootstrap keeps it rather than letting it fall
//! through every cell — see `model::replayed_at_session_start`.
//!
//! **`nxs prime`'s `next` section is deliberately NOT hinted.** The table gives item reach a `—` in
//! the prime column, and a marker there would be a third place one memory is signalled. The hint
//! belongs to the live work list (`nxf next`), which is where a reader acts on it.
//!
//! **Why flow reads memory's view.** `nxf` and `nxm` are two personas of one binary over ONE `.nxs/`
//! workspace, and the `memories` view lives in that shared db. The dependency runs one way only —
//! flow reads memory, memory knows nothing of flow (spec §4.4) — and it is gated on the memory
//! module being ACTIVE in the workspace config, exactly like the `nxs prime` fan-out. A flow-only
//! workspace therefore never opens memory's store, never materializes its view, and renders byte-
//! identically to before.
//!
//! **What opening memory's store actually does** (PR #282 review, Code Quality #2). This is the
//! same [`open_memory_store`](MemoryWorkspaceExt::open_memory_store) `nxm` itself calls, so it is
//! not a pure read: it applies memory's view DDL and can refold the `memories` view. Both are
//! wanted here, not tolerated:
//!
//! - the DDL is idempotent `CREATE … IF NOT EXISTS` (`schema::apply_is_idempotent`), and the gate
//!   above means the module is active — so the view already exists and nothing is created;
//! - `refold_if_behind` is O(1) unless the shared log advanced past memory's watermark, which
//!   happens after a foundation-only sync pull (aye.36). Skipping it would make this join serve a
//!   **stale** view — a read-only open would trade a correct answer for a fast wrong one.
//!
//! **Why here and not in `crates/facade`.** flow's facade is the SemVer-pinned embedding contract
//! and is deliberately product-clean, so the join rides one layer above it — like the sparse
//! `custom` join `prime --json` attaches and the `ee2h` presentation labels. It is `pub` because
//! the E9 MCP server has to attach the SAME join: `flow_show`/`flow_next` are pinned byte-identical
//! to `nxf show`/`nxf next --json`, and a seam-local second copy of the rule is exactly how the two
//! views drift apart. An embedding app that holds neither reaches the rule directly through
//! `nexus_memory::engine::Engine::memories_about`.

use crate::error::Result;
use crate::workspace::{Workspace, WorkspaceConfig};
use nexus_memory::facade::{self, MemoryRecord};
use nexus_memory::workspace::{MemoryWorkspaceExt, MEMORY_MODULE};
use serde_json::Value;
use std::collections::BTreeMap;

/// The item-scoped memories filed against each of `ids`, bucketed by item id — key-sorted within a
/// bucket, and an id with none is ABSENT from the map (so "does this item carry any?" is one
/// lookup). Empty when the workspace has no memory module, which is what keeps a flow-only
/// workspace untouched by all of this.
///
/// **A failure to open or read memory's store propagates** (PR #282 review, Integrity #1). It was
/// briefly a stderr warning plus an empty map, which is wrong on the surface that matters most:
/// stdout carries the `--json` record and an MCP client never sees stderr at all, so a tool call
/// would return a SUCCESSFUL result whose missing enrichment is indistinguishable from "this item
/// genuinely has none". The structured `NxfError` envelope exists for exactly this, and the
/// alternative — a second, partially-observable degraded path — is a silent failure with extra
/// steps. It cannot fire spuriously either: the gate below means the workspace declares memory
/// active, and flow's own store just opened from the same file, so a failure here is a real fault.
pub fn about(ws: &Workspace, ids: &[&str]) -> Result<BTreeMap<String, Vec<MemoryRecord>>> {
    if ids.is_empty() || !is_active(&ws.config) {
        return Ok(BTreeMap::new());
    }
    facade::memories_about(&ws.open_memory_store()?, ids)
}

/// The item-scoped memories filed against one item — [`about`] for a single id, the read `show`
/// performs.
pub fn of_item(ws: &Workspace, id: &str) -> Result<Vec<MemoryRecord>> {
    Ok(about(ws, &[id])?.remove(id).unwrap_or_default())
}

/// Attach the retrieval rule's full-text cell to a `show --json` object: the item's memories as
/// canonical records under `memories`. **Sparse** — an item with none leaves the value untouched,
/// so every pre-rule consumer keeps parsing exactly what it parsed before (the same contract the
/// `labels`/`conversations` joins follow).
pub fn attach_to_show(records: &[MemoryRecord], value: &mut Value) {
    if !records.is_empty() {
        value["memories"] = serde_json::to_value(records).expect("memory records serialize");
    }
}

/// Attach the retrieval rule's hint cell to a `next --json` array: a `memories` COUNT per row, the
/// machine form of the human `(2 memories)` marker. A count and not the records, for the same
/// reason the human view prints one — `next` is the work list, `show` is where the text lives
/// (mirroring the `conversations` count the facade attaches). Sparse: a row with none is
/// byte-identical to the pre-rule output.
pub fn attach_counts_to_next(ws: &Workspace, ids: &[&str], value: &mut Value) -> Result<()> {
    splice_counts(&about(ws, ids)?, ids, value);
    Ok(())
}

/// The pure half of [`attach_counts_to_next`] — split out so the index alignment it rests on can be
/// exercised directly (PR #282 review, Test Quality #2), rather than only through a subprocess.
///
/// `ids` projects the same items the rows do, in the same order, so they align by index. That is the
/// contract `attach_prime_custom` relies on, asserted the same way: a future divergence would
/// otherwise silently cross-wire one item's count onto another item's row instead of failing.
fn splice_counts(memories: &BTreeMap<String, Vec<MemoryRecord>>, ids: &[&str], value: &mut Value) {
    if memories.is_empty() {
        return;
    }
    let Some(rows) = value.as_array_mut() else {
        return;
    };
    debug_assert_eq!(
        rows.len(),
        ids.len(),
        "splice_counts: next rows/ids length mismatch ({} vs {})",
        rows.len(),
        ids.len(),
    );
    for (row, id) in rows.iter_mut().zip(ids.iter()) {
        if let Some(n) = memories.get(*id).map(Vec::len) {
            row["memories"] = serde_json::json!(n);
        }
    }
}

/// Whether the memory module is active in this workspace — the same `active_modules` gate the
/// `nxs prime` fan-out and the MCP tool registration read. Inactive means memory was never set up
/// here (`nxm init`), so there is no `memories` view to consult and none must be created.
fn is_active(config: &WorkspaceConfig) -> bool {
    config.active_modules.iter().any(|m| m == MEMORY_MODULE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(modules: &[&str]) -> WorkspaceConfig {
        WorkspaceConfig {
            active_modules: modules.iter().map(|m| m.to_string()).collect(),
            ..WorkspaceConfig::default()
        }
    }

    fn record(key: &str) -> MemoryRecord {
        MemoryRecord {
            key: key.to_string(),
            body: Some("body".to_string()),
            author: "alice".to_string(),
            updated: String::new(),
            active: true,
            category: "unsorted".to_string(),
            scope: "item".to_string(),
            refs: vec!["ab12.0001".to_string()],
            ordinal: None,
            introduction: Some(format!("what `{key}` is about")),
        }
    }

    fn bucket(pairs: &[(&str, usize)]) -> BTreeMap<String, Vec<MemoryRecord>> {
        pairs
            .iter()
            .map(|(id, n)| {
                (
                    (*id).to_string(),
                    (0..*n).map(|i| record(&format!("k{i}"))).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn the_gate_is_the_workspaces_active_module_set() {
        assert!(is_active(&config(&["flow", "memory"])));
        assert!(is_active(&config(&["memory"])));
        assert!(!is_active(&config(&["flow"])), "flow-only reads nothing");
        assert!(!is_active(&config(&[])), "a bare workspace reads nothing");
    }

    #[test]
    fn show_carries_the_records_and_stays_byte_identical_without_any() {
        let mut v = json!({ "item": { "id": "ab12.0001" } });
        attach_to_show(&[], &mut v);
        assert_eq!(
            v,
            json!({ "item": { "id": "ab12.0001" } }),
            "no memories ⇒ no key at all"
        );

        attach_to_show(&[record("ticket")], &mut v);
        assert_eq!(v["memories"][0]["key"], "ticket");
        assert_eq!(
            v["memories"][0]["body"], "body",
            "the full record, not a count"
        );
    }

    #[test]
    fn next_rows_get_their_own_items_count_and_only_theirs() {
        let mut v = json!([{ "id": "ab12.0001" }, { "id": "ab12.0002" }, { "id": "ab12.0003" }]);
        splice_counts(
            &bucket(&[("ab12.0001", 2), ("ab12.0003", 1)]),
            &["ab12.0001", "ab12.0002", "ab12.0003"],
            &mut v,
        );
        assert_eq!(v[0]["memories"], 2);
        assert!(
            v[1].get("memories").is_none(),
            "sparse: a row with none is untouched"
        );
        assert_eq!(v[2]["memories"], 1);
    }

    #[test]
    fn an_empty_bucket_map_leaves_every_row_untouched() {
        let before = json!([{ "id": "ab12.0001" }]);
        let mut v = before.clone();
        splice_counts(&BTreeMap::new(), &["ab12.0001"], &mut v);
        assert_eq!(v, before);
    }

    /// The alignment guard fires rather than cross-wiring counts onto the wrong rows. `debug_assert`
    /// compiles out under `--release` (which this repo also runs as a gate), so the test is gated on
    /// the same flag the assertion is.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "rows/ids length mismatch")]
    fn a_rows_ids_mismatch_is_caught_instead_of_mislabelling_a_row() {
        let mut v = json!([{ "id": "ab12.0001" }, { "id": "ab12.0002" }]);
        splice_counts(&bucket(&[("ab12.0001", 1)]), &["ab12.0001"], &mut v);
    }
}
