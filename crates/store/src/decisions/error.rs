//! Why a decision was not written (RFC 0003, "Writing a decision"): every refusal names its
//! problem, and nothing of the decision is written.

use std::fmt;

use crate::id::SpaceId;
use crate::space_kind::WrongSpace;
use crate::write::WriteError;

/// Why a decision was refused, or could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionError {
    /// A required field is empty: `title`, `why`, `instead of` or, when given, `decided by`.
    Missing { field: &'static str },
    /// The title holds a line break.
    TitleNotOneLine,
    /// The title is longer than 200 characters.
    TitleTooLong { chars: usize },
    /// No topic is given.
    NoTopic,
    /// A topic is not a lowercase tag.
    NotATopic { text: String },
    /// The date is not a calendar date written `YYYY-MM-DD`.
    NotADate { text: String },
    /// A ticket label is neither a ticket id nor a range of them.
    NotATicket { text: String },
    /// The pull request's URL is not an `https` URL.
    NotAnHttpsUrl { text: String },
    /// The slug given is not one.
    NotASlug { text: String },
    /// The title has no word to make a slug of, and no slug is given.
    NoSlug,
    /// No decision of that handle or object id is in the local copy.
    SupersedesUnknown { key: String },
    /// The handle names several decisions, listed by their object ids.
    SupersedesAmbiguous { handle: String, ids: Vec<String> },
    /// The decision names its own handle among those it supersedes.
    SupersedesItself,
    /// The decision it would supersede was made after it, on `date`.
    SupersedesLater { key: String, date: String },
    /// Who decided is not given, and the writer's role record does not say it: the member has
    /// none, or the ticket it names cannot be read.
    NoRole,
    /// The repository space is not held by this database yet.
    NotHeld { space: SpaceId },
    /// Decisions may not be written into this space.
    WrongSpace(WrongSpace),
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for DecisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecisionError::Missing { field } => write!(f, "a decision needs a {field}"),
            DecisionError::TitleNotOneLine => {
                f.write_str("the title is more than one line: state the decision in one line")
            }
            DecisionError::TitleTooLong { chars } => write!(
                f,
                "the title has {chars} characters: state the decision in at most 200"
            ),
            DecisionError::NoTopic => f.write_str("a decision needs at least one topic"),
            DecisionError::NotATopic { text } => write!(
                f,
                "{text:?} is not a topic: one is a lowercase tag of letters, digits and -, \
                 starting with a letter or a digit"
            ),
            DecisionError::NotADate { text } => {
                write!(f, "{text:?} is not a calendar date written YYYY-MM-DD")
            }
            DecisionError::NotATicket { text } => write!(
                f,
                "{text:?} is not a ticket label: one is a ticket id like NEX-22, or a range like \
                 NEX-6 to NEX-13"
            ),
            DecisionError::NotAnHttpsUrl { text } => {
                write!(f, "{text:?} is not an https URL of a pull request")
            }
            DecisionError::NotASlug { text } => write!(
                f,
                "{text:?} is not a slug: one is lowercase letters and digits in words joined by \
                 single -"
            ),
            DecisionError::NoSlug => f.write_str(
                "the title has no letter or digit to make the handle's slug of: give a slug",
            ),
            DecisionError::SupersedesUnknown { key } => {
                write!(f, "no decision {key} is in the local copy to supersede")
            }
            DecisionError::SupersedesAmbiguous { handle, ids } => write!(
                f,
                "the handle {handle} names {} decisions; name the one to supersede by its \
                 object id: {}",
                ids.len(),
                ids.join(", ")
            ),
            DecisionError::SupersedesItself => f.write_str("a decision cannot supersede itself"),
            DecisionError::SupersedesLater { key, date } => write!(
                f,
                "{key} was decided on {date}, after this decision: a decision supersedes only \
                 decisions made on or before its own date"
            ),
            DecisionError::NoRole => {
                f.write_str("this member's role record does not say who decided: name who decided")
            }
            DecisionError::NotHeld { space } => write!(
                f,
                "this device does not hold the space {space} yet; sync it with nxs sync first"
            ),
            DecisionError::WrongSpace(e) => e.fmt(f),
            DecisionError::Database { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for DecisionError {}

impl From<WriteError> for DecisionError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::WrongSpace(e) => DecisionError::WrongSpace(e),
            WriteError::NotOpen(space) | WriteError::NotHeld(space) => {
                DecisionError::NotHeld { space }
            }
            e => DecisionError::Database {
                message: e.to_string(),
            },
        }
    }
}

impl From<nocentraldb::Error> for DecisionError {
    fn from(e: nocentraldb::Error) -> Self {
        DecisionError::Database {
            message: e.to_string(),
        }
    }
}
