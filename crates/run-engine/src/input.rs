//! What an INPUT does to state. An input is a claim about the world: it records the fact and moves the
//! slot it names. What follows from it is `decide`'s, in decisions folded after it.

use nexus_run_contracts::{
    Disposition, EndCause, Event, EventBody, HeartbeatKind, ReasonCode, SessionFact,
};

use crate::guard::{delivery_key, ignored_reason, is_deferred, resume_target};
use crate::state::{
    Asked, GraceFired, Ladder, ParkPhase, Phase, Process, RunState, SlotCondition, SlotState,
    StartCause, TreePhase,
};

/// Folds one input. One `decide` ignored folds to nothing.
pub(crate) fn apply_input(state: &mut RunState, input: &Event) {
    if ignored_reason(state, input).is_some() {
        return;
    }
    let at = input.at;
    match &input.body {
        EventBody::Commissioned {
            workspace_id,
            flow,
            policy,
            root_thread,
            requester,
            ..
        } => {
            state.commissioned = true;
            state.workspace = workspace_id.clone();
            state.flow_hash = flow.hash.clone();
            state.policy = policy.clone();
            state.root_thread = Some(root_thread.clone());
            state.requester = requester.clone();
        }
        EventBody::RootOpened { thread, message } => {
            state.root_thread = Some(thread.clone());
            state.root_message = Some(message.clone());
            state.condition = None;
        }
        EventBody::ClaimTaken => state.claim_since = Some(at),
        EventBody::TreePrepared { base, found } => {
            state.phase = Phase::Running {
                tree: TreePhase::Prepared,
            };
            state.base = Some(base.clone());
            state.found = found.clone();
            state.condition = None;
            state.tree_cancel_owed = false;
        }
        EventBody::TreeRefused { .. } => {
            state.tree_refusals += 1;
            state.tree_cancel_owed = false;
        }
        EventBody::TreeParked { outcome } => {
            if let Phase::Finishing { tree, .. } = &mut state.phase {
                *tree = ParkPhase::Parked {
                    outcome: outcome.clone(),
                };
            }
        }
        EventBody::Delivered { .. } => {
            if let Some(key) = delivery_key(input) {
                state.delivered.insert(key.to_owned());
            }
        }
        EventBody::Nudged { session_key } => {
            if let Some(index) = state.find(session_key) {
                let slot = &mut state.slots[index];
                if let SlotState::Quiet { open_call, .. } = slot.state {
                    slot.state = SlotState::Nudged { at, open_call };
                }
            }
        }
        EventBody::StopSignalled { session_key, .. } => {
            if let Some(index) = state.find(session_key) {
                if let Some(ladder) = state.slots[index].state.ladder_mut() {
                    ladder.signals_sent = ladder.signals_sent.saturating_add(1);
                    ladder.total += 1;
                }
            }
        }
        EventBody::SessionFact { session_key, fact } => {
            if let Some(index) = state.find(session_key) {
                apply_fact(state, index, fact, input);
            }
        }
        EventBody::Reply {
            session_key,
            message,
            escalate,
            needs_rework,
        } => {
            if let Some(index) = state.find(session_key) {
                apply_reply(state, index, message, *escalate, *needs_rework);
            }
        }
        EventBody::Answer {
            question,
            disposition,
            message,
        } => apply_answer(state, question, *disposition, message),
        EventBody::Resume { slot } => {
            if let Some(index) = resume_target(state, slot.as_ref()) {
                let record = &mut state.slots[index];
                let would_not_stop = matches!(
                    &record.condition,
                    Some(SlotCondition::Parked { reason, .. }) if *reason == ReasonCode::WouldNotStop
                );
                record.condition = None;
                if would_not_stop {
                    // It signals again and never restarts: a new round of the ladder, unforced.
                    if let Some(ladder) = record.state.ladder_mut() {
                        ladder.signals_sent = 0;
                        ladder.fired = GraceFired::None;
                    }
                } else {
                    record.fresh = true;
                }
            }
        }
        EventBody::ResumeRun => {
            state.condition = None;
            state.root_periods = 0;
        }
        EventBody::Withdrawn { .. } => {
            state.withdrawn = true;
            for record in &mut state.slots {
                if let Some(ladder) = record.state.ladder_mut() {
                    ladder.forced = true;
                }
                if matches!(
                    &record.condition,
                    Some(SlotCondition::Parked { reason, .. }) if *reason == ReasonCode::WouldNotStop
                ) {
                    record.condition = None;
                }
            }
        }
        EventBody::TimerFired { timer, fire_at } => {
            let deferred = is_deferred(state, timer, *fire_at, input.at);
            let armed = state.timers.remove(timer);
            if !deferred {
                if let Some(armed) = armed {
                    apply_firing(state, timer, armed.purpose, input);
                }
            }
        }
        EventBody::ClaimBusy { .. }
        | EventBody::ClaimReleased
        | EventBody::Decision(_)
        | EventBody::Unknown(_) => {}
    }
}

fn apply_firing(
    state: &mut RunState,
    timer: &str,
    purpose: nexus_run_contracts::TimerPurpose,
    input: &Event,
) {
    use nexus_run_contracts::TimerPurpose::{RootDeadline, StopGrace, TreeWindow, WindDown};
    match purpose {
        RootDeadline => state.root_periods += 1,
        TreeWindow => state.tree_cancel_owed = true,
        StopGrace => {
            let fired = if timer.starts_with("stop_grace/2/") {
                GraceFired::Second
            } else {
                GraceFired::First
            };
            if let Some(ladder) = timer_record(state, timer).and_then(|r| r.state.ladder_mut()) {
                ladder.fired = ladder.fired.max(fired);
            }
        }
        WindDown => {
            let forced = state.withdrawn;
            if let Some(record) = timer_record(state, timer) {
                start_reply_stop(&mut record.state, input, forced);
            }
        }
        _ => {}
    }
}

