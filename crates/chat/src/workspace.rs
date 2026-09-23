//! Workspace resolution is foundation-owned (the `.nxs/` directory belongs to the platform, not
//! chat); this module re-exports it so `nexus_chat::workspace::*` paths resolve, and adds the chat
//! half: opening the *chat* store (registers the message reducer + creates the six views) and
//! building chat's foundation config. Mirrors flow's/memory's `WorkspaceExt`.

pub use nxs_foundation::workspace::*;

use crate::error::{NxfError, Result};
use crate::store::ChatStore;
use std::path::{Path, PathBuf};

/// chat's module key in the foundation config's `active_modules` (spec §7). The `nxc init` that
/// *activates* it is T3; T2 only needs the constant + store-open + a fresh-workspace config.
pub const CHAT_MODULE: &str = "chat";

/// The MINTING WORKSPACE IDENTITY (spec §2.2) every message, thread and channel written in `ws` is
/// stamped with, and the `<origin>` half of every qualified handle `<origin>/<agent>` minted there:
/// **the workspace's own replica prefix**.
///
/// **A per-workspace value, and M1 says so in as many words**: "in M1 it is the local workspace
/// identity"; the org/repo-qualified form is finalized with the federation bridge (nxf 6j6v.kz8p).
/// One string per workspace, the same for every caller in it — and when 6j6v.kz8p gives a workspace
/// a real federated identity, this function is the one place it lands.
///
/// **Written once because BOTH seams read it** (nxf 6j6v.07me). `cli.rs`'s `origin()` falls back to
/// it when `NXC_ORIGIN` is unset, and [`crate::engine::Engine::origin`] answers with it for every
/// call through the handle — so the two seams cannot end up naming the same workspace differently,
/// which is the failure that would make an app's `<prefix>/coder` and a terminal's `local/coder`
/// two different members of one declared channel. `tests/parity.rs`'s
/// `both_adapters_resolve_one_origin_for_one_workspace` is the gate on that agreement; there is no
/// second derivation of this rule anywhere for it to drift from.
///
/// **It was the constant `"local"` until the owner ruled otherwise** (2026-08-21, on 6j6v.07me):
/// the first build kept the CLI's old default for behaviour-equality, and the owner chose the
/// replica prefix instead, following the item's own wording — `Engine::workspace()` already holds
/// the replica identity, so a workspace-independent literal made every workspace on a machine mint
/// under one name. The user-visible consequence is that an agent's qualified handle is
/// `<prefix>/coder`, not `local/coder`.
///
/// **The prefix is REASSIGNABLE, and that hazard is accepted rather than solved here.**
/// `foundation::workspace::adopt_prefix` rewrites `replica.prefix` when sync finds a colliding one,
/// and every `<prefix>/<agent>` already written then names a workspace identity that no longer
/// exists. The owner took that knowingly; it is tracked as nxf 6j6v.dnzt. Do not reach for
/// `replica_uuid` here to dodge it — that is 26 characters no surface ever shows, and the trade was
/// made with both sides on the table.
pub fn origin_of(ws: &Workspace) -> &str {
    &ws.replica.prefix
}

/// Where a workspace's persona/channel/flow declarations belong: `<workspace-root>/.nxs-personas`,
/// sibling to the `.nxs/` directory itself (nxf 6j6v.dvyq — THE break of that item).
///
/// The dot prefix and the product name are both deliberate: the folder is nxsflow's own, it sits in
/// the USER's repository next to `.nxs/`, and a bare `roles/` at a project root is a name any
/// project might already have taken for something else entirely.
pub const PERSONAS_DIR: &str = ".nxs-personas";

/// The LEGACY declaration folder, `<workspace-root>/roles`, which this workspace still READS (and
/// reports) but never writes (nxf 6j6v.dvyq step 4). See [`crate::definitions::DeclarationSource`]
/// for the resolution order and for why the engine does not migrate the folder itself.
pub const LEGACY_ROLES_DIR: &str = "roles";

/// Opens chat's [`ChatStore`] over a resolved [`Workspace`]. WAL mode lets parallel `nxc` processes
/// serialize their short write transactions instead of failing with "database is locked". Bring this
/// trait into scope to call `ws.open_chat_store()`.
pub trait ChatWorkspaceExt {
    fn open_chat_store(&self) -> Result<ChatStore>;

    /// The project root this workspace sits in: `Workspace.dir` IS the `.nxs/` directory (confirmed
    /// by `Workspace::db_path` joining `db.sqlite` straight onto `dir`), so the root is its parent.
    fn workspace_root(&self) -> Result<&Path>;

    /// Where a declaration BELONGS in this workspace: `<workspace-root>/.nxs-personas`
    /// ([`PERSONAS_DIR`]). This is the answer every surface renders when there is nothing declared
    /// — it names the place, not what happens to be there today, so it is well-defined even for a
    /// workspace that has no declaration folder at all.
    fn personas_dir(&self) -> Result<PathBuf>;

    /// The legacy `<workspace-root>/roles` folder ([`LEGACY_ROLES_DIR`]) — the location
    /// declarations lived at before 6j6v.dvyq. Resolved, not necessarily existing.
    fn legacy_roles_dir(&self) -> Result<PathBuf>;

    /// The folder declarations are actually READ from, resolved over both locations — see
    /// [`crate::definitions::DeclarationSource::locate`], which owns the rule. Shared by every
    /// declared-persona/channel/flow loader, so the CLI and [`crate::engine::Engine`] cannot
    /// resolve it differently (independent review, Integrity #3/Code Quality #1, High).
    fn declaration_dir(&self) -> Result<PathBuf>;
}

impl ChatWorkspaceExt for Workspace {
    fn open_chat_store(&self) -> Result<ChatStore> {
        let path = self.db_path_str()?;
        let store = ChatStore::open(&path, self.replica.site_id)
            .map_err(|e| NxfError::io(format!("opening chat store at {path}: {e}")))?;
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
        Ok(store)
    }

    fn workspace_root(&self) -> Result<&Path> {
        self.dir.parent().ok_or_else(|| {
            NxfError::io(format!(
                "workspace directory has no parent to resolve {PERSONAS_DIR}/ against"
            ))
        })
    }

    fn personas_dir(&self) -> Result<PathBuf> {
        Ok(self.workspace_root()?.join(PERSONAS_DIR))
    }

    fn legacy_roles_dir(&self) -> Result<PathBuf> {
        Ok(self.workspace_root()?.join(LEGACY_ROLES_DIR))
    }

    fn declaration_dir(&self) -> Result<PathBuf> {
        Ok(crate::definitions::DeclarationSource::locate(self.workspace_root()?).read_dir)
    }
}

/// Build the foundation config for a FRESH chat-only workspace: register `chat` as the sole active
/// module. When chat joins an EXISTING workspace, `foundation::setup` loads that config unchanged
/// (the `nxc init` that registers chat via `activate_module` is T3); this seeds only a brand-new
/// `.nxs/` (used by the T2 verb tests to stand up a workspace without the T3 `init` verb).
pub fn chat_config() -> WorkspaceConfig {
    WorkspaceConfig {
        active_modules: vec![CHAT_MODULE.to_string()],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_config_registers_only_chat() {
        assert_eq!(chat_config().active_modules, vec!["chat".to_string()]);
    }

    #[test]
    fn open_chat_store_materializes_views_over_a_fresh_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = setup(tmp.path(), &chat_config()).unwrap();
        let store = ws.open_chat_store().unwrap();
        // The `messages` view exists after open (the store applied the DDL + registered the reducer).
        let n: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
