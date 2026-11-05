//! The pure reducer of the nxc run engine.

mod flow;

pub use flow::{
    compile, ChannelDecl, CompileRefusal, DeclaredTree, Expects, FanOutExpects, FlowDef,
    IgnoredTimeout, OnComplete, StepDecl, StepDef, StepKind, Synthesis, WorkingTree, SUPERVISOR,
    SYNTH,
};
