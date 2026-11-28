//! Minting a key (RFC 0002, "Ticket ids", "The number"): [`mint`] gives the key of the next ticket the member the database runs as
//! creates in a space: its own tag, and the highest number among the keys with that tag in its
//! copy, plus one; 1 when its copy holds none. NoCentralDB queries an index only with its whole
//! equality prefix fixed, so `byKey` cannot list the keys of one tag: the keys are read through
//! `byUpdatedAt`, which fixes nothing and so holds every ticket of the space.
//!
//! **A freed tag.** A tag passes on once its member is removed (NEX-73), and the guard on that
//! sits at minting (NEX-236): a member whose tag a removed member's record carries mints only
//! once its copy of the space has completed a sync since the database opened, which shows it
//! every key the tag's earlier holders created.

use nocentraldb::{MemberId, Space};

use super::{Key, KeyError, TICKETS};
use crate::id::SpaceId;
use crate::members::{self, record, tags::Tag};
use crate::opened::Opened;

/// The index that holds every ticket of a space.
const EVERY_TICKET: &str = "byUpdatedAt";

/// The key of the next ticket the database's member creates in the open space `space`. A freed
/// tag is refused until the copy has synced.
pub async fn mint(opened: &Opened, space: &SpaceId) -> Result<Key, KeyError> {
    let held = held(opened, space)?;
    let synced = held.status().current().initial_sync_complete;
    mint_synced(opened, space, synced).await
}

/// [`mint`], with whether the copy of `space` has completed a sync given.
pub(crate) async fn mint_synced(
    opened: &Opened,
    space: &SpaceId,
    synced: bool,
) -> Result<Key, KeyError> {
    let held = held(opened, space)?;
    let me = me(held).await?;
    let tag = record::read(opened.db(), held, me)
        .await?
        .and_then(|r| r.tag)
        .and_then(|t| t.parse::<Tag>().ok())
        .ok_or(KeyError::NoTag { space: *space })?;
    if !synced && freed(opened, held, me, tag).await? {
        return Err(KeyError::NotSynced {
            space: *space,
            tag: tag.to_string(),
        });
    }
    let highest = every_key(opened, space)
        .await?
        .into_iter()
        .filter(|key| key.tag == tag)
        .map(|key| key.number)
        .max()
        .unwrap_or(0);
    Ok(Key {
        tag,
        number: highest + 1,
    })
}

/// What the database holds of the open space `space`.
fn held<'a>(opened: &'a Opened, space: &SpaceId) -> Result<&'a Space, KeyError> {
    match opened.spaces().iter().find(|(id, _)| id == space) {
        Some((_, Some(held))) => Ok(held),
        _ => Err(KeyError::NotHeld { space: *space }),
    }
}

/// The member the database runs as, in `space`.
async fn me(space: &Space) -> Result<MemberId, KeyError> {
    space
        .members()
        .list()
        .await?
        .into_iter()
        .find(|m| m.is_self)
        .map(|m| m.id)
        .ok_or(KeyError::NotHeld {
            space: SpaceId(space.id()),
        })
}

/// Whether a record of a member other than `me` that is no longer a member of `space` carries
/// `tag`.
async fn freed(opened: &Opened, space: &Space, me: MemberId, tag: Tag) -> Result<bool, KeyError> {
    let me = me.to_string();
    let current: Vec<String> = members::ids(space)
        .await?
        .iter()
        .map(MemberId::to_string)
        .collect();
    Ok(record::tags(opened.db(), space)
        .await?
        .into_iter()
        .any(|(member, held)| held == Some(tag) && member != me && !current.contains(&member)))
}

/// The key of every ticket in the open space `space` that holds one; a text that is no key is
/// left out.
async fn every_key(opened: &Opened, space: &SpaceId) -> Result<Vec<Key>, KeyError> {
    let tickets = opened
        .find(space, TICKETS, EVERY_TICKET, Vec::new())
        .await?;
    let mut keys = Vec::new();
    for ticket in &tickets {
        if let Some(key) = ticket.get::<&str>("key")?.and_then(|k| k.parse().ok()) {
            keys.push(key);
        }
    }
    Ok(keys)
}
