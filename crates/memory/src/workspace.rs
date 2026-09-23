//! Workspace resolution is foundation-owned (the `.nxs/` directory belongs to the platform, not
//! memory); this module re-exports it so `nexus_memory::workspace::*` paths resolve, and adds the
//! memory-product half: opening the *memory* store (registers the fact reducer + creates the
//! `memories` view) and building memory's foundation config. Mirrors flow's `WorkspaceExt`.

pub use nxs_foundation::workspace::*;

use crate::error::{NxfError, Result};
use crate::store::MemoryStore;

/// memory's module key in the foundation config's `active_modules` (spec §6.3/§7).
pub const MEMORY_MODULE: &str = "memory";

/// Opens memory's [`MemoryStore`] over a resolved [`Workspace`]. WAL mode lets parallel `nxm`
/// processes serialize their short write transactions instead of failing with "database is locked".
/// Bring this trait into scope to call `ws.open_memory_store()`.
pub trait MemoryWorkspaceExt {
    fn open_memory_store(&self) -> Result<MemoryStore>;
}

impl MemoryWorkspaceExt for Workspace {
    fn open_memory_store(&self) -> Result<MemoryStore> {
        let path = self.db_path_str()?;
        let store = MemoryStore::open(&path, self.replica.site_id)
            .map_err(|e| NxfError::io(format!("opening memory store at {path}: {e}")))?;
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
        Ok(store)
    }
}

/// Build the foundation config for a FRESH memory-only workspace: register `memory` as the sole
/// active module. When memory joins an EXISTING workspace, `foundation::setup` loads that config
/// unchanged, so `nxm init` registers memory via [`activate_module`](nxs_foundation::workspace)
/// instead — this seeds only a brand-new `.nxs/`.
pub fn memory_config() -> WorkspaceConfig {
    WorkspaceConfig {
        active_modules: vec![MEMORY_MODULE.to_string()],
        ..Default::default()
    }
}
