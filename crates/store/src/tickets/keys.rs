//! A ticket's key (RFC 0002, "Ticket ids"): the creating member's tag, a dot, and a number that
//! counts up per tag, such as `ka.42`.
//!
//! **Minting.** [`mint`] gives the key of the next ticket the member the database runs as
//! creates in a space: its own tag, and the highest number among the keys with that tag in its
//! copy, plus one; 1 when its copy holds none. NoCentralDB queries an index only with its whole
//! equality prefix fixed, so `byKey` cannot list the keys of one tag: the keys are read through
//! `byUpdatedAt`, which fixes nothing and so holds every ticket of the space.
//!
//! **A freed tag.** A tag passes on once its member is removed (NEX-73), and the guard on that
//! sits at minting (NEX-236): a member whose tag a removed member's record carries mints only
//! once its copy of the space has completed a sync since the database opened, which shows it
//! every key the tag's earlier holders created.

use std::fmt;
use std::str::FromStr;

use nocentraldb::{MemberId, Space};

use crate::id::SpaceId;
use crate::members::{self, record, tags::Tag, MemberError};
use crate::opened::Opened;

/// The collection of the tickets.
pub(crate) const TICKETS: &str = "ticketsTicket";

/// The index that holds every ticket of a space.
const EVERY_TICKET: &str = "byUpdatedAt";

/// A ticket's key, such as `ka.42`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub tag: Tag,
    /// 1 or more.
    pub number: u64,
}

/// A text that is no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAKey;

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.tag, self.number)
    }
}

impl FromStr for Key {
    type Err = NotAKey;

    fn from_str(text: &str) -> Result<Self, NotAKey> {
        let (tag, number) = text.split_once('.').ok_or(NotAKey)?;
        let tag: Tag = tag.parse().map_err(|_| NotAKey)?;
        let canonical = !number.is_empty()
            && !number.starts_with('0')
            && number.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return Err(NotAKey);
        }
        let number = number.parse().map_err(|_| NotAKey)?;
        Ok(Key { tag, number })
    }
}

/// Why a key could not be minted or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The space is not one of the open spaces, or the database holds none of it yet.
    NotHeld { space: SpaceId },
    /// The member has no tag in the space, so it cannot mint a key there.
    NoTag { space: SpaceId },
    /// The member's tag was another member's before, and the member's copy has not completed a
    /// sync: a key the earlier holder created may not have reached it yet.
    NotSynced { space: SpaceId, tag: String },
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::NotHeld { space } => write!(
                f,
                "this database does not hold the space {space} yet; sync it with nxs sync first"
            ),
            KeyError::NoTag { space } => write!(
                f,
                "this member has no tag in the space {space}; nxs member retag gives it one"
            ),
            KeyError::NotSynced { space, tag } => write!(
                f,
                "the tag {tag} was another member's before, and this copy of the space {space} has not synced yet; sync with nxs sync first"
            ),
            KeyError::Database { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<nocentraldb::Error> for KeyError {
    fn from(e: nocentraldb::Error) -> Self {
        KeyError::Database {
            message: e.to_string(),
        }
    }
}

impl From<MemberError> for KeyError {
    fn from(e: MemberError) -> Self {
        KeyError::Database {
            message: e.to_string(),
        }
    }
}

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
    let tickets = opened.find(space, TICKETS, EVERY_TICKET, Vec::new()).await?;
    let mut keys = Vec::new();
    for ticket in &tickets {
        if let Some(key) = ticket.get::<&str>("key")?.and_then(|k| k.parse().ok()) {
            keys.push(key);
        }
    }
    Ok(keys)
}
