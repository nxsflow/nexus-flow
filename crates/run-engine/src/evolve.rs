//! `evolve` applies any event. Replay is `fold(evolve, initial, log)`. An unknown INPUT leaves the state
//! unchanged; an unknown DECISION is an error, because folding past it would diverge silently (§13).

use nexus_run_contracts::{
    Decision, Event, EventBody, Handle, Outcome, ReasonCode, Seq, Slot, StepId, TimerPurpose,
    Timestamp,
};

use crate::flow::SYNTH;
use crate::input::apply_input;
use crate::state::{
    timer_slot, AfterStop, ArmedTimer, Asked, FinishedStep, Ladder, ParkPhase, Phase, Question,
    RunCondition, RunState, SlotCondition, SlotRecord, SlotState, StartCause, TreePhase, TurnKind,
    TurnPart, VerdictRef,
};

/// A decision kind this version does not know, read from the log of a newer writer. The host does not
/// step such a run; status projects it read-only, `waiting_for: "a newer nxs"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownDecision {
    pub kind: String,
    pub seq: Seq,
}

/// Applies one event of the log.
pub fn evolve(mut state: RunState, event: &Event) -> Result<RunState, UnknownDecision> {
    match &event.body {
        EventBody::Unknown(unknown) if event.is_unknown_decision() => Err(UnknownDecision {
            kind: unknown.kind.clone(),
            seq: event.seq,
        }),
        EventBody::Decision(decision) => {
            apply_decision(&mut state, decision, event.at);
            state.seq = event.seq;
            Ok(state)
        }
        _ => {
            apply_input(&mut state, event);
            state.seq = event.seq;
            Ok(state)
        }
    }
}

/// What a DECISION does to state. `decide` folds through this as it goes, so the list it returns and the
/// state a replay reaches are the same thing. `at` is the instant of the input that caused it, which is
/// the envelope instant a host writes on the decisions it appends with that input.
pub(crate) fn apply_decision(state: &mut RunState, decision: &Decision, at: Timestamp) {
    match decision {
        Decision::RootRequested | Decision::WithdrawForced | Decision::Ignored { .. } => {}
        Decision::ClaimRequested => state.phase = Phase::Queued { behind: None },
        Decision::RunQueued { behind } => {
            state.phase = Phase::Queued {
                behind: behind.clone(),
            };
            state.claim_changes += 1;
        }
        Decision::StepStarted {
            step,
            visit,
            attempt,
            members,
        } => apply_step_started(state, step, *visit, *attempt, members, at),
        Decision::StepFinished {
            step,
            visit,
            outcome,
        } => state.finished_steps.push(FinishedStep {
            step: step.clone(),
            visit: *visit,
            outcome: *outcome,
            at,
        }),
        Decision::Rework {
            from,
            declaring,
            pass,
            ..
        } => {
            state.passes.insert(declaring.clone(), *pass);
            state.pending_start = state
                .verdict_of(from)
                .map(|verdict| StartCause::Rework { verdict });
        }
        Decision::Escalated {
            question,
            slot,
            message,
        } => state.questions.push(Question {
            id: question.clone(),
            asked: Asked::Escalation {
                slot: slot.clone(),
                message: message.clone(),
            },
            since: at,
            answered: false,
        }),
        Decision::Ceiling {
            question,
            step,
            pass,
            last_verdict,
        } => state.questions.push(Question {
            id: question.clone(),
            asked: Asked::Ceiling {
                step: step.clone(),
                pass: *pass,
                last_verdict: last_verdict.clone(),
            },
            since: at,
            answered: false,
        }),
        Decision::TimerSet {
            timer,
            fire_at,
            purpose,
        } => {
            state.timers.insert(
                timer.clone(),
                ArmedTimer {
                    fire_at: *fire_at,
                    purpose: *purpose,
                },
            );
            if *purpose == TimerPurpose::StartDeadline {
                arm_start(state, timer, *fire_at);
            }
        }
        Decision::TimerCleared { timer } => {
            state.timers.remove(timer);
        }
        Decision::Liveness {
            slot,
            reason,
            since,
        } => apply_liveness(state, slot, reason, *since),
        Decision::Held {
            slot,
            reason,
            until,
        } => {
            let order = next_order(state);
            if let Some(record) = record_of(state, slot) {
                record.holds += 1;
                record.condition = Some(SlotCondition::Held {
                    reason: reason.clone(),
                    until: *until,
                    order,
                });
            }
        }
        Decision::Parked {
            slot: Some(slot),
            reason,
        } => {
            let order = next_order(state);
            if let Some(record) = record_of(state, slot) {
                record.condition = Some(SlotCondition::Parked {
                    reason: reason.clone(),
                    order,
                    since: at,
                });
            }
        }
        Decision::Parked { slot: None, reason } => {
            let order = next_order(state);
            state.condition = Some(RunCondition {
                reason: reason.clone(),
                order,
                since: at,
            });
        }
        Decision::Finishing { outcome, reason } => {
            apply_finishing(state, *outcome, reason.clone(), at);
        }
        Decision::Finished { outcome, reason } => {
            state.phase = Phase::Finished {
                outcome: *outcome,
                reason: reason.clone(),
            };
            state.timers.clear();
        }
    }
}

