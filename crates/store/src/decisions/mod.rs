//! The decisions module's part of the store (RFC 0003): writing a decision ([`add`]) with the
//! checks of "Writing a decision", its handle ([`handle`]), its ticket labels ([`labels`]) and its
//! date ([`date`]), and the write-once guard over its four collections ([`guard`]).

mod add;
pub mod date;
mod error;
pub mod guard;
pub mod handle;
pub mod labels;
mod types;

pub use add::{add, with_handle};
pub use error::DecisionError;
pub use types::{Decision, Linked, NewDecision, Subject, SubjectKind};
