//! **The host the background service runs** (6j6v.pn9c; §8.1): one per registered workspace that has a
//! `run.sqlite`, over the workspace's own ports, with the Claude adapter as its backend, the real `GitTree` as
//! its directory and the machine's clock. The service builds it here, because the adapter is a crate only the
//! new engine's crates may import (§5, `cargo xtask layering check`).
//!
//! And the one question the service asks besides `tick`: whether a session of an open run may still have a
//! process, so the machine is held awake while it does (§10, `a_run_is_alive`).

use std::path::Path;
use std::sync::Arc;

use nexus_run_contracts::{SessionBackend, StoreError};
use nexus_run_session_claude::ClaudeBackend;
use nexus_run_store_sqlite::SqliteRunStore;

use crate::bridge::{live_in, LiveSession, Refused, HOST_BUSY_TIMEOUT};
use crate::git_tree::{GitBounds, GitTree};
use crate::ports::SystemClock;
use crate::tree_ledger::TreeLedger;
use crate::{Host, Ports};

/// The service's host over `run_sqlite`. `workspace_id` is the chat origin the bridge commissions with, so the
/// host's `host-seen` record is the one a send reads (§8.1). The adapter starts from the bundle the sidecar port
/// names; where it names none, the port refuses the start before the adapter is asked.
pub fn service_host(
    run_sqlite: &Path,
    ports: Ports,
    workspace_id: &str,
) -> Result<Host, StoreError> {
    let store = Arc::new(SqliteRunStore::open(run_sqlite, HOST_BUSY_TIMEOUT)?);
    let bundle = ports.sidecar.bundle_path().unwrap_or_default();
    let backend = Arc::new(ClaudeBackend::new(ports.layout.agent_logs.clone(), bundle));
    let tree = Arc::new(GitTree::new(
        &ports.layout.root,
        TreeLedger::new(&ports.layout.agent_logs, workspace_id),
        GitBounds::default(),
    ));
    Ok(Host::new(
        store,
        ports,
        backend,
        tree,
        Arc::new(SystemClock),
    ))
}

impl Host {
    /// What `a_run_is_alive` asks of this host's workspace, over the host's own connection.
    pub fn live_sessions(&self) -> Result<Vec<LiveSession>, Refused> {
        live_in(&self.store)
    }

    /// The backend's name, as its capabilities state it. Public for the service's tests and this crate's
    /// integration test, which prove the service's host runs the Claude adapter.
    pub fn backend_name(&self) -> String {
        SessionBackend::capabilities(&*self.backend).backend
    }
}
