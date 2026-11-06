//! The raw outcome the sidecar reports, mapped to the contract's cause by a table, here and nowhere else (§9). The
//! sidecar decides nothing and the reducer sees only the cause: the raw message stays in the session's ledger on this
//! machine and never reaches a fact.

use nexus_run_contracts::{EndCause, Timestamp};
use serde_json::Value;

/// The window a rejected rate limit names, in seconds since the epoch.
fn resets_at(raw: &Value) -> Option<Timestamp> {
    let seconds = raw.get("rate_limit")?.get("resetsAt")?.as_i64()?;
    Some(Timestamp::from_unix_millis(seconds.checked_mul(1000)?))
}

fn rate_limit_rejected(raw: &Value) -> bool {
    raw.get("rate_limit")
        .and_then(|limit| limit.get("status"))
        .and_then(Value::as_str)
        == Some("rejected")
}

/// `{ kind, subtype, is_error, message, error, rate_limit, refused }`. What it does not recognise is `exit`: restart,
/// then park (§6.5).
pub fn map_raw(raw: &Value) -> EndCause {
    let kind = raw.get("kind").and_then(Value::as_str);
    if kind == Some("stopped") {
        return EndCause::StoppedByHost;
    }
    if kind != Some("result") {
        return EndCause::Exit { status: None };
    }
    if let Some(refused) = raw.get("refused").filter(|r| r.is_object()) {
        let text = |field: &str| {
            refused
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return EndCause::PermissionRefused {
            rule: text("rule"),
            program: text("program"),
        };
    }
    let is_error = raw.get("is_error").and_then(Value::as_bool) == Some(true);
    if !is_error && raw.get("subtype").and_then(Value::as_str) == Some("success") {
        return EndCause::TurnComplete;
    }
    let error = raw.get("error").and_then(Value::as_str);
    if rate_limit_rejected(raw) || error == Some("rate_limit") {
        return EndCause::UsageLimit {
            resets_at: resets_at(raw),
        };
    }
    if error == Some("overloaded") {
        return EndCause::ProviderUnavailable;
    }
    EndCause::Exit { status: None }
}
