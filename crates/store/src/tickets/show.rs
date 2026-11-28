//! Reading one ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam").
//! Every field the record has is filled from the stored ticket; the derived state, the reverse
//! edges, notes, work links and references read `None` or empty until the tickets that compute
//! and write them land.

use nocentraldb::{Object, ObjectId};

use super::keys::TICKETS;
use super::{Derived, Ticket, TicketError};
use crate::id::SpaceId;
use crate::opened::Opened;

/// The ticket of the repository space of `opened` that `text` names, by key or object id.
pub async fn show(_opened: &Opened, _text: &str) -> Result<Ticket, TicketError> {
    unimplemented!("NEX-110: show")
}

/// The ticket `id` of the open space `space`, as `show` prints it.
pub(super) async fn read(
    opened: &Opened,
    space: &SpaceId,
    id: ObjectId,
) -> Result<Ticket, TicketError> {
    let object = opened
        .get(space, TICKETS, id)
        .await?
        .ok_or_else(|| TicketError::Database {
            message: format!("the ticket {id} is not in the local copy"),
        })?;
    let text = |field| text(&object, field);
    Ok(Ticket {
        id: id.to_string(),
        key: text("key")?.unwrap_or_default(),
        kind: text("type")?.unwrap_or_default(),
        title: text("title")?.unwrap_or_default(),
        status: text("status")?.unwrap_or_default(),
        resolution: text("resolution")?,
        priority: object.get::<i64>("priority")?,
        assignee: None,
        parent: None,
        labels: strings(&object, "labels")?,
        due: text("due")?,
        defer_until: text("deferUntil")?,
        created_at: text("createdAt")?,
        updated_at: text("updatedAt")?,
        closed_at: text("closedAt")?,
        archived_at: text("archivedAt")?,
        request: None,
        derived: Derived::default(),
        description: text("description")?.unwrap_or_default(),
        design: text("design")?.unwrap_or_default(),
        completion_criterion: text("completionCriterion")?.unwrap_or_default(),
        closing_comment: text("closingComment")?,
        replaced_by: None,
        replaces: Vec::new(),
        deps: Vec::new(),
        dependents: Vec::new(),
        children: Vec::new(),
        contributes_to: Vec::new(),
        mentions: Vec::new(),
        mentioned_by: Vec::new(),
        notes: Vec::new(),
        work: Vec::new(),
        references: Vec::new(),
    })
}

fn text(object: &Object, field: &str) -> Result<Option<String>, TicketError> {
    Ok(object.get::<&str>(field)?.map(str::to_owned))
}

/// The elements of the set `field`, in byte order.
fn strings(object: &Object, field: &str) -> Result<Vec<String>, TicketError> {
    let mut list = Vec::new();
    for value in object.get::<&[nocentraldb::Value]>(field)?.unwrap_or_default() {
        list.push(<&str>::try_from(value)?.to_owned());
    }
    list.sort();
    Ok(list)
}