fn next_order(state: &mut RunState) -> u64 {
    state.next_order += 1;
    state.next_order
}

fn record_of<'a>(state: &'a mut RunState, slot: &Slot) -> Option<&'a mut SlotRecord> {
    let index = state.find_slot(slot)?;
    state.slots.get_mut(index)
}

/// Arming the start deadline is what takes a waiting slot to `Starting`, so the two never disagree: no
/// timer means no `StartSession`.
fn arm_start(state: &mut RunState, timer: &str, deadline: Timestamp) {
    let Some(slot) = timer_slot(timer) else {
        return;
    };
    if let Some(record) = record_of(state, &slot) {
        if matches!(
            record.state,
            SlotState::Pending | SlotState::Starting { .. }
        ) {
            record.state = SlotState::Starting { deadline };
        }
    }
}

fn apply_liveness(state: &mut RunState, slot: &Slot, reason: &ReasonCode, since: Timestamp) {
    let forced = state.withdrawn;
    let Some(record) = record_of(state, slot) else {
        return;
    };
    let open_call = match record.state {
        SlotState::Running { open_call, .. } | SlotState::Nudged { open_call, .. } => open_call,
        _ => None,
    };
    record.state = if *reason == ReasonCode::Quiet {
        SlotState::Quiet { since, open_call }
    } else {
        SlotState::Stopping {
            ladder: Ladder::new(since, forced),
            after: AfterStop::Restart {
                exhausted: reason.clone(),
            },
        }
    };
}

fn apply_finishing(
    state: &mut RunState,
    outcome: Outcome,
    reason: Option<ReasonCode>,
    at: Timestamp,
) {
    state.phase = Phase::Finishing {
        outcome,
        reason,
        tree: ParkPhase::Unparked,
    };
    state.condition = None;
    state.tree_cancel_owed = false;
    let forced = state.withdrawn;
    for record in &mut state.slots {
        record.condition = None;
        record.state = match &record.state {
            SlotState::Pending => SlotState::Ended {
                cause: nexus_run_contracts::EndCause::StoppedByHost,
            },
            SlotState::Starting { .. }
            | SlotState::Running { .. }
            | SlotState::Quiet { .. }
            | SlotState::Nudged { .. } => SlotState::Stopping {
                ladder: Ladder::new(at, forced),
                after: AfterStop::Nothing,
            },
            SlotState::Stopping { ladder, .. } => SlotState::Stopping {
                ladder: *ladder,
                after: AfterStop::Nothing,
            },
            other => other.clone(),
        };
    }
}

fn apply_step_started(
    state: &mut RunState,
    step: &StepId,
    visit: u32,
    attempt: u32,
    members: &[Handle],
    at: Timestamp,
) {
    if matches!(state.phase, Phase::Queued { .. } | Phase::Created) {
        state.phase = Phase::Running {
            tree: TreePhase::Unprepared,
        };
    }
    state.visits.insert(step.clone(), visit);
    state.current_step = Some(step.clone());
    let cause = state.pending_start.take();
    for member in members {
        let mut record = new_record(state, step, visit, attempt, member, cause.as_ref());
        record.created_at = at;
        state.slots.push(record);
    }
}

fn new_record(
    state: &RunState,
    step: &StepId,
    visit: u32,
    attempt: u32,
    member: &Handle,
    cause: Option<&StartCause>,
) -> SlotRecord {
    let declared_resume = member != SYNTH && state.def.step(step).is_some_and(|s| s.resume);
    let prior = state.latest(step, visit, member);
    let mut record = SlotRecord {
        slot: Slot {
            step: step.clone(),
            visit,
            member: member.clone(),
            attempt,
        },
        state: SlotState::Pending,
        created_at: Timestamp::from_unix_millis(0),
        resume: declared_resume,
        turn: Vec::new(),
        base_turn: Vec::new(),
        restarts: 0,
        holds: 0,
        reminded: false,
        condition: None,
        fresh: false,
        blind: false,
        sampling: false,
        backend_session: None,
        last_fact: None,
        pids: Vec::new(),
        refused: None,
    };
    match prior {
        None => {
            record.base_turn = first_turn(state, step, member, visit, cause);
            record.turn.clone_from(&record.base_turn);
        }
        Some(prior) => next_attempt(&mut record, prior, cause, declared_resume),
    }
    record
}

