//! The rank of a goal in focus (RFC 0005, "The order of the goals in focus").
//!
//! A rank is a string of the characters `0` to `9` and `a` to `z`, compared by its bytes, lowest
//! first, as NoCentralDB compares text in an index. Read as the digits of a base-36 fraction, a
//! new rank lies strictly between the ranks of its two neighbours, so a move writes the rank of
//! the goal it moves and no other. A rank never ends in `0`, so there is always room below it, and
//! it grows by one character only when no shorter string fits between its neighbours.

use std::cmp::Ordering;

/// The characters of a rank, in the order of their bytes.
const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Why no rank was computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankError {
    /// A neighbour's rank is empty, holds a character outside `0-9a-z`, or ends in `0`.
    Malformed(String),
    /// The rank to go before is not strictly lower than the rank to go after.
    NotOrdered { before: String, after: String },
}

/// The shortest rank strictly between `before` and `after`, the ranks of the goals the moved goal
/// goes after and before. `None` is an open end: `(None, Some(first))` places a goal first,
/// `(Some(last), None)` last, and `(None, None)` gives the first goal of an empty order.
pub fn rank_between(before: Option<&str>, after: Option<&str>) -> Result<String, RankError> {
    let low = before.map(digits).transpose()?.unwrap_or_default();
    let high = after.map(digits).transpose()?;
    if let (Some(a), Some(b)) = (before, after) {
        if a >= b {
            return Err(RankError::NotOrdered {
                before: a.to_owned(),
                after: b.to_owned(),
            });
        }
    }
    let rank = midpoint(&low, high.as_deref());
    Ok(rank
        .into_iter()
        .map(|d| DIGITS[d as usize] as char)
        .collect())
}

/// A rank's digits, 0 to 35, or why it is not a rank.
fn digits(rank: &str) -> Result<Vec<u8>, RankError> {
    let malformed = || RankError::Malformed(rank.to_owned());
    if rank.is_empty() || rank.ends_with('0') {
        return Err(malformed());
    }
    rank.bytes()
        .map(|c| DIGITS.iter().position(|&d| d == c).map(|d| d as u8))
        .collect::<Option<_>>()
        .ok_or_else(malformed)
}

/// The shortest digits strictly between `low` and `high`, where `low` (possibly empty) lies below
/// `high` and neither ends in `0`; `None` stands for no upper bound.
fn midpoint(low: &[u8], high: Option<&[u8]>) -> Vec<u8> {
    if let Some(high) = high {
        // Every rank between the two starts with the digits they share, reading `low` as padded
        // with zeros. `high` cannot be used up here, since it neither ends in `0` nor is a prefix
        // of `low`.
        let shared = high
            .iter()
            .enumerate()
            .take_while(|&(i, &d)| low.get(i).copied().unwrap_or(0) == d)
            .count();
        if shared > 0 {
            let mut out = high[..shared].to_vec();
            out.extend(midpoint(
                low.get(shared..).unwrap_or(&[]),
                Some(&high[shared..]),
            ));
            return out;
        }
    }
    let lo = low.first().copied().unwrap_or(0);
    let hi = high.map_or(DIGITS.len() as u8, |h| h[0]);
    if hi - lo > 1 {
        // One digit fits between them; the middle one leaves room on both sides.
        return vec![(lo + hi).div_ceil(2)];
    }
    match high {
        // `high`'s first digit alone lies below `high` and above `low`.
        Some(high) if high.len() > 1 => vec![high[0]],
        // Nothing of this length fits: keep `low`'s digit and go one deeper, above the rest of
        // `low` and without an upper bound.
        _ => {
            let mut out = vec![lo];
            out.extend(midpoint(low.get(1..).unwrap_or(&[]), None));
            out
        }
    }
}

/// Where a goal in focus stands in the order: by rank, then, for two goals with the same rank, by
/// key, then by object id, each compared by its bytes, so every member computes one order from the
/// same goals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place<'a> {
    pub rank: &'a str,
    pub key: &'a str,
    pub object_id: &'a str,
}

impl Ord for Place<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.rank, self.key, self.object_id).cmp(&(other.rank, other.key, other.object_id))
    }
}

impl PartialOrd for Place<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
