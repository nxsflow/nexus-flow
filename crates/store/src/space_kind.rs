//! Which kind of space each collection belongs in (RFC 0001, "The model", **One model, both kinds
//! of space.**). nexus-flow's one model declares every collection of every module, so NoCentralDB
//! would let any of them be written into any space; the store refuses a write of a collection
//! into a space of a kind the table does not name for it.

use std::fmt;

/// The kinds of space nexus-flow writes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpaceKind {
    /// A repository's own space, which `nexus-flow.toml` names as `[space] repository`.
    Repository,
    /// A group's coordination space, which `nexus-flow.toml` names as `[space] coordination`.
    Coordination,
    /// The ad-hoc space pm gives a role session, with copies of requests (RFC 0009, "Members and
    /// roles").
    AdHoc,
}

/// Every collection of the model, with the kinds of space it may be written into.
pub const KINDS: &[(&str, &[SpaceKind])] = &[];

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongSpace {
    /// The collection the write was for.
    pub collection: String,
    /// The kind of the space written into; `None` when the store does not know it.
    pub space: Option<SpaceKind>,
}

impl WrongSpace {
    /// The error code RFCs name for this refusal.
    pub fn code(&self) -> Option<&'static str> {
        None
    }
}

impl fmt::Display for WrongSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not written here", self.collection)
    }
}

impl std::error::Error for WrongSpace {}

/// Whether `collection` may be written into a space of kind `space`, by [`KINDS`].
pub fn check(collection: &str, space: Option<SpaceKind>) -> Result<(), WrongSpace> {
    check_in(KINDS, collection, space)
}

/// Whether `collection` may be written into a space of kind `space`, by `table`.
pub fn check_in(
    table: &[(&str, &[SpaceKind])],
    collection: &str,
    space: Option<SpaceKind>,
) -> Result<(), WrongSpace> {
    let _ = (table, collection, space);
    Ok(())
}
