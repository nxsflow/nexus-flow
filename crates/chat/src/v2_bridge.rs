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
use nexus_run_host::bridge::{self, BridgeCtx, Refused, SqliteRunStore, LEASE_EXPIRY};
use nexus_run_host::Layout;

use crate::awaiting::Await;
use crate::definitions::Definitions;
use crate::error::{NxfError, Result};
use crate::model::Refs;
use crate::orchestration::{Ctx, FailedConsequence};
use crate::ports_v1::V1Ports;
use crate::surface::{SendToReceipt, TargetKind};

/// `run.sqlite` beside the workspace's `db.sqlite`, in the same `.nxs` directory.
pub fn run_sqlite(db_path: &str) -> PathBuf {
    Path::new(db_path).with_file_name("run.sqlite")
}

/// What `nxs service status` is for, said where a send to a v2 channel finds nobody attending (§8.1).
pub const NOT_ATTENDED: &str = "no service attends this workspace — `nxs service status`";

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

/// **The refusal for a `run.sqlite` that does not answer**, with what to do about it: a verb is never run
/// against the old tables because the new engine's file could not be read.
pub fn store_refusal(path: &Path, reason: &str) -> NxfError {
    NxfError::new(crate::error::ErrorKind::Conflict, format!(
        "{} did not answer: {reason}. Nothing was written. A locked file answers again once its \
         writer lets go — run the command again; `nxs service status` names the service. A file written \
         by a newer nxs needs that nxs (`nxs self-update`). A damaged one: move it aside (`mv {} {}.damaged`), \
         which abandons every run on the new engine recorded in it — their threads stay readable in \
         `nxc threads show` — and leaves the old engine's runs untouched.",
        path.display(),
        path.display(),
        path.display(),
    ))
}

fn refusal_of(path: &Path, refused: Refused) -> NxfError {
    match refused {
        Refused::Store(error) => store_refusal(path, &error.to_string()),
        Refused::Compile(_) => NxfError::validation(refused.to_string()),
        Refused::Port(_) => NxfError::io(refused.to_string()),
        other => NxfError::new(crate::error::ErrorKind::Conflict, other.to_string()),
    }
}

/// The persona handle of a v2 session whose slot the run's log cannot place: a session all the same, never
/// the owner, and a handle no declaration can have (a declared handle never starts with `__`).
pub const UNPLACED_V2_SESSION: &str = "__unplaced_v2_session__";

/// A session of a run on the new engine, as the old gates must see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Session {
    /// The persona its slot runs, which is what it is to every old gate.
    pub handle: String,
    pub session_key: String,
}

/// **Whether `session` is a session of a run on the new engine** — the one lookup, made before every
/// caller check on both engines (§10). `None` for no session, for no `run.sqlite`, and for a key the run
/// store does not know.
pub fn session_of(
    db_path: &str,
    origin: &str,
    defs: &Definitions,
    session: Option<&str>,
) -> Result<Option<V2Session>> {
    let Some(session) = session else {
        return Ok(None);
    };
    let path = run_sqlite(db_path);
    let store = match bridge::open_store(&path) {
        Ok(None) => return Ok(None),
        Ok(Some(store)) => store,
        Err(error) => return Err(store_refusal(&path, &error.to_string())),
    };
    let ctx = bridge_ctx(store, db_path, origin, defs);
    let Some(route) = bridge::session_of(&ctx, session).map_err(|r| refusal_of(&path, r))? else {
        return Ok(None);
    };
    let handle = bridge::view(&ctx.store, &route.run)
        .map_err(|r| refusal_of(&path, r))?
        .and_then(|view| {
            let index = view.state.find(session)?;
            Some(view.state.slots[index].slot.member.to_string())
        })
        // A key the run store routes and its log cannot place is still a session — never the owner.
        .unwrap_or_else(|| UNPLACED_V2_SESSION.to_owned());
    Ok(Some(V2Session {
        handle,
        session_key: session.to_owned(),
    }))
}

/// **Who is calling, with the v2 lookup first** — the one resolution every caller check of both engines
/// goes through (§10): an explicit `--persona` is the claim a host made and stands; otherwise a session of a
/// run on the new engine is the persona its slot runs, and only then does the old session map answer.
pub fn resolve_caller(
    store: &crate::store::ChatStore,
    defs: &Definitions,
    declared: Option<&str>,
    session: Option<&str>,
    db_path: &str,
    origin: &str,
) -> Result<(crate::persona::Identity, Option<V2Session>)> {
    if declared.is_some() {
        let identity = crate::persona::resolve_identity(store, defs.roles(), declared, session)?;
        return Ok((identity, None));
    }
    let v2 = session_of(db_path, origin, defs, session)?;
    let identity = match &v2 {
        Some(v2) => crate::persona::Identity::Persona(v2.handle.clone()),
        None => crate::persona::resolve_identity(store, defs.roles(), None, session)?,
    };
    Ok((identity, v2))
}

/// No v2 session commissions in this slice (§10).
pub fn session_cannot_send(session: &V2Session) -> NxfError {
    NxfError::forbidden(format!(
        "a session of a run on the new engine cannot commission in this slice — ask for what you \
         need with `nxc reply --escalate` (you are `{}`, session {}); nothing was sent",
        session.handle, session.session_key
    ))
}

/// No persona commissions a v2 run in this slice (owner decision of 2026-11-04, §10).
pub fn persona_cannot_send(channel: &str) -> NxfError {
    NxfError::forbidden(format!(
        "{channel} runs on the new engine, which cannot yet answer a persona; commission it from the \
         terminal until the personas move (6j6v.2vtw) — nothing was sent"
    ))
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
    // One requester, one form: a host may hand its user qualified or bare, and the record is the qualified
    // handle every v1 sender has — what the root's requester checks compare against.
    let sender = format!(
        "{}/{}",
        ctx.origin,
        crate::orchestration::bare_handle_of(ctx.actor)
    );
    let attended = attended(&bridge, now).map_err(|r| refusal_of(&path, r))?;
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
    let mut warnings = Vec::new();
    if !attended {
        eprintln!("warning: {NOT_ATTENDED}");
        warnings.push(FailedConsequence::service_not_running(
            &root,
            NOT_ATTENDED.to_owned(),
        ));
    }
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

/// Whether a service has stepped this workspace's runs within two lease expiries (§8.1).
fn attended(ctx: &BridgeCtx, now: Timestamp) -> std::result::Result<bool, Refused> {
    let window = 2 * LEASE_EXPIRY.0 as i64 * 1000;
    Ok(ctx
        .store
        .host_seen(&ctx.workspace_id)?
        .is_some_and(|at| now.unix_millis() - at.unix_millis() <= window))
}
