//! The `requestsRepository` records of a coordination space: read through the index `byName`,
//! every record in name order, and written through the store's one write path.

use nocentraldb::{Object, ObjectId, Value};

use super::{CoordinationError, Repository};
use crate::id::SpaceId;
use crate::opened::Opened;
use crate::space_kind::SpaceKind;

/// The records' collection.
pub(crate) const COLLECTION: &str = "requestsRepository";

/// The index that lists every record in name order.
const BY_NAME: &str = "byName";

/// A record with its object id.
pub(crate) struct Record {
    pub(crate) id: ObjectId,
    pub(crate) repository: Repository,
}

/// Every record of the coordination space `space`, in name order.
pub(crate) async fn all(
    opened: &Opened,
    space: &SpaceId,
) -> Result<Vec<Record>, CoordinationError> {
    let objects = opened.find(space, COLLECTION, BY_NAME, Vec::new()).await?;
    objects.iter().map(record).collect()
}

fn record(object: &Object) -> Result<Record, CoordinationError> {
    Ok(Record {
        id: object.id(),
        repository: Repository {
            name: object.get::<&str>("name")?.unwrap_or_default().to_owned(),
            space: object.get::<&str>("space")?.unwrap_or_default().to_owned(),
            left: object.get::<bool>("left")?.unwrap_or(false),
        },
    })
}

/// The open coordination space: the one `nexus-flow.toml` names, or the space opened by id.
fn coordination(opened: &Opened) -> Result<SpaceId, CoordinationError> {
    opened
        .spaces()
        .iter()
        .map(|(id, _)| *id)
        .find(|id| opened.kind(id) == Some(SpaceKind::Coordination))
        .ok_or(CoordinationError::NotJoined)
}

/// The repositories of the open coordination space, in name order, as the local copy holds them.
pub async fn list(opened: &Opened) -> Result<Vec<Repository>, CoordinationError> {
    let space = coordination(opened)?;
    let records = all(opened, &space).await?;
    Ok(records.into_iter().map(|r| r.repository).collect())
}

/// The one repository of the open coordination space named `name`. A name two records hold is
/// [`CoordinationError::Ambiguous`], with both, until one is renamed.
pub async fn named(opened: &Opened, name: &str) -> Result<Repository, CoordinationError> {
    let mut matching: Vec<Repository> = list(opened)
        .await?
        .into_iter()
        .filter(|r| r.name == name)
        .collect();
    match matching.len() {
        0 => Err(CoordinationError::NotFound {
            name: name.to_owned(),
        }),
        1 => Ok(matching.remove(0)),
        _ => Err(CoordinationError::Ambiguous {
            name: name.to_owned(),
            matching,
        }),
    }
}

/// Gives the repository of the space `own` the record `name`, not left, in the coordination space
/// `space`: a new record when none has its space, otherwise each of its records renamed and with
/// `left` cleared where they differ. Returns whether it wrote anything.
pub(crate) async fn upsert(
    opened: &Opened,
    space: &SpaceId,
    records: &[&Record],
    own: &str,
    name: &str,
) -> Result<bool, CoordinationError> {
    if records.is_empty() {
        opened
            .write(space, |w| {
                w.create(
                    COLLECTION,
                    [("name", Value::from(name)), ("space", Value::from(own))],
                )
                .map(drop)
            })
            .await?;
        return Ok(true);
    }
    let changed: Vec<&&Record> = records
        .iter()
        .filter(|r| r.repository.name != name || r.repository.left)
        .collect();
    if changed.is_empty() {
        return Ok(false);
    }
    opened
        .write(space, |w| {
            for record in &changed {
                let mut values = vec![("name", Value::from(name))];
                if record.repository.left {
                    values.push(("left", Value::clear()));
                }
                w.update(COLLECTION, record.id, values)?;
            }
            Ok(())
        })
        .await?;
    Ok(true)
}

/// Sets `left` on each of `records` that has not left, in the coordination space `space`.
pub(crate) async fn set_left(
    opened: &Opened,
    space: &SpaceId,
    records: &[&Record],
) -> Result<bool, CoordinationError> {
    let staying: Vec<&&Record> = records.iter().filter(|r| !r.repository.left).collect();
    if staying.is_empty() {
        return Ok(false);
    }
    opened
        .write(space, |w| {
            for record in &staying {
                w.update(COLLECTION, record.id, [("left", Value::from(true))])?;
            }
            Ok(())
        })
        .await?;
    Ok(true)
}
