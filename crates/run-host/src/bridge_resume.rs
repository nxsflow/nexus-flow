//! `nxc resume --thread <root>` as a library function (IN3, p71m note 01M6N98GQ2F9K4D3A23X730PN1; 6j6v.y6v7), beside
//! the other bridge entry points in [`crate::bridge`] and on their rules: checked before anything is written.

use nexus_run_contracts::{Event, EventBody, Origin, RunStore, Timestamp};

use crate::bridge::{
    check_requester, input_key, root_route, state_of, BridgeCtx, Caller, Refused, RunView,
};

/// What a `resume` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// Enqueued: `resume` for a held or parked slot, `resume_run` (`run_level`) for a run parked on its own
    /// condition (`root_failed`, `tree_refused`, `poison_input`).
    Resumed { run_level: bool },
    /// A hold whose window has not passed, and no `--force`: nothing was written, and the hold lifts by
    /// itself at `until`.
    NotYet { until: Timestamp },
}

/// **`nxc resume --thread <root>`** (IN3, p71m note 01M6N98GQ2F9K4D3A23X730PN1): the requester's explicit
/// action, the one way a held or parked run restarts by hand. A plain root reply never does (§6.3). By
/// r426's mapping: a run whose only condition is its own takes `resume_run`; a held or parked slot takes
/// `resume { slot: None }`, the oldest condition. Before a hold's `until` it answers not yet, unless `force`.
pub fn resume(
    ctx: &BridgeCtx,
    root: &str,
    caller: &Caller,
    force: bool,
    now: Timestamp,
) -> Result<ResumeOutcome, Refused> {
    let run = root_route(ctx, root)?;
    let state = state_of(ctx, &run)?;
    if state.is_finished() {
        return Err(Refused::RunFinished);
    }
    check_requester(&state, caller)?;
    let body = match state.view() {
        RunView::NeedsDecision { .. } => return Err(Refused::AskedAQuestion),
        RunView::Held { until, .. } if !force && now < until => {
            return Ok(ResumeOutcome::NotYet { until });
        }
        RunView::Held { .. } => EventBody::Resume { slot: None },
        RunView::Parked { .. } => {
            let a_slot_is_parked = state
                .slots
                .iter()
                .any(|record| record.condition.is_some() && !state.is_superseded(record));
            match a_slot_is_parked || state.condition.is_none() {
                true => EventBody::Resume { slot: None },
                false => EventBody::ResumeRun,
            }
        }
        _ => return Err(Refused::NothingOnHold),
    };
    let run_level = matches!(body, EventBody::ResumeRun);
    let event = Event::new(run.clone(), 0, now, input_key(&run), Origin::Cli, body);
    ctx.store.enqueue(&run, &event, &[])?;
    Ok(ResumeOutcome::Resumed { run_level })
}
