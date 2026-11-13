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

use nexus_run_contracts::{LivenessPolicy, RunStore, Timestamp};
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
    /// The step thread the session answers in: what its id-free `nxc reply` resolves to (§10).
    pub thread: String,
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
    let thread = ctx
        .store
        .get(&format!("session/{session}"))
        .map_err(|error| store_refusal(&path, &error.to_string()))?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
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
        thread,
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
    // On the receipt only: the CLI prints a receipt's warnings, and an embedder reads them from it.
    if !attended {
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

/// Whether a service has stepped this workspace's runs within two lease expiries of now (§8.1). Either
/// side: after clock skew a record from beyond the window in the future proves nothing about now.
fn attended(ctx: &BridgeCtx, now: Timestamp) -> std::result::Result<bool, Refused> {
    let window = 2 * LEASE_EXPIRY.0 * 1000;
    Ok(ctx
        .store
        .host_seen(&ctx.workspace_id)?
        .is_some_and(|at| now.unix_millis().abs_diff(at.unix_millis()) <= window))
}

// ---- after the send: reply, withdraw, resume and status on a run (6j6v.y6v7) -------------------------

/// The bridge's connection over `ctx`'s workspace, or `None` where there is no `run.sqlite`.
fn open_bridge(
    db_path: &str,
    origin: &str,
    defs: &Definitions,
) -> Result<Option<(BridgeCtx, PathBuf)>> {
    let path = run_sqlite(db_path);
    match bridge::open_store(&path) {
        Ok(None) => Ok(None),
        Ok(Some(store)) => Ok(Some((bridge_ctx(store, db_path, origin, defs), path))),
        Err(error) => Err(store_refusal(&path, &error.to_string())),
    }
}

/// A thread of a run on the new engine, with the connection that found it.
struct RunThread {
    bridge: BridgeCtx,
    path: PathBuf,
    route: bridge::ThreadRoute,
}

/// **Whether `thread` belongs to a run on the new engine** — the lookup every verb below makes FIRST, so a
/// v2 thread never falls through to the old tables. `None` only for no `run.sqlite`, or a thread it does not
/// route; a file that does not answer refuses (§10).
fn run_thread(ctx: &Ctx, thread: &str) -> Result<Option<RunThread>> {
    let Some((bridge, path)) = open_bridge(ctx.db_path, ctx.origin, ctx.defs)? else {
        return Ok(None);
    };
    let route = bridge::thread_route(&bridge, thread).map_err(|r| refusal_of(&path, r))?;
    Ok(route.map(|route| RunThread {
        bridge,
        path,
        route,
    }))
}

fn now_of(ctx: &Ctx) -> Result<Timestamp> {
    Timestamp::parse(ctx.now)
        .ok_or_else(|| NxfError::validation(format!("`{}` is not an RFC 3339 instant", ctx.now)))
}

/// The run's fold, as the bridge reads it; a run it cannot read at all is refused as finished, which is what
/// every bridge verb answers for it.
fn view_of(thread: &RunThread) -> Result<bridge::RunState> {
    bridge::view(&thread.bridge.store, &thread.route.run)
        .map_err(|r| refusal_of(&thread.path, r))?
        .map(|view| view.state)
        .ok_or_else(|| refusal_of(&thread.path, Refused::RunFinished))
}

/// **The requester at the terminal, or the refusal that says who may** (§10): a session of a run on the new
/// engine is never the requester, nor is a persona session or any caller presenting a session; and the
/// caller's handle is compared with the recorded requester through `same_identity`, so a host handing its
/// user bare or qualified is one person (memory declared-members-bare-caller-qualified). What the bridge is
/// handed is the recorded requester itself, so its own check compares a value with itself.
fn the_requester(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    state: &bridge::RunState,
    verb: &str,
) -> Result<bridge::Caller> {
    let (identity, v2) =
        resolve_caller(store, ctx.defs, None, ctx.session, ctx.db_path, ctx.origin)?;
    if let Some(v2) = v2 {
        return Err(NxfError::forbidden(format!(
            "a session of a run on the new engine cannot {verb} on a run's root thread — that is the \
             requester's, at the terminal. Say what you need in your own thread with `nxc reply \
             --escalate` (you are `{}`, session {}); nothing was written",
            v2.handle, v2.session_key
        )));
    }
    if let crate::persona::Identity::Persona(handle) = &identity {
        return Err(not_the_requester(verb, &state.requester, Some(handle)));
    }
    if ctx.session.is_some()
        || !crate::facade::same_identity(Some(state.requester.as_str()), ctx.actor)
    {
        return Err(not_the_requester(verb, &state.requester, None));
    }
    Ok(bridge::Caller {
        actor: state.requester.clone(),
        session: None,
    })
}

fn not_the_requester(verb: &str, requester: &str, persona: Option<&str>) -> NxfError {
    let you = persona.map_or_else(String::new, |handle| format!(" (you are `{handle}`)"));
    NxfError::forbidden(format!(
        "only {requester}, who commissioned this run, can {verb} on its root thread, from the \
         terminal{you}; nothing was written"
    ))
}

/// A bridge refusal as the old surface words it, naming the thread where that helps.
fn refused_on(thread: &RunThread, refused: Refused) -> NxfError {
    match refused {
        Refused::NotTheSlot { owed_by } => NxfError::forbidden(format!(
            "this thread is answered by {owed_by}'s own session only — the session the run started \
             for that step; nothing was written"
        )),
        Refused::RunFinished => NxfError::validation(
            "the run has finished; start a new commission with `nxc send` — nothing was written",
        ),
        other => refusal_of(&thread.path, other),
    }
}

/// **A reply into a thread of a run on the new engine** — `None` when `req.thread` is not one, so the old
/// engine answers. Checked before anything is written (IN1): a STEP thread is answered by its slot's own
/// session only, while the slot waits for a reply; the ROOT by the requester only, and only while the run
/// asks a question — a held or parked run is resumed with `nxc resume` and never by a reply (IN3), a working
/// one has nothing to answer, a finished one takes a new commission. `--accept` is valid on the root at a
/// ceiling only.
pub fn reply_in_a_run(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    req: &crate::surface::ReplyThreadRequest,
) -> Result<Option<crate::orchestration::ReplyReceipt>> {
    let Some(thread) = run_thread(ctx, req.thread)? else {
        return Ok(None);
    };
    if req.machine.is_some() {
        return Err(NxfError::validation(
            "a run on the new engine runs on this machine; `--machine` names nothing here — \
             nothing was written",
        ));
    }
    let now = now_of(ctx)?;
    let state = view_of(&thread)?;
    let root = state
        .root_thread
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| req.thread.to_owned());
    let message = match &thread.route.session_key {
        Some(key) => reply_in_a_step(ctx, store, &thread, key, req, now)?,
        None => answer_on_the_root(ctx, store, &thread, &state, req, now)?,
    };
    let at_root = thread.route.session_key.is_none();
    Ok(Some(crate::orchestration::ReplyReceipt {
        posted: true,
        message_id: Some(message.to_string()),
        thread_id: Some(req.thread.to_owned()),
        resumed: false,
        woke: None,
        wake_skipped: None,
        completed: None,
        warnings: Vec::new(),
        held: None,
        handed_to: None,
        border: None,
        // The requester follows the run as the send's receipt told them to; a session's turn ends here.
        await_: at_root.then(|| Await::run(&root)),
    }))
}

