//! Flow definitions as data (§6.1). The host compiles a channel declaration at commission time and pins
//! the result, so `decide` receives a definition and compiles nothing, and a later edit of the
//! declaration changes new runs only (RN7).

use std::collections::BTreeSet;

use nexus_run_contracts::{FlowRef, Handle, StepId};
use serde::{Deserialize, Serialize};

/// The reserved identity the steps' threads are opened as, as the old two-level channel does (§6.1).
pub const SUPERVISOR: &str = "__channel__";
/// The reserved identity of a fan-out's consolidator (§6.4 item 7).
pub const SYNTH: &str = "__synth__";

/// A channel as declared: the plain shape this crate reads. The host converts what its declaration port
/// hands it into this. `Default`s are the declaration language's own, so what a channel leaves out means
/// here what it means to the old engine.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChannelDecl {
    pub name: String,
    #[serde(default)]
    pub members: Vec<Handle>,
    #[serde(default)]
    pub steps: Vec<StepDecl>,
    #[serde(default)]
    pub expects: Expects,
    #[serde(default)]
    pub on_complete: OnComplete,
    #[serde(default)]
    pub summary_prompt: Option<String>,
    #[serde(default)]
    pub summary_model: Option<String>,
    #[serde(default)]
    pub working_tree: DeclaredTree,
    #[serde(default)]
    pub timeout: Option<String>,
}

/// One declared step.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StepDecl {
    pub id: StepId,
    pub target: String,
    #[serde(default)]
    pub next: Option<StepId>,
    #[serde(default)]
    pub on_needs_rework: Option<StepId>,
    #[serde(default)]
    pub max_passes: Option<u32>,
    #[serde(default)]
    pub input: Option<Vec<StepId>>,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
}

/// Which members must answer a channel.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Expects {
    #[default]
    #[serde(rename = "all")]
    All,
    #[serde(untagged)]
    Subset(Vec<Handle>),
}

/// What happens to a channel's replies once every member answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnComplete {
    #[default]
    PassThrough,
    Summarize,
}

/// A channel's declared use of the working copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredTree {
    #[default]
    Shared,
    Exclusive,
}

/// A compiled flow. Slice 1 has exclusive flows only: a shared flow has no directory of its own and no
/// tree effects are defined for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowDef {
    pub name: String,
    pub steps: Vec<StepDef>,
    pub working_tree: WorkingTree,
    /// `timeout:` declarations slice 1 ignores, for the commission receipt: never dropped silently.
    pub ignored_timeouts: Vec<IgnoredTimeout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkingTree {
    Exclusive,
}

/// One `timeout:` slice 1 does not act on: the channel or step that declared it, and its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredTimeout {
    pub scope: String,
    pub value: String,
}

/// One compiled step (§6.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepDef {
    pub id: StepId,
    pub kind: StepKind,
    pub next: Option<StepId>,
    pub on_needs_rework: Option<StepId>,
    /// The ceiling of this step's back edge: it sits on the step that declares the edge.
    pub max_passes: u32,
    /// `None` is the preceding step's reply; an empty list hands nothing.
    pub input: Option<Vec<StepId>>,
    pub resume: bool,
    pub task: Option<String>,
    pub stage: Option<String>,
    /// The channel this step's threads open in, which is what the read filters resolve visibility from.
    pub channel: String,
    pub opener: Handle,
}

impl StepDef {
    /// Who this step commissions, the consolidator not among them.
    pub fn members(&self) -> Vec<Handle> {
        match &self.kind {
            StepKind::Single { target } => vec![target.clone()],
            StepKind::FanOut { members, .. } => members.clone(),
        }
    }

    pub fn has_synthesis(&self) -> bool {
        matches!(
            self.kind,
            StepKind::FanOut {
                synthesis: Some(_),
                ..
            }
        )
    }

    pub fn is_fan_out(&self) -> bool {
        matches!(self.kind, StepKind::FanOut { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Single {
        target: Handle,
    },
    FanOut {
        members: Vec<Handle>,
        expects: FanOutExpects,
        synthesis: Option<Synthesis>,
    },
}

/// A fan-out waits for every member: "never judge with fewer answers" (RN3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanOutExpects {
    All,
}

/// The consolidator's model and prompt, from the channel's `summary_*`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synthesis {
    pub model: Option<String>,
    pub prompt: String,
}

impl FlowDef {
    /// The bytes `flow/<hash>` holds and the hash is taken over. Struct fields serialise in declaration
    /// order and the definition holds no map, so one definition has one form.
    pub fn canonical_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a flow definition is plain data")
    }

    /// The reference `commissioned.flow` pins.
    pub fn flow_ref(&self) -> FlowRef {
        FlowRef::of_canonical_json(&self.canonical_json())
    }

    pub fn step(&self, id: &str) -> Option<&StepDef> {
        self.steps.iter().find(|step| step.id == id)
    }
}

/// Why a declaration does not compile in slice 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileRefusal {
    SharedTree {
        channel: String,
    },
    ExpectsSubset {
        channel: String,
    },
    PassThroughFanOut {
        channel: String,
    },
    SummaryWithoutPrompt {
        channel: String,
    },
    /// An id outside `[A-Za-z0-9_-]`, or a reserved `__name__`: it becomes a ledger path and a key.
    BadId {
        id: String,
    },
    UnknownStep {
        name: String,
    },
    NoSteps {
        channel: String,
    },
    /// A fan-out with nobody to commission opens no slot: no timer, no reply, and the run hangs (RN3).
    NoMembers {
        channel: String,
    },
    /// `FlowDef::step` finds the first of two, so edges and visits would address the wrong one.
    DuplicateStep {
        id: String,
    },
    /// Two slots with one key: the second reply reads as a duplicate and the step never completes.
    DuplicateMember {
        channel: String,
        handle: String,
    },
    /// A back edge that can never be taken: the first verdict sending work back is already the ceiling.
    ZeroMaxPasses {
        step: String,
    },
    /// A chain of `next` with no end: only back edges have a ceiling, so nothing would stop it.
    NextCycle {
        step: String,
    },
}

