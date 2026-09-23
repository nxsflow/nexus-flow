//! `nxs`'s view of the workspace registry: the presentation, over the data seam in `nxs-service`.
//!
//! The registry itself — `~/.nexusflow/workspaces.toml`, its shape, its atomic writer, its
//! load/upsert/remove — moved to `crates/service` (6j6v.5zst), so an embedding app can tell the
//! background service about a workspace without linking this CLI crate. What stays here is the part
//! that is genuinely CLI: rendering the registry for `nxs mcp workspaces`, and the thin wrappers
//! that resolve the REAL `~/.nexusflow` for callers inside this crate.
//!
//! **`~/.nexusflow` here, and in every comment below, means THIS PROCESS'S service instance**
//! (nxf 6j6v.gd9p): the production one by that name, or `~/.nexusflow-<name>` when this process
//! is a development BUILD that `NXS_SERVICE_INSTANCE` names — an installed binary is always the
//! production one (nxf 6j6v.cvpy). The wrappers say `ServiceHome::resolve()` rather than the
//! string precisely so there is one place that decides which.

use nxs_service::registry;
use nxs_service::ServiceHome;
use serde_json::Value;

use crate::error::{NxfError, Result};

// Re-exported so this crate's existing call sites (the MCP seam, the sync module, the daemon) keep
// naming one place for registry access, and so the atomic writer they all share stays reachable
// under the name they already use.
pub use nxs_service::atomic::{write_atomic, write_atomic_private, write_new_exclusive};
pub use nxs_service::registry::{list_value_from, load_from, upsert_into};
pub use nxs_service::WorkspaceEntry;

/// [`list_value_from`] against the real `~/.nexusflow/workspaces.toml` — the builder both the
/// `list_workspaces` tool and `nxs mcp workspaces` render.
pub fn list_value() -> Result<Value> {
    registry::list_value_from(&ServiceHome::resolve()?.registry())
}

/// [`load_from`] against the real `~/.nexusflow/workspaces.toml` — the typed counterpart to
/// [`list_value`], for a consumer (the service's sweep) that needs the entries themselves rather
/// than the rendered JSON array.
pub fn load_registered() -> Result<Vec<WorkspaceEntry>> {
    registry::load_from(&ServiceHome::resolve()?.registry())
}

/// Auto-register a workspace `path` (already absolutized by the caller) into the real registry — the
/// `nxs sync bind` / `nxs mcp install --workspace <path>` upkeep hook.
///
/// The path is INTENTIONALLY not existence-checked (0jq8 Code #2): registration is lazy, matching
/// the per-call `workspace` override — you may register a board that isn't materialized yet or lives
/// on another machine. A stale/typo'd entry is harmless: nothing treats a registry entry as
/// pre-validated (Integrity #3), so resolving it still walks up for a `.nxs/` and surfaces the
/// normal `no_workspace` error. Since 6j6v.5zst a dead entry can also just be removed, with
/// [`unregister_workspace`].
pub fn register_workspace(path: &str) -> Result<bool> {
    ServiceHome::resolve()?.register(std::path::Path::new(path), &cwd()?)
}

/// Take a workspace back out of the real registry. Returns whether anything was removed; removing
/// an entry that is not there is a successful no-op.
pub fn unregister_workspace(path: &str) -> Result<bool> {
    ServiceHome::resolve()?.deregister(std::path::Path::new(path), &cwd()?)
}

/// The working directory a RELATIVE registry path is resolved against. Both wrappers above are
/// called with absolute paths today, so this only ever supplies a fallback the seam requires —
/// but the seam takes it explicitly rather than reading the process, which is what makes it
/// testable from an app.
fn cwd() -> Result<std::path::PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading the current directory: {e}")))
}

/// `nxs mcp workspaces`: render the registry (0jq8). `--json` prints the SAME canonical array the
/// `list_workspaces` tool returns — via the shared [`list_value_from`] builder, so the two seams are
/// byte-identical by construction (the parity partner); otherwise the aligned human table from
/// [`render_human`].
pub fn workspaces(json: bool) -> Result<()> {
    let path = ServiceHome::resolve()?.registry();
    if json {
        println!("{}", registry::list_value_from(&path)?);
    } else {
        print!("{}", render_human(&registry::load_from(&path)?));
    }
    Ok(())
}

/// Render the human (non-`--json`) `nxs mcp workspaces` output: an aligned `name  path` table (the
/// name column padded to the widest), or a register hint when the registry is empty. Pure — returns
/// the full string (trailing newline included) so it is unit-testable without capturing stdout.
fn render_human(entries: &[WorkspaceEntry]) -> String {
    use std::fmt::Write as _;
    if entries.is_empty() {
        return "No workspaces registered. Register one with `nxs mcp install --workspace <path>`, \
                or add it to ~/.nexusflow/workspaces.toml by hand.\n"
            .to_string();
    }
    let width = entries.iter().map(|e| e.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for e in entries {
        // Writing to a String is infallible.
        let _ = writeln!(out, "  {:width$}  {}", e.name, e.path);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_human_empty_shows_the_register_hint() {
        let s = render_human(&[]);
        assert!(
            s.contains("No workspaces registered"),
            "hint present: {s:?}"
        );
        assert!(
            s.contains("nxs mcp install --workspace"),
            "hint names how to register: {s:?}"
        );
    }

    #[test]
    fn render_human_aligns_paths_into_one_column() {
        let entries = vec![
            WorkspaceEntry {
                name: "a".into(),
                path: "/a".into(),
            },
            WorkspaceEntry {
                name: "longer".into(),
                path: "/b".into(),
            },
        ];
        let lines: Vec<String> = render_human(&entries).lines().map(str::to_string).collect();
        assert_eq!(lines.len(), 2, "one row per workspace: {lines:?}");
        assert!(
            lines[0].contains("a") && lines[0].ends_with("/a"),
            "{lines:?}"
        );
        assert!(
            lines[1].contains("longer") && lines[1].ends_with("/b"),
            "{lines:?}"
        );
        assert_eq!(
            lines[0].find("/a"),
            lines[1].find("/b"),
            "the short name is padded so both paths start in the same column: {lines:?}"
        );
    }
}
