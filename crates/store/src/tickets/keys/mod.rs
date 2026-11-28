//! A ticket's key (RFC 0002, "Ticket ids"): the creating member's tag, a dot, and a number that
//! counts up per tag, such as `ka.42`. [`mint`] gives the key of a new ticket, [`resolve`] finds
//! the ticket an argument names, and [`shown`] writes a key as `nxs` shows it.

use std::fmt;
use std::str::FromStr;

use nocentraldb::ObjectId;

use crate::members::tags::Tag;

mod error;
mod mint;
mod resolve;

pub use error::KeyError;
pub use mint::mint;
#[cfg(test)]
pub(crate) use mint::mint_synced;
pub use resolve::resolve;

/// The collection of the tickets.
pub(crate) const TICKETS: &str = "ticketsTicket";

/// The fewest digits of an object id that [`shown`] gives after a shared key, as in `ka.42~3f9a`.
pub const SHORTEST: usize = 4;

/// The index of the tickets by key.
pub(crate) const BY_KEY: &str = "byKey";

/// A ticket's key, such as `ka.42`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub tag: Tag,
    /// 1 or more.
    pub number: u64,
}

/// A text that is no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAKey;

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.tag, self.number)
    }
}

impl FromStr for Key {
    type Err = NotAKey;

    fn from_str(text: &str) -> Result<Self, NotAKey> {
        let (tag, number) = text.split_once('.').ok_or(NotAKey)?;
        let tag: Tag = tag.parse().map_err(|_| NotAKey)?;
        let canonical = !number.is_empty()
            && !number.starts_with('0')
            && number.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return Err(NotAKey);
        }
        let number = number.parse().map_err(|_| NotAKey)?;
        Ok(Key { tag, number })
    }
}

/// Each ticket's key as `nxs` shows it, in the order given: the key alone, or, for a key another
/// ticket in `tickets` holds too, the key, `~` and the first digits of its object id, at least
/// [`SHORTEST`] and as many as tell it apart from the others, the way git abbreviates a hash.
pub fn shown(tickets: &[(String, ObjectId)]) -> Vec<String> {
    tickets
        .iter()
        .map(|(key, id)| {
            let hex = id.to_string();
            let others: Vec<String> = tickets
                .iter()
                .filter(|(other, other_id)| other == key && other_id != id)
                .map(|(_, other_id)| other_id.to_string())
                .collect();
            if others.is_empty() {
                return key.clone();
            }
            let digits = (SHORTEST..hex.len())
                .find(|&n| others.iter().all(|other| other[..n] != hex[..n]))
                .unwrap_or(hex.len());
            format!("{key}~{}", &hex[..digits])
        })
        .collect()
}
