//! Run state and member slot state (§6.3).
//!
//! `NeedsDecision`, `Held` and `Parked` are projections of slot facts onto the run, because a fan-out has
//! three slots that may be in three conditions at once. [`RunState::view`] is that projection, and this
//! is its precedence, which holds whatever else the sibling slots are doing:
//!
//! | Open question | Held slot | Parked slot or run condition | The run is |
//! |---|---|---|---|
//! | yes | any | any | `NeedsDecision` |
//! | no | yes | any | `Held`, `until` the earliest |
//! | no | no | yes | `Parked`, the oldest condition's reason |
//! | no | no | no | `Running` |
//!
//! Only a seat's latest attempt counts: a superseded attempt's condition is over.

use std::collections::{BTreeMap, BTreeSet};

use nexus_run_contracts::{
    Base, EndCause, Handle, Key, LivenessPolicy, MessageId, Outcome, ParkOutcome, ReasonCode, RunId,
    Secs, Seq, Slot, StepId, StepOutcome, TimerPurpose, Timestamp,
};

use crate::flow::{FlowDef, SYNTH};

mod examples;
pub use examples::ROW_COUNT;

/// Everything `fold(evolve, initial, log)` knows about a run. It holds the run's pinned definition,
/// because `evolve` and `pending_effects` have no other way to know what a step's turn is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct RunState {
    pub run: RunId,
    pub def: FlowDef,
    /// The last event folded.
    pub seq: Seq,
    pub commissioned: bool,
    pub workspace: String,
    pub flow_hash: String,
    pub requester: Handle,
    pub policy: LivenessPolicy,
    pub root_thread: Option<MessageId>,
    /// Set by `root_opened`: until then the run has no thread that could carry anything.
    pub root_message: Option<MessageId>,
    pub phase: Phase,
    /// A park no slot holds: `root_failed`, `tree_refused`, `poison_input`.
    pub condition: Option<RunCondition>,
    /// Every attempt of every seat, in the order they started.
    pub slots: Vec<SlotRecord>,
    /// One monotonic number per step per run, in the session key.
    pub visits: BTreeMap<StepId, u32>,
    /// The per-edge count the ceiling reads, per DECLARING step.
    pub passes: BTreeMap<StepId, u32>,
    pub current_step: Option<StepId>,
    pub finished_steps: Vec<FinishedStep>,
    pub questions: Vec<Question>,
    pub timers: BTreeMap<String, ArmedTimer>,
    /// What the next `step_started` is started BY, where that is more than the flow going on.
    pub pending_start: Option<StartCause>,
    pub claim_since: Option<Timestamp>,
    /// How often the holder changed: the `<changes>` of the key `claim_busy/<run>/<changes>`.
    pub claim_changes: u32,
    /// How many refusals folded: the `<n>` of the key `tree_refused/<run>/<n>`.
    pub tree_refusals: u32,
    /// `root_deadline` periods gone by with no `root_opened`.
    pub root_periods: u32,
    /// `tree_window` closed: the host cancels what is in flight, and its `tree_refused` is the count.
    pub tree_cancel_owed: bool,
    pub base: Option<Base>,
    pub found: Option<ParkOutcome>,
    /// The effect keys `delivered` recorded.
    pub delivered: BTreeSet<String>,
    /// A person withdrew: every stop from here on is forced, and only this ever forces one.
    pub withdrawn: bool,
    /// Hands out the age of a condition, so "the oldest" needs no clock.
    pub next_order: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Created,
    Queued {
        behind: Option<RunId>,
    },
    Running {
        tree: TreePhase,
    },
    Finishing {
        outcome: Outcome,
        reason: Option<ReasonCode>,
        tree: ParkPhase,
    },
    Finished {
        outcome: Outcome,
        reason: Option<ReasonCode>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreePhase {
    Unprepared,
    Prepared,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParkPhase {
    Unparked,
    Parked { outcome: ParkOutcome },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCondition {
    pub reason: ReasonCode,
    pub order: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedStep {
    pub step: StepId,
    pub visit: u32,
    pub outcome: StepOutcome,
    pub at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmedTimer {
    pub fire_at: Timestamp,
    pub purpose: TimerPurpose,
}

/// A question a person owes an answer to. Its id is the key of the event that asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: Key,
    pub asked: Asked,
    pub since: Timestamp,
    pub answered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    Escalation {
        slot: Slot,
        message: MessageId,
    },
    Ceiling {
        step: StepId,
        pass: u32,
        last_verdict: MessageId,
    },
}

/// A reply that stands as a step's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerdictRef {
    pub step: StepId,
    pub member: Handle,
    pub message: MessageId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartCause {
    /// A back edge: the turn carries the verdict that sent the work back, whatever `input` declares.
    Rework { verdict: VerdictRef },
    /// An answered escalation: the same session gets the answer as its next turn.
    Answer { message: MessageId },
    /// An accepted ceiling: the next step gets the reasons and the overruled verdict as its input.
    Accept {
        answer: MessageId,
        verdict: VerdictRef,
    },
}

/// One part of a turn: ids only, composed by the host through its ports (§8.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnPart {
    pub kind: TurnKind,
    pub message: Option<MessageId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnKind {
    Root,
    Reply { step: StepId, member: Handle },
    Task,
    Obligation,
    /// The commissioner's answer or reasons.
    Answer,
    /// "You ended without replying", on a resumed session.
    Reminder,
}

/// One attempt of one seat: (step, visit, member, attempt).
#[derive(Clone, Debug, PartialEq)]
pub struct SlotRecord {
    pub slot: Slot,
    pub state: SlotState,
    /// Whether this attempt's `StartSession` resumes the persona's backend session.
    pub resume: bool,
    pub turn: Vec<TurnPart>,
    /// The turn the seat started with, which a plain restart goes back to.
    pub base_turn: Vec<TurnPart>,
    /// Restarts spent, counted apart from answers and holds: an answered escalation spends none.
    pub restarts: u32,
    /// Holds spent, against `max_holds`.
    pub holds: u32,
    pub reminded: bool,
    pub condition: Option<SlotCondition>,
    /// A person resumed this condition: the next attempt starts with full budgets.
    pub fresh: bool,
    pub blind: bool,
    /// A `subprocess_moved` proved this host samples, so `call_bound` does not apply.
    pub sampling: bool,
    pub backend_session: Option<String>,
    pub last_fact: Option<Timestamp>,
    pub pids: Vec<u32>,
    /// `<rule>: <program>`, when the session reported a refusal.
    pub refused: Option<String>,
}

/// A hold or a park on a slot: a resumable condition, with no question id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotCondition {
    Held {
        reason: ReasonCode,
        until: Timestamp,
        order: u64,
    },
    Parked {
        reason: ReasonCode,
        order: u64,
    },
}

impl SlotCondition {
    pub fn order(&self) -> u64 {
        match self {
            Self::Held { order, .. } | Self::Parked { order, .. } => *order,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// Waiting for the tree, or for the persona's previous process to end.
    Pending,
    Starting {
        deadline: Timestamp,
    },
    Running {
        session: String,
        open_call: Option<Timestamp>,
    },
    Quiet {
        since: Timestamp,
        open_call: Option<Timestamp>,
    },
    Nudged {
        at: Timestamp,
        open_call: Option<Timestamp>,
    },
    Stopping {
        ladder: Ladder,
        after: AfterStop,
    },
    Replied {
        message: MessageId,
        escalate: bool,
        needs_rework: bool,
        process: Process,
        overdue: bool,
        /// The stop of an overdue reply: the same ladder as `Stopping`'s.
        ladder: Option<Ladder>,
    },
    Ended {
        cause: EndCause,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Process {
    Live,
    Ended,
}

/// What a confirmed end of this stop leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterStop {
    /// Nothing: the attempt is superseded, or the run is finishing.
    Nothing,
    /// The engine stopped it to restart the seat; out of attempts, it parks with this reason.
    Restart { exhausted: ReasonCode },
}

/// The stop ladder of one session (§6.5): what the signal table reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ladder {
    pub since: Timestamp,
    pub forced: bool,
    /// Signals of this round, which the table reads. A person's `resume` starts a round.
    pub signals_sent: u8,
    /// Signals of the slot's whole life, so `nth` never repeats and a restart owes none the log records.
    pub total: u32,
    pub fired: GraceFired,
}

impl Ladder {
    pub fn new(since: Timestamp, forced: bool) -> Self {
        Self {
            since,
            forced,
            signals_sent: 0,
            total: 0,
            fired: GraceFired::None,
        }
    }
}

/// How many of `stop_grace/1` and `/2` have fired, folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GraceFired {
    None,
    First,
    Second,
}

impl SlotState {
    /// Whether a process of this attempt may exist. An end means leader dead AND group empty.
    pub fn is_live(&self) -> bool {
        match self {
            Self::Pending | Self::Ended { .. } => false,
            Self::Replied { process, .. } => *process == Process::Live,
            Self::Starting { .. }
            | Self::Running { .. }
            | Self::Quiet { .. }
            | Self::Nudged { .. }
            | Self::Stopping { .. } => true,
        }
    }

    /// The ladder of a stop under way, on either state that can carry one.
    pub fn ladder(&self) -> Option<&Ladder> {
        match self {
            Self::Stopping { ladder, .. } => Some(ladder),
            Self::Replied { ladder, .. } => ladder.as_ref(),
            _ => None,
        }
    }

    pub fn ladder_mut(&mut self) -> Option<&mut Ladder> {
        match self {
            Self::Stopping { ladder, .. } => Some(ladder),
            Self::Replied { ladder, .. } => ladder.as_mut(),
            _ => None,
        }
    }
}

/// The run as a person reads it: the states of §6.3, with the projections resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum RunView {
    Created,
    Queued {
        behind: Option<RunId>,
    },
    Running {
        tree: TreePhase,
    },
    NeedsDecision {
        questions: Vec<Key>,
    },
    Held {
        reason: ReasonCode,
        until: Timestamp,
    },
    Parked {
        reason: ReasonCode,
    },
    Finishing {
        outcome: Outcome,
    },
    Finished {
        outcome: Outcome,
    },
}

impl RunState {
    /// The state a fold starts from, with the run's pinned definition.
    pub fn initial(run: RunId, def: FlowDef) -> Self {
        Self {
            run,
            def,
            seq: 0,
            commissioned: false,
            workspace: String::new(),
            flow_hash: String::new(),
            requester: String::new(),
            policy: LivenessPolicy::default(),
            root_thread: None,
            root_message: None,
            phase: Phase::Created,
            condition: None,
            slots: Vec::new(),
            visits: BTreeMap::new(),
            passes: BTreeMap::new(),
            current_step: None,
            finished_steps: Vec::new(),
            questions: Vec::new(),
            timers: BTreeMap::new(),
            pending_start: None,
            claim_since: None,
            claim_changes: 0,
            tree_refusals: 0,
            root_periods: 0,
            tree_cancel_owed: false,
            base: None,
            found: None,
            delivered: BTreeSet::new(),
            withdrawn: false,
            next_order: 0,
        }
    }

    /// The projection of §6.3. See the table at the top of this file.
    pub fn view(&self) -> RunView {
        match &self.phase {
            Phase::Finished { outcome, .. } => {
                return RunView::Finished { outcome: *outcome }
            }
            Phase::Finishing { outcome, .. } => {
                return RunView::Finishing { outcome: *outcome }
            }
            Phase::Queued { behind } => {
                return RunView::Queued {
                    behind: behind.clone(),
                }
            }
            Phase::Created | Phase::Running { .. } => {}
        }
        let questions: Vec<Key> = self.open_questions().map(|q| q.id.clone()).collect();
        if !questions.is_empty() {
            return RunView::NeedsDecision { questions };
        }
        if let Some((reason, until)) = self.earliest_hold() {
            return RunView::Held { reason, until };
        }
        if let Some(reason) = self.oldest_park() {
            return RunView::Parked { reason };
        }
        match &self.phase {
            Phase::Running { tree } => RunView::Running { tree: *tree },
            _ => RunView::Created,
        }
    }

    fn earliest_hold(&self) -> Option<(ReasonCode, Timestamp)> {
        self.current_conditions()
            .filter_map(|condition| match condition {
                SlotCondition::Held { reason, until, .. } => Some((reason.clone(), *until)),
                SlotCondition::Parked { .. } => None,
            })
            .min_by_key(|(_, until)| *until)
    }

    fn oldest_park(&self) -> Option<ReasonCode> {
        let slots = self.current_conditions().filter_map(|c| match c {
            SlotCondition::Parked { reason, order } => Some((*order, reason.clone())),
            SlotCondition::Held { .. } => None,
        });
        let run = self.condition.iter().map(|c| (c.order, c.reason.clone()));
        slots
            .chain(run)
            .min_by_key(|(order, _)| *order)
            .map(|(_, reason)| reason)
    }

    /// The conditions on seats' latest attempts.
    fn current_conditions(&self) -> impl Iterator<Item = &SlotCondition> {
        self.slots
            .iter()
            .filter(|record| !self.is_superseded(record))
            .filter_map(|record| record.condition.as_ref())
    }

    pub fn open_questions(&self) -> impl Iterator<Item = &Question> {
        self.questions.iter().filter(|q| !q.answered)
    }

    pub fn outcome(&self) -> Option<Outcome> {
        match &self.phase {
            Phase::Finishing { outcome, .. } | Phase::Finished { outcome, .. } => Some(*outcome),
            _ => None,
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, Phase::Finished { .. })
    }

    pub fn is_finishing(&self) -> bool {
        matches!(self.phase, Phase::Finishing { .. })
    }

    pub fn tree_prepared(&self) -> bool {
        matches!(
            self.phase,
            Phase::Running {
                tree: TreePhase::Prepared
            }
        )
    }

    pub fn session_key(&self, slot: &Slot) -> String {
        slot.session_key(&self.run)
    }

    /// The record of the attempt with this session key.
    pub fn find(&self, session_key: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|record| self.session_key(&record.slot) == session_key)
    }

    /// The record of exactly this slot.
    pub fn find_slot(&self, slot: &Slot) -> Option<usize> {
        self.slots.iter().position(|record| record.slot == *slot)
    }

    /// The latest attempt of a seat.
    pub fn latest(&self, step: &str, visit: u32, member: &str) -> Option<&SlotRecord> {
        self.slots
            .iter()
            .rev()
            .find(|r| r.slot.step == step && r.slot.visit == visit && r.slot.member == member)
    }

    /// Whether a later attempt of this seat exists.
    pub fn is_superseded(&self, record: &SlotRecord) -> bool {
        self.latest(&record.slot.step, record.slot.visit, &record.slot.member)
            .is_some_and(|latest| latest.slot.attempt != record.slot.attempt)
    }

    /// Whether a process of this persona may be alive in this run. Two processes never share one
    /// backend session, so a start of the same persona waits for this to be false.
    pub fn persona_is_live(&self, member: &str) -> bool {
        self.slots
            .iter()
            .any(|record| record.slot.member == member && record.state.is_live())
    }

    pub fn any_live(&self) -> bool {
        self.slots.iter().any(|record| record.state.is_live())
    }

    /// The reply that stands as this step's verdict: on its latest visit, the consolidator's where it
    /// has one, otherwise the member's.
    pub fn verdict_of(&self, step: &str) -> Option<VerdictRef> {
        let visit = *self.visits.get(step)?;
        let fan_out = self.def.step(step).is_some_and(|s| s.is_fan_out());
        self.slots
            .iter()
            .rev()
            .filter(|r| r.slot.step == step && r.slot.visit == visit)
            .filter(|r| !fan_out || r.slot.member == SYNTH)
            .find_map(|r| match &r.state {
                SlotState::Replied { message, .. } => Some(VerdictRef {
                    step: step.to_owned(),
                    member: r.slot.member.clone(),
                    message: message.clone(),
                }),
                _ => None,
            })
    }

    pub fn pass_of(&self, step: &str) -> u32 {
        self.passes.get(step).copied().unwrap_or(0)
    }

    /// `policy` read as milliseconds, for comparing two instants.
    pub(crate) fn millis(span: Secs) -> i64 {
        i64::try_from(span.0.saturating_mul(1000)).unwrap_or(i64::MAX)
    }
}

/// The reason a stop's last rung parks with.
pub(crate) trait ReasonCodeExt {
    fn is_would_not_stop(&self) -> bool;
}

impl ReasonCodeExt for ReasonCode {
    fn is_would_not_stop(&self) -> bool {
        *self == Self::WouldNotStop
    }
}

/// The session key's tail, which every slot timer's id ends in.
pub(crate) fn slot_suffix(slot: &Slot) -> String {
    format!(
        "{}/v{}/{}/a{}",
        slot.step, slot.visit, slot.member, slot.attempt
    )
}

/// The id of a slot's timer: the first one's `start_deadline/build/v1/coder/a1`.
pub fn slot_timer(purpose: TimerPurpose, slot: &Slot) -> String {
    format!("{}/{}", purpose_word(purpose), slot_suffix(slot))
}

/// The id of the `nth` grace of a stop: `stop_grace/1/...` or `stop_grace/2/...`.
pub fn grace_timer(nth: u8, slot: &Slot) -> String {
    format!("stop_grace/{nth}/{}", slot_suffix(slot))
}

pub const ROOT_DEADLINE_TIMER: &str = "root_deadline";
pub const TREE_WINDOW_TIMER: &str = "tree_window";

fn purpose_word(purpose: TimerPurpose) -> &'static str {
    match purpose {
        TimerPurpose::RootDeadline => "root_deadline",
        TimerPurpose::StartDeadline => "start_deadline",
        TimerPurpose::Quiet => "quiet",
        TimerPurpose::Confirm => "confirm",
        TimerPurpose::CallBound => "call_bound",
        TimerPurpose::SubprocessIdle => "subprocess_idle",
        TimerPurpose::StopGrace => "stop_grace",
        TimerPurpose::WindDown => "wind_down",
        TimerPurpose::HoldRetry => "hold_retry",
        TimerPurpose::TreeWindow => "tree_window",
    }
}

/// The slot a timer id names, by its last four segments. Ids hold no other `/`, by the compiler's rule.
pub(crate) fn timer_slot(timer: &str) -> Option<Slot> {
    let mut tail = timer.rsplitn(5, '/');
    let attempt = tail.next()?.strip_prefix('a')?.parse().ok()?;
    let member = tail.next()?.to_owned();
    let visit = tail.next()?.strip_prefix('v')?.parse().ok()?;
    let step = tail.next()?.to_owned();
    tail.next()?;
    Some(Slot {
        step,
        visit,
        member,
        attempt,
    })
}

#[cfg(test)]
mod tests;
