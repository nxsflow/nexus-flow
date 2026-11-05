//! `decide` is the only place a transition is chosen. It returns decision events and performs nothing.
//!
//! It works on a copy of the state with the input folded in, and folds each decision as it emits it, so
//! what comes later in a list reads the state the earlier ones made, and a replay cannot reach another.

use nexus_run_contracts::{
    Decision, Disposition, Event, EventBody, ReasonCode, Secs, Slot, TimerPurpose, Timestamp,
    WithdrawnBy,
};

use crate::evolve::apply_decision;
use crate::flow::FlowDef;
use crate::guard::{ignored_reason, is_permanent, resume_target};
use crate::input::apply_input;
use crate::state::{
    slot_timer, Asked, Phase, RunState, SlotCondition, ROOT_DEADLINE_TIMER, TREE_WINDOW_TIMER,
};

mod route;
mod session;
mod timers;

/// The next decisions of a run for one input. An input that changes nothing returns one `ignored`.
///
/// `def` must be the definition `state` was pinned to: `evolve` reads `state.def`, so another one would
/// make this list and its replay disagree.
pub fn decide(state: &RunState, input: &Event, def: &FlowDef) -> Vec<Decision> {
    if let Some(reason) = ignored_reason(state, input) {
        return vec![Decision::Ignored {
            input_seq: input.seq,
            reason,
        }];
    }
    let mut folded = state.clone();
    apply_input(&mut folded, input);
    let mut emit = Emit {
        before: state,
        state: folded,
        at: input.at,
        def,
        out: Vec::new(),
    };
    emit.input(input);
    emit.out
}

/// The key a host writes a decision under, with the input's own key and the decision's place in the list.
/// A question's id IS the key of the event that asked (§6.3), which is what an answer names.
pub fn decision_key(input_key: &str, index: usize, decision: &Decision) -> String {
    match decision {
        Decision::Escalated { question, .. } | Decision::Ceiling { question, .. } => {
            question.clone()
        }
        _ => format!("{input_key}/d{index}"),
    }
}

/// The list being written, and the state it has reached.
pub(crate) struct Emit<'a> {
    pub before: &'a RunState,
    pub state: RunState,
    pub at: Timestamp,
    pub def: &'a FlowDef,
    pub out: Vec<Decision>,
}

