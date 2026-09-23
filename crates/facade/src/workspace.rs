//! Workspace resolution is foundation-owned (the on-disk directory belongs to the platform, not
//! flow); this module re-exports it so existing `nexus_flow_facade::workspace::*` paths resolve.
//! flow's store-open lives here as an extension trait — opening the *flow* store is a product
//! concern (it registers the task reducer + creates the flow views), so the foundation `Workspace`
//! stays domain-agnostic.

pub use nxs_foundation::workspace::*;

use crate::error::{NxfError, Result};
use nexus_flow_core::store::Store;
use std::path::Path;

/// Opens flow's [`Store`] over a resolved [`Workspace`] — the flow-product half of workspace
/// resolution. WAL mode lets parallel `nxf` processes serialize their short write transactions
/// instead of failing with "database is locked". Bring this trait into scope to call
/// `ws.open_store()`.
pub trait WorkspaceExt {
    fn open_store(&self) -> Result<Store>;
}

impl WorkspaceExt for Workspace {
    fn open_store(&self) -> Result<Store> {
        let path = self.db_path_str()?;
        let store = Store::open(&path, self.replica.site_id)
            .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))?;
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
        Ok(store)
    }
}

// ---- flow's plugin selection (nexus-flow-aye.2.2) --------------------------
//
// `plugin` is flow's PRESENTATION axis (spec §4.4), not a foundation concern — so it lives over
// the foundation's domain-agnostic config seam (`WorkspaceConfig::products`), not as a field in it.
// flow reads its `[flow]` section and OWNS the default; the foundation stays plugin-agnostic.

/// The flow module key in the foundation config's product seam (spec §7).
const FLOW_MODULE: &str = "flow";
/// The plugin key inside flow's config section.
const PLUGIN_KEY: &str = "plugin";
/// flow's default plugin — OWNED by flow (aye.2.2 moved it off the foundation). Hit only when a
/// workspace selects none (e.g. an unreadable/sectionless config.toml).
pub const DEFAULT_PLUGIN: &str = "issue-tracker";

/// flow's active plugin for `config` (presentation axis, §4.4): the `[flow] plugin` section, else a
/// legacy top-level `plugin = "..."` (pre-rework workspaces, now captured by the foundation's
/// product seam), else flow's own [`DEFAULT_PLUGIN`]. flow owns this — the foundation never reads it.
pub fn flow_plugin(config: &WorkspaceConfig) -> String {
    config
        .products
        .get(FLOW_MODULE)
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(PLUGIN_KEY))
        .and_then(|v| v.as_str())
        .or_else(|| config.products.get(PLUGIN_KEY).and_then(|v| v.as_str()))
        .unwrap_or(DEFAULT_PLUGIN)
        .to_string()
}

/// Build the foundation config for a fresh flow workspace: register `flow` as an active module and
/// seat its chosen plugin under the `[flow]` product section (spec §7).
pub fn flow_config(plugin: &str) -> WorkspaceConfig {
    let mut flow = toml::value::Table::new();
    flow.insert(
        PLUGIN_KEY.to_string(),
        toml::Value::String(plugin.to_string()),
    );
    let mut products = toml::value::Table::new();
    products.insert(FLOW_MODULE.to_string(), toml::Value::Table(flow));
    WorkspaceConfig {
        active_modules: vec![FLOW_MODULE.to_string()],
        products,
    }
}

/// Create a fresh flow workspace at `start` with `plugin` selected — flow's product init seam over
/// the foundation: it refuses a nested workspace (foundation policy) then materializes `.nxs/` via
/// the foundation with flow's config.
pub fn init_flow(start: &Path, plugin: &str) -> Result<Workspace> {
    nxs_foundation::workspace::init(start, &flow_config(plugin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_plugin_defaults_to_issue_tracker_when_unset() {
        // No `[flow]` section, no legacy key → flow's OWN default (not the foundation's).
        assert_eq!(flow_plugin(&WorkspaceConfig::default()), DEFAULT_PLUGIN);
    }

    #[test]
    fn flow_plugin_reads_the_flow_section() {
        let cfg = flow_config("personal-todo");
        assert_eq!(flow_plugin(&cfg), "personal-todo");
        // flow_config registers flow as active and writes the `[flow]` section.
        assert_eq!(cfg.active_modules, vec!["flow".to_string()]);
        assert_eq!(
            cfg.products["flow"]["plugin"].as_str(),
            Some("personal-todo")
        );
    }

    #[test]
    fn flow_plugin_reads_a_legacy_top_level_plugin() {
        // A pre-rework config (`plugin = "..."`) is now captured by the foundation's product seam;
        // flow's reader falls back to it so shipped workspaces resolve unchanged.
        let cfg: WorkspaceConfig = toml::from_str("plugin = \"personal-todo\"\n").unwrap();
        assert_eq!(flow_plugin(&cfg), "personal-todo");
    }

    #[test]
    fn flow_section_wins_over_a_legacy_top_level_plugin() {
        // If both are present (a transitional config), the canonical `[flow]` section is preferred.
        let cfg: WorkspaceConfig =
            toml::from_str("plugin = \"issue-tracker\"\n[flow]\nplugin = \"personal-todo\"\n")
                .unwrap();
        assert_eq!(flow_plugin(&cfg), "personal-todo");
    }
}
