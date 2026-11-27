//! The tags in use in the spaces a member is added to (RFC 0002, "Ticket ids"), and the tag the
//! new member gets.
//!
//! A tag is **held** while a current member's record carries it, and **free** otherwise, a tag
//! only removed members' records carry included: RFC 0002 lets a freed tag pass on, and the guard
//! on that sits at minting (NEX-236). A member added again gets its own tag back when no current
//! member holds it, since the keys with it are its own.

use std::collections::BTreeSet;

use nocentraldb::{Db, MemberId, Space};

use super::tags::{self, Tag};
use super::{ids, record, MemberError};

/// The tags current members hold in one or more spaces.
#[derive(Debug, Default)]
pub(crate) struct Taken {
    held: BTreeSet<Tag>,
}

impl Taken {
    /// The tags in use in each of `spaces`.
    pub(crate) async fn read<'a>(
        db: &Db,
        spaces: impl IntoIterator<Item = &'a Space>,
    ) -> Result<Taken, MemberError> {
        let mut taken = Taken::default();
        for space in spaces {
            let current: BTreeSet<String> =
                ids(space).await?.iter().map(MemberId::to_string).collect();
            for (member, tag) in record::tags(db, space).await? {
                if let (Some(tag), true) = (tag, current.contains(&member)) {
                    taken.held.insert(tag);
                }
            }
        }
        Ok(taken)
    }

    /// Refuses `tag` when a current member holds it.
    pub(crate) fn check(&self, tag: Tag) -> Result<(), MemberError> {
        if self.held.contains(&tag) {
            return Err(MemberError::TagHeld {
                tag: tag.to_string(),
            });
        }
        Ok(())
    }

    /// Refuses when current members hold every tag.
    pub(crate) fn any_free(&self) -> Result<(), MemberError> {
        if self.held.len() >= tags::all().count() {
            return Err(MemberError::NoFreeTag);
        }
        Ok(())
    }

    /// The tag `member` gets: `named` when the caller named one, else its own earlier tag `own`
    /// when no current member holds it, else a free tag picked by its id.
    pub(crate) fn choose(
        &self,
        named: Option<Tag>,
        own: Option<&str>,
        member: MemberId,
    ) -> Result<Tag, MemberError> {
        if let Some(tag) = named {
            return Ok(tag);
        }
        if let Some(own) = own.and_then(|t| t.parse::<Tag>().ok()) {
            if !self.held.contains(&own) {
                return Ok(own);
            }
        }
        tags::pick(&self.held, &seed(member)).ok_or(MemberError::NoFreeTag)
    }
}

/// Eight bytes of `member`'s id, which is random, to spread the picks of inviters that pick at
/// the same time.
fn seed(member: MemberId) -> [u8; 8] {
    let hex = member.to_string();
    let value = u64::from_str_radix(&hex[..16], 16).unwrap_or_default();
    value.to_le_bytes()
}
