//! Reading the objects of an open space (RFC 0001, "Reading and writing"): one by its id, or
//! those an index finds.

use nocentraldb::{Object, ObjectId, Value};

use crate::id::SpaceId;
use crate::opened::Opened;

/// The page size of a read through an index, NoCentralDB's largest.
const PAGE: usize = 1000;

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

    /// Every object of `collection` in the open space `space` that the index `index` finds with
    /// its equality prefix fixed to `equal`, in index order; none when the database holds none
    /// of the space.
    pub async fn find(
        &self,
        space: &SpaceId,
        collection: &str,
        index: &str,
        equal: Vec<Value>,
    ) -> Result<Vec<Object>, nocentraldb::Error> {
        let Some((_, Some(held))) = self.spaces().iter().find(|(open, _)| open == space) else {
            return Ok(Vec::new());
        };
        let query = self.db().index(collection, index)?.equal(equal).take(PAGE);
        let mut page = held.find(query.clone()).await?;
        let mut objects = std::mem::take(&mut page.items);
        while let (true, Some(cursor)) = (page.has_more, page.cursor.take()) {
            page = held.find(query.clone().after(cursor)).await?;
            objects.append(&mut page.items);
        }
        Ok(objects)
    }
}
