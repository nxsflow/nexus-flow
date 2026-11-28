//! The checks of a new ticket that need no database (RFC 0002, "What the model is generated
//! from", "The ticket", "Who holds a ticket"): its type is one of six, its title one line, its
//! description given, its priority `P0` to `P4` or the ordinal `0` to `4`, its times RFC 3339,
//! its assignee an identity and its labels not blank.

use super::time::is_time;
use super::{NewTicket, TicketError};

/// The ticket types, in the `next` order's type order.
pub const TYPES: [&str; 6] = ["epic", "bug", "question", "decision", "feature", "chore"];

/// The type of an epic, the only type that may be a parent.
pub const EPIC: &str = "epic";

/// A new ticket after the checks that need no database; its priority as the ordinal.
pub(super) struct Checked {
    pub new: NewTicket,
    pub priority: Option<i64>,
}

pub(super) fn check(new: NewTicket) -> Result<Checked, TicketError> {
    if new.kind.trim().is_empty() {
        return Err(TicketError::Missing { field: "type" });
    }
    if !TYPES.contains(&new.kind.as_str()) {
        return Err(TicketError::NotAType { text: new.kind });
    }
    if new.title.trim().is_empty() {
        return Err(TicketError::Missing { field: "title" });
    }
    if new.title.contains(['\n', '\r']) {
        return Err(TicketError::TitleNotOneLine);
    }
    if new.description.trim().is_empty() {
        return Err(TicketError::Missing {
            field: "description",
        });
    }
    let priority = new.priority.as_deref().map(priority).transpose()?;
    for (field, time) in [("due", &new.due), ("defer", &new.defer_until)] {
        if let Some(text) = time.as_deref().filter(|t| !is_time(t)) {
            return Err(TicketError::NotATime {
                field,
                text: text.to_owned(),
            });
        }
    }
    if let Some(text) = new.assignee.as_deref().filter(|a| !is_identity(a)) {
        return Err(TicketError::NotAnIdentity {
            text: text.to_owned(),
        });
    }
    if let Some(text) = new.labels.iter().find(|l| l.trim().is_empty()) {
        return Err(TicketError::NotALabel { text: text.clone() });
    }
    Ok(Checked { new, priority })
}

/// The ordinal of `P0` to `P4` or `0` to `4`.
pub fn priority(text: &str) -> Result<i64, TicketError> {
    let digit = text.strip_prefix('P').unwrap_or(text);
    match digit.as_bytes() {
        [d @ b'0'..=b'4'] => Ok(i64::from(d - b'0')),
        _ => Err(TicketError::NotAPriority {
            text: text.to_owned(),
        }),
    }
}

/// `member:` or `account:`, then an id on one line: a member id, or an account fingerprint,
/// which is written in blocks of five digits with spaces between.
fn is_identity(text: &str) -> bool {
    ["member:", "account:"].iter().any(|prefix| {
        text.strip_prefix(prefix)
            .is_some_and(|id| !id.trim().is_empty() && !id.contains(['\n', '\r']))
    })
}
