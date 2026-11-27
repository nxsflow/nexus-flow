//! The decision log is append-only (RFC 0003, "Append-only"): a decision and the topic, subject
//! and supersession objects that belong to it are written in one change and never changed again.
//! A decision is corrected by a new decision that supersedes it. These are the collections the
//! store's write-once guard (`crate::write_once`) refuses every change to.

/// The decision collections, all four write-once.
pub const WRITE_ONCE: &[&str] = &[
    "decisionsDecision",
    "decisionsTopic",
    "decisionsSubject",
    "decisionsSupersession",
];
