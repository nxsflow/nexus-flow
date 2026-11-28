//! Creating a ticket (RFC 0002, "The ticket", "Who created a ticket"): one change writes the
//! ticket with its minted key, the fields the writer gives, `createdBy` and `updatedBy` as the
//! member the database runs as, and `createdAt` and `updatedAt` as the same RFC 3339 instant.

use nocentraldb::{Object, Value};

use crate::members::tests::{block_on, Repo, DEVICE_TAG};
use crate::opened::Opened;
use crate::tickets::{self, keys, NewTicket, Ticket};

/// A ticket with everything `nxs ticket create` requires.
pub(super) fn new(kind: &str, title: &str) -> NewTicket {
    NewTicket {
        kind: kind.to_owned(),
        title: title.to_owned(),
        description: "Why it exists".to_owned(),
        ..NewTicket::default()
    }
}

/// Creates `new` in the repository space.
pub(super) fn create(opened: &Opened, new: NewTicket) -> Ticket {
    block_on(tickets::create(opened, new)).unwrap()
}

/// The stored object of the ticket `id`.
pub(super) fn stored(opened: &Opened, id: &str) -> Object {
    let (space, _) = &opened.spaces()[0];
    block_on(opened.get(space, keys::TICKETS, id.parse().unwrap()))
        .unwrap()
        .expect("the ticket is stored")
}

fn text(object: &Object, field: &str) -> Option<String> {
    object.get::<&str>(field).unwrap().map(str::to_owned)
}

fn labels(object: &Object) -> Vec<String> {
    let mut labels: Vec<String> = object
        .get::<&[Value]>("labels")
        .unwrap()
        .unwrap_or_default()
        .iter()
        .map(|v| <&str>::try_from(v).unwrap().to_owned())
        .collect();
    labels.sort();
    labels
}

/// Whether `text` is an RFC 3339 instant in UTC to the second, such as `2026-11-28T09:00:00Z`.
fn is_instant(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z'
        && [0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
            .into_iter()
            .all(|r| b[r].iter().all(u8::is_ascii_digit))
}

#[test]
fn create_writes_the_ticket_its_key_its_author_and_its_times_in_one_change() {
    let repo = Repo::new("ticket-create");
    let opened = repo.opened();
    let me = repo.list().remove(0).member;
    let new = NewTicket {
        design: Some("The path".to_owned()),
        completion_criterion: Some("It holds".to_owned()),
        due: Some("2026-12-01".to_owned()),
        defer_until: Some("2026-11-30T09:00:00Z".to_owned()),
        labels: vec!["rfc".to_owned(), "cli".to_owned()],
        assignee: Some(format!("member:{me}")),
        ..new("feature", "A ticket")
    };

    let ticket = create(&opened, new);

    assert_eq!(ticket.key, format!("{DEVICE_TAG}.1"));
    let object = stored(&opened, &ticket.id);
    let field = |name| text(&object, name);
    assert_eq!(field("key"), Some(format!("{DEVICE_TAG}.1")));
    assert_eq!(field("type").as_deref(), Some("feature"));
    assert_eq!(field("title").as_deref(), Some("A ticket"));
    assert_eq!(field("description").as_deref(), Some("Why it exists"));
    assert_eq!(field("design").as_deref(), Some("The path"));
    assert_eq!(field("completionCriterion").as_deref(), Some("It holds"));
    assert_eq!(field("status").as_deref(), Some("open"));
    assert_eq!(field("due").as_deref(), Some("2026-12-01"));
    assert_eq!(field("deferUntil").as_deref(), Some("2026-11-30T09:00:00Z"));
    assert_eq!(field("assignee"), Some(format!("member:{me}")));
    assert_eq!(labels(&object), ["cli", "rfc"]);
    assert_eq!(field("createdBy"), Some(me.clone()));
    assert_eq!(field("updatedBy"), Some(me));
    let created = field("createdAt").expect("createdAt is written");
    assert!(is_instant(&created), "{created}");
    assert_eq!(field("updatedAt"), Some(created.clone()));
    assert_eq!(ticket.created_at, Some(created));
    for unset in ["resolution", "closedAt", "archivedAt", "v1Id"] {
        assert_eq!(field(unset), None, "{unset}");
    }
    assert_eq!(object.get::<i64>("priority").unwrap(), None);
}

#[test]
fn the_next_ticket_counts_up() {
    let repo = Repo::new("ticket-create-next");
    let opened = repo.opened();

    create(&opened, new("feature", "The first"));
    let second = create(&opened, new("bug", "The second"));

    assert_eq!(second.key, format!("{DEVICE_TAG}.2"));
}
