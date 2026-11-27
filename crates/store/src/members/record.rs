//! The role record (RFC 0001, "The role record"): one `spaceMember` object per member nexus-flow
//! added to a space, with `member`, `role`, `ticket` and the member's ticket `tag` (RFC 0002,
//! "Ticket ids"), written by the member that adds it. It is found by its member through the index
//! `byMember`; every record of a space, a removed member's included, is read through `byTag`.
//!
//! A member added again after its removal gets the record it had, rewritten, so that a space
//! holds one record per member as long as no two admins add it at once offline; when they do,
//! the record the index gives first is read and rewritten.

use nocentraldb::{Db, MemberId, Object, Reference, Space, Value};

use super::tags::Tag;
use super::{MemberError, TicketRef};
use crate::id::SpaceId;
use crate::opened::Opened;

/// The collection of the role records.
pub(crate) const COLLECTION: &str = "spaceMember";

/// The index of the records by member.
const BY_MEMBER: &str = "byMember";

/// The index of every record, in tag order.
const BY_TAG: &str = "byTag";

/// The most records one page of `byTag` holds.
const PAGE: usize = 1000;

/// What a member's record holds besides its member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    /// `None` for a record only `nxs member retag` wrote, for a member nexus-flow did not add.
    pub role: Option<String>,
    pub ticket: Option<TicketRef>,
    /// The member's ticket tag as written; `None` when it has none.
    pub tag: Option<String>,
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
    let tag: Option<&str> = object.get("tag")?;
    Ok(Some(Record {
        role: role.map(str::to_owned),
        ticket: ticket.map(|r| TicketRef {
            space: SpaceId(r.space),
            object: r.id,
        }),
        tag: tag.map(str::to_owned),
    }))
}

/// The member and the tag of every record in `space`, a removed member's included, in tag order.
/// A record without a tag, or with a text that is no tag, gives `None`.
pub(crate) async fn tags(
    db: &Db,
    space: &Space,
) -> Result<Vec<(String, Option<Tag>)>, MemberError> {
    let mut found = Vec::new();
    let mut after = None;
    loop {
        let mut query = db.index(COLLECTION, BY_TAG)?.take(PAGE);
        if let Some(cursor) = after.take() {
            query = query.after(cursor);
        }
        let page = space.find(query).await?;
        for object in &page.items {
            let member: Option<&str> = object.get("member")?;
            let tag: Option<&str> = object.get("tag")?;
            found.push((
                member.unwrap_or_default().to_owned(),
                tag.and_then(|t| t.parse().ok()),
            ));
        }
        match page.cursor {
            Some(cursor) if page.has_more => after = Some(cursor),
            _ => return Ok(found),
        }
    }
}

/// Writes the record of `member` in the open space `id` with `role`, `ticket` and `tag`: a new one, or
/// the one it already has, rewritten. It goes through the store's one write path
/// ([`Opened::write`]), which checks that the collection belongs in the space's kind.
pub(crate) async fn write(
    opened: &Opened,
    id: &SpaceId,
    space: &Space,
    member: MemberId,
    role: &str,
    ticket: Option<&TicketRef>,
    tag: Option<&Tag>,
) -> Result<(), MemberError> {
    let ticket = match ticket {
        Some(t) => Value::from(Reference::new(t.space.0, t.object)),
        None => Value::clear(),
    };
    let tag = match tag {
        Some(tag) => Value::from(tag.as_str()),
        None => Value::clear(),
    };
    let values = [
        ("role", Value::from(role)),
        ("ticket", ticket),
        ("tag", tag),
    ];
    let existing = find(opened.db(), space, member).await?.map(|o| o.id());
    opened
        .write(id, |w| match existing {
            Some(object) => w.update(COLLECTION, object, values),
            None => {
                let member = Value::from(member.to_string());
                let values = std::iter::once(("member", member)).chain(values);
                w.create(COLLECTION, values).map(|_| ())
            }
        })
        .await?;
    Ok(())
}

/// Sets the tag of `member`'s record in the open space `id` to `tag`, keeping the rest of it; a
/// member without a record, such as the device that created the space, gets one with its member
/// and tag only.
pub(crate) async fn set_tag(
    opened: &Opened,
    id: &SpaceId,
    space: &Space,
    member: MemberId,
    tag: &Tag,
) -> Result<(), MemberError> {
    let existing = find(opened.db(), space, member).await?.map(|o| o.id());
    let tag = Value::from(tag.as_str());
    opened
        .write(id, |w| match existing {
            Some(object) => w.update(COLLECTION, object, [("tag", tag)]),
            None => {
                let member = Value::from(member.to_string());
                w.create(COLLECTION, [("member", member), ("tag", tag)])
                    .map(|_| ())
            }
        })
        .await?;
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
