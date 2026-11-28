//! The store's one write path (RFC 0001, "Reading and writing"): every write of nexus-flow goes
//! through [`Opened::write`], for every member and every caller, as one change of one open space.
//! Each write a caller records is checked before NoCentralDB sees it: the collection must belong
//! in the space's kind ([`Opened::check_write`]), and a set, delete or restore of an existing object
//! must not be one of a write-once collection (`crate::write_once`), such as a decision. A
//! collection only the move from v1 writes (`crate::migration`) is written only through
//! [`Opened::write_as_move`]. A refused write refuses the whole change, so nothing of it is
//! written.

use std::fmt;

use nocentraldb::{ObjectId, Transaction, Value, Written};

use crate::id::SpaceId;
use crate::migration::{self, MoveOnly};
use crate::opened::Opened;
use crate::space_kind::{self, SpaceKind, WrongSpace};
use crate::space_marker::MarkerWrite;
use crate::write_once::{self, Change, WriteOnce};

/// Why a write was refused or failed. A refusal writes nothing of the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The collection does not belong in the space's kind.
    WrongSpace(WrongSpace),
    /// The change would rewrite, delete or restore an existing object of a write-once
    /// collection.
    WriteOnce(WriteOnce),
    /// The collection is written only by the move from v1, and the write is not the move's.
    MoveOnly(MoveOnly),
    /// The collection is the marker that names the space's kind, written only when the space is
    /// created.
    Marker(MarkerWrite),
    /// The space is not one of the open spaces.
    NotOpen(SpaceId),
    /// The space is open, but the database holds none of it yet.
    NotHeld(SpaceId),
    /// NoCentralDB refused or failed the write.
    Database(nocentraldb::Error),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::WrongSpace(e) => e.fmt(f),
            WriteError::WriteOnce(e) => e.fmt(f),
            WriteError::MoveOnly(e) => e.fmt(f),
            WriteError::Marker(e) => e.fmt(f),
            WriteError::NotOpen(space) => write!(f, "the space {space} is not open"),
            WriteError::NotHeld(space) => write!(
                f,
                "the space {space} is not held by this database yet: sync it first"
            ),
            WriteError::Database(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<WrongSpace> for WriteError {
    fn from(e: WrongSpace) -> Self {
        WriteError::WrongSpace(e)
    }
}

impl From<WriteOnce> for WriteError {
    fn from(e: WriteOnce) -> Self {
        WriteError::WriteOnce(e)
    }
}

impl From<MoveOnly> for WriteError {
    fn from(e: MoveOnly) -> Self {
        WriteError::MoveOnly(e)
    }
}

impl From<nocentraldb::Error> for WriteError {
    fn from(e: nocentraldb::Error) -> Self {
        WriteError::Database(e)
    }
}

/// The writes of one change, each checked before it is recorded.
pub struct Write<'t, 'm> {
    tx: &'t mut Transaction<'m>,
    kind: Option<SpaceKind>,
    /// Whether the change is the move's, which alone writes the move's collections.
    by_move: bool,
}

impl Write<'_, '_> {
    /// Records the creation of an object of `collection` with `values`, and returns its id.
    pub fn create<'v>(
        &mut self,
        collection: &str,
        values: impl IntoIterator<Item = (&'v str, Value)>,
    ) -> Result<ObjectId, WriteError> {
        self.check(collection)?;
        Ok(self.tx.create(collection, values)?)
    }

    /// Records a write of `values` to the existing object `id` of `collection`, which must not be
    /// write-once.
    pub fn update<'v>(
        &mut self,
        collection: &str,
        id: ObjectId,
        values: impl IntoIterator<Item = (&'v str, Value)>,
    ) -> Result<(), WriteError> {
        self.check_change(collection, Change::Set)?;
        Ok(self.tx.update(collection, id, values)?)
    }

    /// Records the deletion of the existing object `id` of `collection`, which must not be
    /// write-once.
    pub fn delete(&mut self, collection: &str, id: ObjectId) -> Result<(), WriteError> {
        self.check_change(collection, Change::Delete)?;
        Ok(self.tx.delete(collection, id)?)
    }

    /// Records the restore of the deleted object `id` of `collection`, which must not be
    /// write-once.
    pub fn restore(&mut self, collection: &str, id: ObjectId) -> Result<(), WriteError> {
        self.check_change(collection, Change::Restore)?;
        Ok(self.tx.restore(collection, id)?)
    }

    fn check(&self, collection: &str) -> Result<(), WriteError> {
        space_kind::check(collection, self.kind)?;
        Ok(migration::check(collection, self.by_move)?)
    }

    /// The checks of a change to an existing object: its collection's kind, and that it is not
    /// write-once.
    fn check_change(&self, collection: &str, change: Change) -> Result<(), WriteError> {
        self.check(collection)?;
        Ok(write_once::check(collection, change)?)
    }
}

impl Opened {
    /// Writes what `record` records into the open space `space`, as one change. When `record`
    /// returns an error, or a write it records is refused, nothing is written and the error is
    /// returned.
    pub async fn write<F>(&self, space: &SpaceId, record: F) -> Result<Written, WriteError>
    where
        F: FnOnce(&mut Write<'_, '_>) -> Result<(), WriteError>,
    {
        self.write_by(space, false, record).await
    }

    /// [`Opened::write`], as a change of the move from v1 (RFC 0008), which alone writes a
    /// `migrationRun` or a `migrationRecord`. Only `nxs migrate` calls it; every other check
    /// holds as for any write, so a record still takes no write after its create.
    pub async fn write_as_move<F>(&self, space: &SpaceId, record: F) -> Result<Written, WriteError>
    where
        F: FnOnce(&mut Write<'_, '_>) -> Result<(), WriteError>,
    {
        self.write_by(space, true, record).await
    }

    async fn write_by<F>(
        &self,
        space: &SpaceId,
        by_move: bool,
        record: F,
    ) -> Result<Written, WriteError>
    where
        F: FnOnce(&mut Write<'_, '_>) -> Result<(), WriteError>,
    {
        let Some((_, held)) = self.spaces().iter().find(|(id, _)| id == space) else {
            return Err(WriteError::NotOpen(*space));
        };
        let Some(target) = held else {
            return Err(WriteError::NotHeld(*space));
        };
        let kind = self.kind(space);
        let mut refused = None;
        let written = target
            .write(|tx| {
                let outcome = record(&mut Write { tx, kind, by_move });
                outcome.map_err(|e| {
                    refused = Some(e);
                    abort(tx)
                })
            })
            .await;
        match (refused, written) {
            (Some(e), _) => Err(e),
            (None, written) => Ok(written?),
        }
    }
}

/// An error that makes NoCentralDB write nothing of the change. The SDK offers no way to make one,
/// so it is the error of recording a create in a collection no model can name: the empty name.
fn abort(tx: &mut Transaction<'_>) -> nocentraldb::Error {
    tx.create("", std::iter::empty())
        .expect_err("no collection is named \"\"")
}