fn reply_in_a_step(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    thread: &RunThread,
    key: &str,
    req: &crate::surface::ReplyThreadRequest,
    now: Timestamp,
) -> Result<nexus_run_contracts::MessageId> {
    if req.accept {
        return Err(NxfError::validation(
            "`--accept` overrules a verdict on the ROOT of a run you commissioned, at its ceiling; a \
             step's thread is answered plainly, with `--escalate` or `--needs-rework` — nothing was \
             written",
        ));
    }
    let (_, v2) = resolve_caller(store, ctx.defs, None, ctx.session, ctx.db_path, ctx.origin)?;
    // The slot's own session presents its key; its handle is the slot's, read from the run's log — the
    // bridge then compares the slot with itself. Anyone else is handed on as they are, and refused there.
    let caller = match v2 {
        Some(v2) if v2.session_key == key => bridge::Caller {
            actor: v2.handle,
            session: Some(v2.session_key),
        },
        _ => bridge::Caller {
            actor: crate::orchestration::bare_handle_of(ctx.actor).to_owned(),
            session: ctx.session.map(str::to_owned),
        },
    };
    bridge::reply(
        &thread.bridge,
        req.thread,
        &caller,
        req.body,
        bridge::ReplyFlags {
            escalate: req.escalate,
            needs_rework: req.needs_rework,
        },
        now,
    )
    .map_err(|r| refused_on(thread, r))
}

