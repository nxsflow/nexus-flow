//! **The bridge lines from the old code paths into the new engine** (6j6v.zjyv §10; ticket 6j6v.b8tj): where
//! a verb of the old surface meets a channel declared `engine: v2` or a session of a run on the new engine,
//! it comes through here, so each such question has one answer and one place to delete.
//!
//! What this part covers: the send switch (a terminal's send to a v2 channel is commissioned through
//! `run_host::commission`, and `channel_open` is never entered), the two send refusals (a persona's send to a
//! v2 channel, and any send from a v2 session), and [`session_of`] — the ONE lookup every caller check on
//! both engines makes first, so a v2 session is a SESSION to every old gate and never "the owner at the
//! terminal".
//!
//! **Every lookup into `run.sqlite` answers `Result`.** Only a MISSING file means "not v2"; a file that is
//! locked, damaged or written by a newer nxs refuses the verb with the reason and the recovery, and never
//! falls through to the old tables ([`store_refusal`]).
// v2-bridge: deleted with the old engine (6j6v.2vtw)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexus_run_contracts::{LivenessPolicy, Timestamp};
use nexus_run_host::bridge::{self, BridgeCtx, Refused, SqliteRunStore};
use nexus_run_host::Layout;

use crate::awaiting::Await;
use crate::definitions::Definitions;
use crate::error::{NxfError, Result};
use crate::model::Refs;
use crate::orchestration::Ctx;
use crate::ports_v1::V1Ports;
use crate::surface::{SendToReceipt, TargetKind};

/// `run.sqlite` beside the workspace's `db.sqlite`, in the same `.nxs` directory.
pub fn run_sqlite(db_path: &str) -> PathBuf {
    Path::new(db_path).with_file_name("run.sqlite")
}

/// The workspace root the `.nxs` directory of `db_path` sits in, and where the agent logs are kept.
fn layout(db_path: &str) -> Layout {
    let nxs = Path::new(db_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    Layout {
        root: nxs.parent().map(Path::to_path_buf).unwrap_or_default(),
        agent_logs: nxs.join("agent-logs"),
        session_env: Vec::new(),
    }
}

fn bridge_ctx(store: SqliteRunStore, db_path: &str, origin: &str, defs: &Definitions) -> BridgeCtx {
    let ports = Arc::new(V1Ports::with_definitions(db_path, origin, defs.clone()));
    BridgeCtx {
        store: Arc::new(store),
        messages: ports.clone(),
        declarations: ports,
        layout: layout(db_path),
        workspace_id: origin.to_owned(),
    }
}

fn store_refusal(path: &Path, reason: &str) -> NxfError {
    NxfError::io(format!("{}: {reason}", path.display()))
}

fn refusal_of(path: &Path, refused: Refused) -> NxfError {
    match refused {
        Refused::Store(error) => store_refusal(path, &error.to_string()),
        other => NxfError::io(other.to_string()),
    }
}

/// **A terminal's send to a v2 channel**: one transaction in `run.sqlite` opens the run, and the receipt
/// carries the root thread it minted at once. Nothing is posted here — the host posts the root message once
/// the run's log can answer for it (§6.4 item 1), which is why `message_id` is `None`.
pub fn commission(
    ctx: &Ctx,
    channel: &str,
    body: &str,
    refs: Refs,
    refs_warning: Option<String>,
) -> Result<SendToReceipt> {
    let path = run_sqlite(ctx.db_path);
    let store = SqliteRunStore::open(&path, bridge::BRIDGE_BUSY_TIMEOUT)
        .map_err(|error| store_refusal(&path, &error.to_string()))?;
    let bridge = bridge_ctx(store, ctx.db_path, ctx.origin, ctx.defs);
    let now = Timestamp::parse(ctx.now)
        .ok_or_else(|| NxfError::validation(format!("`{}` is not an RFC 3339 instant", ctx.now)))?;
    let sender = format!("{}/{}", ctx.origin, ctx.actor);
    let warnings = Vec::new();
    let receipt = bridge::commission(
        &bridge,
        channel,
        body,
        &nexus_run_host::ports::Refs {
            nxf_ids: refs.nxf_ids,
        },
        &sender,
        LivenessPolicy::default(),
        now,
    )
    .map_err(|r| refusal_of(&path, r))?;
    for timeout in &receipt.ignored_timeouts {
        eprintln!(
            "warning: {channel} declares `timeout: {timeout}`, which the new engine does not read yet"
        );
    }
    let root = receipt.root_thread;
    Ok(SendToReceipt {
        thread_id: root.clone(),
        message_id: None,
        to: channel.to_owned(),
        target: TargetKind::Channel,
        channel: channel.to_owned(),
        session: None,
        queued_behind: None,
        queue_position: None,
        expects: Vec::new(),
        deadline: None,
        // Nothing was started by this call: the host starts the run's sessions on its own tick.
        spawned: false,
        handed_to: None,
        border: None,
        warnings,
        refs_warning,
        await_: Await::run(&root),
    })
}
