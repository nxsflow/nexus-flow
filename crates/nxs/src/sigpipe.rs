//! SIGPIPE disposition control — shared by the binary entry (`main`) and the MCP server (`serve`).
//!
//! The Rust runtime installs SIGPIPE as SIG_IGN before `main`, which turns a write to a
//! reader-closed pipe into an `EPIPE` error rather than terminating the process. The two personas
//! this binary serves want opposite dispositions:
//!
//! - **One-shot CLI filters** (`nxf blocked | head`, `nxf list | grep -m1 …`): SIG_IGN makes
//!   `println!` unwrap that `EPIPE` into a panic ("failed printing to stdout: Broken pipe").
//!   [`reset_to_default`] restores SIG_DFL at process start (4yt4) so the process dies quietly by
//!   the signal, exactly like every other Unix filter.
//! - **The long-running `nxs mcp serve` stdio server**: it propagates a stdio transport failure
//!   (the host closing the pipe) through rmcp as a structured `NxfError::io` error and a clean
//!   exit — a `--json`-consuming host gets a diagnostic, not a signal death. SIG_DFL (inherited
//!   from `main`) would kill it *before* that path runs, so [`restore_ignore`] re-establishes
//!   SIG_IGN at the top of `serve`, scoping the 4yt4 reset away from that persona.

/// Restore the default SIGPIPE disposition (SIG_DFL): a closed downstream pipe terminates the
/// process by signal instead of surfacing an `EPIPE` that `println!` panics on. Called once at the
/// very start of `main`, before any output, threads, or spawned children (the disposition is
/// inherited, so it also spares spawned children the SIG_IGN footgun).
#[cfg(unix)]
pub fn reset_to_default() {
    // SAFETY: a single async-signal-safe `signal` call at process start; the previous handler is
    // intentionally discarded.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// Re-establish SIG_IGN (the Rust-runtime default): a closed downstream pipe surfaces as an
/// `EPIPE` write error the caller can handle, rather than killing the process by signal. Called at
/// the top of `nxs mcp serve` so its graceful stdio-transport error path survives the [`reset_to_default`]
/// applied in `main`.
#[cfg(unix)]
pub fn restore_ignore() {
    // SAFETY: a single async-signal-safe `signal` call, before the MCP server writes any protocol
    // bytes to stdout; the previous handler is intentionally discarded.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(not(unix))]
pub fn reset_to_default() {}

#[cfg(not(unix))]
pub fn restore_ignore() {}
