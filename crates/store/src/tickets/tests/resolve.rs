//! Resolving a key or an object id (RFC 0002, "Ticket ids", "Two members with one tag").

use nocentraldb::{ObjectId, Value};

use crate::id::SpaceId;
use crate::members::tests::{block_on, Repo};
use crate::opened::Opened;
use crate::tickets::keys::{self, KeyError, TICKETS};

/// Writes a ticket with `key` into the repository space, and returns its object id.
fn ticket(opened: &Opened, space: &SpaceId, key: &str) -> ObjectId {
    let written = block_on(opened.write(space, |w| {
        w.create(
            TICKETS,
            [
                ("key", Value::from(key)),
                ("type", Value::from("feature")),
                ("title", Value::from("A ticket")),
                ("status", Value::from("open")),
            ],
        )
        .map(drop)
    }))
    .unwrap();
    written.ids[0]
}

#[test]
fn an_unknown_key_fails_with_key_not_found() {
    let repo = Repo::new("key-resolve-unknown");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];
    ticket(&opened, space, "ka.1");

    let refused = block_on(keys::resolve(&opened, space, "ka.2"));

    assert_eq!(
        refused,
        Err(KeyError::KeyNotFound {
            key: "ka.2".to_owned()
        })
    );
}
