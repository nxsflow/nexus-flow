//! One row of v1's op log.

/// One op of v1's log, every column as v1 stored it (RFC 0008, "What v1 holds").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Op {
    pub op_id: String,
    pub lamport: i64,
    pub site: i64,
    /// The product: `task` for flow, `fact` for memory, chat's kinds otherwise.
    pub domain: String,
    pub target_kind: String,
    pub target_id: String,
    pub field: String,
    pub op_type: String,
    pub value: Option<String>,
    /// The self-declared author, free text v1 never checked.
    pub author: Option<String>,
    pub wall_clock: Option<String>,
    pub key_id: Option<String>,
    pub sig: Option<String>,
    /// The receiver's verdict: `own`, `verified`, `invalid` or `unsigned`.
    pub provenance: String,
}
