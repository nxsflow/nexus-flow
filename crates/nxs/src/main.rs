//! nxs — the platform umbrella CLI (binary entry point) AND the compile-time composition root
//! (5jz.1).
//!
//! A thin shim: the parser definition, dispatch, and verb bodies live in the library crate
//! (`src/lib.rs` → `cli`). The three `use … as _` imports are LOAD-BEARING: they force the binary to
//! link each suite module so its `inventory::submit!(ModuleInit)` self-registration is present in
//! the final image — that is how `nxs init`/`prime` see flow + memory + chat in the roster without
//! `nxs` hardcoding them. Dropping a `use` would silently empty that module from the roster.

// Force-link the suite modules so their compile-time roster registrations are collected (5jz.1).
use nexus_chat as _;
use nexus_flow_cli as _;
use nexus_memory as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Restore SIG_DFL for the one-shot CLI filters (`nxf blocked | head`) so a closed pipe kills
    // the process quietly instead of panicking on EPIPE (4yt4). `nxs mcp serve` scopes this back to
    // SIG_IGN at its own entry, so its graceful stdio-error path survives (see `nxs::sigpipe`).
    nxs::sigpipe::reset_to_default();
    nxs::run()
}