fn answer_on_the_root(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    thread: &RunThread,
    state: &bridge::RunState,
    req: &crate::surface::ReplyThreadRequest,
    now: Timestamp,
) -> Result<nexus_run_contracts::MessageId> {
    use bridge::RunView;
    if state.is_finished() {
        return Err(refused_on(thread, Refused::RunFinished));
    }
    if req.escalate || req.needs_rework {
        return Err(NxfError::validation(
            "a run's root thread is answered plainly, or with `--accept` at a ceiling; \
             `--escalate` and `--needs-rework` are said in a step's thread — nothing was written",
        ));
    }
    let caller = the_requester(ctx, store, state, "answer")?;
    let root = req.thread;
    match state.view() {
        RunView::NeedsDecision { .. } => {}
        RunView::Held { .. } | RunView::Parked { .. } => {
            return Err(NxfError::validation(format!(
                "the run on thread {root} is on hold or parked and asks no question, so a reply does \
                 not restart it: read why with `nxc status --thread {root}`, restart it with `nxc \
                 resume --thread {root}`, or take it back with `nxc withdraw --thread {root}` — \
                 nothing was written"
            )));
        }
        RunView::Finishing { .. } | RunView::Finished { .. } => {
            return Err(refused_on(thread, Refused::RunFinished));
        }
        _ => {
            return Err(NxfError::validation(format!(
                "the run on thread {root} is working and asks no question: follow it with `nxc \
                 status --thread {root}`; adding to a running run arrives in a later slice \
                 (6j6v.3rrv), or take it back with `nxc withdraw --thread {root}` — nothing was \
                 written"
            )));
        }
    }
    bridge::answer(&thread.bridge, root, &caller, req.body, req.accept, now)
        .map_err(|r| refused_on(thread, r))
}

/// The refusal of a verb that takes a run's ROOT, typed on one of its step threads.
fn takes_the_root(verb: &str, state: &bridge::RunState) -> NxfError {
    let root = state.root_thread.as_ref().map_or_else(
        || "its root thread".to_owned(),
        |root| format!("thread {root}"),
    );
    NxfError::validation(format!(
        "`nxc {verb}` takes the run's root, {root}, not a step's thread; nothing was written"
    ))
}

/// **`nxc withdraw` on a run on the new engine** — `None` when `thread` is not one. The requester's, from the
/// terminal, on the root; valid in every state but finished (§6.4 item 10).
pub fn withdraw_a_run(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    thread: &str,
) -> Result<Option<crate::orchestration::WithdrawReceipt>> {
    let Some(run_thread) = run_thread(ctx, thread)? else {
        return Ok(None);
    };
    let now = now_of(ctx)?;
    let state = view_of(&run_thread)?;
    if run_thread.route.session_key.is_some() {
        return Err(takes_the_root("withdraw", &state));
    }
    if state.is_finished() {
        return Err(refused_on(&run_thread, Refused::RunFinished));
    }
    let caller = the_requester(ctx, store, &state, "withdraw")?;
    let body = format!("withdrawn by {}", caller.actor);
    bridge::withdraw(&run_thread.bridge, thread, &caller, &body, now)
        .map_err(|r| refused_on(&run_thread, r))?;
    Ok(Some(crate::orchestration::WithdrawReceipt::of_a_run(
        thread,
        run_thread.route.run.as_str(),
    )))
}

