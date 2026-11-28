//! Creating a ticket (RFC 0002, "The ticket", "Who created a ticket"): one change writes the
//! ticket with its minted key, the fields the writer gives, `createdBy` and `updatedBy` as the
//! member the database runs as, and `createdAt` and `updatedAt` as the same RFC 3339 instant.

use nocentraldb::{Object, Value};

use crate::members::tests::{block_on, Repo, DEVICE_TAG};
use crate::opened::Opened;
use crate::tickets::{self, keys, NewTicket, Ticket, TicketError};

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

#[test]
fn a_priority_is_stored_as_its_ordinal_from_the_label_or_the_number() {
    let repo = Repo::new("ticket-create-priority");
    let opened = repo.opened();

    for (given, ordinal) in [("P2", 2), ("2", 2), ("P0", 0), ("4", 4)] {
        let new = NewTicket {
            priority: Some(given.to_owned()),
            ..new("feature", given)
        };
        let ticket = create(&opened, new);
        let object = stored(&opened, &ticket.id);
        assert_eq!(object.get::<i64>("priority").unwrap(), Some(ordinal), "{given}");
        assert_eq!(ticket.priority, Some(ordinal), "{given}");
    }
    let unset = create(&opened, new("feature", "No priority"));
    assert_eq!(unset.priority, None);
}

/// The tickets of the repository space, deleted or not, as the local copy holds them.
fn count(opened: &Opened) -> usize {
    let (space, _) = &opened.spaces()[0];
    block_on(opened.find(space, keys::TICKETS, "byUpdatedAt", Vec::new()))
        .unwrap()
        .len()
}

#[test]
fn each_field_a_writer_gets_wrong_is_refused_and_nothing_is_written() {
    let repo = Repo::new("ticket-create-refused");
    let opened = repo.opened();
    let cases = [
        (new("", "A ticket"), TicketError::Missing { field: "type" }),
        (
            new("story", "A ticket"),
            TicketError::NotAType {
                text: "story".to_owned(),
            },
        ),
        (new("feature", " "), TicketError::Missing { field: "title" }),
        (new("feature", "Two\nlines"), TicketError::TitleNotOneLine),
        (
            NewTicket {
                description: "\n".to_owned(),
                ..new("feature", "A ticket")
            },
            TicketError::Missing {
                field: "description",
            },
        ),
        (
            NewTicket {
                priority: Some("P5".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotAPriority {
                text: "P5".to_owned(),
            },
        ),
        (
            NewTicket {
                priority: Some("high".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotAPriority {
                text: "high".to_owned(),
            },
        ),
        (
            NewTicket {
                due: Some("tomorrow".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotATime {
                field: "due",
                text: "tomorrow".to_owned(),
            },
        ),
        (
            NewTicket {
                defer_until: Some("2026-13-01".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotATime {
                field: "defer",
                text: "2026-13-01".to_owned(),
            },
        ),
        (
            NewTicket {
                assignee: Some("Carsten".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotAnIdentity {
                text: "Carsten".to_owned(),
            },
        ),
        (
            NewTicket {
                assignee: Some("member:".to_owned()),
                ..new("feature", "A ticket")
            },
            TicketError::NotAnIdentity {
                text: "member:".to_owned(),
            },
        ),
        (
            NewTicket {
                labels: vec!["rfc".to_owned(), " ".to_owned()],
                ..new("feature", "A ticket")
            },
            TicketError::NotALabel {
                text: " ".to_owned(),
            },
        ),
    ];

    for (new, expected) in cases {
        let refused = block_on(tickets::create(&opened, new));
        assert_eq!(refused, Err(expected));
    }
    assert_eq!(count(&opened), 0, "nothing is written");
}