impl Emit<'_> {
    pub fn push(&mut self, decision: Decision) {
        apply_decision(&mut self.state, &decision, self.at);
        self.out.push(decision);
    }

    pub fn set_timer(&mut self, timer: String, span: Secs, purpose: TimerPurpose) {
        self.set_timer_at(timer, self.at.plus(span), purpose);
    }

    pub fn set_timer_at(&mut self, timer: String, fire_at: Timestamp, purpose: TimerPurpose) {
        self.push(Decision::TimerSet {
            timer,
            fire_at,
            purpose,
        });
    }

    pub fn set_slot_timer(&mut self, slot: &Slot, span: Secs, purpose: TimerPurpose) {
        self.set_timer(slot_timer(&purpose, slot), span, purpose);
    }

    /// Cancels a timer if it is armed: `timer_cleared` is only ever a cancellation.
    pub fn clear_timer(&mut self, timer: &str) {
        if self.state.timers.contains_key(timer) {
            self.push(Decision::TimerCleared {
                timer: timer.to_owned(),
            });
        }
    }

    pub fn clear_slot_timer(&mut self, slot: &Slot, purpose: TimerPurpose) {
        self.clear_timer(&slot_timer(&purpose, slot));
    }

    /// Cancels every timer of this slot.
    pub fn clear_slot_timers(&mut self, slot: &Slot) {
        let suffix = format!("/{}", crate::state::slot_suffix(slot));
        let armed: Vec<String> = self
            .state
            .timers
            .keys()
            .filter(|timer| timer.ends_with(&suffix))
            .cloned()
            .collect();
        for timer in armed {
            self.clear_timer(&timer);
        }
    }

    fn input(&mut self, input: &Event) {
        match &input.body {
            EventBody::Commissioned { .. } => {
                self.push(Decision::RootRequested);
                let span = self.state.policy.root_deadline;
                self.set_timer(
                    ROOT_DEADLINE_TIMER.to_owned(),
                    span,
                    TimerPurpose::RootDeadline,
                );
            }
            EventBody::RootOpened { .. } => {
                if matches!(self.state.phase, Phase::Created) {
                    self.clear_timer(ROOT_DEADLINE_TIMER);
                    self.push(Decision::ClaimRequested);
                }
            }
            EventBody::ClaimTaken => {
                if let Some(first) = self.def.steps.first() {
                    let first = first.id.clone();
                    self.start_step(&first);
                }
                self.set_tree_window();
            }
            EventBody::ClaimBusy { holder } => self.push(Decision::RunQueued {
                behind: Some(holder.clone()),
            }),
            EventBody::ClaimReleased => self.finished(),
            EventBody::TreePrepared { .. } => {
                self.clear_timer(TREE_WINDOW_TIMER);
                self.arm_ready();
            }
            EventBody::TreeRefused { reason } => {
                // A transient refusal inside the window is counted and retried; the window decides. A
                // refusal the host enqueues for a run already parked is the count of its cancel.
                if is_permanent(reason) && self.before.condition.is_none() {
                    self.clear_timer(TREE_WINDOW_TIMER);
                    self.push(Decision::Parked {
                        slot: None,
                        reason: ReasonCode::TreeRefused,
                    });
                }
            }
            EventBody::TreeParked { .. } | EventBody::Delivered { .. } => {}
            EventBody::Nudged { session_key } => self.nudged(session_key),
            EventBody::StopSignalled { session_key, .. } => self.stop_signalled(session_key),
            EventBody::SessionFact { session_key, fact } => self.fact(session_key, fact),
            EventBody::Reply { session_key, .. } => self.reply(session_key),
            EventBody::Answer {
                question,
                disposition,
                ..
            } => self.answer(question, disposition),
            EventBody::Resume { slot } => self.resume(slot.as_ref()),
            EventBody::ResumeRun => self.resume_run(),
            EventBody::Withdrawn { by, .. } => self.withdrawn(by),
            EventBody::TimerFired { timer, fire_at } => self.timer_fired(timer, *fire_at),
            EventBody::Decision(_) | EventBody::Unknown(_) => {}
        }
    }

    pub fn set_tree_window(&mut self) {
        let span = self.state.policy.tree_window;
        self.set_timer(TREE_WINDOW_TIMER.to_owned(), span, TimerPurpose::TreeWindow);
    }

    fn finished(&mut self) {
        if let Phase::Finishing {
            outcome, reason, ..
        } = &self.state.phase
        {
            let (outcome, reason) = (outcome.clone(), reason.clone());
            self.push(Decision::Finished { outcome, reason });
        }
    }

    fn answer(&mut self, question: &str, disposition: &Disposition) {
        let Some(asked) = self
            .before
            .questions
            .iter()
            .find(|q| q.id == question)
            .map(|q| q.asked.clone())
        else {
            return;
        };
        match asked {
            Asked::Escalation { slot, .. } => self.restart(&slot),
            Asked::Ceiling { step, pass, .. } => match disposition {
                Disposition::Continue => self.one_more_pass(&step, pass),
                Disposition::Accept => self.accept(&step),
                // The guard ignores an unknown disposition, so it never gets here.
                Disposition::Unknown(_) => {}
            },
        }
    }

    fn resume(&mut self, slot: Option<&Slot>) {
        let Some(index) = resume_target(self.before, slot) else {
            return;
        };
        let record = &self.before.slots[index];
        let slot = record.slot.clone();
        match &record.condition {
            Some(SlotCondition::Held { .. }) => {
                self.clear_slot_timer(&slot, TimerPurpose::HoldRetry);
                self.restart(&slot);
            }
            Some(SlotCondition::Parked { reason, .. }) if *reason == ReasonCode::WouldNotStop => {}
            Some(SlotCondition::Parked { .. }) => self.restart(&slot),
            None => {}
        }
    }

    fn resume_run(&mut self) {
        let Some(condition) = &self.before.condition else {
            return;
        };
        match condition.reason {
            ReasonCode::RootFailed => {
                let span = self.state.policy.root_deadline;
                self.set_timer(
                    ROOT_DEADLINE_TIMER.to_owned(),
                    span,
                    TimerPurpose::RootDeadline,
                );
            }
            ReasonCode::TreeRefused => self.set_tree_window(),
            _ => {}
        }
    }

    fn withdrawn(&mut self, by: &WithdrawnBy) {
        if self.before.is_finishing() {
            self.push(Decision::WithdrawForced);
            return;
        }
        let reason = match by {
            WithdrawnBy::Requester => ReasonCode::WithdrawnByRequester,
            WithdrawnBy::Owner => ReasonCode::WithdrawnByOwner,
            // The human exit never closes: a party this version does not know still withdraws.
            WithdrawnBy::Unknown(raw) => ReasonCode::Unknown(raw.clone()),
        };
        self.finish_run(nexus_run_contracts::Outcome::Withdrawn, reason);
    }
}

#[cfg(test)]
mod tests;
