//! The store's one write path (RFC 0001, "Reading and writing"): every write of nexus-flow goes
//! through [`Opened::write`], for every member and every caller, as one change of one open space.
//! Each write a caller records is checked before NoCentralDB sees it: the collection must belong
//! in the space's kind ([`Opened::check_write`]). A refused write refuses the whole change, so
//! nothing of it is written.

use std::fmt;

use nocentraldb::{ObjectId, Transaction, Value, Written};

use crate::id::SpaceId;
use crate::opened::Opened;
use crate::space_kind::{self, SpaceKind, WrongSpace};

/// Why a write was refused or failed. A refusal writes nothing of the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The collection does not belong in the space's kind.
    WrongSpace(WrongSpace),
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

impl From<nocentraldb::Error> for WriteError {
    fn from(e: nocentraldb::Error) -> Self {
        WriteError::Database(e)
    }
}

/// The writes of one change, each checked before it is recorded.
pub struct Write<'t, 'm> {
    tx: &'t mut Transaction<'m>,
    kind: Option<SpaceKind>,
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

    /// Records a write of `values` to the existing object `id` of `collection`.
    pub fn update<'v>(
        &mut self,
        collection: &str,
        id: ObjectId,
        values: impl IntoIterator<Item = (&'v str, Value)>,
    ) -> Result<(), WriteError> {
        self.check(collection)?;
        Ok(self.tx.update(collection, id, values)?)
    }

    /// Records the deletion of the existing object `id` of `collection`.
    pub fn delete(&mut self, collection: &str, id: ObjectId) -> Result<(), WriteError> {
        self.check(collection)?;
        Ok(self.tx.delete(collection, id)?)
    }

    /// Records the restore of the deleted object `id` of `collection`.
    pub fn restore(&mut self, collection: &str, id: ObjectId) -> Result<(), WriteError> {
        self.check(collection)?;
        Ok(self.tx.restore(collection, id)?)
    }

    fn check(&self, collection: &str) -> Result<(), WriteError> {
        Ok(space_kind::check(collection, self.kind)?)
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
                let outcome = record(&mut Write { tx, kind });
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
