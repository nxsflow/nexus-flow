//! The structured error type — now owned by `nexus-flow-facade` as pure data so the CLI, the
//! MCP server, and embedding apps share one error contract (E5 #9t7.1). Re-exported here so the
//! `crate::error::…` paths throughout the CLI keep resolving unchanged.
//!
//! Rendering an error to a process is presentation, so it stays CLI-side: [`emit`] writes the
//! `--json` envelope to stdout (or the human message to stderr) and returns the exit code. The
//! facade error type carries no such method — an embedding app (Tauri) has no process stdout.

pub use nexus_flow_facade::error::*;

/// Render `err` and return the process exit code. In `--json` mode the structured envelope
/// `{"error":{"kind":..,"msg":..}}` goes to stdout; otherwise the human message goes to stderr.
pub fn emit(err: &NxfError, json: bool) -> i32 {
    if json {
        let env = serde_json::json!({
            "error": { "kind": err.kind.as_str(), "msg": err.msg }
        });
        println!("{env}");
    } else {
        // Frame human errors like successful output (nexus-flow-vwx): the leading blank line is
        // emitted by `run()` on stdout before the command runs; the trailing blank follows the
        // message here on stderr.
        eprintln!("error: {}", err.msg);
        eprintln!();
    }
    1
}
