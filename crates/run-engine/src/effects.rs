//! `pending_effects`, derived from state alone. The host executes each through its port, adapter or tree
//! worker and enqueues the result as an input, so a crash between an append and an effect is recovered by
//! deriving it again (§6.2, §8.3). Effects carry ids, never text.

use nexus_run_contracts::{
    Key, MessageId, Outcome, ParkOutcome, RunId, SessionKey, StepId, StopKind,
};

use crate::state::{
    GraceFired, ParkPhase, Phase, RunState, SlotRecord, SlotState, TreePhase, TurnPart,
};

/// The signal the stop table owes.
pub type SignalKind = StopKind;

const NUDGE_TEXT: &str = "You have gone quiet. Say what you are doing, or reply on your thread.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    OpenRoot {
        run: RunId,
    },
    TakeClaim {
        workspace: String,
    },
    PrepareTree {
        workspace: String,
    },
    /// `tree_window` closed: the host cancels what is in flight, if anything, and enqueues the standing
    /// refusal as `tree_refused`, which is the count.
    CancelTree {
        workspace: String,
    },
    /// Executed only while the claim key holds this run; otherwise the host enqueues `not_held`.
    ParkTree {
        workspace: String,
        occasion: Outcome,
    },
    ReleaseClaim {
        workspace: String,
    },
    StartSession {
        session_key: SessionKey,
        turn: Vec<TurnPart>,
        resume: bool,
    },
    NudgeSession {
        session_key: SessionKey,
        text: String,
    },
    /// `nth` rises over the slot's whole life, so the result input's key never repeats.
    StopSession {
        session_key: SessionKey,
        kind: StopKind,
        nth: u32,
    },
    Deliver {
        thread: MessageId,
        body: DeliverBody,
        key: Key,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliverBody {
    Message(MessageId),
    Discharge {
        step: Option<StepId>,
        pass: u32,
        verdict: Option<MessageId>,
        parked: Option<ParkOutcome>,
    },
    ParkNotice {
        branch: String,
        commit: String,
    },
    Question(Key),
}

/// What the run owes the outside world now.
pub fn pending_effects(state: &RunState) -> Vec<Effect> {
    let workspace = state.workspace.clone();
    match &state.phase {
        Phase::Finished { .. } => Vec::new(),
        Phase::Created => {
            if state.commissioned && state.root_message.is_none() && state.condition.is_none() {
                vec![Effect::OpenRoot {
                    run: state.run.clone(),
                }]
            } else {
                Vec::new()
            }
        }
        Phase::Queued { .. } => vec![Effect::TakeClaim { workspace }],
        Phase::Running { tree } => running_effects(state, *tree, workspace),
        Phase::Finishing { tree, outcome, .. } => {
            finishing_effects(state, tree, *outcome, workspace)
        }
    }
}

fn running_effects(state: &RunState, tree: TreePhase, workspace: String) -> Vec<Effect> {
    if state.tree_cancel_owed {
        return vec![Effect::CancelTree { workspace }];
    }
    if tree == TreePhase::Unprepared {
        if state.condition.is_some() {
            return Vec::new();
        }
        return vec![Effect::PrepareTree { workspace }];
    }
    let mut effects: Vec<Effect> = state
        .slots
        .iter()
        .filter_map(|record| slot_effect(state, record))
        .collect();
    effects.extend(deliver_effects(state));
    effects
}

fn slot_effect(state: &RunState, record: &SlotRecord) -> Option<Effect> {
    let session_key = state.session_key(&record.slot);
    match &record.state {
        SlotState::Starting { .. } => Some(Effect::StartSession {
            session_key,
            turn: record.turn.clone(),
            resume: record.resume,
        }),
        SlotState::Quiet { .. } => Some(Effect::NudgeSession {
            session_key,
            text: NUDGE_TEXT.to_owned(),
        }),
        _ => stop_effect(record, session_key),
    }
}

fn stop_effect(record: &SlotRecord, session_key: SessionKey) -> Option<Effect> {
    let ladder = record.state.ladder()?;
    let kind = stop_signal(ladder.forced, ladder.signals_sent, ladder.fired)?;
    Some(Effect::StopSession {
        session_key,
        kind,
        nth: ladder.total + 1,
    })
}

/// In this order, each owed only once the one before is done: the stops; once none is live, the park; the
/// deliveries; the release. The claim goes last, so the next run finds a tree with nothing of this one in
/// it (§6.4 item 9).
fn finishing_effects(
    state: &RunState,
    tree: &ParkPhase,
    occasion: Outcome,
    workspace: String,
) -> Vec<Effect> {
    let stops: Vec<Effect> = state
        .slots
        .iter()
        .filter_map(|record| stop_effect(record, state.session_key(&record.slot)))
        .collect();
    if state.any_live() {
        return stops;
    }
    if *tree == ParkPhase::Unparked {
        return vec![Effect::ParkTree {
            workspace,
            occasion,
        }];
    }
    let deliveries = deliver_effects(state);
    if !deliveries.is_empty() {
        return deliveries;
    }
    vec![Effect::ReleaseClaim { workspace }]
}

fn deliver_effects(state: &RunState) -> Vec<Effect> {
    let thread = state.root_thread.clone().unwrap_or_default();
    owed_deliveries(state)
        .into_iter()
        .map(|(key, body)| Effect::Deliver {
            thread: thread.clone(),
            body,
            key,
        })
        .collect()
}

/// The deliveries `delivered` has not recorded: a question until then, in `NeedsDecision`; the result
/// and a park notice in `Finishing`. A run with no root thread yet delivers nothing.
pub(crate) fn owed_deliveries(state: &RunState) -> Vec<(Key, DeliverBody)> {
    if state.root_message.is_none() {
        return Vec::new();
    }
    let mut owed: Vec<(Key, DeliverBody)> = match &state.phase {
        Phase::Finishing {
            outcome,
            tree: ParkPhase::Parked { outcome: parked },
            ..
        } => result_deliveries(state, *outcome, parked),
        Phase::Running { .. } => state
            .open_questions()
            .map(|q| {
                (
                    format!("question/{}", q.id),
                    DeliverBody::Question(q.id.clone()),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    owed.retain(|(key, _)| !state.delivered.contains(key));
    owed
}

fn result_deliveries(
    state: &RunState,
    outcome: Outcome,
    parked: &ParkOutcome,
) -> Vec<(Key, DeliverBody)> {
    let run = &state.run;
    let step = state.current_step.clone();
    let verdict = step.as_deref().and_then(|step| state.verdict_of(step));
    let result = (
        format!("result/{run}"),
        match (outcome, &verdict) {
            (Outcome::Withdrawn | Outcome::Failed, _) | (_, None) => DeliverBody::Discharge {
                pass: step.as_deref().map_or(0, |step| state.pass_of(step)),
                step: step.clone(),
                verdict: last_verdict(state),
                parked: Some(parked.clone()),
            },
            (_, Some(verdict)) => DeliverBody::Message(verdict.message.clone()),
        },
    );
    let mut deliveries = vec![result];
    if outcome != Outcome::Withdrawn {
        // Only a commit made, or local commits ahead of the remote: a clean branch owes no notice.
        if let ParkOutcome::Committed { branch, commit }
        | ParkOutcome::OnOwnBranch { branch, commit } = parked
        {
            deliveries.push((
                format!("park/{run}"),
                DeliverBody::ParkNotice {
                    branch: branch.clone(),
                    commit: commit.clone(),
                },
            ));
        }
    }
    deliveries
}

/// The newest verdict of any step, for a discharge: the current step may have none of its own yet.
fn last_verdict(state: &RunState) -> Option<MessageId> {
    state
        .finished_steps
        .iter()
        .rev()
        .find_map(|finished| state.verdict_of(&finished.step))
        .map(|verdict| verdict.message)
}

/// Whether the run may let go of the claim: every stop confirmed, the park recorded, every delivery done.
pub(crate) fn ready_to_release(state: &RunState) -> bool {
    matches!(
        state.phase,
        Phase::Finishing {
            tree: ParkPhase::Parked { .. },
            ..
        }
    ) && !state.any_live()
        && owed_deliveries(state).is_empty()
}

/// The stop table of §6.5: the signal owed for (`forced`, `signals_sent`, the grace firings folded), or
/// none. Unforced, it is two SIGTERMs and never anything else: a SIGKILL is a person's act. Forced, a
/// withdrawal shortens the ladder by the second SIGTERM and never skips the first grace; arriving in the
/// second grace, it waits for that grace to end; after the third signal nothing more is owed.
pub fn stop_signal(forced: bool, signals_sent: u8, fired: GraceFired) -> Option<SignalKind> {
    match (forced, signals_sent, fired) {
        (_, 0, _) => Some(StopKind::Term),
        (false, 1, GraceFired::First | GraceFired::Second) => Some(StopKind::Term),
        (true, 1, GraceFired::First | GraceFired::Second) => Some(StopKind::Kill),
        (true, 2, GraceFired::Second) => Some(StopKind::Kill),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
