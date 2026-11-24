//! The role record (RFC 0001, "The role record"): one `spaceMember` object per member nexus-flow
//! added to a space, with `member`, `role` and `ticket`, written by the member that adds it. It
//! is found by its member through the index `byMember`.
//!
//! A member added again after its removal gets the record it had, rewritten, so that a space
//! holds one record per member as long as no two admins add it at once offline; when they do,
//! the record the index gives first is read and rewritten.

use nocentraldb::{Db, MemberId, Object, Reference, Space, Value};

use super::{MemberError, TicketRef};
use crate::id::SpaceId;

/// The collection of the role records.
pub(crate) const COLLECTION: &str = "spaceMember";

/// The index of the records by member.
const BY_MEMBER: &str = "byMember";

/// What a member's record holds besides its member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub role: String,
    pub ticket: Option<TicketRef>,
}

/// The record of `member` in `space`; `None` when it has none.
pub(crate) async fn read(
    db: &Db,
    space: &Space,
    member: MemberId,
) -> Result<Option<Record>, MemberError> {
    let Some(object) = find(db, space, member).await? else {
        return Ok(None);
    };
    let role: Option<&str> = object.get("role")?;
    let ticket: Option<Reference> = object.get("ticket")?;
    Ok(Some(Record {
        role: role.unwrap_or_default().to_owned(),
        ticket: ticket.map(|r| TicketRef {
            space: SpaceId(r.space),
            object: r.id,
        }),
    }))
}

/// Writes the record of `member` in `space` with `role` and `ticket`: a new one, or the one it
/// already has, rewritten.
pub(crate) async fn write(
    db: &Db,
    space: &Space,
    member: MemberId,
    role: &str,
    ticket: Option<&TicketRef>,
) -> Result<(), MemberError> {
    let ticket = match ticket {
        Some(t) => Value::from(Reference::new(t.space.0, t.object)),
        None => Value::clear(),
    };
    let values = [("role", Value::from(role)), ("ticket", ticket)];
    match find(db, space, member).await? {
        Some(object) => {
            space.update(COLLECTION, object.id(), values).await?;
        }
        None => {
            let member = Value::from(member.to_string());
            let values = std::iter::once(("member", member)).chain(values);
            space.create(COLLECTION, values).await?;
        }
    }
    Ok(())
}

/// The first record of `member` in `space`, by the index.
async fn find(db: &Db, space: &Space, member: MemberId) -> Result<Option<Object>, MemberError> {
    let query = db
        .index(COLLECTION, BY_MEMBER)?
        .equal([member.to_string()])
        .take(1);
    let page = space.find(query).await?;
    Ok(page.items.into_iter().next())
}
