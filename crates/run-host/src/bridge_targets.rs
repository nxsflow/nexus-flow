//! The send's check that every persona a flow would start is declared (6j6v.yw6x), beside the other bridge
//! entry points in [`crate::bridge`] and on their rules: checked before anything is written.

use nexus_run_engine::{FlowDef, StepKind};

use crate::bridge::{BridgeCtx, Refused};

/// A slot the flow would start names a handle the declarations do not resolve to a persona: the run could
/// only fail when that step is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedTarget {
    pub step: String,
    pub target: String,
    /// The nested channel a fan-out member came from; `None` is the step's own target.
    pub member_of: Option<String>,
}

impl std::fmt::Display for UnresolvedTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            step,
            target,
            member_of,
        } = self;
        f.write_str("the channel does not compile: ")?;
        match member_of {
            None => write!(
                f,
                "step `{step}` targets `{target}`, which is neither a persona the declarations hold nor \
                 a channel this flow fans out to"
            )?,
            Some(channel) => write!(
                f,
                "step `{step}` fans out to `{target}`, a member of `{channel}` that no persona \
                 declaration resolves"
            )?,
        }
        f.write_str(
            " (declarations.persona refused); nothing was written. `nxc list` names what this \
             workspace declares",
        )
    }
}

/// **Every persona a slot of `def` would start is declared**, checked before anything is written. The compile
/// alone cannot tell: a target that is no channel it knows compiles to `single`, so a nested channel the
/// declarations could not find became a persona that does not exist, stored under the flow hash.
pub(crate) fn check_slot_targets(ctx: &BridgeCtx, def: &FlowDef) -> Result<(), Refused> {
    let unresolved = |handle: &str| ctx.declarations.persona(handle).is_err();
    for step in &def.steps {
        let (handles, member_of) = match &step.kind {
            StepKind::Single { target } => (std::slice::from_ref(target), None),
            StepKind::FanOut { members, .. } => (members.as_slice(), Some(step.channel.clone())),
        };
        if let Some(target) = handles.iter().find(|handle| unresolved(handle)) {
            return Err(Refused::UnresolvedTarget(UnresolvedTarget {
                step: step.id.clone(),
                target: target.clone(),
                member_of,
            }));
        }
    }
    Ok(())
}
