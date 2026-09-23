//! nxs — the platform umbrella over the composing suite (flow/memory/chat) on the ONE shared
//! `.nxs/` substrate (spec `docs/specs/nxs-platform-foundation.md` §6).
//!
//! `nxs` owns the project-/platform-wide concerns that are no module's job: it assembles the
//! shared agent file from each active module's manifest and wires the **single** SessionStart
//! hook → `nxs prime` (§6.2), fans `prime` out over the active modules (§6.3), and carries the
//! foundation-only verbs `migrate`/`doctor`/`status`. It hardcodes NO product vocabulary nor even
//! the roster: every module SELF-REGISTERS a [`nxs_init::ModuleInit`] descriptor at compile time
//! (5jz.1), and `nxs` LINKS the modules (main.rs `use … as _`) to collect it. `prime`/
//! `agent-manifest` still reach the modules by shelling out to their binaries (TB-5); only the
//! init PATH composes them as a library.
//!
//! - [`registry`]/[`assembler`]/[`spawn`]: re-exported from [`nxs_init`] — the roster the umbrella
//!   reads ([`nxs_init::roster`]), the agent-file assembler (§6.2), the hardened shell-out (TB-5).
//! - [`guide`]: the suite-wide reading of the three building blocks' own guides (6j6v.9e3r) —
//!   the same shell-out shape as `prime`, over a surface that needs no workspace and no clock.
//! - [`migrate`]/[`doctor`]: the foundation-only verbs over the shared substrate.
//! - [`advertise`]: the cross-sell of not-yet-active modules the umbrella init renders.
//! - [`cli`]: the `nxs` command tree + verb bodies.

// The platform glue moved to `nxs-init` (so the modules can depend on it without a cycle back
// through `nxs`); re-export under the historical paths so the verb bodies + tests read unchanged.
pub use nxs_init::{advertise, assembler, module as registry, spawn};

/// What `nxs init` does about the background service (nxf 6j6v.npf9): put the new workspace on
/// the list the service attends, and offer to install the service itself.
pub mod background_service;
pub mod cli;
pub mod doctor;
pub mod error;
/// The suite-wide guide fan-out (6j6v.9e3r): `nxs guide` over the three building blocks' own
/// guides, modelled on [`prime`].
pub mod guide;
pub mod init;
pub mod machines;
/// The MCP seam (E9 #76u): `nxs mcp serve` exposes the active modules' read ops as MCP tools over
/// stdio. Feature-gated — the lean build links no rmcp/tokio.
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod migrate;
pub mod migrate_beads;
pub mod prime;
pub mod selfheal;
pub mod setup;
pub mod sigpipe;
pub mod sync;
/// The multi-workspace registry (`~/.nexusflow/workspaces.toml`, E9 0jq8). Ungated — unlike `mcp`,
/// which reads it too — because the sync daemon (8kvr) iterates it in the lean build as well and
/// must not pull in rmcp/tokio to do so.
pub mod workspaces;

pub use cli::{command, run};
