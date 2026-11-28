//! Reading a ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam",
//! "Who holds a ticket"): by key or by object id, with its parent's key, its children in key
//! order and its assignee as a holder; an unknown ticket is `key_not_found`.

use super::create::{create, new};
use crate::members::tests::{block_on, Repo, DEVICE_TAG};
use crate::tickets::keys::KeyError;
use crate::tickets::{self, Holder, NewTicket, TicketError};

#[test]
fn show_reads_the_ticket_create_wrote_by_key_or_object_id() {
    let repo = Repo::new("ticket-show");
    let opened = repo.opened();
    let created = create(
        &opened,
        NewTicket {
            priority: Some("P1".to_owned()),
            labels: vec!["rfc".to_owned()],
            ..new("bug", "A bug")
        },
    );

    let by_key = block_on(tickets::show(&opened, &created.key)).unwrap();
    let by_id = block_on(tickets::show(&opened, &created.id)).unwrap();

    assert_eq!(by_key, created);
    assert_eq!(by_id, created);
}

#[test]
fn an_unknown_ticket_is_key_not_found() {
    let repo = Repo::new("ticket-show-unknown");
    let opened = repo.opened();

    let refused = block_on(tickets::show(&opened, &format!("{DEVICE_TAG}.4")));

    assert_eq!(
        refused,
        Err(TicketError::Key(KeyError::KeyNotFound {
            key: format!("{DEVICE_TAG}.4")
        }))
    );
}

#[test]
fn an_epic_lists_its_children_in_key_order() {
    let repo = Repo::new("ticket-show-children");
    let opened = repo.opened();
    let epic = create(&opened, new("epic", "The epic"));
    let mut keys = Vec::new();
    for n in 0..10 {
        let child = NewTicket {
            parent: Some(epic.key.clone()),
            ..new("chore", &format!("Child {n}"))
        };
        keys.push(create(&opened, child).key);
    }

    let shown = block_on(tickets::show(&opened, &epic.key)).unwrap();

    assert_eq!(keys.last().unwrap(), &format!("{DEVICE_TAG}.11"));
    assert_eq!(shown.children, keys);
}

#[test]
fn an_assignee_is_a_member_or_a_person_with_its_role() {
    let repo = Repo::new("ticket-show-assignee");
    let opened = repo.opened();
    let device = repo.list().remove(0);
    let holder = |assignee: String| {
        let new = NewTicket {
            assignee: Some(assignee),
            ..new("feature", "Held")
        };
        create(&opened, new).assignee.expect("an assignee")
    };
    let holds = |kind: &str, id: &str, role: Option<&str>| Holder {
        kind: kind.to_owned(),
        id: id.to_owned(),
        role: role.map(str::to_owned),
    };

    assert_eq!(
        holder(format!("member:{}", device.member)),
        holds("member", &device.member, Some("owner"))
    );
    assert_eq!(
        holder(format!("account:{}", device.account)),
        holds("person", &device.account, Some("owner"))
    );
    assert_eq!(
        holder("account:someone-else".to_owned()),
        holds("person", "someone-else", None)
    );
    assert_eq!(
        holder("member:gone".to_owned()),
        holds("member", "gone", None)
    );
    assert_eq!(create(&opened, new("feature", "Free")).assignee, None);
}
