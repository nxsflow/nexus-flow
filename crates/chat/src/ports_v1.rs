//! **The new engine's ports, implemented on the old side** (6j6v.zjyv §5; ticket 6j6v.b8tj): the chat model,
//! the declarations and the sidecar bundle keep their one home in this crate, and the host reaches them only
//! through these three traits, which is how the new crates stay free of old imports.
//!
//! [`V1Ports`] is one value behind all three, because all three read the same workspace: an
//! `Arc<V1Ports>` coerces to each `Arc<dyn …Port>` the host's `Ports` holds.
//!
//! **Declarations are composed from the REGISTERED root** ([`V1Ports::for_root`]): a session of a run works
//! in `<root>-v2`, whose `.nxs` link resolves to the same workspace, but the standing text of its prompt
//! comes from the root the service registered — the clone's own copies matter only to what the session reads
//! as files (§6.6, "What the directory proves").
//!
//! Every line here goes when the old engine does.
// v2-bridge: deleted with the old engine (6j6v.2vtw)

use std::path::PathBuf;

use nexus_run_contracts::MessageId;
use nexus_run_host::ports::{
    self, DeclarationPort, Message, MessagePort, PersonaDecl, PortError, PortResult, PromptBundle,
    PromptFor, SidecarPort, TurnPart,
};

use crate::definitions::Definitions;

fn refused(error: impl std::fmt::Display) -> PortError {
    PortError {
        what: error.to_string(),
    }
}

/// The host's ports over this workspace's chat store and declarations.
#[derive(Debug)]
pub struct V1Ports {
    defs: Definitions,
}

impl V1Ports {
    /// Over declarations a caller already resolved — the CLI's and an `Engine`'s, for a bridge verb run in
    /// the caller's own process.
    pub fn with_definitions(_db_path: &str, _origin: &str, defs: Definitions) -> V1Ports {
        V1Ports { defs }
    }
}

impl MessagePort for V1Ports {
    fn post(&self, _: &str, _: &str, _: &str, _: &ports::Refs, _: &str) -> PortResult<MessageId> {
        Err(refused("not built yet"))
    }
    fn find(&self, _: &str, _: &str) -> PortResult<Option<MessageId>> {
        Err(refused("not built yet"))
    }
    fn read(&self, _: &str) -> PortResult<Option<Message>> {
        Err(refused("not built yet"))
    }
    fn open_thread(&self, _: &str, _: Option<&str>, _: &str, _: &str) -> PortResult<()> {
        Err(refused("not built yet"))
    }
}

impl DeclarationPort for V1Ports {
    /// The channel as the old side declares it, in the engine's own plain shape: the two read the same
    /// declaration language, so the chat declaration's wire form is the engine's input.
    fn channel(&self, name: &str) -> PortResult<ports::ChannelDecl> {
        let decl = self
            .defs
            .channel(name)
            .ok_or_else(|| refused(format!("no channel `{name}` is declared")))?;
        let wire = serde_json::to_value(decl).map_err(refused)?;
        serde_json::from_value(wire).map_err(refused)
    }

    fn persona(&self, _: &str) -> PortResult<PersonaDecl> {
        Err(refused("not built yet"))
    }
    fn prompt(&self, _: &str, _: PromptFor) -> PortResult<PromptBundle> {
        Err(refused("not built yet"))
    }
    fn turn(&self, _: &str, _: &[TurnPart]) -> PortResult<String> {
        Err(refused("not built yet"))
    }
}

impl SidecarPort for V1Ports {
    fn bundle_path(&self) -> PortResult<PathBuf> {
        Err(refused("not built yet"))
    }
}
