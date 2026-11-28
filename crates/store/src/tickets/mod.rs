//! The tickets module's part of the store (RFC 0002): a ticket's key, minted for a new ticket
//! and resolved from an argument ([`keys`]); creating a ticket ([`create`]) and reading it as
//! `nxs ticket show` prints it ([`show`]).

pub mod check;
mod create;
mod error;
pub mod keys;
mod show;
pub mod time;
mod types;

pub use create::create;
pub use error::TicketError;
pub use show::show;
pub use types::{
    Derived, Edge, FocusGoal, Holder, NewTicket, Note, Reference, Ticket, TicketRequest, WorkLink,
};

#[cfg(test)]
mod tests;
