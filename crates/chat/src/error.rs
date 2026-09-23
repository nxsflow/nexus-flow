//! The structured error envelope for `nxc`, re-exported from the foundation (the same closed
//! `kind` + human `msg` shape every nxs consumer shares) plus the process-rendering [`emit`].
//! Rendering an error to a process is presentation, so it stays CLI-side (mirrors flow's/memory's
//! `error`).

pub use nxs_foundation::error::*;

/// Render `err` and return the process exit code. In `--json` mode the structured envelope
/// `{"error":{"kind":..,"msg":..}}` goes to stdout; otherwise the human message goes to stderr.
pub fn emit(err: &NxfError, json: bool) -> i32 {
    if json {
        let env = serde_json::json!({ "error": { "kind": err.kind.as_str(), "msg": err.msg } });
        println!("{env}");
    } else {
        eprintln!("error: {}", err.msg);
    }
    1
}
