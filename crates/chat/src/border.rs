//! **A conversation across the border of a workspace** (nxf 6j6v.70dy: 6j6v.q32p, 6j6v.t5xb,
//! 6j6v.4gp2, 6j6v.szc5) — the persona of one workspace commissions a persona of another workspace
//! on the same machine, and the answer comes back.
//!
//! # The path
//!
//! It is the in-house consultation's, with one handover in the middle:
//!
//! 1. The caller runs `nxc send --to <owner>/<repo>/<persona>`. Its coordinator records the
//!    commission in its OWN log ([`commission`]) — a thread under the caller's current one, exactly
//!    as an in-house consultation, expecting a reply from the receiver's participant
//!    (`<receiver prefix>/<persona>`). The caller ends its turn and waits, as it would in-house.
//! 2. [`hand_over`] copies the ops of that ONE thread into the receiver's log. Only the border
//!    thread crosses: its register ops, its messages and its direct conversation. What either side
//!    commissions behind it stays where it was written.
//! 3. The receiver's coordinator — the same verb, run in the receiver's workspace — checks access
//!    ([`Refusal`]) and starts its persona in ITS working copy, with ITS board and memory.
//! 4. The answer, a question or an escalation is copied back by the same verb and wakes the
//!    paused caller.
//!
//! # One mechanism
//!
//! The handover is a verb, `nxc handover`, like `nxc tick`: the background service runs it on its
//! pass for a workspace with an open border thread, and `nxc send` and `nxc reply` on a border
//! thread run it at once — here, and then in the peer's own workspace through [`Peers::hand_over_in`]
//! — so delivery does not need a running service. Each run acts only in its own workspace; copying
//! ops is the one thing it does to the other log, and [`nxs_foundation::store::Store::apply`] is
//! idempotent per op id, which is also why nothing is echoed: an op the other side already holds is
//! not copied again.
//!
//! # Who is believed
//!
//! A copied op keeps its author's signature, and the receiving store verifies it like any op that
//! arrived over sync. Whether an action may follow is the trust list's answer (6j6v.pzkb), in BOTH
//! directions: the receiver must trust the caller's workspace for the commission to start anything,
//! and the caller's workspace must trust the receiver for the answer to wake anybody — which is why
//! [`commission`] refuses up front when it does not. The ROLE is stamped by the caller's
//! coordinator into the signed commission ([`BorderRef::role`]), from the session map — never by
//! the agent, and never for a caller without a session: a person does not come in from outside in
//! this slice.

use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{NxfError, Result};
use crate::model::{Disposition, MessageKind, Priority, Refs};
use crate::orchestration::{self as orch, Ctx};
use crate::store::{ChatStore, MessageRow};

/// **An address from another workspace**: `<owner>/<repo>/<persona>` — the workspace's name
/// (`nxs_foundation::workspace_name`), then the persona's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address<'a> {
    pub workspace: &'a str,
    pub persona: &'a str,
}

impl std::fmt::Display for Address<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.workspace, self.persona)
    }
}

/// Read `to` as an address from another workspace: the last segment is the persona, everything
/// before it a valid workspace name. `None` for anything else — a bare handle, a qualified
/// participant (`ab12/pm`, one slash) and a malformed name all keep meaning what they meant.
pub fn parse_address(to: &str) -> Option<Address<'_>> {
    let (workspace, persona) = to.rsplit_once('/')?;
    if persona.is_empty() || !workspace.contains('/') {
        return None;
    }
    nxs_foundation::workspace_name::is_valid_name(workspace)
        .then_some(Address { workspace, persona })
}

/// **What the caller's coordinator stamps into a border commission** — rides on
/// [`Refs::border`], so it is part of what the caller's key signs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorderRef {
    /// The caller's workspace, by name.
    pub from: String,
    /// The address the commission was sent to, `<owner>/<repo>/<persona>`.
    pub to: String,
    /// The caller's role, bare — read by the caller's coordinator from the session it started.
    /// `None` for a caller without a session; the receiver admits no such caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The caller's depth in its own chain. The receiver starts its persona one deeper, so the cap
    /// holds for the whole chain across both workspaces.
    #[serde(default)]
    pub hop: u32,
    /// On the receiver's answer only: the reason the commission was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
}

/// **Why a border commission was refused** — named, so a caller learns which of its two checks
/// failed, or that the address led nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The caller's workspace is not one this machine knows by that name, or the key that signed
    /// the commission is not that workspace's.
    WorkspaceUnknown,
    /// The addressed workspace is not on this machine.
    NotOnThisMachine,
    /// The caller's workspace is known but not on the receiver's trust list.
    NotTrusted,
    /// The caller's role is not admitted by the receiver's `addressable.external`, or there is no
    /// role at all (a person).
    RoleNotAdmitted,
    /// The receiver declares no persona of that name.
    NoSuchPersona,
    /// Starting the receiver's persona would pass the depth cap of the whole chain.
    DepthLimit,
}

