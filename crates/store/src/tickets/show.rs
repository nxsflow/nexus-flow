//! Reading one ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam").
//! Every field the record has is filled from the stored ticket; the derived state, the reverse
//! edges, notes, work links and references read `None` or empty until the tickets that compute
//! and write them land.

use std::cmp::Ordering;

use nocentraldb::{Object, ObjectId, Value};

use super::create::repository;
use super::keys::{self, shown, Key, BY_KEY, TICKETS};
use super::{Derived, Edge, Holder, Ticket, TicketError};
use crate::members::{self, OWNER};
use crate::id::SpaceId;
use crate::opened::Opened;

/// The ticket of the repository space of `opened` that `text` names, by key or object id.
pub async fn show(opened: &Opened, text: &str) -> Result<Ticket, TicketError> {
    let (space, _) = repository(opened)?;
    let id = keys::resolve(opened, &space, text).await?;
    read(opened, &space, id).await
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
    let parent = match object.get::<ObjectId>("parent")? {
        Some(parent) => linked(opened, space, parent).await?.map(|(key, _)| key),
        None => None,
    };
    let mut deps = Vec::new();
    for dep in links(&object, "dependsOn")? {
        if let Some((key, status)) = linked(opened, space, dep).await? {
            deps.push(Edge {
                key,
                status,
                done: None,
                on_main: None,
            });
        }
    }
    deps.sort_by(|a, b| by_key(&a.key, &b.key));
    let mut children = Vec::new();
    for child in opened
        .find(space, TICKETS, BY_PARENT, vec![Value::from(id)])
        .await?
    {
        children.push(shown_key(opened, space, &child).await?);
    }
    children.sort_by(|a, b| by_key(a, b));
    let assignee = match text(&object, "assignee")? {
        Some(assignee) => Some(holder(opened, &assignee).await?),
        None => None,
    };
    let key = shown_key(opened, space, &object).await?;
    let text = |field| text(&object, field);
    Ok(Ticket {
        id: id.to_string(),
        key,
        kind: text("type")?.unwrap_or_default(),
        title: text("title")?.unwrap_or_default(),
        status: text("status")?.unwrap_or_default(),
        resolution: text("resolution")?,
        priority: object.get::<i64>("priority")?,
        assignee,
        parent,
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
        deps,
        dependents: Vec::new(),
        children,
        contributes_to: Vec::new(),
        mentions: Vec::new(),
        mentioned_by: Vec::new(),
        notes: Vec::new(),
        work: Vec::new(),
        references: Vec::new(),
    })
}

/// The index of the tickets by their parent epic.
const BY_PARENT: &str = "byParent";

/// The holder `assignee` names (RFC 0002, "Who holds a ticket"): a member with the role its
/// record gives it, a person, `owner` when a member of the person's account holds the role
/// `owner`, or text as the move from v1 wrote it.
async fn holder(opened: &Opened, assignee: &str) -> Result<Holder, TicketError> {
    let members = members::list(opened).await?;
    let (kind, id, role) = if let Some(id) = assignee.strip_prefix("member:") {
        let role = members
            .iter()
            .find(|m| m.member == id)
            .and_then(|m| m.role.clone());
        ("member", id, role)
    } else if let Some(id) = assignee.strip_prefix("account:") {
        let owner = members
            .iter()
            .any(|m| m.account == id && m.role.as_deref() == Some(OWNER));
        ("person", id, owner.then(|| OWNER.to_owned()))
    } else {
        ("text", assignee, None)
    };
    Ok(Holder {
        kind: kind.to_owned(),
        id: id.to_owned(),
        role,
    })
}

/// The key, as `nxs` shows it, and the status of the ticket `id` a link names; `None` when the
/// local copy does not hold it.
async fn linked(
    opened: &Opened,
    space: &SpaceId,
    id: ObjectId,
) -> Result<Option<(String, String)>, TicketError> {
    let Some(object) = opened.get(space, TICKETS, id).await? else {
        return Ok(None);
    };
    let key = shown_key(opened, space, &object).await?;
    Ok(Some((key, text(&object, "status")?.unwrap_or_default())))
}

/// The key of the ticket `object` as `nxs` shows it: with `~` and the first digits of its object
/// id when another ticket of the space holds the same key.
async fn shown_key(
    opened: &Opened,
    space: &SpaceId,
    object: &Object,
) -> Result<String, TicketError> {
    let key = text(object, "key")?.unwrap_or_default();
    let mut holders: Vec<(String, ObjectId)> = opened
        .find(space, TICKETS, BY_KEY, vec![Value::from(key.as_str())])
        .await?
        .iter()
        .map(|t| (key.clone(), t.id()))
        .collect();
    if !holders.iter().any(|(_, id)| *id == object.id()) {
        holders.push((key.clone(), object.id()));
    }
    let at = holders
        .iter()
        .position(|(_, id)| *id == object.id())
        .expect("the ticket is among the holders");
    Ok(shown(&holders).swap_remove(at))
}

/// Keys in key order: by tag, then by number, then as text; a text that is no key after every
/// key.
pub(super) fn by_key(a: &str, b: &str) -> Ordering {
    let parse = |text: &str| text.split('~').next().and_then(|k| k.parse::<Key>().ok());
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.cmp(b)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// The object ids of the many links `field`.
fn links(object: &Object, field: &str) -> Result<Vec<ObjectId>, TicketError> {
    let mut list = Vec::new();
    for value in object.get::<&[Value]>(field)?.unwrap_or_default() {
        list.push(ObjectId::try_from(value)?);
    }
    Ok(list)
}

fn text(object: &Object, field: &str) -> Result<Option<String>, TicketError> {
    Ok(object.get::<&str>(field)?.map(str::to_owned))
}

/// The elements of the set `field`, in byte order.
fn strings(object: &Object, field: &str) -> Result<Vec<String>, TicketError> {
    let mut list = Vec::new();
    for value in object.get::<&[Value]>(field)?.unwrap_or_default() {
        list.push(<&str>::try_from(value)?.to_owned());
    }
    list.sort();
    Ok(list)
}
