//! The tickets module on the seam (RFC 0002, "Commands, JSON and the seam"): an open engine
//! creates a ticket and reads one back, the counterparts of `nxs ticket create` and
//! `nxs ticket show`, with the derived state computed in the store.

use nexus_flow_store::tickets::{self, NewTicket, Ticket};

use crate::engine::{Engine, Error};

impl Engine {
    /// Creates `ticket` in the repository space, after the checks of RFC 0002, with a key minted
    /// for it, as one change; returns it as [`Engine::ticket`] reads it. A refusal names its
    /// problem and writes nothing.
    pub async fn ticket_create(&self, ticket: NewTicket) -> Result<Ticket, Error> {
        Ok(tickets::create(&self.opened, ticket).await?)
    }

    /// The ticket of the repository space `key` names, by key or object id, as `nxs ticket show`
    /// prints it.
    pub async fn ticket(&self, key: &str) -> Result<Ticket, Error> {
        Ok(tickets::show(&self.opened, key).await?)
    }
}
