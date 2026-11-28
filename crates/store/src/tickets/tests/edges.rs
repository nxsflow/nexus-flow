//! A new ticket's edges (RFC 0002, "What the model is generated from", "Collections"): only an
//! epic may be a parent and an epic has none, so every other type has at most one epic parent;
//! a parent and every dependency must be a ticket of the space. A refusal writes nothing.

use nocentraldb::{ObjectId, Value};

use super::create::{create, new, stored};
use crate::members::tests::{block_on, Repo, DEVICE_TAG};
use crate::opened::Opened;
use crate::tickets::keys::{self, KeyError};
use crate::tickets::{self, Edge, NewTicket, TicketError};

fn count(opened: &Opened) -> usize {
    let (space, _) = &opened.spaces()[0];
    block_on(opened.find(space, keys::TICKETS, "byUpdatedAt", Vec::new()))
        .unwrap()
        .len()
}

fn links(opened: &Opened, id: &str, field: &str) -> Vec<ObjectId> {
    stored(opened, id)
        .get::<&[Value]>(field)
        .unwrap()
        .unwrap_or_default()
        .iter()
        .map(|v| ObjectId::try_from(v).unwrap())
        .collect()
}

#[test]
fn a_ticket_belongs_to_the_epic_its_parent_names() {
    let repo = Repo::new("ticket-parent-epic");
    let opened = repo.opened();
    let epic = create(&opened, new("epic", "The epic"));

    let child = create(
        &opened,
        NewTicket {
            parent: Some(epic.key.clone()),
            ..new("feature", "The child")
        },
    );

    let parent = stored(&opened, &child.id)
        .get::<ObjectId>("parent")
        .unwrap();
    assert_eq!(parent.map(|p| p.to_string()), Some(epic.id));
    assert_eq!(child.parent, Some(epic.key));
}

#[test]
fn only_an_epic_may_be_a_parent_and_an_epic_has_none() {
    let repo = Repo::new("ticket-parent-refused");
    let opened = repo.opened();
    let epic = create(&opened, new("epic", "The epic"));
    let feature = create(&opened, new("feature", "A feature"));
    let before = count(&opened);

    let under_feature = block_on(tickets::create(
        &opened,
        NewTicket {
            parent: Some(feature.key.clone()),
            ..new("bug", "Under a feature")
        },
    ));
    let epic_under_epic = block_on(tickets::create(
        &opened,
        NewTicket {
            parent: Some(epic.key.clone()),
            ..new("epic", "Under an epic")
        },
    ));
    let unknown = block_on(tickets::create(
        &opened,
        NewTicket {
            parent: Some(format!("{DEVICE_TAG}.99")),
            ..new("bug", "Under nothing")
        },
    ));

    assert_eq!(
        under_feature,
        Err(TicketError::ParentNotEpic { key: feature.key })
    );
    assert_eq!(epic_under_epic, Err(TicketError::EpicWithParent));
    assert_eq!(
        unknown,
        Err(TicketError::Key(KeyError::KeyNotFound {
            key: format!("{DEVICE_TAG}.99")
        }))
    );
    assert_eq!(count(&opened), before, "nothing is written");
}

#[test]
fn a_ticket_depends_on_the_tickets_it_names() {
    let repo = Repo::new("ticket-depends-on");
    let opened = repo.opened();
    let first = create(&opened, new("feature", "The first"));
    let second = create(&opened, new("question", "The second"));

    let ticket = create(
        &opened,
        NewTicket {
            depends_on: vec![second.key.clone(), first.id.clone(), first.key.clone()],
            ..new("feature", "Blocked")
        },
    );

    let mut stored = links(&opened, &ticket.id, "dependsOn");
    stored.sort_by_key(ObjectId::to_string);
    let mut expected: Vec<ObjectId> = [&first.id, &second.id]
        .iter()
        .map(|id| id.parse().unwrap())
        .collect();
    expected.sort_by_key(ObjectId::to_string);
    assert_eq!(stored, expected);
    let edge = |key: &str| Edge {
        key: key.to_owned(),
        status: "open".to_owned(),
        done: None,
        on_main: None,
    };
    assert_eq!(ticket.deps, [edge(&first.key), edge(&second.key)]);
}

#[test]
fn an_unknown_dependency_is_refused_and_nothing_is_written() {
    let repo = Repo::new("ticket-depends-on-unknown");
    let opened = repo.opened();
    let first = create(&opened, new("feature", "The first"));

    let refused = block_on(tickets::create(
        &opened,
        NewTicket {
            depends_on: vec![first.key, format!("{DEVICE_TAG}.7")],
            ..new("feature", "Blocked")
        },
    ));

    assert_eq!(
        refused,
        Err(TicketError::Key(KeyError::KeyNotFound {
            key: format!("{DEVICE_TAG}.7")
        }))
    );
    assert_eq!(count(&opened), 1);
}
