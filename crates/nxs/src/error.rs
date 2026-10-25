//! The structured error envelope for `nxs`, re-exported from the foundation (the same closed
//! `kind` + human `msg` shape every nxs consumer shares) plus the process-rendering [`emit`].
//! Rendering an error to a process is presentation, so it stays CLI-side (mirrors `nxf`/`nxm`).

pub use nxs_foundation::error::*;

/// The exit code of a pass whose bearer token ran out mid-way (6j6v.2tn0) — `EX_TEMPFAIL` from
/// `sysexits.h`, "a temporary failure; the user is invited to retry": what was moved under the old
/// token is recorded, and the same command with a fresh one resumes there. Every other error is 1.
pub const EXIT_TOKEN_EXPIRED: i32 = 75;

/// Render `err` and return the process exit code. In `--json` mode the structured envelope
/// `{"error":{"kind":..,"msg":..}}` goes to stdout; otherwise the human message goes to stderr.
pub fn emit(err: &NxfError, json: bool) -> i32 {
    if json {
        let env = serde_json::json!({
            "error": { "kind": err.kind.as_str(), "msg": err.msg }
        });
        println!("{env}");
    } else {
        eprintln!("error: {}", err.msg);
    }
    match err.kind {
        ErrorKind::TokenExpired => EXIT_TOKEN_EXPIRED,
        _ => 1,
    }
}
