//! Which inputs change nothing. `decide` answers these with one `ignored { reason }`, and folding such an
//! input changes no state, so the two can never disagree: both ask here.

use nexus_run_contracts::{
    Disposition, EndCause, Event, EventBody, ReasonCode, SessionFact, Slot, TimerPurpose, TreeRefusal,
};

use crate::effects::{owed_deliveries, ready_to_release};
use crate::state::{Asked, ParkPhase, Phase, RunState, SlotRecord, SlotState, TreePhase};

/// Why this input changes nothing, or `None` when it is to be acted on.
pub(crate) fn ignored_reason(state: &RunState, input: &Event) -> Option<ReasonCode> {
    use ReasonCode::{AlreadyFinishing, FinalState, OutOfState, StaleTimer, UnknownSession};
    if state.is_finished() {
        return Some(FinalState);
    }
    let finishing = state.is_finishing().then_some(AlreadyFinishing);
    let out_of_state = |applies: bool| (!applies).then_some(OutOfState);
    match &input.body {
        EventBody::Commissioned { .. } => out_of_state(!state.commissioned),
        EventBody::RootOpened { .. } => out_of_state(
            state.root_message.is_none()
                && matches!(state.phase, Phase::Created | Phase::Finishing { .. }),
        ),
        EventBody::ClaimTaken => {
            finishing.or(out_of_state(matches!(state.phase, Phase::Queued { .. })))
        }
        EventBody::ClaimBusy { holder } => finishing.or(out_of_state(match &state.phase {
            Phase::Queued { behind } => behind.as_ref() != Some(holder),
            _ => false,
        })),
        EventBody::ClaimReleased => out_of_state(ready_to_release(state)),
        EventBody::TreePrepared { .. } | EventBody::TreeRefused { .. } => {
            finishing.or(out_of_state(matches!(
                state.phase,
                Phase::Running {
                    tree: TreePhase::Unprepared
                }
            )))
        }
        EventBody::TreeParked { .. } => out_of_state(matches!(
            state.phase,
            Phase::Finishing {
                tree: ParkPhase::Unparked,
                ..
            }
        )),
        EventBody::Delivered { .. } => out_of_state(delivery_key(input).is_some_and(|key| {
            owed_deliveries(state).iter().any(|(owed, _)| owed == key)
        })),
        EventBody::Nudged { session_key } => match record(state, session_key) {
            None => Some(UnknownSession),
            Some(r) => out_of_state(matches!(r.state, SlotState::Quiet { .. })),
        },
        EventBody::StopSignalled {
            session_key, nth, ..
        } => match record(state, session_key) {
            None => Some(UnknownSession),
            Some(r) => out_of_state(r.state.ladder().is_some_and(|l| *nth == l.total + 1)),
        },
        EventBody::SessionFact { session_key, fact } => match record(state, session_key) {
            None => Some(UnknownSession),
            Some(r) => fact_ignored(state, r, fact),
        },
        EventBody::Reply { session_key, .. } => match record(state, session_key) {
            None => Some(UnknownSession),
            Some(r) => reply_ignored(state, r),
        },
        EventBody::Answer {
            question,
            disposition,
            ..
        } => finishing.or(out_of_state(state.open_questions().any(|q| {
            q.id == *question
                && !(*disposition == Disposition::Accept
                    && matches!(q.asked, Asked::Escalation { .. }))
        }))),
        EventBody::Resume { slot } => {
            finishing.or(out_of_state(resume_target(state, slot.as_ref()).is_some()))
        }
        EventBody::ResumeRun => finishing.or(out_of_state(state.condition.is_some())),
        EventBody::Withdrawn { .. } => {
            let open_stop = state
                .slots
                .iter()
                .any(|r| r.state.ladder().is_some_and(|l| !l.forced));
            (state.is_finishing() && !open_stop).then_some(AlreadyFinishing)
        }
        EventBody::TimerFired { timer, fire_at } => {
            let armed = state.timers.get(timer);
            (armed.map(|a| a.fire_at) != Some(*fire_at)).then_some(StaleTimer)
        }
        EventBody::Decision(_) | EventBody::Unknown(_) => Some(OutOfState),
    }
}

fn record<'a>(state: &'a RunState, session_key: &str) -> Option<&'a SlotRecord> {
    state.find(session_key).map(|index| &state.slots[index])
}

