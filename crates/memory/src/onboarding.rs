//! nxm's declared agent-file contribution (spec §6.2 / §5.2). nxm does not *write* the shared agent
//! files — since P3-S5 the multi-module assembler in `nxs` does (`nxm init` delegates to it).
//!
//! Since nexus-flow-0lj.2 nxm no longer declares its OWN AGENTS.md block either: the shared
//! files carry a single, `nxs`-owned discovery pointer to `nxs prime` (the umbrella composes it from
//! the active tools). The memory rule (durable knowledge ONLY via `nxm remember`, never a
//! `MEMORY.md`), the command list, and the compaction-recovery hint now live in `nxm prime` (fanned
//! out from `nxs prime`), so nxm declares only its `prime` fan-out target + the hook to wire,
//! surfaced as data via `nxm agent-manifest --json`.

use nxs_foundation::manifest::{AgentManifest, HookSpec, SESSION_START};

/// The command that carries nxm's session bootstrap — declared in the manifest as the fan-out
/// target the umbrella's `nxs prime` hook dispatches to (NOT what the host hook runs directly).
pub const PRIME_COMMAND: &str = "nxm prime";

/// nxm's declared contribution to the shared agent file (spec §6.2), surfaced by
/// `nxm agent-manifest --json`. Since nexus-flow-0lj.2 this is just the `prime` fan-out target + the
/// hook to wire; the umbrella assembler writes the single nxs-owned AGENTS.md discovery pointer
/// itself, so nxm declares no per-module block (the memory rule + commands live in `nxm prime`). The
/// declared `hook.command` is nxm's own prime (the fan-out target), not the wired hook.
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
        // The declared hook command is nxm's own prime — the fan-out target, NOT the wired hook
        // (the assembler always wires `nxs prime`).
        assert_eq!(m.hook.command, PRIME_COMMAND);
        assert_eq!(m.hook.event, SESSION_START);
    }
}