/// Compiles `flow` for slice 1. A step whose target is `review`'s name is a fan-out from that channel;
/// every other target is a persona.
pub fn compile(flow: &ChannelDecl, review: &ChannelDecl) -> Result<FlowDef, CompileRefusal> {
    if flow.working_tree != DeclaredTree::Exclusive {
        return Err(CompileRefusal::SharedTree {
            channel: flow.name.clone(),
        });
    }
    if flow.steps.is_empty() {
        return Err(CompileRefusal::NoSteps {
            channel: flow.name.clone(),
        });
    }
    let mut ignored_timeouts = Vec::new();
    let mut note = |scope: &str, timeout: &Option<String>| {
        if let Some(value) = timeout {
            ignored_timeouts.push(IgnoredTimeout {
                scope: scope.to_owned(),
                value: value.clone(),
            });
        }
    };
    note(&flow.name, &flow.timeout);
    let mut steps = Vec::with_capacity(flow.steps.len());
    for declared in &flow.steps {
        note(&declared.id, &declared.timeout);
        let is_fan_out = declared.target == review.name;
        if is_fan_out {
            note(&review.name, &review.timeout);
        }
        steps.push(compile_step(flow, review, declared, is_fan_out)?);
    }
    let ids: Vec<StepId> = steps.iter().map(|step| step.id.clone()).collect();
    if let Some(id) = first_duplicate(&ids) {
        return Err(CompileRefusal::DuplicateStep { id: id.clone() });
    }
    check_references(&steps)?;
    check_next_chains(&steps)?;
    Ok(FlowDef {
        name: flow.name.clone(),
        steps,
        working_tree: WorkingTree::Exclusive,
        ignored_timeouts,
    })
}

fn compile_step(
    flow: &ChannelDecl,
    review: &ChannelDecl,
    declared: &StepDecl,
    is_fan_out: bool,
) -> Result<StepDef, CompileRefusal> {
    check_id(&declared.id)?;
    if declared.max_passes == Some(0) {
        return Err(CompileRefusal::ZeroMaxPasses {
            step: declared.id.clone(),
        });
    }
    let (kind, channel) = if is_fan_out {
        (fan_out(review)?, review.name.clone())
    } else {
        check_id(&declared.target)?;
        let target = declared.target.clone();
        (StepKind::Single { target }, flow.name.clone())
    };
    Ok(StepDef {
        id: declared.id.clone(),
        kind,
        next: declared.next.clone(),
        on_needs_rework: declared.on_needs_rework.clone(),
        max_passes: declared.max_passes.unwrap_or(1),
        input: declared.input.clone(),
        resume: declared.resume,
        task: declared.task.clone(),
        stage: declared.stage.clone(),
        channel,
        opener: SUPERVISOR.to_owned(),
    })
}

fn fan_out(review: &ChannelDecl) -> Result<StepKind, CompileRefusal> {
    let channel = || review.name.clone();
    if review.expects != Expects::All {
        return Err(CompileRefusal::ExpectsSubset { channel: channel() });
    }
    if review.on_complete == OnComplete::PassThrough {
        return Err(CompileRefusal::PassThroughFanOut { channel: channel() });
    }
    let prompt = review
        .summary_prompt
        .clone()
        .ok_or_else(|| CompileRefusal::SummaryWithoutPrompt { channel: channel() })?;
    if review.members.is_empty() {
        return Err(CompileRefusal::NoMembers { channel: channel() });
    }
    for member in &review.members {
        check_id(member)?;
    }
    if let Some(handle) = first_duplicate(&review.members) {
        return Err(CompileRefusal::DuplicateMember {
            channel: channel(),
            handle: handle.clone(),
        });
    }
    Ok(StepKind::FanOut {
        members: review.members.clone(),
        expects: FanOutExpects::All,
        synthesis: Some(Synthesis {
            model: review.summary_model.clone(),
            prompt,
        }),
    })
}

fn check_id(id: &str) -> Result<(), CompileRefusal> {
    let alphabet = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if id.is_empty() || !id.chars().all(alphabet) || id.starts_with("__") {
        return Err(CompileRefusal::BadId { id: id.to_owned() });
    }
    Ok(())
}

fn first_duplicate(ids: &[String]) -> Option<&String> {
    let mut seen = BTreeSet::new();
    ids.iter().find(|id| !seen.insert(*id))
}

fn check_references(steps: &[StepDef]) -> Result<(), CompileRefusal> {
    let known = |name: &String| steps.iter().any(|step| step.id == *name);
    for step in steps {
        let inputs = step.input.iter().flatten();
        for name in step.next.iter().chain(&step.on_needs_rework).chain(inputs) {
            if !known(name) {
                return Err(CompileRefusal::UnknownStep { name: name.clone() });
            }
        }
    }
    Ok(())
}

/// Over `next` alone: a back edge is meant to cycle, and `max_passes` bounds it.
fn check_next_chains(steps: &[StepDef]) -> Result<(), CompileRefusal> {
    let next_of = |step: &StepDef| {
        let next = step.next.as_ref()?;
        steps.iter().find(|candidate| candidate.id == *next)
    };
    for start in steps {
        let mut at = start;
        // A chain with an end visits every step at most once.
        for _ in 0..steps.len() {
            match next_of(at) {
                Some(next) => at = next,
                None => break,
            }
        }
        if next_of(at).is_some() {
            return Err(CompileRefusal::NextCycle {
                step: start.id.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
