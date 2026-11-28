//! Why a key could not be minted or resolved.

use std::fmt;

use crate::id::SpaceId;
use crate::members::MemberError;

/// Why a key could not be minted or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The space is not one of the open spaces, or the database holds none of it yet.
    NotHeld { space: SpaceId },
    /// The member has no tag in the space, so it cannot mint a key there.
    NoTag { space: SpaceId },
    /// The member's tag was another member's before, and the member's copy has not completed a
    /// sync: a key the earlier holder created may not have reached it yet.
    NotSynced { space: SpaceId, tag: String },
    /// No ticket of the space holds the key or the object id.
    KeyNotFound { key: String },
    /// Two or more tickets hold the key; `tickets` lists each as [`shown`](super::shown) gives it, in order.
    AmbiguousKey { key: String, tickets: Vec<String> },
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::NotHeld { space } => write!(
                f,
                "this database does not hold the space {space} yet; sync it with nxs sync first"
            ),
            KeyError::NoTag { space } => write!(
                f,
                "this member has no tag in the space {space}; nxs member retag gives it one"
            ),
            KeyError::NotSynced { space, tag } => write!(
                f,
                "the tag {tag} was another member's before, and this copy of the space {space} has not synced yet; sync with nxs sync first"
            ),
            KeyError::KeyNotFound { key } => write!(f, "no ticket of this space is {key}"),
            KeyError::AmbiguousKey { key, tickets } => write!(
                f,
                "{key} names {} tickets: {}; name one of them",
                tickets.len(),
                tickets.join(", ")
            ),
            KeyError::Database { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<nocentraldb::Error> for KeyError {
    fn from(e: nocentraldb::Error) -> Self {
        KeyError::Database {
            message: e.to_string(),
        }
    }
}

impl From<MemberError> for KeyError {
    fn from(e: MemberError) -> Self {
        KeyError::Database {
            message: e.to_string(),
        }
    }
}
