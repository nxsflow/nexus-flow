//! The bridge entry points as library functions (§6.4 item 1, §10): what the CLI calls, over the bridge's own
//! store connection with its 5 s timeout. The wiring into `surface` is ticket b7ck's.
//!
//! The rule of all of them is IN1: check before writing, and refuse with the reason and the alternative. A
//! lookup that fails refuses the verb and never falls through to the old tables; only a MISSING file means
//! "not v2", which is what [`open_store`] answers.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nexus_run_contracts::status::{Owner, TreeStatus};
use nexus_run_contracts::{
    Cas, Decision, Disposition, Event, EventBody, Handle, LivenessPolicy, LocalStatus, MessageId,
    Origin, Refusal, RunId, RunStatus, RunStore, SessionKey, StoreError, Timestamp, WithdrawnBy,
    LOCAL_TENANT,
};
use nexus_run_engine::{compile, status as project, Asked, ChannelDecl, RunState, SlotState};
use nexus_run_store_sqlite::{SqliteRunStore, LEASE_EXPIRY};
use ulid::Ulid;

use crate::effects::{PendingCommission, CHAT_REFUSED};
use crate::fold::{fold_events, pinned_def, Folded};
use crate::git_tree::tree_state;
use crate::host::Layout;
use crate::ports::{DeclarationPort, MessagePort, PortError, Refs};
use crate::session_stub::{ProcessGuard, SystemProcesses};
use crate::start::{mint_thread_id, ThreadRoute};
use crate::tree_effects::StoredRefusal;
use crate::tree_ledger::TreeLedger;

/// The bridge's `busy_timeout`, the old store's value, so a reply posted into the chat store is not followed
/// by a visible enqueue failure over a short stall (§7).
pub const BRIDGE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// The host's, below its 200 ms budget, so a blocked call cannot outlast the tick (§7).
pub const HOST_BUSY_TIMEOUT: Duration = Duration::from_millis(100);

/// The bridge's connection. `Ok(None)` is the one way to say "not v2": no `run.sqlite` here.
pub fn open_store(path: &Path) -> Result<Option<SqliteRunStore>, StoreError> {
    if !path.exists() {
        return Ok(None);
    }
    SqliteRunStore::open(path, BRIDGE_BUSY_TIMEOUT).map(Some)
}

/// What every entry point needs.
#[derive(Clone)]
pub struct BridgeCtx {
    pub store: Arc<SqliteRunStore>,
    pub messages: Arc<dyn MessagePort>,
    pub declarations: Arc<dyn DeclarationPort>,
    pub layout: Layout,
    pub workspace_id: String,
}

/// Who is calling: a session's `NXC_ACTOR` and `NXC_SESSION`, or a person with no session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub actor: Handle,
    pub session: Option<SessionKey>,
}

