//! The move from v1 (RFC 0008): a run of `nxs migrate` is one `migrationRun` object, and each v1
//! op or decision file it moved is one `migrationRecord` object. Only the move writes either
//! collection ("only `nxs migrate` writes them"), through its own write path,
//! [`Opened::write_as_move`](crate::opened::Opened::write_as_move); every other write that names
//! one is refused ([`MOVE_ONLY`]). A record is the record of what happened, so the store's
//! write-once guard (`crate::write_once`) refuses every change to an existing one, the move's
//! included ([`WRITE_ONCE`]). A run is not write-once: its `state`, `last`, `endedAt` and
//! `report` change as it goes on.

use std::fmt;

/// The collections only the move writes.
pub const MOVE_ONLY: &[&str] = &["migrationRun", "migrationRecord"];

/// The move's write-once collections: its records.
pub const WRITE_ONCE: &[&str] = &["migrationRecord"];

/// Why a write was refused: its collection is written only by the move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveOnly {
    /// The collection the write was for.
    pub collection: String,
}

impl fmt::Display for MoveOnly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is written only by nxs migrate, the move from v1, so this write is refused",
            self.collection
        )
    }
}

impl std::error::Error for MoveOnly {}

/// Whether a write of `collection` may be made, by the move when `by_move`.
pub fn check(collection: &str, by_move: bool) -> Result<(), MoveOnly> {
    if by_move || !MOVE_ONLY.contains(&collection) {
        Ok(())
    } else {
        Err(MoveOnly {
            collection: collection.to_owned(),
        })
    }
}
