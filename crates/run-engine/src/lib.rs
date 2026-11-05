//! The pure reducer of the nxc run engine: `decide` chooses a transition, `evolve` applies an event,
//! `pending_effects` derives what the run owes from state alone. No I/O, no clock, no threads, and the
//! layering gate holds that. Specification: §6 of the description of epic 6j6v.zjyv.

mod decide;
mod effects;
mod evolve;
mod flow;
mod guard;
mod input;
mod state;
mod status;

pub use decide::decide;
pub use effects::{pending_effects, stop_signal, DeliverBody, Effect, SignalKind};
pub use evolve::{evolve, UnknownDecision};
pub use guard::DELIVERED_PREFIX;
pub use status::status;
pub use flow::{
    compile, ChannelDecl, CompileRefusal, DeclaredTree, Expects, FanOutExpects, FlowDef,
    IgnoredTimeout, OnComplete, StepDecl, StepDef, StepKind, Synthesis, WorkingTree, SUPERVISOR, SYNTH,
};
pub use state::{
    grace_timer, slot_timer, AfterStop, ArmedTimer, Asked, FinishedStep, GraceFired, Ladder, ParkPhase,
    Phase, Process, Question, RunCondition, RunState, RunView, SlotCondition, SlotRecord, SlotState,
    StartCause, TreePhase, TurnKind, TurnPart, VerdictRef, ROOT_DEADLINE_TIMER, TREE_WINDOW_TIMER,
};
