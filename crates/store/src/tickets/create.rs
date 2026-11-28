//! Creating a ticket (RFC 0002, "The ticket", "Who created a ticket", "Commands, JSON and the
//! seam").

use super::{NewTicket, Ticket, TicketError};
use crate::opened::Opened;

/// Checks `new` and writes it into the repository space of `opened`, as one change, with a key
/// minted for it; returns it as [`show`](super::show) reads it.
pub async fn create(_opened: &Opened, _new: NewTicket) -> Result<Ticket, TicketError> {
    unimplemented!("NEX-110: create")
}
