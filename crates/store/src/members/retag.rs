//! Giving a member a new ticket tag (RFC 0002, "Two members with one tag"): `nxs member retag
//! <member> <tag>` writes `tag` into the member's record in every open space it is a member of,
//! after the checks `nxs member add` makes of a named tag, so a refusal changes nothing. Only the
//! records change: the keys already written keep the tag they were written with.
//!
//! A member without a record, such as the device that created a space before `nxs init` wrote
//! one, gets one holding only its member and tag.

use super::taken::Taken;
use super::{listed, parse_tag, record, spaces_of, Member, MemberError};
use crate::opened::Opened;

/// Gives `member` the tag `tag`, and returns it as the first space it is a member of lists it. A
/// member that already holds `tag` is returned unchanged.
pub async fn retag(opened: &Opened, member: &str, tag: &str) -> Result<Member, MemberError> {
    let tag = parse_tag(tag)?;
    let (id, spaces) = spaces_of(opened, member).await?;
    let (_, first) = spaces[0];
    let before = listed(opened.db(), first, id).await?;
    if before.tag.as_deref() == Some(tag.as_str()) {
        return Ok(before);
    }
    Taken::read(opened.db(), spaces.iter().map(|(_, space)| *space))
        .await?
        .check(tag)?;
    for (space_id, space) in &spaces {
        record::set_tag(opened, space_id, space, id, &tag).await?;
    }
    listed(opened.db(), first, id).await
}
