//! The record of the device that creates a repository space (RFC 0001, "The role record";
//! RFC 0002, "Ticket ids"; decided by the owner, NEX-240): `init` writes it, with role `owner`
//! and a free tag, so that device can mint ticket keys at once. Only the creation of the space
//! writes it, so `init` run again writes no second record.

use super::record;
use super::taken::Taken;
use super::MemberError;
use crate::opened::Opened;

/// The role of the device that creates a space.
pub const OWNER: &str = "owner";

/// Writes the `owner` record of this database's member, with a free tag picked by its id, in
/// the repository space of `opened`, which this member has just created.
pub(crate) async fn write(opened: &Opened) -> Result<(), MemberError> {
    let Some((id, Some(space))) = opened.spaces().first() else {
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
    let taken = Taken::read(opened.db(), [space]).await?;
    let tag = taken.choose(None, None, me)?;
    record::write(opened, id, space, me, OWNER, None, Some(&tag)).await
}
