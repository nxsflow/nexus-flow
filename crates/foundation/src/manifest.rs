//! The agent-file Manifest contract (spec §6.2). Each nxs module CLI declares its contribution
//! to the shared agent file (AGENTS.md, plus the SessionStart hook) **as data** by emitting
//! `<cli> agent-manifest --json`. The `nxs` umbrella (P3) collects the manifests of the active
//! modules and ASSEMBLES the shared files + one SessionStart hook PER ACTIVE MODULE from them (nxf
//! n2m6 + a2a1; it was a single hook until the host's limit turned out to be per hook output) — it
//! hardcodes
//! no per-module knowledge, only this contract. In P1 only flow exists, so `nxf init` self-
//! assembles its own AGENTS.md from its own manifest (the multi-module assembler is P3).
//!
//! This module owns only the SHAPE. Since nexus-flow-0lj.2 a module no longer declares its OWN
//! AGENTS.md block: the shared agent file carries a single, `nxs`-owned discovery pointer to
//! `nxs prime` (the umbrella composes it from the active modules' keys), and a module's operating
//! instructions + recovery hint live in its `prime` output. So the manifest declares only the
//! per-module `prime` fan-out target + the hook the umbrella wires.
//!
//! **`CLAUDE.md` is no longer one of the destinations** (nxf 6j6v.q6e3): it belongs to the host
//! that RUNS the wired hook, so a block there is a second delivery of what the hook already gave
//! the session — and, through the block's `@NEXUS_MEMORY.md` line, the memory's whole body text
//! instead of its budgeted index. The assembler only ever cleans that file now.

use serde::{Deserialize, Serialize};

/// One module's declared contribution to the shared agent file (spec §6.2). The JSON field order
/// is part of the contract (serde preserves declaration order), so the `--json` form is stable for
/// goldens and for the P3 assembler.
///
/// Since nexus-flow-0lj.2 it carries **no per-module Markdown block**: the assembler writes a single
/// `nxs`-owned discovery line (naming the active tools + pointing at `nxs prime`), not each module's
/// own section. A module declares only its `prime` fan-out target and the hook to wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentManifest {
    /// The module's own prime command (e.g. `nxf prime`) — the **fan-out target** the umbrella's
    /// single `nxs prime` hook dispatches to, NOT a user-facing recommendation (nexus-flow-fyr).
    /// The user-facing entry is always the umbrella `nxs prime`; the AGENTS.md discovery pointer the
    /// assembler writes points there, while this field stays the internal per-module seam.
    pub prime_command: String,
    /// The host hook the module needs wired: a SessionStart event running `prime_command`.
    pub hook: HookSpec,
}

/// A declared host hook: run `command` on `event`. The only need in P1 is SessionStart → prime,
/// but the typed shape keeps the contract honest (the assembler reads `event`, never assumes it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    /// The host event the command runs on (e.g. `SessionStart`).
    pub event: String,
    /// The command to run on that event (e.g. `nxf prime`).
    pub command: String,
}

/// The canonical `SessionStart` event name a module declares for its prime hook.
pub const SESSION_START: &str = "SessionStart";

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentManifest {
        AgentManifest {
            prime_command: "nxf prime".to_string(),
            hook: HookSpec {
                event: SESSION_START.to_string(),
                command: "nxf prime".to_string(),
            },
        }
    }

    #[test]
    fn json_field_order_is_the_stable_contract() {
        // serde preserves declaration order, so the `--json` form is byte-stable for goldens and
        // the P3 assembler: prime_command, then hook{event,command} (the per-module Markdown blocks
        // were dropped in nexus-flow-0lj.2 — AGENTS.md now carries one nxs-owned discovery pointer).
        let json = serde_json::to_string(&sample()).unwrap();
        let order = |needle: &str| json.find(needle).expect("field present");
        assert!(order("\"prime_command\"") < order("\"hook\""));
        assert!(
            order("\"event\"") < order("\"command\""),
            "hook field order"
        );
    }

    #[test]
    fn round_trips_through_json() {
        let m = sample();
        let json = serde_json::to_string(&m).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back, "manifest survives a JSON round-trip");
    }
}
