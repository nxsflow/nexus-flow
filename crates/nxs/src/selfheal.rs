//! Self-heal a workspace's `active_modules` (nexus-flow-cur): back-fill any module that is
//! INITIALIZED in the workspace (it carries a product section) but is missing from
//! `config.active_modules`, so `nxs prime` never silently drops it — WITHOUT the user first
//! running an explicit `nxs migrate`.
//!
//! The drift this repairs: a pre-umbrella (v0.5.x) `nxf init` writes a `[flow]` product section
//! (or the legacy top-level `plugin` scalar) but predates the active-module registry, so
//! `active_modules` is empty. `nxs prime` fans out over `active_modules` only, so it yields
//! nothing — and after `nxm init` it yields ONLY memory, never the initialized flow. The same
//! idempotent back-fill `nxs migrate` already performs is applied here at prime/store-open time.
//!
//! Symmetric over the roster: every self-registered roster entry whose product section is
//! present-but-inactive is registered (flow is the only module that writes one today — memory
//! records itself solely via `active_modules` — so flow is the only live case, but a future
//! module that ships a `[<key>]` section is covered automatically). A module with NO product
//! section (a memory-only workspace's absent `[flow]`) is left untouched: presence is decided by
//! the on-disk product section, never by "the binary exists", so flow is never conjured into a
//! workspace where it was never initialized. Registration only ever APPENDS (via
//! [`workspace::activate_module`]), so a sibling already in `active_modules` is never displaced.

use crate::error::Result;
use crate::registry::ModuleInit;
use nxs_foundation::workspace::{self, Workspace, WorkspaceConfig};

/// flow's roster key — also the name of its `config.toml` product section. A pre-registry flow
/// workspace is detected (and back-filled) by it or the legacy `plugin` scalar.
pub(crate) const FLOW_MODULE: &str = "flow";
/// The reserved legacy top-level scalar (`plugin = "..."`) a pre-rework flow workspace carries in
/// the foundation's product seam; also marks the workspace as flow's.
pub(crate) const LEGACY_PLUGIN_KEY: &str = "plugin";

/// Register every initialized-but-inactive roster module in the workspace's `active_modules`,
/// in roster order. Idempotent: a workspace already on the umbrella model (every present module
/// active) is a no-op with no write. Returns the modules newly registered (empty ⇒ nothing to do).
pub fn backfill_active_modules(roster: &[&ModuleInit], ws: &Workspace) -> Result<Vec<String>> {
    let mut registered = Vec::new();
    for module in roster {
        let already_active = ws.config.active_modules.iter().any(|m| m == module.key);
        if already_active || !section_present(&ws.config, module.key) {
            continue;
        }
        if workspace::activate_module(&ws.dir, module.key)? {
            registered.push(module.key.to_string());
        }
    }
    Ok(registered)
}

