//! The status projection, pure. The host adds `owner` and the live `tree` fields around it (§6.6).

use nexus_run_contracts::status::{
    ClaimStatus, FlowStatus, MemberStatus, Needs, NeedsKind, Owner, RunState as StatusState,
    RunStatus, SlotLiveness, StepStatus, TreeStatus, Verdict,
};
use nexus_run_contracts::{Outcome, ReasonCode, Timestamp, CONTRACT_V};

use crate::state::{
    Asked, ParkPhase, Phase, RunState, RunView, SlotCondition, SlotRecord, SlotState,
};

/// Run status at `now`. `pending_inputs` are inputs still in the inbox: reported as pending, so status
/// never claims a reply arrived before the owner folded it.
pub fn status(state: &RunState, pending_inputs: u32, now: Timestamp) -> RunStatus {
    let view = state.view();
    let outcome = match view {
        RunView::Finished { outcome } => Some(outcome),
        _ => None,
    };
    RunStatus {
        v: CONTRACT_V,
        run: state.run.clone(),
        flow: FlowStatus {
            name: state.def.name.clone(),
            hash: state.flow_hash.clone(),
        },
        seq: state.seq,
        finished: state.is_finished(),
        outcome,
        state: status_state(&view),
        claim: ClaimStatus {
            workspace_id: state.workspace.clone(),
            held: state.claim_since.is_some() && !state.is_finished(),
            since: state.claim_since,
        },
        owner: Owner {
            attended: false,
            last_seen: None,
        },
        tree: TreeStatus {
            path: String::new(),
            base: state.base.clone(),
            found: state.found.clone(),
            commit: None,
            branch: None,
            dirty: None,
            in_flight: None,
            parked: parked_outcome(state),
            refusal: None,
        },
        steps: step_statuses(state, now),
        pending_inputs,
        waiting_for: waiting_for(state, &view),
        needs: needs(state, &view, now),
        reason: reason(state, &view),
        next_action: next_action(state, &view),
    }
}

fn status_state(view: &RunView) -> StatusState {
    match view {
        RunView::Created => StatusState::Created,
        RunView::Queued { behind } => StatusState::Queued {
            behind: behind.clone(),
        },
        RunView::Running { .. } => StatusState::Running,
        RunView::NeedsDecision { .. } => StatusState::NeedsDecision,
        RunView::Held { until, .. } => StatusState::Held { until: *until },
        RunView::Parked { .. } => StatusState::Parked,
        RunView::Finishing { .. } => StatusState::Finishing,
        RunView::Finished { .. } => StatusState::Finished,
    }
}

fn parked_outcome(state: &RunState) -> Option<nexus_run_contracts::ParkOutcome> {
    match &state.phase {
        Phase::Finishing {
            tree: ParkPhase::Parked { outcome },
            ..
        } => Some(outcome.clone()),
        _ => None,
    }
}

fn step_statuses(state: &RunState, now: Timestamp) -> Vec<StepStatus> {
    let mut steps = Vec::new();
    for (id, visit) in &state.visits {
        let members: Vec<MemberStatus> = state
            .slots
            .iter()
            .filter(|r| r.slot.step == *id && r.slot.visit == *visit && !state.is_superseded(r))
            .map(|record| member_status(state, record, now))
            .collect();
        let finished = state
            .finished_steps
            .iter()
            .rev()
            .find(|f| f.step == *id && f.visit == *visit);
        steps.push(StepStatus {
            id: id.clone(),
            visit: *visit,
            pass: state.pass_of(id),
            state: if finished.is_some() { "done" } else { "open" }.to_owned(),
            outcome: finished.map(|f| f.outcome),
            started: None,
            ended: finished.map(|f| f.at),
            members,
        });
    }
    steps
}