/// Why a verb was refused, before anything was written unless it says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not one of this store's threads, or not the kind of thread the verb takes.
    NotARunThread {
        thread: String,
    },
    /// The caller is not the slot that owes the reply: this one is.
    NotTheSlot {
        owed_by: Handle,
    },
    /// The slot is past the point of replying.
    NotWaitingForReply,
    NotTheRequester,
    /// `start a new commission`: IN3.
    RunFinished,
    /// "The run is working": there is no question to answer, and a hold or a park is resumed, not answered.
    NoOpenQuestion,
    /// `--accept` on an escalation (§6.4 item 6).
    AcceptAtACeilingOnly,
    /// The message is posted and the run will not hear of it: the one visible retry (§6.4 item 5).
    PostedNotEnqueued {
        message: MessageId,
    },
    Compile(String),
    Store(StoreError),
    Port(PortError),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARunThread { thread } => write!(f, "{thread} is no thread of a run here"),
            Self::NotTheSlot { owed_by } => write!(f, "this reply is owed by {owed_by}"),
            Self::NotWaitingForReply => f.write_str("this thread is not waiting for a reply"),
            Self::NotTheRequester => f.write_str("only the requester of the run can do this"),
            Self::RunFinished => {
                f.write_str("the run has finished; start a new commission with `nxc send`")
            }
            Self::NoOpenQuestion => f.write_str(
                "the run asks no question: see `nxc status`, and `nxc resume` or `nxc withdraw`",
            ),
            Self::AcceptAtACeilingOnly => {
                f.write_str("--accept is valid at a ceiling only; answer an escalation plainly")
            }
            Self::PostedNotEnqueued { message } => write!(
                f,
                "message {message} is posted but the run has not heard of it: run the command again"
            ),
            Self::Compile(what) => write!(f, "the channel does not compile: {what}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Port(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Refused {}

impl From<StoreError> for Refused {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<PortError> for Refused {
    fn from(error: PortError) -> Self {
        Self::Port(error)
    }
}

/// What `nxc send` prints: the thread id at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommissionReceipt {
    pub run: RunId,
    pub root_thread: MessageId,
    /// `timeout:` declarations slice 1 ignores: reported, never dropped silently (§6.1).
    pub ignored_timeouts: Vec<String>,
}

/// One transaction opens a run, one store holds it, and nothing is posted before the log can answer for it
/// (§6.4 item 1). The commission text waits in `pending/<run>`, in `run.sqlite` but not in the log.
pub fn commission(
    ctx: &BridgeCtx,
    channel: &str,
    body: &str,
    refs: &Refs,
    sender: &str,
    policy: LivenessPolicy,
    now: Timestamp,
) -> Result<CommissionReceipt, Refused> {
    let flow = ctx.declarations.channel(channel)?;
    let review = review_of(ctx, &flow);
    let def =
        compile(&flow, &review).map_err(|refusal| Refused::Compile(format!("{refusal:?}")))?;
    let run = RunId::new(Ulid::new());
    let root_thread = mint_thread_id();
    let hash = def.flow_ref();
    let pending = PendingCommission {
        root_thread: root_thread.clone(),
        channel: channel.to_owned(),
        sender: sender.to_owned(),
        body: body.to_owned(),
        refs: refs.clone(),
    };
    let route = ThreadRoute {
        run: run.clone(),
        session_key: None,
    };
    let event = Event::new(
        run.clone(),
        0,
        now,
        format!("commissioned/{run}"),
        Origin::Cli,
        EventBody::Commissioned {
            run: run.clone(),
            workspace_id: ctx.workspace_id.clone(),
            tenant: LOCAL_TENANT.to_owned(),
            flow: hash.clone(),
            policy,
            root_thread: root_thread.clone(),
            requester: sender.to_owned(),
        },
    );
    let flow_key = format!("flow/{}", hash.hash);
    let cas = vec![
        Cas {
            key: format!("pending/{run}"),
            from: None,
            to: Some(encode(&pending)?),
        },
        Cas {
            key: format!("thread/{root_thread}"),
            from: None,
            to: Some(encode(&route)?),
        },
    ];
    // Content-addressed: skipped when the key holds it, and a racing send's `CasFailed` is retried once.
    let mut retried = false;
    loop {
        let mut this_try = cas.clone();
        if ctx.store.get(&flow_key)?.is_none() {
            this_try.push(Cas {
                key: flow_key.clone(),
                from: None,
                to: Some(def.canonical_json()),
            });
        }
        match ctx.store.enqueue(&run, &event, &this_try) {
            Ok(_) => break,
            Err(StoreError::CasFailed { key }) if key == flow_key && !retried => {
                retried = true;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(CommissionReceipt {
        run,
        root_thread,
        ignored_timeouts: def
            .ignored_timeouts
            .iter()
            .map(|timeout| format!("{}: {}", timeout.scope, timeout.value))
            .collect(),
    })
}

/// The channel a step fans out to: the first step target the declarations know as a channel.
fn review_of(ctx: &BridgeCtx, flow: &ChannelDecl) -> ChannelDecl {
    flow.steps
        .iter()
        .find_map(|step| ctx.declarations.channel(&step.target).ok())
        .unwrap_or_default()
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, Refused> {
    serde_json::to_vec(value).map_err(|error| {
        Refused::Store(StoreError::Backend {
            what: error.to_string(),
        })
    })
}

/// A run as far as it folds.
pub struct View {
    pub state: RunState,
    pub read_only: Option<&'static str>,
}

/// The fold of a run for reading. A log it cannot fold whole is projected read-only, with what did fold (§13).
pub fn view(store: &SqliteRunStore, run: &RunId) -> Result<Option<View>, Refused> {
    let (events, damaged) = match store.load(run, 0) {
        Ok(events) => (events, false),
        Err(StoreError::DamagedRow { seq: Some(_), .. }) => (store.load_readable(run)?, true),
        Err(error) => return Err(error.into()),
    };
    let first = match events.first() {
        Some(first) => Some(first.clone()),
        None => store.next_input(run).ok().flatten(),
    };
    let Some(first) = first else {
        return Ok(None);
    };
    let Some(def) = pinned_def(store, &first) else {
        return Ok(None);
    };
    let unfoldable = "a fold that does not panic";
    Ok(match fold_events(run, def.clone(), &events) {
        Folded::Whole(state) => Some(View {
            state: with_commission(state, &first, &events),
            read_only: damaged.then_some(unfoldable),
        }),
        Folded::NewerNxs(state) => state.map(|state| View {
            state,
            read_only: Some("a newer nxs"),
        }),
        Folded::Unfoldable(state) => state.map(|state| View {
            state,
            read_only: Some(unfoldable),
        }),
    })
}

/// A run whose `commissioned` still waits in the inbox reads as commissioned to the bridge: it can already
/// be withdrawn, and its requester is known.
fn with_commission(state: RunState, first: &Event, events: &[Event]) -> RunState {
    if events.is_empty() {
        if let Ok(folded) = nexus_run_engine::evolve(state.clone(), first) {
            let mut folded = folded;
            folded.seq = 0;
            return folded;
        }
    }
    state
}

fn route_of(ctx: &BridgeCtx, thread: &str) -> Result<ThreadRoute, Refused> {
    let not_ours = || Refused::NotARunThread {
        thread: thread.to_owned(),
    };
    let bytes = ctx
        .store
        .get(&format!("thread/{thread}"))?
        .ok_or_else(not_ours)?;
    serde_json::from_slice(&bytes).map_err(|_| {
        Refused::Store(StoreError::DamagedRow {
            key: format!("thread/{thread}"),
            seq: None,
        })
    })
}

fn state_of(ctx: &BridgeCtx, run: &RunId) -> Result<RunState, Refused> {
    let view = view(&ctx.store, run)?.ok_or(Refused::RunFinished)?;
    Ok(view.state)
}

fn input_key(run: &RunId) -> String {
    format!("input/{run}/{}", Ulid::new())
}

/// Posts, then enqueues. A crash between them leaves a message the run never folds, and the caller is told
/// with the message id and the command to run again: nothing retries silently.
#[allow(clippy::too_many_arguments)]
fn post_and_enqueue(
    ctx: &BridgeCtx,
    run: &RunId,
    thread: &str,
    sender: &str,
    text: &str,
    key: &str,
    now: Timestamp,
    body: impl FnOnce(&MessageId) -> EventBody,
) -> Result<MessageId, Refused> {
    let message = ctx
        .messages
        .post(thread, sender, text, &Refs::default(), key)?;
    let event = Event::new(run.clone(), 0, now, key, Origin::Cli, body(&message));
    match ctx.store.enqueue(run, &event, &[]) {
        Ok(_) => Ok(message),
        Err(_) => Err(Refused::PostedNotEnqueued { message }),
    }
}

/// How a session ends its turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplyFlags {
    pub escalate: bool,
    pub needs_rework: bool,
}

/// A step's party answers in its thread: accepted while its slot is waiting for a reply, from that slot's own
/// session only (§6.4 item 5). Any other caller is refused naming the slot that owes it.
pub fn reply(
    ctx: &BridgeCtx,
    thread: &str,
    caller: &Caller,
    text: &str,
    flags: ReplyFlags,
    now: Timestamp,
) -> Result<MessageId, Refused> {
    let route = route_of(ctx, thread)?;
    let Some(session_key) = route.session_key else {
        return Err(Refused::NotARunThread {
            thread: thread.to_owned(),
        });
    };
    let state = state_of(ctx, &route.run)?;
    if state.is_finished() {
        return Err(Refused::RunFinished);
    }
    let index = state
        .find(&session_key)
        .ok_or(Refused::NotWaitingForReply)?;
    let record = &state.slots[index];
    if !(caller.actor == record.slot.member && caller.session.as_deref() == Some(&*session_key)) {
        return Err(Refused::NotTheSlot {
            owed_by: record.slot.member.clone(),
        });
    }
    if !matches!(
        record.state,
        SlotState::Starting { .. }
            | SlotState::Running { .. }
            | SlotState::Quiet { .. }
            | SlotState::Nudged { .. }
    ) {
        return Err(Refused::NotWaitingForReply);
    }
    let key = input_key(&route.run);
    post_and_enqueue(
        ctx,
        &route.run,
        thread,
        &caller.actor,
        text,
        &key,
        now,
        |message| EventBody::Reply {
            session_key: session_key.clone(),
            message: message.clone(),
            escalate: flags.escalate,
            needs_rework: flags.needs_rework,
        },
    )
}

/// The caller is the person who commissioned the run, at the terminal: no session.
fn check_requester(state: &RunState, caller: &Caller) -> Result<(), Refused> {
    if caller.session.is_none() && caller.actor == state.requester {
        Ok(())
    } else {
        Err(Refused::NotTheRequester)
    }
}

fn root_route(ctx: &BridgeCtx, thread: &str) -> Result<RunId, Refused> {
    let route = route_of(ctx, thread)?;
    if route.session_key.is_some() {
        return Err(Refused::NotARunThread {
            thread: thread.to_owned(),
        });
    }
    Ok(route.run)
}

/// The commissioner answers on the root: the oldest question no pending answer already addresses, so two
/// answers before a fold address two questions (§6.4 item 8). `accept` is for a ceiling only.
pub fn answer(
    ctx: &BridgeCtx,
    root: &str,
    caller: &Caller,
    text: &str,
    accept: bool,
    now: Timestamp,
) -> Result<MessageId, Refused> {
    let run = root_route(ctx, root)?;
    let state = state_of(ctx, &run)?;
    if state.is_finished() {
        return Err(Refused::RunFinished);
    }
    check_requester(&state, caller)?;
    let addressed: Vec<String> = ctx
        .store
        .waiting_inputs(&run)?
        .into_iter()
        .filter_map(|input| match input {
            Ok(Event {
                body: EventBody::Answer { question, .. },
                ..
            }) => Some(question),
            _ => None,
        })
        .collect();
    let question = state
        .open_questions()
        .find(|question| !addressed.contains(&question.id))
        .ok_or(Refused::NoOpenQuestion)?;
    if accept && matches!(question.asked, Asked::Escalation { .. }) {
        return Err(Refused::AcceptAtACeilingOnly);
    }
    let id = question.id.clone();
    let disposition = if accept {
        Disposition::Accept
    } else {
        Disposition::Continue
    };
    let key = input_key(&run);
    post_and_enqueue(ctx, &run, root, &caller.actor, text, &key, now, |message| {
        EventBody::Answer {
            question: id,
            disposition,
            message: message.clone(),
        }
    })
}

/// The human exit, valid in every non-final state (§6.4 item 10). A run with no root thread yet has nowhere to
/// post, and its discharge is the caller's to print.
pub fn withdraw(
    ctx: &BridgeCtx,
    root: &str,
    caller: &Caller,
    text: &str,
    now: Timestamp,
) -> Result<Option<MessageId>, Refused> {
    let run = root_route(ctx, root)?;
    let state = state_of(ctx, &run)?;
    if state.is_finished() {
        return Err(Refused::RunFinished);
    }
    check_requester(&state, caller)?;
    let key = input_key(&run);
    if state.root_message.is_some() {
        return post_and_enqueue(ctx, &run, root, &caller.actor, text, &key, now, |message| {
            EventBody::Withdrawn {
                message: Some(message.clone()),
                by: WithdrawnBy::Requester,
            }
        })
        .map(Some);
    }
    let event = Event::new(
        run.clone(),
        0,
        now,
        key,
        Origin::Cli,
        EventBody::Withdrawn {
            message: None,
            by: WithdrawnBy::Requester,
        },
    );
    ctx.store.enqueue(&run, &event, &[])?;
    Ok(None)
}

/// Contract (d): the pure projection, with what only this machine can say around it. A reader without the
/// host sees `local` absent as a whole.
pub fn status(ctx: &BridgeCtx, run: &RunId, now: Timestamp) -> Result<Option<RunStatus>, Refused> {
    let Some(view) = view(&ctx.store, run)? else {
        return Ok(None);
    };
    let mut projected = project(&view.state);
    if let Some(why) = view.read_only {
        projected.waiting_for = Some(why.to_owned());
    } else if ctx
        .store
        .get(&format!("effect/{run}/{CHAT_REFUSED}"))?
        .is_some()
    {
        projected.waiting_for = Some("the chat store".to_owned());
    }
    if let Some(row) = poisoned_row(&ctx.store, run, &view.state)? {
        projected.next_action = Some(format!(
            "the input {row} could not be folded and was set aside; {}",
            projected.next_action.unwrap_or_default()
        ));
    }
    projected.local = Some(local(ctx, run, &view.state, now)?);
    Ok(Some(projected))
}

/// The row a `parked { poison_input }` set aside, named by the event's own key (§13).
fn poisoned_row(
    store: &SqliteRunStore,
    run: &RunId,
    state: &RunState,
) -> Result<Option<String>, Refused> {
    let parked_on_poison = state
        .condition
        .as_ref()
        .is_some_and(|c| c.reason == nexus_run_contracts::ReasonCode::PoisonInput);
    if !parked_on_poison {
        return Ok(None);
    }
    let prefix = format!("poison/{run}/");
    let events = store.load_readable(run)?;
    Ok(events.iter().rev().find_map(|event| {
        let parked = matches!(event.body, EventBody::Decision(Decision::Parked { .. }));
        event
            .key
            .strip_prefix(&prefix)
            .filter(|_| parked)
            .map(str::to_owned)
    }))
}

fn local(
    ctx: &BridgeCtx,
    run: &RunId,
    state: &RunState,
    now: Timestamp,
) -> Result<LocalStatus, Refused> {
    let last_seen = ctx.store.host_seen(&ctx.workspace_id)?;
    let window = 2 * LEASE_EXPIRY.0 as i64 * 1000;
    let attended = last_seen.is_some_and(|at| now.unix_millis() - at.unix_millis() <= window);
    let dir = ctx.layout.v2_dir();
    let live = tree_state(&ctx.layout.root, &dir)
        .ok()
        .filter(|tree| !tree.missing);
    let refusal: Option<StoredRefusal> = ctx
        .store
        .get(&format!("tree_refusal/{run}"))?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let ledger = TreeLedger::new(&ctx.layout.agent_logs, &ctx.workspace_id);
    let guard: &dyn ProcessGuard = &SystemProcesses;
    Ok(LocalStatus {
        observed_at: now,
        owner: Owner {
            attended,
            last_seen,
        },
        pending_inputs: u32::try_from(ctx.store.waiting_inputs(run)?.len()).unwrap_or(u32::MAX),
        tree: TreeStatus {
            path: dir.to_string_lossy().into_owned(),
            base: state.base.clone(),
            found: state.found.clone(),
            commit: live.as_ref().and_then(|tree| tree.commit.clone()),
            branch: live.as_ref().and_then(|tree| tree.branch.clone()),
            dirty: live.as_ref().map(|tree| tree.dirty),
            in_flight: ledger.in_flight(guard),
            parked: state.parked.clone(),
            refusal: refusal.map(|stored| Refusal {
                reason: stored.reason,
                text: stored.text,
                at: stored.at,
            }),
        },
    })
}

/// One session of an open run that may have a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    pub run: RunId,
    pub session_key: SessionKey,
    pub handle: Handle,
}

/// What `a_run_is_alive` asks: a replied slot still winding down counts (§10).
pub fn live_sessions(ctx: &BridgeCtx) -> Result<Vec<LiveSession>, Refused> {
    let mut live = Vec::new();
    for row in ctx.store.open_runs()? {
        let Some(view) = view(&ctx.store, &row.run)? else {
            continue;
        };
        for record in &view.state.slots {
            if record.state.is_live() {
                live.push(LiveSession {
                    run: row.run.clone(),
                    session_key: view.state.session_key(&record.slot),
                    handle: record.slot.member.clone(),
                });
            }
        }
    }
    Ok(live)
}

/// The run and the thread of a session key, or `None` when this store does not know it: how every caller
/// check on both engines tells a v2 session (§10).
pub fn session_of(ctx: &BridgeCtx, session_key: &str) -> Result<Option<ThreadRoute>, Refused> {
    let Some(thread) = ctx.store.get(&format!("session/{session_key}"))? else {
        return Ok(None);
    };
    let thread = String::from_utf8_lossy(&thread).into_owned();
    route_of(ctx, &thread).map(Some)
}
