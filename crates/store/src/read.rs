//! Reading one object of an open space (RFC 0001, "Reading and writing").

use nocentraldb::{Object, ObjectId};

use crate::id::SpaceId;
use crate::opened::Opened;

impl Opened {
    /// The object `id` of `collection` in the open space `space`, deleted or not; `None` when the
    /// database holds no such object, or none of the space.
    pub async fn get(
        &self,
        space: &SpaceId,
        collection: &str,
        id: ObjectId,
    ) -> Result<Option<Object>, nocentraldb::Error> {
        match self.spaces().iter().find(|(open, _)| open == space) {
            Some((_, Some(held))) => held.get(collection, id).await,
            _ => Ok(None),
        }
    }
}