/// **`nxc resume --thread <root>` on a run on the new engine** (IN3) — `None` when `thread` is not one. The
/// requester's, from the terminal: before a hold's `until` it says not yet, and `force` restarts now.
pub fn resume_a_run(
    ctx: &Ctx,
    store: &crate::store::ChatStore,
    thread: &str,
    force: bool,
) -> Result<Option<crate::orchestration::ResumeReceipt>> {
    use crate::orchestration::{ResumeOutcome, ResumeReceipt};
    let Some(run_thread) = run_thread(ctx, thread)? else {
        return Ok(None);
    };
    let now = now_of(ctx)?;
    let state = view_of(&run_thread)?;
    if run_thread.route.session_key.is_some() {
        return Err(takes_the_root("resume", &state));
    }
    if state.is_finished() {
        return Err(refused_on(&run_thread, Refused::RunFinished));
    }
    let caller = the_requester(ctx, store, &state, "resume")?;
    let outcome = bridge::resume(&run_thread.bridge, thread, &caller, force, now)
        .map_err(|r| refused_on(&run_thread, r))?;
    Ok(Some(ResumeReceipt {
        session: None,
        thread: Some(thread.to_owned()),
        outcome: match outcome {
            bridge::ResumeOutcome::Resumed { .. } => ResumeOutcome::Resumed,
            bridge::ResumeOutcome::NotYet { until } => ResumeOutcome::NotYet {
                until: Some(until.to_string()),
            },
        },
        armed_for: None,
        warnings: Vec::new(),
        run: Some(run_thread.route.run.to_string()),
    }))
}

/// **The thread a session of a run on the new engine answers in** — what its id-free `nxc reply` means, asked
/// before the old engine's `sole_open_thread_for_session` (§10). `None` for any other caller.
pub fn thread_of_session(ctx: &Ctx) -> Result<Option<String>> {
    Ok(session_of(ctx.db_path, ctx.origin, ctx.defs, ctx.session)?.map(|v2| v2.thread))
}

/// **The runs on the new engine as `nxc status` reads them** (6j6v.zjyv §10) — over `run.sqlite` alone, with no
/// declarations parsed, because status is the verb that must work against a workspace in any state.
pub struct RunReader {
    bridge: BridgeCtx,
    path: PathBuf,
}

impl RunReader {
    /// `None` where there is no `run.sqlite`; a file that does not answer refuses with the way out.
    pub fn at_workspace(db_path: &str, origin: &str) -> Result<Option<RunReader>> {
        Ok(open_bridge(db_path, origin, &Definitions::default())?
            .map(|(bridge, path)| RunReader { bridge, path }))
    }

    fn refusal_here(&self, refused: Refused) -> NxfError {
        refusal_of(&self.path, refused)
    }

    /// The run a thread belongs to, root or step; `None` for a thread of the old engine.
    pub fn run_of_thread(&self, thread: &str) -> Result<Option<nexus_run_contracts::RunId>> {
        Ok(bridge::thread_route(&self.bridge, thread)
            .map_err(|r| self.refusal_here(r))?
            .map(|route| route.run))
    }

