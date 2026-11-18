//! **A run of another workspace** (6j6v.4b43): a run whose `workspace_id` is not the one this host attends — a
//! send from before the workspace had its origin, left in `run.sqlite` under `local`, or one from a workspace
//! renamed since. The claim key is per workspace id, so such a run would hold `claim/<its id>` beside this
//! workspace's own claim, on the one `<root>-v2` both resolve to. So it is never claimed here, a claim it took
//! before this check prepares no tree and starts no session, and a park never touches a directory it does not
//! hold. The service log says so once per run, and status names the workspace it waits for and the exit.
//!
//! The exit: its requester is an identity of a workspace that is not this one, and nobody at this terminal can
//! act as it. So the owner at this workspace's terminal withdraws it, recorded as the owner's
//! (`withdrawn_by_owner`, §6.4 item 10). Only such a run: on a run of this workspace the requester's exit stays
//! the requester's.

use nexus_run_contracts::{
    Event, EventBody, MessageId, Origin, RunId, RunStatus, RunStore, Timestamp, WithdrawnBy,
};
use nexus_run_engine::RunState;

use crate::bridge::{
    input_key, post_and_enqueue, root_route, state_of, BridgeCtx, Caller, Refused,
};
use crate::host::Host;

/// The once-per-run key of the log line.
const LOGGED: &str = "foreign workspace";

impl Host {
    /// Whether a run under `workspace` is another workspace's.
    pub(crate) fn foreign(&self, workspace: &str) -> bool {
        workspace != self.workspace_id
    }

    /// Says once, in the service log, that this run is not this workspace's and is left alone here.
    pub(crate) fn note_foreign(&mut self, run: &RunId, workspace: &str) {
        if self.cache(run).logged.insert(LOGGED.to_owned()) {
            self.log(
                run,
                "is not claimed here: it was commissioned under workspace",
                workspace,
            );
        }
    }
}

/// Status of a run of another workspace: what it waits for is a service of that workspace, and the exit is the
/// owner's withdraw. Ids only, like every status text.
pub(crate) fn annotate(ctx: &BridgeCtx, state: &RunState, status: &mut RunStatus) {
    if state.workspace == ctx.workspace_id || state.is_finished() {
        return;
    }
    status.waiting_for = Some(format!("a service of workspace {}", state.workspace));
    let root = state.root_thread.as_deref().unwrap_or("<its root thread>");
    status.next_action = Some(format!(
        "this run was commissioned under workspace {}, and this workspace is {}: no service here starts \
         it. Withdraw it with `nxc withdraw --thread {root}`",
        state.workspace, ctx.workspace_id
    ));
}

/// **The owner's withdraw of a run of another workspace.** The caller is a person at this workspace's terminal
/// (no session); the run is one this workspace never claims. Anything else is the requester's exit, refused here
/// as `NotTheRequester` before anything is written.
pub fn withdraw_as_owner(
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
    if caller.session.is_some() || state.workspace == ctx.workspace_id {
        return Err(Refused::NotTheRequester);
    }
    record_withdrawal(
        ctx,
        root,
        &state,
        &caller.actor,
        text,
        WithdrawnBy::Owner,
        now,
    )
}

/// The one way a withdrawal is recorded, the requester's and the owner's alike: posted on the root when the run
/// has its root message, otherwise straight into the inbox, and its discharge is then the caller's to print.
pub(crate) fn record_withdrawal(
    ctx: &BridgeCtx,
    root: &str,
    state: &RunState,
    actor: &str,
    text: &str,
    by: WithdrawnBy,
    now: Timestamp,
) -> Result<Option<MessageId>, Refused> {
    let run = &state.run;
    let key = input_key(run);
    if state.root_message.is_some() {
        return post_and_enqueue(ctx, run, root, actor, text, &key, now, |message| {
            EventBody::Withdrawn {
                message: Some(message.clone()),
                by,
            }
        })
        .map(Some);
    }
    let body = EventBody::Withdrawn { message: None, by };
    let event = Event::new(run.clone(), 0, now, key, Origin::Cli, body);
    ctx.store.enqueue(run, &event, &[])?;
    Ok(None)
}
