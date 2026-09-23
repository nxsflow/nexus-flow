//! Sync binding state lives in `.nxs/sync.toml`. The sync VERB (bind/run) moved to the umbrella as
//! `nxs sync` (aye.2.1): with one shared op-log, sync is a platform operation, not a flow-module
//! one. flow only needs to know WHETHER the workspace is bound, to gate `prime`'s sync hints —
//! a pure existence check, independent of who wrote the file.

use crate::workspace::Workspace;

const SYNC_META_FILE: &str = "sync.toml";

/// Whether this workspace is bound to a sync stream, by the mere presence of `.nxs/sync.toml`.
/// Deliberately a pure existence check: `prime` uses it to decide whether to surface sync hints and
/// must stay infallible and side-effect-free even if the meta is unreadable — orientation text
/// never fails the command.
pub fn is_bound(ws: &Workspace) -> bool {
    ws.dir.join(SYNC_META_FILE).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace;
    use tempfile::TempDir;

    #[test]
    fn is_bound_reflects_the_presence_of_sync_toml() {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
        assert!(!is_bound(&ws), "a fresh workspace is unbound");
        std::fs::write(ws.dir.join(SYNC_META_FILE), "stream_id = \"s\"\n").unwrap();
        assert!(is_bound(&ws), "a present sync.toml means bound");
    }
}
