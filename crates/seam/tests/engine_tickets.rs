//! `Engine::ticket_create` and `Engine::ticket` (RFC 0002, "Commands, JSON and the seam"): the
//! seam's counterparts of `nxs ticket create` and `nxs ticket show` create a ticket with a minted
//! key and read it back by key or object id; a refusal is a ticket error and writes nothing.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{Engine, Error, KeyError, NewTicket, TicketError};
use nexus_flow_store::space::init;

fn engine(test: &str) -> (std::path::PathBuf, Engine) {
    let base = scratch(test);
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();
    (base, engine)
}

fn new(kind: &str, title: &str) -> NewTicket {
    NewTicket {
        kind: kind.to_owned(),
        title: title.to_owned(),
        description: "Why it exists".to_owned(),
        ..NewTicket::default()
    }
}

#[test]
fn an_engine_creates_a_ticket_and_reads_it_back() {
    let (base, engine) = engine("engine-ticket-create");
    let epic = block_on(engine.ticket_create(new("epic", "The epic"))).unwrap();

    let created = block_on(engine.ticket_create(NewTicket {
        parent: Some(epic.key.clone()),
        priority: Some("P2".to_owned()),
        ..new("feature", "A feature")
    }))
    .unwrap();

    assert!(created.key.ends_with(".2"), "{}", created.key);
    assert_eq!(created.priority, Some(2));
    assert_eq!(created.parent.as_ref(), Some(&epic.key));
    assert_eq!(block_on(engine.ticket(&created.key)).unwrap(), created);
    assert_eq!(block_on(engine.ticket(&created.id)).unwrap(), created);
    assert_eq!(
        block_on(engine.ticket(&epic.key)).unwrap().children,
        [created.key]
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_refused_ticket_is_a_ticket_error() {
    let (base, engine) = engine("engine-ticket-refused");

    let refused = block_on(engine.ticket_create(new("story", "Refused"))).unwrap_err();
    let unknown = block_on(engine.ticket("zz.1")).unwrap_err();

    assert!(matches!(
        refused,
        Error::Ticket(TicketError::NotAType { .. })
    ));
    assert!(matches!(
        unknown,
        Error::Ticket(TicketError::Key(KeyError::KeyNotFound { .. }))
    ));
    fs::remove_dir_all(&base).unwrap();
}
