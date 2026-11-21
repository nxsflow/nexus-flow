//! The rank of a goal in focus (RFC 0005, "The order of the goals in focus").

/// Why no rank was computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankError {
    /// A neighbour's rank is empty, holds a character outside `0-9a-z`, or ends in `0`.
    Malformed(String),
    /// The rank to go before is not strictly lower than the rank to go after.
    NotOrdered { before: String, after: String },
}

/// The rank strictly between `before` and `after`; `None` is an open end.
pub fn rank_between(_before: Option<&str>, _after: Option<&str>) -> Result<String, RankError> {
    Ok(String::new())
}

/// Where a goal in focus stands in the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place<'a> {
    pub rank: &'a str,
    pub key: &'a str,
    pub object_id: &'a str,
}

impl Ord for Place<'_> {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for Place<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
