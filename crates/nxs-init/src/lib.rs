//! nxs-init — the shared init/compose library (nexus-flow-5jz.1).
//!
//! Holds the platform glue the umbrella needs to compose its modules at COMPILE time, kept in a
//! crate the modules can depend on WITHOUT a cycle back through `nxs`:
//!
//! - [`module`]: the [`module::ModuleInit`] self-registration descriptor + the `inventory` roster
//!   (`init` and `prime` both read it) — replaces the umbrella's old hardcoded `KNOWN_MODULES`.
//! - [`spawn`]: the hardened shell-out to a module's standalone binary (`prime`/`agent-manifest`).
//! - [`assembler`]: the multi-module agent-file assembler + one SessionStart hook per active
//!   module (§6.2; a single hook until nxf n2m6 + a2a1 — see that module's own doc).
//! - [`service`]: what every init path does about the background service — the registry write that
//!   gives a workspace its clock, shared so the umbrella's path and each module's own front door
//!   cannot drift apart (nxf 6j6v.y12q).
//!
//! The init DRIVER (the chooser/welcome/summary) and `prime`/`selfheal` stay in `nxs`; they read
//! the roster from here and take it as a parameter, so they stay unit-testable without linking any
//! module. See `docs/specs/nxs-platform.md` §6.3.

pub mod advertise;
pub mod assembler;
pub mod beads;
pub mod frontdoor;
pub mod module;
pub mod service;
pub mod spawn;

pub use module::{
    binary_for, find, resolve_active, resolve_known, roster, sort_roster, FirstCommand, InitFn,
    InitRequest, ModuleInit,
};
