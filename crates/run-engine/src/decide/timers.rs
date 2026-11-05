//! A timer fired. `decide` acts only if it is still the armed one (`stale_timer` otherwise, in the
//! guard), and a destructive one that fired far past its instant is put off once (the wake rule).

use nexus_run_contracts::{Decision, ReasonCode, Slot, TimerPurpose, Timestamp};

use crate::guard::is_deferred;
use crate::state::{grace_nth, timer_slot, ROOT_DEADLINE_TIMER};

use super::Emit;

impl Emit<'_> {
    pub fn timer_fired(&mut self, timer: &str, fire_at: Timestamp) {
        let Some(armed) = self.before.timers.get(timer).cloned() else {
            return;
        };
        if is_deferred(self.before, timer, fire_at, self.at) {
            let span = self.state.policy.wake_jump;
            self.set_timer(timer.to_owned(), span, armed.purpose);
            return;
        }
        let slot = timer_slot(timer);
        match (armed.purpose, slot) {
            (TimerPurpose::RootDeadline, _) => self.root_period(),
            (TimerPurpose::TreeWindow, _) => self.push(Decision::Parked {
                slot: None,
                reason: ReasonCode::TreeRefused,
            }),
            (TimerPurpose::StartDeadline, Some(slot)) => {
                self.conclude(slot, ReasonCode::StartFailed)
            }
            (TimerPurpose::Quiet, Some(slot)) => self.quiet(&slot),
            (TimerPurpose::SubprocessIdle, Some(slot)) => {
                self.clear_slot_timer(&slot, TimerPurpose::CallBound);
                self.quiet(&slot);
            }
            (TimerPurpose::CallBound, Some(slot)) => self.call_bound(&slot),
            (TimerPurpose::Confirm, Some(slot)) => {
                self.clear_slot_timers(&slot);
                self.conclude(slot, ReasonCode::Hung);
            }
            (TimerPurpose::StopGrace, Some(slot)) => self.grace_over(timer, slot),
            (TimerPurpose::HoldRetry, Some(slot)) => self.restart(&slot),
            // `overdue` and the stop it owes are the input's own fold. A decision holding an unknown
            // purpose refuses ownership, so none is ever armed in a state a reader steps.
            (TimerPurpose::WindDown | TimerPurpose::Unknown(_), _) | (_, None) => {}
        }
    }

    /// A period of `root_deadline` with no `root_opened`. The effect is re-executed every tick anyway;
    /// after `max_attempts` periods the RUN parks, with no slot and no thread (§6.5).
    fn root_period(&mut self) {
        if self.state.root_periods >= self.state.policy.max_attempts {
            self.push(Decision::Parked {
                slot: None,
                reason: ReasonCode::RootFailed,
            });
        } else {
            let span = self.state.policy.root_deadline;
            self.set_timer(
                ROOT_DEADLINE_TIMER.to_owned(),
                span,
                TimerPurpose::RootDeadline,
            );
        }
    }

    fn conclude(&mut self, slot: Slot, reason: ReasonCode) {
        self.push(Decision::Liveness {
            slot,
            reason,
            since: self.at,
        });
    }

    fn quiet(&mut self, slot: &Slot) {
        self.conclude(slot.clone(), ReasonCode::Quiet);
        let span = self.state.policy.confirm_within;
        self.set_slot_timer(slot, span, TimerPurpose::Confirm);
    }

    /// `call_bound` binds a blind host only: where the sampler works, a moving call is never bounded and a
    /// still one is `subprocess_idle`'s (SC4, LV1).
    fn call_bound(&mut self, slot: &Slot) {
        let blind = self
            .state
            .find_slot(slot)
            .is_some_and(|index| self.state.slots[index].blind);
        if blind {
            self.clear_slot_timer(slot, TimerPurpose::SubprocessIdle);
            self.quiet(slot);
        }
    }

    /// The second grace ran out. Unforced, that is the last rung: the slot parks `would_not_stop`, the host
    /// never pretends it is gone and never SIGKILLs on its own. Forced, the signal table owes the kill.
    fn grace_over(&mut self, timer: &str, slot: Slot) {
        if grace_nth(timer) != Some(2) {
            return;
        }
        let forced = self
            .state
            .find_slot(&slot)
            .and_then(|index| self.state.slots[index].state.ladder())
            .is_some_and(|ladder| ladder.forced);
        if !forced {
            self.push(Decision::Parked {
                slot: Some(slot),
                reason: ReasonCode::WouldNotStop,
            });
        }
    }
}