impl Refusal {
    /// The reason in words, as a refusal states it.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::WorkspaceUnknown => "workspace unknown",
            Refusal::NotOnThisMachine => "not on this machine",
            Refusal::NotTrusted => "not trusted",
            Refusal::RoleNotAdmitted => "role not admitted",
            Refusal::NoSuchPersona => "no such persona",
            Refusal::DepthLimit => "depth limit reached",
        }
    }
}

/// **Where a border thread stands** — the four states A2A knows, so that a later slice adds a wire
/// protocol and nothing else (6j6v.szc5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BorderState {
    /// In the caller's own log, not handed over yet.
    Submitted,
    /// Handed over; the receiver's persona is running.
    Working,
    /// A question or an escalation is waiting with the commissioner.
    InputRequired,
    /// The answer is there.
    Completed,
    /// Access was refused.
    Rejected,
    /// Withdrawn, or given up after two hours without a sign of life.
    Canceled,
}

impl BorderState {
    pub fn as_str(self) -> &'static str {
        match self {
            BorderState::Submitted => "submitted",
            BorderState::Working => "working",
            BorderState::InputRequired => "input-required",
            BorderState::Completed => "completed",
            BorderState::Rejected => "rejected",
            BorderState::Canceled => "canceled",
        }
    }

    fn parse(s: &str) -> BorderState {
        match s {
            "working" => BorderState::Working,
            "input-required" => BorderState::InputRequired,
            "completed" => BorderState::Completed,
            "rejected" => BorderState::Rejected,
            "canceled" => BorderState::Canceled,
            _ => BorderState::Submitted,
        }
    }

    /// Whether nothing more happens on the thread: a refusal and a cancellation end it. A completed
    /// thread stays open to the exchange — the caller may write again, and the answer to that is
    /// still carried.
    pub fn is_final(self) -> bool {
        matches!(self, BorderState::Rejected | BorderState::Canceled)
    }
}

/// Which side of the border this workspace is on for one thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// This workspace commissioned.
    Outbound,
    /// This workspace was commissioned.
    Inbound,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Outbound => "out",
            Direction::Inbound => "in",
        }
    }
}

/// **One border thread as this workspace records it** — device-local, like the session map: it
/// names where the other half of the conversation lives on THIS machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BorderRow {
    pub thread_id: String,
    pub direction: Direction,
    /// The other workspace, by name.
    pub peer: String,
    /// Outbound: the persona addressed there. Inbound: the persona here that was addressed.
    pub persona: String,
    pub state: BorderState,
    /// Why it was rejected or canceled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Outbound: the caller's session. Inbound: the persona session started for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub updated: String,
}

/// **Another workspace on this machine** — what [`Peers::find`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerWorkspace {
    pub name: String,
    /// The repository root — the directory that holds `.nxs/`.
    pub root: PathBuf,
}

/// **How this host reaches other workspaces on the machine** — the composition root's capability,
/// like [`crate::machine::Machines`]: the `nxs` binary answers it from the service registry
/// (`~/.nexusflow/workspaces.toml`) and runs the peer's handover as a child process in the peer's
/// root. `None` on a [`Ctx`] — the default for an embedding host that supplies none — reaches no
/// other workspace: an address from outside is refused, and nothing else changes.
pub trait Peers: Send + Sync + std::fmt::Debug {
    /// The name of the workspace whose db is `db_path` — stored, or derived and stored.
    fn here(&self, db_path: &str) -> std::result::Result<Option<String>, String>;
    /// Every workspace on this machine that carries `name`. More than one is a refusal that names
    /// all of them.
    fn find(&self, name: &str) -> std::result::Result<Vec<PeerWorkspace>, String>;
    /// Run the handover in `peer`'s own workspace now, and return when it has run. Best effort: the
    /// service runs it on its pass anyway.
    fn hand_over_in(&self, peer: &PeerWorkspace) -> std::result::Result<(), String>;
}

/// **The file the background service looks for** in a workspace's `.nxs/` (nxf 6j6v.4gp2): present
/// exactly while the workspace has a border thread that is not finished, so the service's pass costs
/// one `stat` per workspace instead of opening its database every second.
pub const OPEN_MARKER: &str = "border-open";

/// Keep [`OPEN_MARKER`] in step with the record. `db_path` is the workspace's database, whose
/// directory is `.nxs/`. Best effort: a marker that could not be written costs the service's pass,
/// never a write.
fn refresh_marker(store: &ChatStore, db_path: &str) {
    let Some(dir) = std::path::Path::new(db_path).parent() else {
        return;
    };
    let marker = dir.join(OPEN_MARKER);
    let open = rows(store)
        .map(|rows| rows.iter().any(|r| !r.state.is_final()))
        .unwrap_or(false);
    let _ = if open {
        std::fs::write(&marker, b"")
    } else {
        std::fs::remove_file(&marker).or(Ok(()))
    };
}

/// Hours without a sign of life after which a working border thread is given up — the paused
/// consultation's deadline (6j6v.nj20, owner 2026-09-17).
pub const DEADLINE_HOURS: i64 = 2;

// ---- the record ------------------------------------------------------------------------------

