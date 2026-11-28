//! A ticket's key (RFC 0002, "Ticket ids"): the creating member's tag, a dot, and a number that
//! counts up per tag, such as `ka.42`.
//!
//! **Minting.** [`mint`] gives the key of the next ticket the member the database runs as
//! creates in a space: its own tag, and the highest number among the keys with that tag in its
//! copy, plus one; 1 when its copy holds none.

use std::fmt;
use std::str::FromStr;

use crate::id::SpaceId;
use crate::members::tags::Tag;
use crate::opened::Opened;

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

    fn from_str(_text: &str) -> Result<Self, NotAKey> {
        Err(NotAKey)
    }
}

/// Why a key could not be minted or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Database { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for KeyError {}

/// The key of the next ticket the database's member creates in the open space `space`.
pub async fn mint(_opened: &Opened, _space: &SpaceId) -> Result<Key, KeyError> {
    Err(KeyError::Database {
        message: "not minted yet".to_owned(),
    })
}
