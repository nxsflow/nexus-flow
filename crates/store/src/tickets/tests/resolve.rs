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

/// The first `n` hex digits of `id`.
fn prefix(id: ObjectId, n: usize) -> String {
    id.to_string()[..n].to_owned()
}

#[test]
fn two_tickets_with_one_key_both_list_with_a_suffix() {
    let repo = Repo::new("key-shown-shared");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];
    let first = ticket(&opened, space, "ka.42");
    let second = ticket(&opened, space, "ka.42");
    let alone = ticket(&opened, space, "ka.43");

    let shown = keys::shown(&[
        ("ka.42".to_owned(), first),
        ("ka.42".to_owned(), second),
        ("ka.43".to_owned(), alone),
    ]);

    let suffix = |id: ObjectId| {
        let hex = id.to_string();
        let other = if id == first { second } else { first }.to_string();
        let n = (4..=32)
            .find(|&n| hex[..n] != other[..n])
            .expect("two object ids differ");
        format!("ka.42~{}", prefix(id, n))
    };
    assert_eq!(
        shown,
        vec![suffix(first), suffix(second), "ka.43".to_owned()]
    );
}

#[test]
fn a_bare_key_two_tickets_hold_fails_with_ambiguous_key_listing_both() {
    let repo = Repo::new("key-resolve-ambiguous");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];
    let first = ticket(&opened, space, "ka.42");
    let second = ticket(&opened, space, "ka.42");

    let refused = block_on(keys::resolve(&opened, space, "ka.42"));

    let mut both = keys::shown(&[("ka.42".to_owned(), first), ("ka.42".to_owned(), second)]);
    both.sort();
    assert_eq!(
        refused,
        Err(KeyError::AmbiguousKey {
            key: "ka.42".to_owned(),
            tickets: both.clone()
        })
    );
    for (shown, id) in both.iter().zip({
        let mut ids = [first, second];
        ids.sort_by_key(|id| id.to_string());
        ids
    }) {
        assert_eq!(block_on(keys::resolve(&opened, space, shown)), Ok(id));
    }
}

#[test]
fn an_object_id_resolves_to_its_ticket() {
    let repo = Repo::new("key-resolve-id");
    let opened = repo.opened();
    let (space, _) = &opened.spaces()[0];
    let id = ticket(&opened, space, "ka.7");

    let resolved = block_on(keys::resolve(&opened, space, &id.to_string()));

    assert_eq!(resolved, Ok(id));
}
