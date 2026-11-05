//! What a session's facts and replies lead to: the ladder's re-arming, the table of how a session ends,
//! the restart budget and the holds (§6.5).

use nexus_run_contracts::{
    Decision, EndCause, HeartbeatKind, ReasonCode, SessionFact as Fact, Slot, TimerPurpose,
    Timestamp,
};

use crate::flow::SYNTH;
use crate::state::{grace_timer, AfterStop, SlotState};

use super::Emit;

impl Emit<'_> {
    fn index(&self, session_key: &str) -> Option<usize> {
        self.state.find(session_key)
    }

    pub fn nudged(&mut self, session_key: &str) {
        if let Some(index) = self.index(session_key) {
            let slot = self.state.slots[index].slot.clone();
            let span = self.state.policy.confirm_within;
            self.set_slot_timer(&slot, span, TimerPurpose::Confirm);
        }
    }

    /// A signal was sent: its grace starts. The third signal, a person's, has no rung after it.
    pub fn stop_signalled(&mut self, session_key: &str) {
        let Some(index) = self.index(session_key) else {
            return;
        };
        let record = &self.state.slots[index];
        let slot = record.slot.clone();
        let sent = record
            .state
            .ladder()
            .map_or(0, |ladder| ladder.signals_sent);
        if sent == 1 || sent == 2 {
            let span = self.state.policy.stop_grace;
            self.set_timer(grace_timer(sent, &slot), span, TimerPurpose::StopGrace);
        }
    }

    pub fn fact(&mut self, session_key: &str, fact: &Fact) {
        let Some(index) = self.index(session_key) else {
            return;
        };
        let was = self.before.slots[index].state.clone();
        let slot = self.state.slots[index].slot.clone();
        let was_quiet = matches!(was, SlotState::Quiet { .. } | SlotState::Nudged { .. });
        match fact {
            Fact::Started { .. } => {
                if matches!(was, SlotState::Starting { .. }) {
                    self.clear_slot_timer(&slot, TimerPurpose::StartDeadline);
                    let span = self.state.policy.quiet_after;
                    self.set_slot_timer(&slot, span, TimerPurpose::Quiet);
                }
            }
            Fact::Heartbeat { beat } => self.rearm(index, &slot, was_quiet, Some(*beat)),
            Fact::Usage { .. } | Fact::PermissionRefused { .. } => {
                if was_quiet {
                    self.rearm(index, &slot, true, None);
                }
            }
            Fact::GroupLingering { .. } => self.group_lingering(&slot, &was),
            Fact::Ended { cause } => self.ended(index, &slot, &was, cause),
            Fact::Unknown(_) => {}
        }
    }

    /// A dead leader with a non-empty group is not an end: the slot goes to `Stopping`, unforced, and the
    /// confirmed end, when it comes, restarts it as a lost session.
    fn group_lingering(&mut self, slot: &Slot, was: &SlotState) {
        match was {
            SlotState::Stopping { .. } => {}
            SlotState::Replied { .. } => self.clear_slot_timer(slot, TimerPurpose::WindDown),
            _ => {
                self.clear_slot_timers(slot);
                self.push(Decision::Liveness {
                    slot: slot.clone(),
                    reason: ReasonCode::Lost,
                    since: self.at,
                });
            }
        }
    }

    /// Re-arms the signs of life for what is now true of the slot. While a call is open `quiet` does not
    /// run. `subprocess_idle` needs a host that samples; `call_bound` applies to a BLIND host only, and is
    /// armed from the call's start until a sample proves this host is not blind.
    fn rearm(&mut self, index: usize, slot: &Slot, was_quiet: bool, beat: Option<HeartbeatKind>) {
        use HeartbeatKind::{Blind, CallsClosed, CallsOpen, ModelOutput, SubprocessMoved};
        use TimerPurpose::{CallBound, Confirm, Quiet, SubprocessIdle};
        if was_quiet {
            self.clear_slot_timer(slot, Confirm);
        }
        let record = &self.state.slots[index];
        let open = matches!(
            record.state,
            SlotState::Running {
                open_call: Some(_),
                ..
            }
        );
        let (blind, sampling) = (record.blind, record.sampling);
        let policy = self.state.policy.clone();
        if !open {
            self.clear_slot_timer(slot, SubprocessIdle);
            self.clear_slot_timer(slot, CallBound);
            if was_quiet || matches!(beat, Some(ModelOutput | CallsClosed | SubprocessMoved)) {
                self.set_slot_timer(slot, policy.quiet_after, Quiet);
            }
            return;
        }
        self.clear_slot_timer(slot, Quiet);
        if blind {
            self.clear_slot_timer(slot, SubprocessIdle);
        }
        if sampling {
            self.clear_slot_timer(slot, CallBound);
        }
        let opened_or_back = was_quiet || matches!(beat, Some(CallsOpen));
        if !blind && (opened_or_back || matches!(beat, Some(SubprocessMoved))) {
            self.set_slot_timer(slot, policy.subprocess_idle, SubprocessIdle);
        }
        if !sampling && (opened_or_back || matches!(beat, Some(Blind))) {
            self.set_slot_timer(slot, policy.call_bound, CallBound);
        }
    }

    /// The confirmed end of a process: leader dead and group empty. What it leads to is the state it ended
    /// in. A reply's end only frees its persona; a stop ordered for a restart restarts; an end out of the
    /// blue is the table of §6.5.
    fn ended(&mut self, index: usize, slot: &Slot, was: &SlotState, cause: &EndCause) {
        self.clear_slot_timers(slot);
        if self.state.is_finishing() {
            return;
        }
        match was {
            SlotState::Replied { .. }
            | SlotState::Stopping {
                after: AfterStop::Nothing,
                ..
            } => self.arm_ready(),
            SlotState::Stopping {
                after: AfterStop::Restart { exhausted },
                ..
            } => self.restart_or_park(index, slot, exhausted.clone()),
            _ => match cause {
                EndCause::TurnComplete => {
                    self.restart_or_park(index, slot, ReasonCode::EndedWithoutReply);
                }
                EndCause::Exit { .. } => {
                    self.restart_or_park(index, slot, ReasonCode::StartFailed);
                }
                EndCause::Lost => self.restart_or_park(index, slot, ReasonCode::Lost),
                EndCause::UsageLimit { resets_at } => {
                    self.hold(index, slot, ReasonCode::UsageLimit, *resets_at);
                }
                EndCause::ProviderUnavailable => {
                    self.hold(index, slot, ReasonCode::ProviderUnavailable, None);
                }
                EndCause::PermissionRefused { .. } => self.push(Decision::Parked {
                    slot: Some(slot.clone()),
                    reason: ReasonCode::PermissionRefused,
                }),
                // An ordered stop arrives through `Stopping` or a replied slot, above. No state ordered
                // this one, so nothing would ever follow it: the session is gone, as if lost.
                EndCause::StoppedByHost => self.restart_or_park(index, slot, ReasonCode::Lost),
            },
        }
    }

    /// The next attempt while the budget lasts; after it the slot parks. A fan-out member out of attempts
    /// is `fan_out_incomplete`, so the step never completes with fewer than all its members (RN3).
    pub fn restart_or_park(&mut self, index: usize, slot: &Slot, exhausted: ReasonCode) {
        let attempts = self.state.slots[index].restarts + 1;
        if attempts < self.state.policy.max_attempts {
            self.restart(slot);
            return;
        }
        let in_fan_out = slot.member != SYNTH
            && self
                .def
                .step(&slot.step)
                .is_some_and(|step| step.is_fan_out());
        let reason = if in_fan_out {
            ReasonCode::FanOutIncomplete
        } else {
            exhausted
        };
        self.push(Decision::Parked {
            slot: Some(slot.clone()),
            reason,
        });
    }

    /// On hold, not failed. Holds are counted apart from attempts, up to `max_holds` (cn4p).
    fn hold(
        &mut self,
        index: usize,
        slot: &Slot,
        reason: ReasonCode,
        resets_at: Option<Timestamp>,
    ) {
        if self.state.slots[index].holds >= self.state.policy.max_holds {
            self.push(Decision::Parked {
                slot: Some(slot.clone()),
                reason: ReasonCode::HoldLimit,
            });
            return;
        }
        let until = self.hold_until(resets_at);
        self.push(Decision::Held {
            slot: slot.clone(),
            reason,
            until,
        });
        self.push(Decision::TimerSet {
            timer: crate::state::slot_timer(TimerPurpose::HoldRetry, slot),
            fire_at: until,
            purpose: TimerPurpose::HoldRetry,
        });
    }

    /// The backend's `resets_at` clamped to the policy's window, or the policy's own retry without one.
    fn hold_until(&self, resets_at: Option<Timestamp>) -> Timestamp {
        let policy = &self.state.policy;
        let Some(resets_at) = resets_at else {
            return self.at.plus(policy.hold_retry);
        };
        resets_at
            .max(self.at.plus(policy.hold_retry_min))
            .min(self.at.plus(policy.hold_retry_max))
    }

    /// A reply folded onto its slot. It clears the slot's liveness timers and gives its process its
    /// wind-down; an escalating reply asks at once, once per attempt, and every sibling keeps running.
    pub fn reply(&mut self, session_key: &str) {
        let Some(index) = self.index(session_key) else {
            return;
        };
        let slot = self.state.slots[index].slot.clone();
        self.clear_slot_timers(&slot);
        let span = self.state.policy.stop_grace;
        self.set_slot_timer(&slot, span, TimerPurpose::WindDown);
        if let SlotState::Replied {
            message,
            escalate: true,
            ..
        } = &self.state.slots[index].state
        {
            let message = message.clone();
            self.push(Decision::Escalated {
                question: format!("escalated/{session_key}"),
                slot: slot.clone(),
                message,
            });
        }
        self.after_reply(&slot);
    }
}
