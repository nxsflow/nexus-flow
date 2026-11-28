//! Creating a ticket (RFC 0002, "The ticket", "Who created a ticket", "Commands, JSON and the
//! seam"): `create` mints the ticket's key and writes the ticket through the store's one write
//! path, as one change of the repository space, with `createdBy` and `updatedBy` the member the
//! database runs as (the session's member when `NXS_SESSION` names one, else the device's), and
//! `createdAt` and `updatedAt` the same instant. It returns the ticket as `show` reads it.

use nocentraldb::{MemberId, Space, Value};

use super::check::{check, Checked};
use super::keys::{self, TICKETS};
use super::show::read;
use super::time;
use super::{NewTicket, Ticket, TicketError};
use crate::id::SpaceId;
use crate::opened::Opened;

/// Checks `new` and writes it into the repository space of `opened`, as one change, with a key
/// minted for it; returns it as [`show`](super::show) reads it.
pub async fn create(opened: &Opened, new: NewTicket) -> Result<Ticket, TicketError> {
    let Checked { new, priority } = check(new)?;
    let (space_id, space) = repository(opened)?;
    let key = keys::mint(opened, &space_id).await?.to_string();
    let me = me(space).await?.to_string();
    let now = time::now();
    let mut ticket = None;
    opened
        .write(&space_id, |w| {
            let mut values = vec![
                ("key", Value::from(key.as_str())),
                ("type", Value::from(new.kind.as_str())),
                ("title", Value::from(new.title.as_str())),
                ("description", Value::from(new.description.as_str())),
                ("status", Value::from("open")),
                ("createdAt", Value::from(now.as_str())),
                ("updatedAt", Value::from(now.as_str())),
                ("createdBy", Value::from(me.as_str())),
                ("updatedBy", Value::from(me.as_str())),
            ];
            let optional = [
                ("design", &new.design),
                ("completionCriterion", &new.completion_criterion),
                ("due", &new.due),
                ("deferUntil", &new.defer_until),
                ("assignee", &new.assignee),
            ];
            for (field, value) in optional {
                if let Some(value) = value {
                    values.push((field, Value::from(value.as_str())));
                }
            }
            if let Some(priority) = priority {
                values.push(("priority", Value::from(priority)));
            }
            if !new.labels.is_empty() {
                values.push(("labels", Value::add(new.labels.iter().map(String::as_str))));
            }
            ticket = Some(w.create(TICKETS, values)?);
            Ok(())
        })
        .await?;
    let id = ticket.expect("the ticket is created in the change");
    read(opened, &space_id, id).await
}

/// The repository space: the first open space, which must be held.
pub(super) fn repository(opened: &Opened) -> Result<(SpaceId, &Space), TicketError> {
    match opened.spaces().first() {
        Some((id, Some(space))) => Ok((*id, space)),
        Some((id, None)) => Err(TicketError::NotHeld { space: *id }),
        None => Err(TicketError::Database {
            message: "no repository space is open".to_owned(),
        }),
    }
}

/// The member the database runs as, in `space`.
async fn me(space: &Space) -> Result<MemberId, TicketError> {
    space
        .members()
        .list()
        .await?
        .into_iter()
        .find(|m| m.is_self)
        .map(|m| m.id)
        .ok_or_else(|| TicketError::Database {
            message: "this database's member is not a member of the repository space".to_owned(),
        })
}
