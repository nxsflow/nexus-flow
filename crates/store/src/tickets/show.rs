//! Reading one ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam").

use super::{Ticket, TicketError};
use crate::opened::Opened;

/// The ticket of the repository space of `opened` that `text` names, by key or object id.
pub async fn show(_opened: &Opened, _text: &str) -> Result<Ticket, TicketError> {
    unimplemented!("NEX-110: show")
}