fn fact_ignored(state: &RunState, record: &SlotRecord, fact: &SessionFact) -> Option<ReasonCode> {
    use ReasonCode::{FinalState, OutOfState};
    let talking = matches!(
        record.state,
        SlotState::Running { .. } | SlotState::Quiet { .. } | SlotState::Nudged { .. }
    );
    let out_of_state = |applies: bool| (!applies).then_some(OutOfState);
    match fact {
        SessionFact::Started { .. } => out_of_state(match record.state {
            SlotState::Starting { .. } | SlotState::Stopping { .. } => true,
            // A superseded attempt that reports it started owes a stop, whatever it was.
            SlotState::Ended { .. } => state.is_superseded(record),
            _ => false,
        }),
        SessionFact::Heartbeat { .. } => out_of_state(talking),
        SessionFact::Usage { .. } | SessionFact::PermissionRefused { .. } => None,
        SessionFact::GroupLingering { .. } => out_of_state(record.state.is_live()),
        SessionFact::Ended { .. } => match record.state {
            SlotState::Ended { .. } => Some(FinalState),
            SlotState::Pending => Some(OutOfState),
            SlotState::Replied { process, .. } => {
                (process == crate::state::Process::Ended).then_some(FinalState)
            }
            _ => None,
        },
        SessionFact::Unknown(_) => Some(OutOfState),
    }
}

fn reply_ignored(state: &RunState, record: &SlotRecord) -> Option<ReasonCode> {
    use ReasonCode::{
        AlreadyFinishing, DuplicateReply, FinalState, LateReplyAfterSkip, OutOfState,
    };
    if matches!(record.state, SlotState::Replied { .. }) {
        return Some(DuplicateReply);
    }
    if state.is_superseded(record) {
        return Some(LateReplyAfterSkip);
    }
    if state.is_finishing() {
        return Some(AlreadyFinishing);
    }
    match record.state {
        SlotState::Ended { .. } => Some(FinalState),
        // The engine already decided this attempt is over; its restart will ask again.
        SlotState::Stopping { .. } => Some(LateReplyAfterSkip),
        SlotState::Pending => Some(OutOfState),
        _ => None,
    }
}

/// The key of the effect a `delivered` input records: it is the input's own key, past this prefix.
pub(crate) fn delivery_key(input: &Event) -> Option<&str> {
    input.key.strip_prefix(DELIVERED_PREFIX)
}

/// The host keys a `delivered` input `delivered/<the effect's key>`, which says which delivery it records.
pub const DELIVERED_PREFIX: &str = "delivered/";

/// The condition a `resume` acts on: the named slot's, or without one the oldest there is (§6.5).
pub(crate) fn resume_target(state: &RunState, slot: Option<&Slot>) -> Option<usize> {
    let current = |index: &usize| {
        let record = &state.slots[*index];
        record.condition.is_some() && !state.is_superseded(record)
    };
    match slot {
        Some(slot) => state.find_slot(slot).filter(current),
        None => (0..state.slots.len()).filter(current).min_by_key(|index| {
            state.slots[*index]
                .condition
                .as_ref()
                .map(|condition| condition.order())
        }),
    }
}

/// Whether this destructive timer fired so far past its instant that the host cannot have been looking:
/// a machine waking, or a service coming back. It is put off once, by `wake_jump`, which is two adapter
/// samples: a session that is alive has shown a fact by then and re-armed it, and only a session still
/// silent then takes the rung. Pure, because lateness is in the input (LV1, §12.6).
pub(crate) fn is_deferred(state: &RunState, input: &Event) -> bool {
    let EventBody::TimerFired { timer, fire_at } = &input.body else {
        return false;
    };
    let Some(armed) = state.timers.get(timer) else {
        return false;
    };
    let destructive = match armed.purpose {
        TimerPurpose::StartDeadline
        | TimerPurpose::Confirm
        | TimerPurpose::StopGrace
        | TimerPurpose::WindDown
        | TimerPurpose::TreeWindow
        | TimerPurpose::RootDeadline => true,
        TimerPurpose::Quiet
        | TimerPurpose::SubprocessIdle
        | TimerPurpose::CallBound
        | TimerPurpose::HoldRetry => false,
    };
    let late = input.at.unix_millis() - fire_at.unix_millis();
    destructive && late > RunState::millis(state.policy.wake_jump)
}

/// A refusal no retry can clear: a person has to move the directory (§5).
pub(crate) fn is_permanent(reason: &TreeRefusal) -> bool {
    matches!(
        reason,
        TreeRefusal::NotARepository | TreeRefusal::ForeignDirectory
    )
}

/// The end causes that are a hold rather than a failure.
pub(crate) fn is_hold(cause: &EndCause) -> bool {
    matches!(
        cause,
        EndCause::UsageLimit { .. } | EndCause::ProviderUnavailable
    )
}
