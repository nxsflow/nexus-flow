//! The tickets module on the seam (RFC 0002, "Commands, JSON and the seam"): an open engine
//! creates a ticket and reads one back, the counterparts of `nxs ticket create` and
//! `nxs ticket show`, with the derived state computed in the store.

use nexus_flow_store::tickets::{NewTicket, Ticket};

use crate::engine::{Engine, Error};

impl Engine {
    /// Creates `ticket` in the repository space, after the checks of RFC 0002, with a key minted
    /// for it, as one change; returns it as [`Engine::ticket`] reads it. A refusal names its
    /// problem and writes nothing.
    pub async fn ticket_create(&self, _ticket: NewTicket) -> Result<Ticket, Error> {
        unimplemented!("NEX-110: Engine::ticket_create")
    }

    /// The ticket of the repository space `key` names, by key or object id, as `nxs ticket show`
    /// prints it.
    pub async fn ticket(&self, _key: &str) -> Result<Ticket, Error> {
        unimplemented!("NEX-110: Engine::ticket")
    }
}