    /// What `nxc status --run <id>` names: a run id, or any thread of the run.
    pub fn run_named(&self, id: &str) -> Result<Option<nexus_run_contracts::RunId>> {
        if let Some(run) = self.run_of_thread(id)? {
            return Ok(Some(run));
        }
        let Some(run) = nexus_run_contracts::RunId::parse(id) else {
            return Ok(None);
        };
        Ok(bridge::view(&self.bridge.store, &run)
            .map_err(|r| self.refusal_here(r))?
            .map(|_| run))
    }

    /// Contract (d) of `run`, with `local` (§10, ST2).
    pub fn run_contract(
        &self,
        run: &nexus_run_contracts::RunId,
        now: Timestamp,
    ) -> Result<Option<nexus_run_contracts::RunStatus>> {
        bridge::status(&self.bridge, run, now).map_err(|r| self.refusal_here(r))
    }

    /// The runs a listing shows: the open ones, or every one with `all`.
    pub fn runs_listed(&self, all: bool) -> Result<Vec<nexus_run_contracts::RunId>> {
        let rows = match all {
            true => self.bridge.store.every_run(),
            false => self.bridge.store.open_runs(),
        }
        .map_err(|error| store_refusal(&self.path, &error.to_string()))?;
        Ok(rows.into_iter().map(|row| row.run).collect())
    }

    /// **A run as one operation of the old shape**: the root thread, then one thread per member session the
    /// run started, each mapped from contract (d) (`open` until finished; `awaiting_human` while it needs
    /// a decision, holds or parks; the root's `working_copy` from (d)'s live tree, with no fingerprint).
    pub fn run_as_operation(
        &self,
        run: &nexus_run_contracts::RunId,
        now: Timestamp,
    ) -> Result<Option<crate::facade::StatusOperation>> {
        use nexus_run_contracts::RunState as Contract;
        let Some(status) = self.run_contract(run, now)? else {
            return Ok(None);
        };
        let Some(view) = bridge::view(&self.bridge.store, run).map_err(|r| self.refusal_here(r))?
        else {
            return Ok(None);
        };
        let Some(root) = view.state.root_thread.as_ref().map(ToString::to_string) else {
            return Ok(None);
        };
        // The substrate channel the root lives in, as every operation of the old engine names its own.
        let channel = Some(format!("decl:{}", status.flow.name));
        let awaiting = matches!(
            status.state,
            Contract::NeedsDecision | Contract::Held { .. } | Contract::Parked
        );
        let working_copy = status.local.as_ref().and_then(|local| {
            let tree = &local.tree;
            tree.commit
                .clone()
                .map(|commit| crate::facade::StatusWorkingCopy {
                    commit,
                    branch: tree.branch.clone().unwrap_or_default(),
                    dirty: tree.dirty.unwrap_or(false),
                    fingerprint: None,
                })
        });
        let mut threads = vec![crate::facade::StatusThread::of_a_run(
            root.clone(),
            channel.clone(),
            None,
            !status.finished,
            awaiting,
            Some(view.state.requester.to_string()),
            None,
            working_copy,
        )];
        for step in &status.steps {
            for member in &step.members {
                let Some(thread) = self
                    .bridge
                    .store
                    .get(&format!("session/{}", member.session_key))
                    .map_err(|error| store_refusal(&self.path, &error.to_string()))?
                else {
                    continue;
                };
                let answered = matches!(member.state.as_str(), "replied" | "ended");
                threads.push(crate::facade::StatusThread::of_a_run(
                    String::from_utf8_lossy(&thread).into_owned(),
                    None,
                    Some(root.clone()),
                    step.state == "open" && !answered,
                    false,
                    None,
                    Some(member.session_key.to_string()),
                    None,
                ));
            }
        }
        Ok(Some(crate::facade::StatusOperation::of_a_run(
            root, channel, threads, status,
        )))
    }
}

/// Whether `thread` belongs to a run on the new engine; a `run.sqlite` that does not answer refuses.
pub fn is_a_run_thread(ctx: &Ctx, thread: &str) -> Result<bool> {
    Ok(run_thread(ctx, thread)?.is_some())
}
