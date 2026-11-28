//! The record of the device that creates a space (RFC 0001, "The role record"; RFC 0002, "Ticket
//! ids"; RFC 0009, "Request ids"; decided by the owner, NEX-240): `init` writes it in the
//! repository space, and `create-coordination` in the coordination space, with role `owner` and a
//! free tag, so that device can mint keys at once. In a coordination space it takes the tag the
//! device holds in the repository space, as `nxs member add` gives a member one tag in every
//! space. Only the creation of the space writes it, so no run writes a second record.

use super::record;
use super::taken::Taken;
use super::MemberError;
use crate::id::SpaceId;
use crate::opened::Opened;

/// The role of the device that creates a space.
pub const OWNER: &str = "owner";

/// Writes the `owner` record of this database's member in the open space `id`, which this member
/// has just created. Its tag is the one its record in the repository space carries, when that
/// space is another one and the record has a tag, else a free tag picked by its id.
pub(crate) async fn write(opened: &Opened, id: &SpaceId) -> Result<(), MemberError> {
    let Some((_, Some(space))) = opened.spaces().iter().find(|(open, _)| open == id) else {
        return Err(MemberError::Database {
            message: "the space just created is not held".to_owned(),
        });
    };
    let me = space
        .members()
        .list()
        .await?
        .into_iter()
        .find(|m| m.is_self)
        .ok_or_else(|| MemberError::Database {
            message: "the device is not a member of the space it created".to_owned(),
        })?
        .id;
    let own = match opened.spaces().first() {
        Some((repository, Some(held))) if repository != id => record::read(opened.db(), held, me)
            .await?
            .and_then(|r| r.tag),
        _ => None,
    };
    let taken = Taken::read(opened.db(), [space]).await?;
    let tag = taken.choose(None, own.as_deref(), me)?;
    record::write(opened, id, space, me, OWNER, None, Some(&tag)).await
}
