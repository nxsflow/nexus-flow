//! The structured error type shared by every consumer of the engine.
//!
//! Errors carry a machine-readable `kind` (a closed set) plus a human `msg`, so an agent never has
//! to string-match the message to branch. This is **pure data** — rendering an error (the `--json`
//! `{"error":{kind,msg}}` envelope to stdout, or the human message to stderr) is the consumer's job.

use std::fmt;

/// The closed set of error kinds. The string form is the stable agent contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NoWorkspace,
    NotFound,
    Forbidden,
    Validation,
    Cycle,
    Conflict,
    Io,
    Verification,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::NoWorkspace => "no_workspace",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Forbidden => "forbidden",
            ErrorKind::Validation => "validation",
            ErrorKind::Cycle => "cycle",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Io => "io",
            ErrorKind::Verification => "verification",
        }
    }
}

#[derive(Debug)]
pub struct NxfError {
    pub kind: ErrorKind,
    pub msg: String,
}

impl NxfError {
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> NxfError {
        NxfError {
            kind,
            msg: msg.into(),
        }
    }

    pub fn no_workspace(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::NoWorkspace, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::NotFound, msg)
    }
    pub fn forbidden(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Forbidden, msg)
    }
    pub fn validation(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Validation, msg)
    }
    pub fn cycle(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Cycle, msg)
    }
    pub fn io(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Io, msg)
    }
    pub fn verification(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Verification, msg)
    }
}

impl fmt::Display for NxfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.msg)
    }
}

impl std::error::Error for NxfError {}

/// Map a raw SQLite error to the structured `io` kind — the store boundary's translation (#76u.13).
/// A store read that surfaces a `rusqlite::Error` (e.g. `SQLITE_BUSY` past the busy-timeout, or a
/// malformed row) is, to every seam above it, an i/o-class failure of the durable store; this lets a
/// fallible store read propagate with `?` into any `Result<_, NxfError>` and arrive as `io`, rather
/// than the leaf `.unwrap()` unwinding a long-lived MCP/embed handler. The detail of the underlying
/// error is preserved in `msg` for diagnosis.
///
/// SCOPE (review #145, Integrity #5): this blanket conversion is intended for the **read/derivation
/// io boundary** — `?` on a `Store` read, a `derive::*` query, or a parent-walk read. It deliberately
/// flattens *every* `rusqlite::Error` to `io`. A future WRITE path that `?`s a raw `rusqlite::Error`
/// from a *constraint* failure (`UNIQUE`/`CHECK` → should be `conflict`/`validation`) would silently
/// inherit `io` here. The core write ops don't surface raw `rusqlite::Error` today (they fold ops
/// and `.unwrap()` their own invariants), so no such call site exists — but a new one MUST classify
/// the constraint explicitly (match on `SqliteFailure(.., ConstraintViolation)`) instead of relying
/// on this `From`.
impl From<rusqlite::Error> for NxfError {
    fn from(e: rusqlite::Error) -> NxfError {
        NxfError::io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, NxfError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_kind_has_the_stable_string_form_and_constructor() {
        // §5.1 (nexus-chat) pairs not_found (unknown channel) with forbidden (existing channel,
        // non-member). The string form is the agent contract, so it must be exactly "forbidden".
        assert_eq!(ErrorKind::Forbidden.as_str(), "forbidden");
        let e = NxfError::forbidden("not a member of channel c-1");
        assert_eq!(e.kind, ErrorKind::Forbidden);
        assert_eq!(e.msg, "not a member of channel c-1");
    }
}
