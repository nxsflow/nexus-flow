//! nxf's declared agent-file contribution (spec §6.2). nxf does not *write* the shared agent file —
//! since P3-S5 the multi-module assembler in `nxs` does (`nxf init` delegates to it).
//!
//! Since nexus-flow-0lj.2 nxf no longer declares its OWN AGENTS.md block either: the shared
//! files carry a single, `nxs`-owned discovery pointer to `nxs prime` (the umbrella composes it from
//! the active tools), and nxf's operating instructions + the compaction-recovery hint live in
//! `nxf prime` (fanned out from `nxs prime`). So nxf declares only its `prime` fan-out target + the
//! hook the umbrella wires, surfaced as data via `nxf agent-manifest --json`.

use nexus_flow_facade::manifest::{AgentManifest, HookSpec, SESSION_START};

/// The command that carries nxf's session bootstrap — declared in the manifest as the fan-out
/// target the umbrella's `nxs prime` hook dispatches to (NOT what the host hook runs directly).
pub const PRIME_COMMAND: &str = "nxf prime";

/// nxf's declared contribution to the shared agent file (spec §6.2 / nexus-flow-aye.10), surfaced
/// by `nxf agent-manifest --json`. Since nexus-flow-0lj.2 this is just the `prime` fan-out target +
/// the hook to wire; the umbrella assembler writes the single nxs-owned AGENTS.md discovery pointer
/// itself, so nxf declares no per-module block.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to add that the declared `hook.command`
/// was nxf's own prime "not the hook the assembler wires (always `nxs prime`)". The assembler now
/// wires one SessionStart entry per active module, so this command IS what gets wired — give or
/// take the `|| cat NEXUS_MEMORY.md` fallback the first entry carries.
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
        // The declared hook command is nxf's own prime — the fan-out target, NOT the wired hook
        // (the assembler always wires `nxs prime`).
        assert_eq!(m.hook.command, PRIME_COMMAND);
        assert_eq!(m.hook.event, SESSION_START);
    }
}
