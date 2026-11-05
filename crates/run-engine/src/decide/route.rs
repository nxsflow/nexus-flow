//! Where a run goes when a step is done: the next step, a back edge, the ceiling, the finish.

use nexus_run_contracts::{Decision, Outcome, ReasonCode, Slot, StepOutcome, TimerPurpose};

use crate::flow::SYNTH;
use crate::state::{SlotRecord, SlotState};

use super::Emit;

impl Emit<'_> {
    /// Starts every waiting slot that may start: the tree is prepared and no process of its persona
    /// lives. Its start deadline is armed by this and by nothing else, so a slot waiting for the tree or
    /// for its persona's previous process spends no attempt (§6.5).
    pub fn arm_ready(&mut self) {
        if !self.state.tree_prepared() {
            return;
        }
        let span = self.state.policy.start_deadline;
        let waiting: Vec<Slot> = self
            .state
            .slots
            .iter()
            .filter(|record| matches!(record.state, SlotState::Pending))
            .map(|record| record.slot.clone())
            .collect();
        for slot in waiting {
            if !self.state.persona_is_live(&slot.member) {
                self.set_slot_timer(&slot, span, TimerPurpose::StartDeadline);
            }
        }
    }

    /// The next visit of a step: `visit` is one monotonic number per step per run.
    pub fn start_step(&mut self, step: &str) {
        let Some(def) = self.def.step(step) else {
            return;
        };
        let members = def.members();
        let visit = self.state.visits.get(step).copied().unwrap_or(0) + 1;
        self.push(Decision::StepStarted {
            step: step.to_owned(),
            visit,
            attempt: 1,
            members,
        });
        self.arm_ready();
    }

    /// The next attempt of a seat. Only called once the previous attempt's end is confirmed, or, for an
    /// answer, with the new attempt waiting for that end.
    pub fn restart(&mut self, slot: &Slot) {
        self.push(Decision::StepStarted {
            step: slot.step.clone(),
            visit: slot.visit,
            attempt: slot.attempt + 1,
            members: vec![slot.member.clone()],
        });
        self.arm_ready();
    }

    /// A reply folded: is its step done? Every member's LATEST attempt has replied without `escalate`
    /// and, for a fan-out with a synthesis, the consolidator has replied too (§6.3).
    pub fn after_reply(&mut self, slot: &Slot) {
        let step = slot.step.clone();
        let visit = slot.visit;
        if self.state.current_step.as_deref() != Some(step.as_str())
            || self.state.visits.get(&step) != Some(&visit)
        {
            return;
        }
        let Some(def) = self.def.step(&step) else {
            return;
        };
        let replies: Option<Vec<bool>> = def
            .members()
            .iter()
            .map(|member| {
                self.state
                    .latest(&step, visit, member)
                    .and_then(member_verdict)
            })
            .collect();
        let Some(replies) = replies else {
            return;
        };
        if def.has_synthesis() {
            match self.state.latest(&step, visit, SYNTH) {
                None => {
                    self.push(Decision::StepStarted {
                        step,
                        visit,
                        attempt: 1,
                        members: vec![SYNTH.to_owned()],
                    });
                    self.arm_ready();
                    return;
                }
                Some(record) if !matches!(record.state, SlotState::Replied { .. }) => return,
                Some(_) => {}
            }
        }
        // The routing bit rides on each member's own reply, never on the consolidator's.
        let needs_rework = replies.iter().any(|bit| *bit);
        self.finish_step(&step, visit, needs_rework);
    }

    fn finish_step(&mut self, step: &str, visit: u32, needs_rework: bool) {
        let outcome = if needs_rework {
            StepOutcome::NeedsRework
        } else {
            StepOutcome::Pass
        };
        self.push(Decision::StepFinished {
            step: step.to_owned(),
            visit,
            outcome,
        });
        let Some(def) = self.def.step(step) else {
            return;
        };
        let (next, edge) = (def.next.clone(), def.on_needs_rework.clone());
        let max_passes = def.max_passes;
        match (needs_rework, edge) {
            (true, Some(to)) => {
                let pass = self.state.pass_of(step) + 1;
                if pass <= max_passes {
                    self.rework(step, &to, pass);
                } else {
                    self.ceiling(step, visit, pass);
                }
            }
            _ => match next {
                Some(next) => self.start_step(&next),
                None => self.finish_run(Outcome::Delivered, ReasonCode::Delivered),
            },
        }
    }

    fn rework(&mut self, from: &str, to: &str, pass: u32) {
        self.push(Decision::Rework {
            from: from.to_owned(),
            to: to.to_owned(),
            declaring: from.to_owned(),
            pass,
        });
        self.start_step(to);
    }

    /// The pass a rework would start is over the declaring step's ceiling: the run asks, with the claim
    /// held. It never stops quietly (owner, 2026-08-29).
    fn ceiling(&mut self, step: &str, visit: u32, pass: u32) {
        let Some(verdict) = self.state.verdict_of(step) else {
            return;
        };
        self.push(Decision::Ceiling {
            question: ceiling_key(&self.state.run, step, visit, pass),
            step: step.to_owned(),
            pass,
            last_verdict: verdict.message,
        });
    }

    /// `continue` at the ceiling: exactly ONE more pass of the back edge, and the next ceiling asks again.
    pub fn one_more_pass(&mut self, step: &str, pass: u32) {
        let Some(to) = self.def.step(step).and_then(|s| s.on_needs_rework.clone()) else {
            return;
        };
        self.rework(step, &to, pass);
    }

    /// `accept` at the ceiling: the step after the declaring step, or at the last step the end of the run.
    pub fn accept(&mut self, step: &str) {
        match self.def.step(step).and_then(|s| s.next.clone()) {
            Some(next) => self.start_step(&next),
            None => self.finish_run(Outcome::Accepted, ReasonCode::AcceptedByRequester),
        }
    }

    /// Every outcome goes through `Finishing`. What keeps running is what stops a session: its grace and
    /// its wind-down; every other timer of the run is over.
    pub fn finish_run(&mut self, outcome: Outcome, reason: ReasonCode) {
        self.push(Decision::Finishing {
            outcome,
            reason: Some(reason),
        });
        let over: Vec<String> = self
            .state
            .timers
            .iter()
            .filter(|(_, armed)| {
                !matches!(
                    armed.purpose,
                    TimerPurpose::StopGrace | TimerPurpose::WindDown
                )
            })
            .map(|(timer, _)| timer.clone())
            .collect();
        for timer in over {
            self.clear_timer(&timer);
        }
    }
}

/// A member's latest attempt counts when it replied without `escalate`: `Some(needs_rework)`.
fn member_verdict(record: &SlotRecord) -> Option<bool> {
    match &record.state {
        SlotState::Replied {
            escalate: false,
            needs_rework,
            ..
        } => Some(*needs_rework),
        _ => None,
    }
}

/// The id of a ceiling's question. It is the event's own key, by this deterministic form: `visit` rises,
/// so an accepted ceiling and a later one of the same step and pass stay two questions.
pub fn ceiling_key(run: &nexus_run_contracts::RunId, step: &str, visit: u32, pass: u32) -> String {
    format!("ceiling/{run}/{step}/v{visit}/p{pass}")
}