/// An attempt after the first: what it carries over and what it spends. Attempts only ever rise; an
/// answered escalation and a hold spend no restart, and a person's resume starts the budgets over.
fn next_attempt(
    record: &mut SlotRecord,
    prior: &SlotRecord,
    cause: Option<&StartCause>,
    declared_resume: bool,
) {
    record.base_turn.clone_from(&prior.base_turn);
    record.restarts = prior.restarts;
    record.holds = prior.holds;
    record.reminded = prior.reminded;
    let plain_restart = match (cause, &prior.state) {
        (Some(StartCause::Answer { message }), _) => {
            record.turn = vec![
                part(TurnKind::Answer, Some(message.clone())),
                part(TurnKind::Obligation, None),
            ];
            record.resume = true;
            false
        }
        (_, SlotState::Ended { cause }) if crate::guard::is_hold(cause) => {
            record.turn.clone_from(&prior.turn);
            record.resume = true;
            false
        }
        (
            _,
            SlotState::Ended {
                cause: nexus_run_contracts::EndCause::TurnComplete,
            },
        ) if !prior.reminded => {
            record.turn = vec![
                part(TurnKind::Reminder, None),
                part(TurnKind::Obligation, None),
            ];
            record.resume = true;
            record.reminded = true;
            record.restarts += 1;
            false
        }
        _ => true,
    };
    if plain_restart {
        record.turn.clone_from(&record.base_turn);
        record.resume = declared_resume;
        record.restarts += 1;
    }
    if prior.fresh {
        record.restarts = 0;
        record.holds = 0;
        record.reminded = false;
    }
}

fn part(kind: TurnKind, message: Option<String>) -> TurnPart {
    TurnPart { kind, message }
}

fn reply_part(verdict: &VerdictRef) -> TurnPart {
    part(
        TurnKind::Reply {
            step: verdict.step.clone(),
            member: verdict.member.clone(),
        },
        Some(verdict.message.clone()),
    )
}

/// The first turn of a seat (§6.4 item 3): the root, what `input` names, the task, the obligation. A back
/// edge carries the verdict that sent the work back instead of what `input` declares.
fn first_turn(
    state: &RunState,
    step: &str,
    member: &str,
    visit: u32,
    cause: Option<&StartCause>,
) -> Vec<TurnPart> {
    if member == SYNTH {
        return synthesis_turn(state, step, visit);
    }
    let mut turn = vec![part(TurnKind::Root, state.root_message.clone())];
    match cause {
        Some(StartCause::Rework { verdict }) => turn.push(reply_part(verdict)),
        Some(StartCause::Accept { answer, verdict }) => {
            turn.push(part(TurnKind::Answer, Some(answer.clone())));
            turn.push(reply_part(verdict));
        }
        Some(StartCause::Answer { .. }) | None => {
            let inputs = declared_inputs(state, step);
            turn.extend(
                inputs
                    .iter()
                    .filter_map(|name| state.verdict_of(name))
                    .map(|verdict| reply_part(&verdict)),
            );
        }
    }
    if state.def.step(step).is_some_and(|s| s.task.is_some()) {
        turn.push(part(TurnKind::Task, None));
    }
    turn.push(part(TurnKind::Obligation, None));
    turn
}

/// `input`, or the preceding step where none is declared.
fn declared_inputs(state: &RunState, step: &str) -> Vec<StepId> {
    if let Some(declared) = state.def.step(step).and_then(|s| s.input.clone()) {
        return declared;
    }
    let position = state.def.steps.iter().position(|s| s.id == step);
    match position {
        Some(position) if position > 0 => vec![state.def.steps[position - 1].id.clone()],
        _ => Vec::new(),
    }
}

/// The consolidator's turn: the members' replies of this visit, its prompt, the obligation.
fn synthesis_turn(state: &RunState, step: &str, visit: u32) -> Vec<TurnPart> {
    let members = state
        .def
        .step(step)
        .map(|s| s.members())
        .unwrap_or_default();
    let mut turn: Vec<TurnPart> = members
        .iter()
        .filter_map(|member| state.latest(step, visit, member))
        .filter_map(|record| match &record.state {
            SlotState::Replied { message, .. } => Some(part(
                TurnKind::Reply {
                    step: step.to_owned(),
                    member: record.slot.member.clone(),
                },
                Some(message.clone()),
            )),
            _ => None,
        })
        .collect();
    turn.push(part(TurnKind::Task, None));
    turn.push(part(TurnKind::Obligation, None));
    turn
}
