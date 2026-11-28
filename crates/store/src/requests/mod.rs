//! The requests module's store half (RFC 0009): for now the coordination space itself and the
//! repository records in it ("The coordination space", "Repositories in the space").
//!
//! **Creating.** [`create`] makes a coordination space on the member's database, with the marker
//! that names its kind, writes the record of the checkout's repository into it and names the space
//! under `[space] coordination` in `nexus-flow.toml`. A file that already names one is refused
//! before anything is created, since a repository belongs to at most one.
//!
//! **Joining.** [`join`] writes the record of the checkout's repository into a coordination space
//! the database holds, or clears `left` on the records whose `space` is this repository's, and
//! names the space in the file. `--name` renames those records; a name another repository's record
//! holds is refused.
//!
//! **Leaving.** [`leave`] sets `left` on the repository's records and takes the space out of the
//! file. A record is never deleted: requests to and from it link to it.
//!
//! Every change is checked first (the file's edit, the name, whether the space is held), then the
//! record is written, then the file, so a refusal writes nothing, and running a create's or a
//! join's command again finishes one that failed half-way.
//!
//! **Reading.** [`list`] gives the records of the open coordination space in name order, and
//! [`named`] the one record of a name, refusing a name two records hold as ambiguous.

use std::fmt;

use crate::coordination::JoinError;
use crate::id::SpaceId;
use crate::open::InitError;
use crate::write::WriteError;

mod coordination;
mod name;
mod repositories;

pub use coordination::{create, join, leave};
pub use repositories::{list, named};

/// A repository of the group, as its record holds it (RFC 0009, "Repositories in the space").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Its short name, such as `central-infra`.
    pub name: String,
    /// Its repository space's id, as its `nexus-flow.toml` names it.
    pub space: String,
    /// Whether it has left the group.
    pub left: bool,
}

/// What a create, a join or a leave did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordination {
    /// The coordination space created, joined or left.
    pub coordination: SpaceId,
    /// Whether this run wrote anything: the record or `nexus-flow.toml`.
    pub written: bool,
    /// The checkout's repository as its record holds it now; `None` after leaving a space that
    /// held no record of it.
    pub repository: Option<Repository>,
}

/// Why a coordination space was not created, joined, left or read. A refusal writes nothing.
#[derive(Debug)]
pub enum CoordinationError {
    /// `nexus-flow.toml` cannot take the change, or could not be read or written.
    File(JoinError),
    /// The database is open on a space without a checkout, so it has no `nexus-flow.toml`.
    NoCheckout,
    /// `nexus-flow.toml` names no coordination space, and the database has none open.
    NotJoined,
    /// The database holds none of the coordination space yet: it has to sync first.
    NotHeld { space: SpaceId },
    /// No name was given, and the git remote `origin` gives none.
    NoName,
    /// The text is not a repository name.
    NotAName { text: String },
    /// Another repository's record holds the name.
    NameTaken {
        name: String,
        holders: Vec<Repository>,
    },
    /// No record holds the name.
    NotFound { name: String },
    /// More than one record holds the name.
    Ambiguous {
        name: String,
        matching: Vec<Repository>,
    },
    /// The space could not be opened or created.
    Open(InitError),
    /// The record was refused, or could not be written.
    Write(WriteError),
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for CoordinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoordinationError::File(e) => e.fmt(f),
            CoordinationError::NoCheckout => f.write_str(
                "the database is open on a space without a checkout, so there is no nexus-flow.toml to name the coordination space in",
            ),
            CoordinationError::NotJoined => f.write_str(
                "nexus-flow.toml names no coordination space: create one with nxs space create-coordination, or join one with nxs space join-coordination <space id>",
            ),
            CoordinationError::NotHeld { space } => write!(
                f,
                "this database holds none of the coordination space {space} yet: once an admin has added this member, sync it with nxs sync, then run the command again"
            ),
            CoordinationError::NoName => f.write_str(
                "the git remote origin gives no repository name: name the repository with --name",
            ),
            CoordinationError::NotAName { text } => write!(
                f,
                "{text:?} is not a repository name: one is not empty and holds no space or control character"
            ),
            CoordinationError::NameTaken { name, holders } => write!(
                f,
                "the name {name} is held by the repository of the space {}: give this repository another one with --name",
                spaces(holders)
            ),
            CoordinationError::NotFound { name } => {
                write!(f, "no repository of the coordination space is named {name}")
            }
            CoordinationError::Ambiguous { name, matching } => write!(
                f,
                "the name {name} is held by the repositories of the spaces {}: rename one with nxs space join-coordination <space id> --name <new name>",
                spaces(matching)
            ),
            CoordinationError::Open(e) => e.fmt(f),
            CoordinationError::Write(e) => e.fmt(f),
            CoordinationError::Database { message } => f.write_str(message),
        }
    }
}

fn spaces(repositories: &[Repository]) -> String {
    let spaces: Vec<&str> = repositories.iter().map(|r| r.space.as_str()).collect();
    spaces.join(", ")
}

impl std::error::Error for CoordinationError {}

impl From<JoinError> for CoordinationError {
    fn from(e: JoinError) -> Self {
        CoordinationError::File(e)
    }
}

impl From<InitError> for CoordinationError {
    fn from(e: InitError) -> Self {
        CoordinationError::Open(e)
    }
}

impl From<WriteError> for CoordinationError {
    fn from(e: WriteError) -> Self {
        CoordinationError::Write(e)
    }
}

impl From<nocentraldb::Error> for CoordinationError {
    fn from(e: nocentraldb::Error) -> Self {
        CoordinationError::Database {
            message: e.to_string(),
        }
    }
}