fn timer_record<'a>(
    state: &'a mut RunState,
    timer: &str,
) -> Option<&'a mut crate::state::SlotRecord> {
    let slot = crate::state::timer_slot(timer)?;
    let index = state.find_slot(&slot)?;
    state.slots.get_mut(index)
}

/// A replied slot that is overdue, or whose group lingers, owes the same ladder as a `Stopping` slot.
fn start_reply_stop(slot: &mut SlotState, input: &Event, forced: bool) {
    if let SlotState::Replied {
        overdue, ladder, ..
    } = slot
    {
        *overdue = true;
        ladder.get_or_insert_with(|| Ladder::new(input.at, forced));
    }
}

fn apply_fact(state: &mut RunState, index: usize, fact: &SessionFact, input: &Event) {
    let at = input.at;
    let forced = state.withdrawn;
    let superseded = state.is_superseded(&state.slots[index]);
    let record = &mut state.slots[index];
    record.last_fact = Some(at);
    match fact {
        SessionFact::Started {
            backend_session_id, ..
        } => {
            record.backend_session = Some(backend_session_id.clone());
            match record.state {
                SlotState::Starting { .. } => {
                    record.state = SlotState::Running {
                        session: backend_session_id.clone(),
                        open_call: None,
                    };
                }
                SlotState::Ended { .. } if superseded => {
                    record.state = SlotState::Stopping {
                        ladder: Ladder::new(at, forced),
                        after: crate::state::AfterStop::Nothing,
                    };
                }
                _ => {}
            }
        }
        SessionFact::Heartbeat { beat } => {
            revive(record);
            match beat {
                HeartbeatKind::CallsOpen => set_open_call(record, Some(at)),
                HeartbeatKind::CallsClosed => set_open_call(record, None),
                HeartbeatKind::SubprocessMoved => record.sampling = true,
                HeartbeatKind::Blind => record.blind = true,
                HeartbeatKind::ModelOutput => {}
            }
        }
        SessionFact::Usage { .. } => revive(record),
        SessionFact::PermissionRefused { rule, program } => {
            record.refused = Some(format!("{rule}: {program}"));
            revive(record);
        }
        SessionFact::GroupLingering { pids } => {
            record.pids.clone_from(pids);
            start_reply_stop(&mut record.state, input, forced);
        }
        SessionFact::Ended { cause } => apply_ended(record, cause),
        SessionFact::Unknown(_) => {}
    }
}

/// ANY fact of the session returns a quiet or nudged slot to `Running` (§6.5).
fn revive(record: &mut crate::state::SlotRecord) {
    let open_call = match record.state {
        SlotState::Quiet { open_call, .. } | SlotState::Nudged { open_call, .. } => open_call,
        _ => return,
    };
    record.state = SlotState::Running {
        session: record.backend_session.clone().unwrap_or_default(),
        open_call,
    };
}

fn set_open_call(
    record: &mut crate::state::SlotRecord,
    value: Option<nexus_run_contracts::Timestamp>,
) {
    if let SlotState::Running { open_call, .. } = &mut record.state {
        match value {
            // One busy period: the sidecar reports it once, and a repeat keeps its first instant.
            Some(_) => *open_call = open_call.or(value),
            None => *open_call = None,
        }
    }
}

fn apply_ended(record: &mut crate::state::SlotRecord, cause: &EndCause) {
    if let EndCause::PermissionRefused { rule, program } = cause {
        record.refused = Some(format!("{rule}: {program}"));
    }
    record.condition = None;
    record.state = match &record.state {
        SlotState::Replied {
            message,
            escalate,
            needs_rework,
            overdue,
            ..
        } => SlotState::Replied {
            message: message.clone(),
            escalate: *escalate,
            needs_rework: *needs_rework,
            process: Process::Ended,
            overdue: *overdue,
            ladder: None,
        },
        _ => SlotState::Ended {
            cause: cause.clone(),
        },
    };
}

fn apply_reply(
    state: &mut RunState,
    index: usize,
    message: &str,
    escalate: bool,
    needs_rework: bool,
) {
    let member = state.slots[index].slot.member.clone();
    let step = state.slots[index].slot.step.clone();
    // The synthesis reply carries no routing bit by declaration, and a step with no back edge has
    // nothing the bit could pull: it passes (§6.4 items 3 and 7).
    let is_synth = member == crate::flow::SYNTH;
    let has_edge = state
        .def
        .step(&step)
        .is_some_and(|s| s.on_needs_rework.is_some());
    state.slots[index].state = SlotState::Replied {
        message: message.to_owned(),
        escalate: escalate && !is_synth,
        needs_rework: needs_rework && !is_synth && has_edge,
        process: Process::Live,
        overdue: false,
        ladder: None,
    };
}

fn apply_answer(state: &mut RunState, question: &str, disposition: Disposition, message: &str) {
    let Some(position) = state.questions.iter().position(|q| q.id == question) else {
        return;
    };
    state.questions[position].answered = true;
    let asked = state.questions[position].asked.clone();
    state.pending_start = match (&asked, disposition) {
        (Asked::Escalation { .. }, _) => Some(StartCause::Answer {
            message: message.to_owned(),
        }),
        (Asked::Ceiling { step, .. }, Disposition::Accept) => {
            state.verdict_of(step).map(|verdict| StartCause::Accept {
                answer: message.to_owned(),
                verdict,
            })
        }
        (Asked::Ceiling { .. }, Disposition::Continue) => None,
    };
}
