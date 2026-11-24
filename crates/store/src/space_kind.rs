//! Which kind of space each collection belongs in (RFC 0001, "The model", **One model, both kinds
//! of space.**). nexus-flow's one model declares every collection of every module, so NoCentralDB
//! would let any of them be written into any space; the store refuses a write of a collection
//! into a space of a kind the table does not name for it. The module RFCs say which kind each
//! collection belongs in: tickets, decisions, status notes and the move's runs and records in the
//! repository space (RFC 0002, RFC 0003, RFC 0007, RFC 0008), goals and requests in the coordination space (RFC 0005, RFC 0009), and
//! in an ad-hoc space only copies of requests (RFC 0009, "Members and roles"). `spaceMember`,
//! the frame's own, goes into every space nexus-flow adds members to through `nxs member add`:
//! the repository and the coordination space.
//!
//! A space whose kind the store does not know, such as one opened by id without a checkout that
//! names it, takes no write of a collection that is restricted to some kinds.

use std::fmt;

use crate::model::Module;

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

impl SpaceKind {
    /// Every kind of space.
    pub const ALL: [SpaceKind; 3] = [
        SpaceKind::Repository,
        SpaceKind::Coordination,
        SpaceKind::AdHoc,
    ];

    fn name(self) -> &'static str {
        match self {
            SpaceKind::Repository => "repository",
            SpaceKind::Coordination => "coordination",
            SpaceKind::AdHoc => "ad-hoc",
        }
    }
}

/// Every collection of the model, with the kinds of space it may be written into.
pub const KINDS: &[(&str, &[SpaceKind])] = &[
    (
        "spaceMember",
        &[SpaceKind::Repository, SpaceKind::Coordination],
    ),
    ("ticketsTicket", &[SpaceKind::Repository]),
    ("ticketsNote", &[SpaceKind::Repository]),
    ("ticketsWork", &[SpaceKind::Repository]),
    ("ticketsReference", &[SpaceKind::Repository]),
    ("decisionsDecision", &[SpaceKind::Repository]),
    ("decisionsTopic", &[SpaceKind::Repository]),
    ("decisionsSubject", &[SpaceKind::Repository]),
    ("decisionsSupersession", &[SpaceKind::Repository]),
    ("goalsGoal", &[SpaceKind::Coordination]),
    ("migrationRun", &[SpaceKind::Repository]),
    ("migrationRecord", &[SpaceKind::Repository]),
];

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongSpace {
    /// The collection the write was for.
    pub collection: String,
    /// The kind of the space written into; `None` when the store does not know it.
    pub space: Option<SpaceKind>,
    /// The kinds the collection belongs in; empty for a collection the table does not name.
    pub belongs_in: Vec<SpaceKind>,
}

impl WrongSpace {
    /// The error code a module RFC names for this refusal: `not_a_ticket_space` for a ticket
    /// collection (RFC 0002) and `not_a_repository_space` for a status collection (RFC 0007),
    /// each written into a space known to be of another kind. `None` where no RFC names one.
    pub fn code(&self) -> Option<&'static str> {
        self.space?;
        if !self.belongs_in.contains(&SpaceKind::Repository) {
            return None;
        }
        match module_of(&self.collection)? {
            Module::Tickets => Some("not_a_ticket_space"),
            Module::Status => Some("not_a_repository_space"),
            _ => None,
        }
    }
}

fn module_of(collection: &str) -> Option<Module> {
    Module::ALL
        .into_iter()
        .find(|&module| crate::model::is_collection_name(collection, module))
}

impl fmt::Display for WrongSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let collection = &self.collection;
        if self.belongs_in.is_empty() {
            return write!(
                f,
                "{collection} is not a collection of nexus-flow's model, so it is written into no space"
            );
        }
        let kinds: Vec<String> = self
            .belongs_in
            .iter()
            .map(|kind| format!("a {} space", kind.name()))
            .collect();
        let kinds = kinds.join(" or ");
        match self.space {
            Some(space) => write!(
                f,
                "{collection} belongs in {kinds}, not in a {} space",
                space.name()
            ),
            None => write!(
                f,
                "{collection} belongs in {kinds}, and the kind of this space is not known: it was opened by id without a checkout whose nexus-flow.toml names it"
            ),
        }
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
    let Some(&(_, kinds)) = table.iter().find(|(name, _)| *name == collection) else {
        return Err(WrongSpace {
            collection: collection.to_owned(),
            space,
            belongs_in: Vec::new(),
        });
    };
    let allowed = match space {
        Some(space) => kinds.contains(&space),
        None => SpaceKind::ALL.iter().all(|kind| kinds.contains(kind)),
    };
    if allowed {
        Ok(())
    } else {
        Err(WrongSpace {
            collection: collection.to_owned(),
            space,
            belongs_in: kinds.to_vec(),
        })
    }
}
