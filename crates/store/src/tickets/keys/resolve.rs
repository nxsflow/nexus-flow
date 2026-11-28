//! Resolving the ticket an argument names (RFC 0002, "Ticket ids", "Two members with one tag"):
//! an object id, or a key through `byKey`. A key two tickets hold is told apart by `~` and the
//! first digits of an object id, as [`shown`] writes it; named bare, it is ambiguous.

use nocentraldb::{ObjectId, Value};

use super::{shown, KeyError, BY_KEY, TICKETS};
use crate::id::SpaceId;
use crate::opened::Opened;

/// The object id of the ticket in the open space `space` that `text` names: an object id, a key
/// found through `byKey`, or a key, `~` and the first digits of the object id of one of the
/// tickets that hold it, as [`shown`] gives it.
pub async fn resolve(opened: &Opened, space: &SpaceId, text: &str) -> Result<ObjectId, KeyError> {
    let not_found = || KeyError::KeyNotFound {
        key: text.to_owned(),
    };
    if let Ok(id) = text.parse::<ObjectId>() {
        return match opened.get(space, TICKETS, id).await? {
            Some(ticket) if !ticket.deleted() => Ok(id),
            _ => Err(not_found()),
        };
    }
    let (key, prefix) = match text.split_once('~') {
        Some((key, prefix)) if is_prefix(prefix) => (key, prefix),
        Some(_) => return Err(not_found()),
        None => (text, ""),
    };
    let mut holders: Vec<ObjectId> = opened
        .find(space, TICKETS, BY_KEY, vec![Value::from(key)])
        .await?
        .iter()
        .map(|ticket| ticket.id())
        .collect();
    holders.sort_by_key(ObjectId::to_string);
    let listed: Vec<(String, ObjectId)> = holders.iter().map(|&id| (key.to_owned(), id)).collect();
    let matching: Vec<(String, ObjectId)> = shown(&listed)
        .into_iter()
        .zip(holders)
        .filter(|(_, id)| id.to_string().starts_with(prefix))
        .collect();
    match matching.as_slice() {
        [] => Err(not_found()),
        [(_, id)] => Ok(*id),
        _ => Err(KeyError::AmbiguousKey {
            key: text.to_owned(),
            tickets: matching.into_iter().map(|(shown, _)| shown).collect(),
        }),
    }
}

/// Whether `text` can begin an object id: one or more of its lowercase hex digits.
fn is_prefix(text: &str) -> bool {
    (1..=32).contains(&text.len()) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
