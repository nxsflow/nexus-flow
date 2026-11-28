//! Why a ticket could not be created or read.

use std::fmt;

use super::keys::KeyError;
use crate::id::SpaceId;
use crate::members::MemberError;
use crate::write::WriteError;

/// Why a ticket could not be created or read. A refusal writes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketError {
    /// A required field is missing or blank.
    Missing { field: &'static str },
    /// The title holds a line break.
    TitleNotOneLine,
    /// The type is none of the six.
    NotAType { text: String },
    /// The priority is neither `P0` to `P4` nor `0` to `4`.
    NotAPriority { text: String },
    /// A time is not RFC 3339.
    NotATime { field: &'static str, text: String },
    /// The assignee has neither the prefix `member:` nor `account:`, or nothing after it.
    NotAnIdentity { text: String },
    /// A label is blank.
    NotALabel { text: String },
    /// The parent is not an epic: only an epic may be a parent.
    ParentNotEpic { key: String },
    /// An epic was given a parent: an epic has none.
    EpicWithParent,
    /// The repository space is not open, or not held yet.
    NotHeld { space: SpaceId },
    /// The space is of a kind that holds no tickets, such as a coordination space.
    NotATicketSpace { space: SpaceId },
    /// A key could not be minted or resolved.
    Key(KeyError),
    /// The store refused the write, or the database failed.
    Database { message: String },
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TicketError::Missing { field } => write!(f, "the ticket's {field} is missing"),
            TicketError::TitleNotOneLine => f.write_str("the ticket's title must be one line"),
            TicketError::NotAType { text } => write!(
                f,
                "{text:?} is no ticket type; one of epic, bug, feature, chore, decision, question"
            ),
            TicketError::NotAPriority { text } => {
                write!(f, "{text:?} is no priority; one of P0 to P4, or 0 to 4")
            }
            TicketError::NotATime { field, text } => write!(
                f,
                "the {field} {text:?} is no RFC 3339 time, such as 2026-12-01 or 2026-12-01T09:00:00Z"
            ),
            TicketError::NotAnIdentity { text } => write!(
                f,
                "the assignee {text:?} is no identity; member:<member id> or account:<account fingerprint>"
            ),
            TicketError::NotALabel { text } => write!(f, "the label {text:?} is blank"),
            TicketError::ParentNotEpic { key } => write!(
                f,
                "{key} is no epic, and only an epic may be a parent"
            ),
            TicketError::EpicWithParent => {
                f.write_str("an epic has no parent: tickets have two levels, epics and the rest")
            }
            TicketError::NotHeld { space } => write!(
                f,
                "this database does not hold the space {space} yet; sync it with nxs sync first"
            ),
            TicketError::NotATicketSpace { space } => write!(
                f,
                "the space {space} holds no tickets; tickets live in a repository space"
            ),
            TicketError::Key(e) => e.fmt(f),
            TicketError::Database { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for TicketError {}

impl From<KeyError> for TicketError {
    fn from(e: KeyError) -> Self {
        TicketError::Key(e)
    }
}

impl From<nocentraldb::Error> for TicketError {
    fn from(e: nocentraldb::Error) -> Self {
        TicketError::Database {
            message: e.to_string(),
        }
    }
}

impl From<MemberError> for TicketError {
    fn from(e: MemberError) -> Self {
        TicketError::Database {
            message: e.to_string(),
        }
    }
}

impl From<WriteError> for TicketError {
    fn from(e: WriteError) -> Self {
        TicketError::Database {
            message: e.to_string(),
        }
    }
}