/// The device-local tables this module keeps. Applied with the chat views.
pub(crate) const BORDER_DDL: &str = "
    CREATE TABLE IF NOT EXISTS border_threads(
        thread_id TEXT PRIMARY KEY,
        direction TEXT NOT NULL,
        peer      TEXT NOT NULL,
        persona   TEXT NOT NULL,
        state     TEXT NOT NULL,
        reason    TEXT,
        session   TEXT,
        created   TEXT NOT NULL,
        updated   TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS border_served(
        thread_id  TEXT NOT NULL,
        message_id TEXT NOT NULL,
        PRIMARY KEY(thread_id, message_id)
    );
";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<BorderRow> {
    let direction: String = r.get(1)?;
    let state: String = r.get(4)?;
    Ok(BorderRow {
        thread_id: r.get(0)?,
        direction: if direction == "in" {
            Direction::Inbound
        } else {
            Direction::Outbound
        },
        peer: r.get(2)?,
        persona: r.get(3)?,
        state: BorderState::parse(&state),
        reason: r.get(5)?,
        session: r.get(6)?,
        updated: r.get(7)?,
    })
}

const ROW_COLUMNS: &str = "thread_id, direction, peer, persona, state, reason, session, updated";

/// This workspace's record of `thread`, if it is a border thread.
pub fn row(store: &ChatStore, thread: &str) -> Result<Option<BorderRow>> {
    Ok(store
        .connection()
        .query_row(
            &format!("SELECT {ROW_COLUMNS} FROM border_threads WHERE thread_id = ?1"),
            [thread],
            row_of,
        )
        .optional()?)
}

/// Every border thread this workspace records, oldest first.
pub fn rows(store: &ChatStore) -> Result<Vec<BorderRow>> {
    let conn = store.connection();
    let mut st = conn.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM border_threads ORDER BY created, thread_id"
    ))?;
    let rows = st
        .query_map([], row_of)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether `thread` is a border thread here.
pub fn is_border(store: &ChatStore, thread: &str) -> Result<bool> {
    Ok(row(store, thread)?.is_some())
}

/// Whether `thread` is a border thread this workspace COMMISSIONED. Such a thread expects a reply
/// from the receiver's participant (`<prefix>/<persona>`), and a reader that reduces a handle to
/// its last segment would take it for a chat with this workspace's own persona of that name.
pub(crate) fn is_outbound(store: &ChatStore, thread: &str) -> bool {
    matches!(row(store, thread), Ok(Some(r)) if r.direction == Direction::Outbound)
}

fn insert_row(
    store: &ChatStore,
    thread: &str,
    direction: Direction,
    peer: &str,
    persona: &str,
    session: Option<&str>,
    now: &str,
) -> Result<()> {
    store.connection().execute(
        "INSERT OR IGNORE INTO border_threads
             (thread_id, direction, peer, persona, state, reason, session, created, updated)
         VALUES (?1, ?2, ?3, ?4, 'submitted', NULL, ?5, ?6, ?6)",
        params![thread, direction.as_str(), peer, persona, session, now],
    )?;
    Ok(())
}

fn set_state(
    store: &ChatStore,
    thread: &str,
    state: BorderState,
    reason: Option<&str>,
    now: &str,
) -> Result<()> {
    store.connection().execute(
        "UPDATE border_threads SET state = ?2, reason = COALESCE(?3, reason), updated = ?4
          WHERE thread_id = ?1",
        params![thread, state.as_str(), reason, now],
    )?;
    Ok(())
}

fn set_session(store: &ChatStore, thread: &str, session: &str) -> Result<()> {
    store.connection().execute(
        "UPDATE border_threads SET session = ?2 WHERE thread_id = ?1",
        params![thread, session],
    )?;
    Ok(())
}

/// Mark `message` as handed to this side's party. `true` when it was not marked before — the one
/// question that makes a handover run twice hand nothing over twice.
fn mark_served(store: &ChatStore, thread: &str, message: &str) -> Result<bool> {
    let n = store.connection().execute(
        "INSERT OR IGNORE INTO border_served(thread_id, message_id) VALUES (?1, ?2)",
        params![thread, message],
    )?;
    Ok(n > 0)
}