/// Whether `module`'s product section is present in the workspace config. flow is additionally
/// detected by the legacy top-level `plugin` scalar a pre-rework workspace carries.
fn section_present(config: &WorkspaceConfig, module: &str) -> bool {
    config.products.contains_key(module)
        || (module == FLOW_MODULE && config.products.contains_key(LEGACY_PLUGIN_KEY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::InitRequest;
    use nxs_foundation::workspace::{self, WorkspaceConfig};
    use tempfile::TempDir;

    fn noop(_: &InitRequest) -> Result<()> {
        Ok(())
    }

    // The self-registered roster, as `static` descriptors so a test helper can hand out `'static`
    // refs (the back-fill only reads `.key`; only flow writes a product section, so it is the sole
    // live back-fill case — memory is present for roster fidelity).
    static FLOW: ModuleInit = ModuleInit {
        key: "flow",
        binary: "nxf",
        now_env: "NXF_NOW",
        blurb: "",
        recommended: true,
        order: 10,
        accepts_plugin: true,
        init_fn: noop,
        welcome: None,
        details: None,
        first_command: None,
    };
    static MEMORY: ModuleInit = ModuleInit {
        key: "memory",
        binary: "nxm",
        now_env: "NXM_NOW",
        blurb: "",
        recommended: false,
        order: 20,
        accepts_plugin: false,
        init_fn: noop,
        welcome: None,
        details: None,
        first_command: None,
    };
    fn roster() -> Vec<&'static ModuleInit> {
        vec![&FLOW, &MEMORY]
    }

    /// Build a workspace whose `config.toml` carries `active` modules and an optional `[flow]`
    /// product section (with a plugin), mirroring a pre-umbrella `nxf init` on disk.
    fn workspace_with(active: &[&str], flow_plugin: Option<&str>) -> (TempDir, Workspace) {
        let mut products = toml::value::Table::new();
        if let Some(plugin) = flow_plugin {
            let mut flow = toml::value::Table::new();
            flow.insert("plugin".into(), toml::Value::String(plugin.into()));
            products.insert("flow".into(), toml::Value::Table(flow));
        }
        let cfg = WorkspaceConfig {
            active_modules: active.iter().map(|s| s.to_string()).collect(),
            products,
        };
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &cfg).unwrap();
        (tmp, ws)
    }

    /// The active modules as persisted in `config.toml` after a back-fill (re-read from disk).
    fn persisted_active(ws: &Workspace) -> Vec<String> {
        let raw = std::fs::read_to_string(ws.dir.join("config.toml")).unwrap();
        let cfg: WorkspaceConfig = toml::from_str(&raw).unwrap();
        cfg.active_modules
    }

    #[test]
    fn backfills_flow_when_its_section_is_present_but_inactive() {
        // The core bug: a pre-umbrella flow workspace (a `[flow]` section, empty active_modules).
        let (_t, ws) = workspace_with(&[], Some("issue-tracker"));
        assert_eq!(
            backfill_active_modules(&roster(), &ws).unwrap(),
            vec!["flow".to_string()]
        );
        assert_eq!(persisted_active(&ws), vec!["flow".to_string()], "persisted");
    }

    #[test]
    fn is_a_noop_on_a_healthy_flow_workspace() {
        // flow already active: nothing to register, no write.
        let (_t, ws) = workspace_with(&["flow"], Some("issue-tracker"));
        assert!(backfill_active_modules(&roster(), &ws).unwrap().is_empty());
        assert_eq!(persisted_active(&ws), vec!["flow".to_string()]);
    }

    #[test]
    fn backfills_flow_alongside_an_already_active_memory_without_displacing_it() {
        // The user's reproduced path: `nxm init` over a pre-umbrella flow workspace left
        // active=["memory"] with flow's section still present. flow must be back-filled, memory kept.
        let (_t, ws) = workspace_with(&["memory"], Some("issue-tracker"));
        assert_eq!(
            backfill_active_modules(&roster(), &ws).unwrap(),
            vec!["flow".to_string()]
        );
        assert_eq!(
            persisted_active(&ws),
            vec!["memory".to_string(), "flow".to_string()],
            "memory is not displaced; flow is appended in roster order"
        );
    }

    #[test]
    fn does_not_conjure_flow_for_a_memory_only_workspace() {
        // No `[flow]`/`plugin` section ⇒ flow was never initialized here ⇒ never back-filled.
        let (_t, ws) = workspace_with(&["memory"], None);
        assert!(backfill_active_modules(&roster(), &ws).unwrap().is_empty());
        assert_eq!(persisted_active(&ws), vec!["memory".to_string()]);
    }

    #[test]
    fn backfills_flow_from_a_legacy_top_level_plugin_scalar() {
        // A pre-rework config (`plugin = "..."`, no `[flow]` table) is still a flow workspace.
        let tmp = TempDir::new().unwrap();
        let cfg: WorkspaceConfig = toml::from_str("plugin = \"personal-todo\"\n").unwrap();
        let ws = workspace::setup(tmp.path(), &cfg).unwrap();
        assert_eq!(
            backfill_active_modules(&roster(), &ws).unwrap(),
            vec!["flow".to_string()]
        );
        assert_eq!(persisted_active(&ws), vec!["flow".to_string()]);
    }

    #[test]
    fn is_idempotent_across_repeated_heals() {
        let (_t, ws) = workspace_with(&[], Some("issue-tracker"));
        backfill_active_modules(&roster(), &ws).unwrap();
        // Re-resolve the now-healed workspace and heal again: a clean no-op.
        let ws2 = workspace::discover(ws.dir.parent().unwrap()).unwrap();
        assert!(backfill_active_modules(&roster(), &ws2).unwrap().is_empty());
        assert_eq!(persisted_active(&ws2), vec!["flow".to_string()]);
    }
}