fn member_status(state: &RunState, record: &SlotRecord, now: Timestamp) -> MemberStatus {
    let quiet_for_s = record.last_fact.map_or(0, |at| {
        u64::try_from((now.unix_millis() - at.unix_millis()) / 1000).unwrap_or(0)
    });
    let verdict = match &record.state {
        SlotState::Pending | SlotState::Starting { .. } | SlotState::Quiet { .. } => Verdict::Quiet,
        SlotState::Nudged { .. } => Verdict::Suspect,
        SlotState::Stopping { .. } => Verdict::Hung,
        SlotState::Running { .. } | SlotState::Replied { .. } | SlotState::Ended { .. } => {
            Verdict::Working
        }
    };
    MemberStatus {
        handle: record.slot.member.clone(),
        session_key: state.session_key(&record.slot),
        attempt: record.slot.attempt,
        state: slot_word(&record.state).to_owned(),
        liveness: SlotLiveness {
            last_fact: record.last_fact,
            quiet_for_s,
            verdict,
        },
        pids: record.pids.clone(),
        refused: record.refused.clone(),
    }
}

fn slot_word(state: &SlotState) -> &'static str {
    match state {
        SlotState::Pending => "pending",
        SlotState::Starting { .. } => "starting",
        SlotState::Running { .. } => "running",
        SlotState::Quiet { .. } => "quiet",
        SlotState::Nudged { .. } => "nudged",
        SlotState::Stopping { .. } => "stopping",
        SlotState::Replied { .. } => "replied",
        SlotState::Ended { .. } => "ended",
    }
}

fn waiting_for(state: &RunState, view: &RunView) -> Option<String> {
    match view {
        RunView::Finished { .. } => None,
        RunView::NeedsDecision { .. } | RunView::Parked { .. } | RunView::Held { .. } => {
            Some(format!("a decision of {}", state.requester))
        }
        RunView::Queued { behind } => behind.as_ref().map(|holder| format!("the holder {holder}")),
        RunView::Finishing { .. } => match &state.phase {
            Phase::Finishing {
                tree: ParkPhase::Unparked,
                ..
            } if !state.any_live() => Some("the park".to_owned()),
            _ => None,
        },
        RunView::Created | RunView::Running { .. } => state
            .slots
            .iter()
            .rev()
            .find(|r| !matches!(r.state, SlotState::Replied { .. } | SlotState::Ended { .. }))
            .map(|record| format!("the reply of {}", record.slot.member)),
    }
}

fn needs(state: &RunState, view: &RunView, _now: Timestamp) -> Option<Needs> {
    match view {
        RunView::NeedsDecision { questions } => {
            let first = state.open_questions().next()?;
            Some(Needs {
                kind: NeedsKind::Decision,
                question: questions.first().cloned(),
                since: first.since,
            })
        }
        RunView::Parked { .. } => Some(Needs {
            kind: NeedsKind::Person,
            question: None,
            since: state.claim_since.unwrap_or(Timestamp::from_unix_millis(0)),
        }),
        _ => None,
    }
}

fn reason(state: &RunState, view: &RunView) -> Option<ReasonCode> {
    match view {
        RunView::Held { reason, .. } | RunView::Parked { reason } => Some(reason.clone()),
        RunView::NeedsDecision { .. } => state.open_questions().find_map(|q| match q.asked {
            Asked::Ceiling { .. } => Some(ReasonCode::Ceiling),
            Asked::Escalation { .. } => None,
        }),
        RunView::Finished { .. } | RunView::Finishing { .. } => match &state.phase {
            Phase::Finishing { reason, .. } | Phase::Finished { reason, .. } => {
                // A slot that will not stop parks a finishing run with the outcome kept.
                let stuck = state.slots.iter().find_map(|r| match &r.condition {
                    Some(SlotCondition::Parked { reason, .. }) => Some(reason.clone()),
                    _ => None,
                });
                stuck.or_else(|| reason.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

fn next_action(state: &RunState, view: &RunView) -> Option<String> {
    let root = state.root_thread.as_deref().unwrap_or("<root>");
    match view {
        RunView::NeedsDecision { .. } => Some(format!(
            "nxc reply --thread {root} - answers; nxc withdraw --thread {root} ends the run"
        )),
        RunView::Held { .. } | RunView::Parked { .. } => Some(format!(
            "nxc reply --thread {root} - resumes; nxc withdraw --thread {root} ends the run"
        )),
        RunView::Finished {
            outcome: Outcome::Withdrawn,
        } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests;