fn is_served(store: &ChatStore, thread: &str, message: &str) -> Result<bool> {
    Ok(store
        .connection()
        .query_row(
            "SELECT 1 FROM border_served WHERE thread_id = ?1 AND message_id = ?2",
            params![thread, message],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn border_of(m: &MessageRow) -> Option<BorderRef> {
    let refs: Refs = serde_json::from_str(m.refs.as_deref()?).ok()?;
    refs.border
}

// ---- the other workspace ---------------------------------------------------------------------

/// The other workspace, opened: its log, and who it is.
struct Far {
    /// Its replica prefix — the origin its participants carry.
    prefix: String,
    store: ChatStore,
}

fn open_far(peer: &PeerWorkspace) -> Result<Far> {
    let ws = nxs_foundation::workspace::discover(&peer.root).map_err(|e| {
        NxfError::io(format!(
            "the workspace {} at {} cannot be opened: {}",
            peer.name,
            peer.root.display(),
            e.msg
        ))
    })?;
    let store = ChatStore::open(&ws.db_path().to_string_lossy(), ws.replica.site_id)?;
    Ok(Far {
        prefix: ws.replica.prefix.clone(),
        store,
    })
}

/// The one workspace on this machine named `name`, or the refusal that says why there is none.
fn find_one(peers: &dyn Peers, name: &str) -> Result<std::result::Result<PeerWorkspace, Refusal>> {
    let found = peers.find(name).map_err(NxfError::io)?;
    match found.as_slice() {
        [] => Ok(Err(Refusal::NotOnThisMachine)),
        [one] => Ok(Ok(one.clone())),
        many => Err(NxfError::validation(format!(
            "{} workspaces on this machine are named {name}, so the address is ambiguous: {} — \
             rename one with `nxs name <owner>/<repo>` in its directory",
            many.len(),
            many.iter()
                .map(|p| p.root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// The ids of every op that makes up `thread` in `store`: its register ops, its messages, and the
/// direct conversation it lives in. Nothing else — this is the whole of what crosses the border.
fn thread_op_ids(store: &ChatStore, thread: &str) -> Result<Vec<String>> {
    let channel = store
        .thread_channel(thread)
        .or_else(|| store.thread_channel_via_message(thread))
        .unwrap_or_default();
    let conn = store.connection();
    let mut st = conn.prepare(
        "SELECT op_id FROM ops
          WHERE (target_kind = 'thread' AND target_id = ?1)
             OR (target_kind = 'message'
                 AND target_id IN (SELECT message_id FROM messages WHERE thread_id = ?1))
             OR (target_kind = 'channel' AND target_id = ?2)
             OR (target_kind = 'membership' AND substr(target_id, 1, length(?2) + 1) = ?2 || char(31))
          ORDER BY lamport, site",
    )?;
    let ids = st
        .query_map(params![thread, channel], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

/// Copy the ops of `thread` that `to` does not hold yet from `from`. Returns how many were copied.
fn copy_thread(from: &ChatStore, to: &mut ChatStore, thread: &str) -> Result<usize> {
    let ids = thread_op_ids(from, thread)?;
    let missing: Vec<String> = ids.into_iter().filter(|id| !to.holds_op(id)).collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let ops = from.ops_with_ids(&missing);
    let n = ops.len();
    to.apply(&ops);
    Ok(n)
}

// ---- commissioning ---------------------------------------------------------------------------

/// What [`commission`] did — the border half of a send receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BorderReceipt {
    /// The workspace the commission went to.
    pub peer: String,
    pub state: BorderState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What a border commission recorded: the thread, the message, the participant it expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commissioned {
    pub thread_id: String,
    pub message_id: String,
    pub channel: String,
    pub expects: String,
    pub border: BorderReceipt,
}

/// **Record a commission to a persona of another workspace, and hand it over** (6j6v.4gp2 step 1).
///
/// Everything the receiver will check is decided by the CALLER's coordinator before anything is
/// written, and each refusal says what to do: there is no session (a person does not come in from
/// outside in this slice), the workspace is not on this machine or is ambiguous, or it is not
/// trusted HERE — its answer would arrive unvouched and could wake nobody.
pub fn commission(
    ctx: &Ctx,
    store: &mut ChatStore,
    to: Address<'_>,
    body: &str,
    refs: Refs,
) -> Result<Commissioned> {
    let Some(peers) = ctx.peers else {
        return Err(NxfError::validation(format!(
            "{to} is an address in another workspace, and this host reaches no other workspace"
        )));
    };
    let Some(role) = ctx
        .session
        .and_then(|s| store.session_role(s).ok().flatten())
    else {
        return Err(NxfError::validation(format!(
            "{to} is a persona of another workspace, and only a persona can commission it — a \
             person goes to that workspace and writes there"
        )));
    };
    let here = peers
        .here(ctx.db_path)
        .map_err(NxfError::io)?
        .ok_or_else(|| {
            NxfError::validation(
                "this workspace has no name, so the other side could not tell where the \
                 commission comes from — set it with `nxs name <owner>/<repo>`",
            )
        })?;
    let peer = match find_one(peers, to.workspace)? {
        Ok(p) => p,
        Err(refusal) => {
            return Err(NxfError::not_found(format!(
                "{to}: {} — no workspace named {} is registered with the background service here",
                refusal.text(),
                to.workspace
            )))
        }
    };
    let far = open_far(&peer)?;
    if far.prefix == ctx.origin {
        return Err(NxfError::validation(format!(
            "{} carries the same replica prefix as this workspace ({}), so its participants and \
             ours could not be told apart",
            peer.name, ctx.origin
        )));
    }
    let far_key = far.store.key_id().to_string();
    if !store.is_trusted(&far_key) {
        return Err(NxfError::validation(format!(
            "{to}: {} here — its answer would arrive signed by a key this workspace does not \
             trust, and wake nobody. Trust it first: `nxs sync trust add --workspace {}`",
            Refusal::NotTrusted.text(),
            to.workspace
        )));
    }
    let hop = orch::resolve_hop(ctx, store)?;
    orch::check_depth_guard(hop.get() + 1)?;

    let caller = ctx.caller_handle();
    let target = format!("{}/{}", far.prefix, to.persona);
    let channel = orch::ensure_dm_channel(store, ctx.origin, ctx.actor, &caller, &target)?;
    let parent = orch::ambient_parent(ctx, store);
    let thread = orch::open_child_thread(ctx, store, &channel, parent.as_deref());
    let mut refs = orch::with_return_address(ctx, refs);
    refs.border = Some(BorderRef {
        from: here.clone(),
        to: to.to_string(),
        role: Some(role),
        hop: hop.get(),
        refusal: None,
    });
    let sent = crate::facade::send(
        store,
        crate::facade::SendRequest {
            now: ctx.now,
            origin: ctx.origin,
            actor: ctx.actor,
            channel: &channel,
            body,
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some(&thread),
            refs,
        },
    )?;
    let expects = serde_json::to_string(&[&target]).expect("expects serialize");
    store.set_expects_reply_from(&thread, &expects, &caller);
    insert_row(
        store,
        &thread,
        Direction::Outbound,
        &peer.name,
        to.persona,
        ctx.session,
        ctx.now,
    )?;
    drop(far);
    refresh_marker(store, ctx.db_path);

    after_local_write(ctx, store, &thread);
    let row = row(store, &thread)?.expect("the row was just written");
    Ok(Commissioned {
        thread_id: thread,
        message_id: sent.message_id,
        channel,
        expects: target,
        border: BorderReceipt {
            peer: row.peer,
            state: row.state,
            reason: row.reason,
        },
    })
}

/// **Something was written on a border thread here: carry it across now** — what `nxc send` and
/// `nxc reply` run, so delivery does not wait for the service. Copies this thread over, runs the
/// peer's own handover when something new reached it, and takes in what came back. Never fails
/// the write it follows: what it could not do, the service's pass does.
pub fn after_local_write(ctx: &Ctx, store: &mut ChatStore, thread: &str) {
    let Some(peers) = ctx.peers else { return };
    let Ok(Some(row)) = row(store, thread) else {
        return;
    };
    let peer = match find_one(peers, &row.peer) {
        Ok(Ok(p)) => p,
        Ok(Err(_)) | Err(_) => return,
    };
    let pushed = match open_far(&peer) {
        Ok(mut far) => copy_thread(store, &mut far.store, thread).unwrap_or(0),
        Err(e) => {
            eprintln!(
                "warning: border thread {thread} could not be handed over: {}",
                e.msg
            );
            return;
        }
    };
    if pushed > 0 {
        if let Err(e) = peers.hand_over_in(&peer) {
            eprintln!(
                "warning: the handover in {} did not run ({e}); the background service hands \
                 thread {thread} over on its next pass",
                peer.name
            );
        }
    }
    if let Err(e) = hand_over(ctx, store) {
        eprintln!(
            "warning: taking in what came back on thread {thread} failed: {}",
            e.msg
        );
    }
}

// ---- the handover ----------------------------------------------------------------------------

/// What one handover did — `nxc handover --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HandoverReport {
    /// Border threads exchanged with their peer.
    pub exchanged: Vec<String>,
    /// Inbound commissions admitted: a persona was started for them.
    pub admitted: Vec<String>,
    /// Inbound commissions refused, with the reason.
    pub refused: Vec<(String, Refusal)>,
    /// Sessions handed a message from the other side.
    pub woke: Vec<String>,
    /// What could not be done, per thread.
    pub failed: Vec<(String, String)>,
}

/// **The handover verb** (6j6v.4gp2) — for every border thread of this workspace: copy its ops both
/// ways, then act on what arrived, here and only here. Also finds commissions the other side copied
/// in that this workspace has no record of yet. Idempotent: a second run copies and wakes nothing.
pub fn hand_over(ctx: &Ctx, store: &mut ChatStore) -> Result<HandoverReport> {
    let mut report = HandoverReport::default();
    if let Some(peers) = ctx.peers {
        discover_inbound(ctx, store, peers)?;
        for row in rows(store)? {
            if row.state.is_final() {
                continue;
            }
            match exchange(store, peers, &row) {
                Ok(true) => report.exchanged.push(row.thread_id.clone()),
                Ok(false) => {}
                Err(e) => report.failed.push((row.thread_id.clone(), e.msg)),
            }
        }
    }
    let open: Vec<BorderRow> = rows(store)?
        .into_iter()
        .filter(|r| !r.state.is_final())
        .collect();
    for row in open {
        let acted = match row.direction {
            Direction::Inbound => act_inbound(ctx, store, &row, &mut report),
            Direction::Outbound => act_outbound(ctx, store, &row, &mut report),
        };
        if let Err(e) = acted {
            report.failed.push((row.thread_id.clone(), e.msg));
        }
    }
    refresh_marker(store, ctx.db_path);
    Ok(report)
}

/// Copy one border thread both ways. `true` when anything moved. Runs the peer's handover when
/// something new reached it, so the other side acts on it without waiting for the service.
fn exchange(store: &mut ChatStore, peers: &dyn Peers, row: &BorderRow) -> Result<bool> {
    let peer = match find_one(peers, &row.peer)? {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    let mut far = open_far(&peer)?;
    let pushed = copy_thread(store, &mut far.store, &row.thread_id)?;
    let pulled = copy_thread(&far.store, store, &row.thread_id)?;
    drop(far);
    if pushed > 0 {
        if let Err(e) = peers.hand_over_in(&peer) {
            eprintln!(
                "warning: the handover in {} did not run ({e}); its service picks thread {} up",
                peer.name, row.thread_id
            );
        }
    }
    Ok(pushed + pulled > 0)
}

/// Record every commission another workspace copied in that is addressed to this workspace and has
/// no record here yet.
fn discover_inbound(ctx: &Ctx, store: &mut ChatStore, peers: &dyn Peers) -> Result<()> {
    let Some(here) = peers.here(ctx.db_path).map_err(NxfError::io)? else {
        return Ok(());
    };
    let candidates: Vec<(String, String)> = {
        let conn = store.connection();
        let mut st = conn.prepare(
            "SELECT m.thread_id, m.refs FROM messages m
              WHERE m.thread_id IS NOT NULL
                AND json_extract(m.refs, '$.border.to') IS NOT NULL
                AND json_extract(m.refs, '$.border.refusal') IS NULL
                AND m.thread_id NOT IN (SELECT thread_id FROM border_threads)",
        )?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (thread, refs) in candidates {
        let Ok(refs) = serde_json::from_str::<Refs>(&refs) else {
            continue;
        };
        let Some(b) = refs.border else { continue };
        let Some(to) = parse_address(&b.to) else {
            continue;
        };
        if to.workspace != here {
            continue;
        }
        insert_row(
            store,
            &thread,
            Direction::Inbound,
            &b.from,
            to.persona,
            None,
            ctx.now,
        )?;
    }
    Ok(())
}

/// The commission that opened `thread` — its first message, with the border stamp on it.
fn opening(store: &ChatStore, thread: &str) -> Result<Option<(MessageRow, BorderRef)>> {
    let first = store.messages_in_thread(thread)?.into_iter().next();
    Ok(first.and_then(|m| border_of(&m).map(|b| (m, b))))
}

/// **The receiver's coordinator, on an inbound border thread**: admit or refuse the commission once,
/// then hand the persona whatever the other side writes after it.
fn act_inbound(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &BorderRow,
    report: &mut HandoverReport,
) -> Result<()> {
    let Some((first, stamp)) = opening(store, &row.thread_id)? else {
        return Ok(());
    };
    let participant = ctx.qualify(&row.persona);
    if row.state == BorderState::Submitted {
        match admission(ctx, store, &first, &stamp, &row.persona)? {
            Ok(()) => {
                let session = start_persona(ctx, store, row, &first, &stamp)?;
                set_session(store, &row.thread_id, &session)?;
                set_state(store, &row.thread_id, BorderState::Working, None, ctx.now)?;
                mark_served(store, &row.thread_id, &first.message_id)?;
                report.admitted.push(row.thread_id.clone());
            }
            Err(refusal) => {
                refuse(ctx, store, row, &stamp, refusal)?;
                report.refused.push((row.thread_id.clone(), refusal));
            }
        }
        return Ok(());
    }
    // Admitted earlier: what the caller wrote since goes to the persona's session.
    let Some(session) = row.session.clone() else {
        return Ok(());
    };
    let fresh: Vec<MessageRow> = store
        .acting_messages_in_thread(&row.thread_id)?
        .into_iter()
        .filter(|m| m.sender != participant)
        .filter(|m| !is_served(store, &row.thread_id, &m.message_id).unwrap_or(true))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let hop = orch::checked_hop(stamp.hop.saturating_add(1))?;
    if deliver(
        ctx,
        store,
        &session,
        &row.thread_id,
        &fresh,
        &participant,
        hop,
    ) {
        report.woke.push(session);
    }
    set_state(store, &row.thread_id, BorderState::Working, None, ctx.now)?;
    Ok(())
}

/// **The two checks, both required** (6j6v.t5xb), plus whether the address leads anywhere: the
/// caller's workspace is known here under the key that signed, that key is trusted, the persona
/// exists, its role is admitted by `addressable.external`, and the chain has room for one more.
fn admission(
    ctx: &Ctx,
    store: &ChatStore,
    first: &MessageRow,
    stamp: &BorderRef,
    persona: &str,
) -> Result<std::result::Result<(), Refusal>> {
    let signer = store
        .message_provenance(&first.message_id)?
        .and_then(|p| p.key_id);
    if let Some(peers) = ctx.peers {
        match find_one(peers, &stamp.from) {
            Ok(Ok(peer)) => {
                let known_key = open_far(&peer).ok().map(|f| f.store.key_id().to_string());
                if known_key.is_none() || known_key != signer {
                    return Ok(Err(Refusal::WorkspaceUnknown));
                }
            }
            Ok(Err(_)) | Err(_) => return Ok(Err(Refusal::WorkspaceUnknown)),
        }
    }
    if !first.acts {
        return Ok(Err(Refusal::NotTrusted));
    }
    let Ok(decl) = ctx.defs.role(persona) else {
        return Ok(Err(Refusal::NoSuchPersona));
    };
    let Some(role) = stamp.role.as_deref() else {
        return Ok(Err(Refusal::RoleNotAdmitted));
    };
    if !decl.addressable.admits_external(&stamp.from, role) {
        return Ok(Err(Refusal::RoleNotAdmitted));
    }
    if orch::check_depth_guard(stamp.hop.saturating_add(1)).is_err() {
        return Ok(Err(Refusal::DepthLimit));
    }
    Ok(Ok(()))
}

/// Start the receiver's persona on the border thread: a fresh session, in this workspace's working
/// copy, owing the thread its reply, one hop deeper than the caller.
fn start_persona(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &BorderRow,
    first: &MessageRow,
    stamp: &BorderRef,
) -> Result<String> {
    let decl = ctx.defs.role(&row.persona)?.clone();
    let participant = ctx.qualify(&row.persona);
    let caller = stamp
        .role
        .as_deref()
        .map(|r| format!("{}/{r}", stamp.from))
        .unwrap_or_else(|| stamp.from.clone());
    let body = format!(
        "From {caller}, a persona of another workspace on this machine. Only this thread is \
         shared with it; what you commission to answer it stays in this workspace.\n\n{}",
        first.body
    );
    let guidance = orch::reply_guidance(store, ctx.now, &participant, Some(&row.thread_id));
    let wake = orch::wake_message(&body, false, &guidance);
    let internal = store.mint_session_id();
    store.create_pending_session(&internal, &row.persona)?;
    let hop = orch::checked_hop(stamp.hop)?;
    orch::trigger_role(
        ctx,
        store,
        orch::RoleSpawn {
            decl: &decl,
            internal_session: internal.clone(),
            resume_real: None,
            message: &wake,
            coordinator: crate::worker::Coordinator::Border,
            model_override: None,
            hop,
            chain: orch::ChainMove::Deeper,
            reply_thread: Some(&row.thread_id),
            thread: Some(&row.thread_id),
            priority: Priority::Normal,
            queued_since: None,
        },
    )
    .map_err(|e| NxfError::io(format!("the persona {} did not start: {e}", row.persona)))?;
    Ok(internal)
}

/// Answer a refused commission on its own thread, in the addressed persona's name and marked as
/// the coordinator's refusal, so the caller is woken with the reason — and start nothing.
fn refuse(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &BorderRow,
    stamp: &BorderRef,
    refusal: Refusal,
) -> Result<()> {
    let body = format!(
        "Refused by {}: {}. {}",
        stamp.to,
        refusal.text(),
        match refusal {
            Refusal::WorkspaceUnknown => format!(
                "No workspace on this machine is named {} under the key that signed the commission.",
                stamp.from
            ),
            Refusal::NotOnThisMachine => String::new(),
            Refusal::NotTrusted => format!(
                "{} is not on this workspace's trust list; its owner adds it with `nxs sync trust \
                 add --workspace {}`.",
                stamp.from, stamp.from
            ),
            Refusal::RoleNotAdmitted => format!(
                "`{}` of {} is not listed in the `addressable.external` of `{}`.",
                stamp.role.as_deref().unwrap_or("a person"),
                stamp.from,
                row.persona
            ),
            Refusal::NoSuchPersona => format!("This workspace declares no persona `{}`.", row.persona),
            Refusal::DepthLimit => "The chain of commissions is as deep as it may go.".to_string(),
        }
    );
    let refs = Refs {
        border: Some(BorderRef {
            from: stamp
                .to
                .rsplit_once('/')
                .map(|(w, _)| w.to_string())
                .unwrap_or_default(),
            to: format!("{}/{}", stamp.from, stamp.role.as_deref().unwrap_or("")),
            role: None,
            hop: stamp.hop,
            refusal: Some(refusal),
        }),
        ..Refs::default()
    };
    crate::facade::reply(
        store,
        crate::facade::ReplyRequest {
            now: ctx.now,
            origin: ctx.origin,
            actor: &row.persona,
            target: &row.thread_id,
            body: &body,
            kind: MessageKind::Escalation,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            refs,
            if_unanswered: false,
        },
    )?;
    set_state(
        store,
        &row.thread_id,
        BorderState::Rejected,
        Some(refusal.text()),
        ctx.now,
    )?;
    after_local_write_quiet(ctx, store, &row.thread_id);
    Ok(())
}

/// [`after_local_write`]'s copy-and-kick half alone — for a write the handover itself made, where
/// running the whole handover again from inside it would only recurse.
fn after_local_write_quiet(ctx: &Ctx, store: &mut ChatStore, thread: &str) {
    let Some(peers) = ctx.peers else { return };
    let Ok(Some(row)) = row(store, thread) else {
        return;
    };
    let Ok(Ok(peer)) = find_one(peers, &row.peer) else {
        return;
    };
    let pushed = open_far(&peer)
        .and_then(|mut far| copy_thread(store, &mut far.store, thread))
        .unwrap_or(0);
    if pushed > 0 {
        let _ = peers.hand_over_in(&peer);
    }
}

/// **The caller's coordinator, on an outbound border thread**: hand the paused caller whatever the
/// receiver wrote, and follow the thread's state.
fn act_outbound(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &BorderRow,
    report: &mut HandoverReport,
) -> Result<()> {
    let caller = ctx.qualify(
        &row.session
            .as_deref()
            .and_then(|s| store.session_role(s).ok().flatten())
            .unwrap_or_default(),
    );
    let msgs = store.acting_messages_in_thread(&row.thread_id)?;
    let fresh: Vec<MessageRow> = msgs
        .iter()
        .filter(|m| m.sender != caller && border_of(m).is_none_or(|b| b.refusal.is_some()))
        .filter(|m| !is_served(store, &row.thread_id, &m.message_id).unwrap_or(true))
        .cloned()
        .collect();
    if row.state == BorderState::Submitted && fresh.is_empty() {
        // Handed over, nothing back yet: the receiver is working on it once it holds the thread.
        if store
            .messages_in_thread(&row.thread_id)?
            .iter()
            .any(|m| m.sender != caller)
            || peer_holds(ctx, row)
        {
            set_state(store, &row.thread_id, BorderState::Working, None, ctx.now)?;
        }
        return Ok(());
    }
    let Some(last) = fresh.last() else {
        return Ok(());
    };
    let refusal = fresh
        .iter()
        .find_map(|m| border_of(m).and_then(|b| b.refusal));
    let state = match refusal {
        Some(_) => BorderState::Rejected,
        None if last.kind == crate::model::KIND_ESCALATION => BorderState::InputRequired,
        None => BorderState::Completed,
    };
    set_state(
        store,
        &row.thread_id,
        state,
        refusal.map(Refusal::text),
        ctx.now,
    )?;
    if let Some(session) = row.session.clone() {
        let hop = orch::resolve_hop(ctx, store)?;
        let sender = last.sender.clone();
        if deliver(ctx, store, &session, &row.thread_id, &fresh, &sender, hop) {
            report.woke.push(session);
        }
    } else {
        for m in &fresh {
            mark_served(store, &row.thread_id, &m.message_id)?;
        }
    }
    Ok(())
}

/// Whether the other side already records this thread — the receiver holds it, so it is working.
fn peer_holds(ctx: &Ctx, row: &BorderRow) -> bool {
    let Some(peers) = ctx.peers else { return false };
    let Ok(Ok(peer)) = find_one(peers, &row.peer) else {
        return false;
    };
    open_far(&peer)
        .ok()
        .and_then(|f| self::row(&f.store, &row.thread_id).ok().flatten())
        .is_some()
}

/// Wake `session` with `messages` — resumed, or held while it is mid-turn — and mark them served.
/// `true` when the session was woken now.
fn deliver(
    ctx: &Ctx,
    store: &mut ChatStore,
    session: &str,
    thread: &str,
    messages: &[MessageRow],
    from: &str,
    hop: orch::CheckedHop,
) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    let escalated = messages
        .iter()
        .any(|m| m.kind == crate::model::KIND_ESCALATION);
    let body = messages
        .iter()
        .map(|m| m.body.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let handle = store
        .session_role(session)
        .ok()
        .flatten()
        .map(|r| ctx.qualify(&r))
        .unwrap_or_default();
    let guidance = orch::reply_guidance(store, ctx.now, &handle, Some(thread));
    let wake = orch::wake_message(&body, escalated, &guidance);
    let hold = crate::collecting::Held {
        thread_id: thread.to_string(),
        message_id: last.message_id.clone(),
        sender: from.to_string(),
        body: body.clone(),
        escalated,
    };
    let attempt = orch::resume_return_address(
        ctx,
        store,
        orch::Resume {
            addr: session,
            body: &wake,
            model: None,
            hop,
            thread: Some(thread),
            on_busy: orch::OnBusy::Hold(&hold),
        },
    );
    if attempt.woke.is_some() || attempt.held.is_some() {
        for m in messages {
            let _ = mark_served(store, thread, &m.message_id);
        }
    }
    attempt.woke.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_a_workspace_name_and_a_persona() {
        assert_eq!(
            parse_address("nxsflow/nexus-flow/pm"),
            Some(Address {
                workspace: "nxsflow/nexus-flow",
                persona: "pm"
            })
        );
        assert_eq!(
            parse_address("group/sub/repo/pm").map(|a| a.workspace),
            Some("group/sub/repo")
        );
    }

    #[test]
    fn a_handle_and_a_qualified_participant_are_not_addresses() {
        for to in [
            "pm",
            "ab12/pm",
            "nxsflow/nexus-flow/",
            "NxsFlow/nexus-flow/pm",
            "/x/pm",
        ] {
            assert_eq!(parse_address(to), None, "{to}");
        }
    }

    #[test]
    fn the_states_render_as_a2a_names_them() {
        let names: Vec<&str> = [
            BorderState::Submitted,
            BorderState::Working,
            BorderState::InputRequired,
            BorderState::Completed,
            BorderState::Rejected,
            BorderState::Canceled,
        ]
        .iter()
        .map(|s| s.as_str())
        .collect();
        assert_eq!(
            names,
            [
                "submitted",
                "working",
                "input-required",
                "completed",
                "rejected",
                "canceled"
            ]
        );
    }
}
