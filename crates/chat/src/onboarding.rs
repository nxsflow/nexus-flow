//! nxc's declared agent-file contribution (spec §5.2 / platform §6.2). nxc does not *write* the
//! shared agent file — the multi-module assembler in `nxs` does (`nxc init` delegates to it,
//! mirroring `nxm init`).
//!
//! Like nxm since nexus-flow-0lj.2, nxc declares NO per-module AGENTS.md block: the shared
//! files carry a single, `nxs`-owned discovery pointer to `nxs prime` (the umbrella composes it from
//! the active tools). The chat rule (agent-to-agent coordination goes through `nxc`, not ad-hoc), the
//! core verbs, and the address book now live in `nxc prime` (fanned out from `nxs prime`) — so nxc
//! declares only its `prime` fan-out target + the hook to wire, surfaced as data via
//! `nxc agent-manifest --json`.

use nxs_foundation::manifest::{AgentManifest, HookSpec, SESSION_START};

/// The command that carries nxc's session bootstrap — declared in the manifest as the fan-out target
/// the umbrella's `nxs prime` hook dispatches to (NOT what the host hook runs directly).
pub const PRIME_COMMAND: &str = "nxc prime";

/// nxc's declared contribution to the shared agent file (spec §5.2), surfaced by
/// `nxc agent-manifest --json`. It is just the `prime` fan-out target + the hook to wire; the
/// umbrella assembler writes the single nxs-owned AGENTS.md discovery pointer itself, so nxc declares
/// no per-module block (the chat rule + verbs live in `nxc prime`). The declared `hook.command` is
/// nxc's own prime (the fan-out target), not the wired hook (the assembler always wires `nxs prime`).
pub fn manifest() -> AgentManifest {
    AgentManifest {
        prime_command: PRIME_COMMAND.to_string(),
        hook: HookSpec {
            event: SESSION_START.to_string(),
            command: PRIME_COMMAND.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_prime_as_the_fan_out_target_and_the_session_hook() {
        let m = manifest();
        assert_eq!(m.prime_command, PRIME_COMMAND);
        // The declared hook command is nxc's own prime — the fan-out target, NOT the wired hook
        // (the assembler always wires `nxs prime`).
        assert_eq!(m.hook.command, PRIME_COMMAND);
        assert_eq!(m.hook.event, SESSION_START);
    }
}
