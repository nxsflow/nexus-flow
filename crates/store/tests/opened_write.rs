//! The store's one write path (RFC 0001, "Reading and writing"): every write of every caller goes
//! through `Opened::write`, as one change of one open space, and each write it records is checked
//! first. A collection goes only into a space of a kind it belongs in (RFC 0001, "The model"),
//! and a refused write writes nothing of its change.

mod common;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::init;
use nexus_flow_store::write::WriteError;
use nocentraldb::{ObjectId, Value};

fn ticket(key: &str) -> Vec<(&'static str, Value)> {
    vec![("key", Value::from(key)), ("type", Value::from("chore"))]
}

fn decision(handle: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("handle", Value::from(handle)),
        ("date", Value::from("2026-11-24")),
        ("tickets", Value::from("NEX-126")),
        ("title", Value::from("A decision")),
    ]
}

/// A repository space, freshly made, open on the device's database.
fn repository(test: &str) -> (std::path::PathBuf, Opened, SpaceId) {
    let base = scratch(test);
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();
    (base, opened, report.space.parse().unwrap())
}

#[test]
fn a_write_creates_objects_in_one_change_and_a_read_finds_them() {
    let (base, opened, space) = repository("write-create");

    let written = block_on(opened.write(&space, |w| {
        let decision = w.create("decisionsDecision", decision("2026-11-24-NEX-126-a"))?;
        w.create(
            "decisionsTopic",
            [
                ("decision", Value::from(decision)),
                ("topic", Value::from("store")),
            ],
        )?;
        Ok(())
    }))
    .unwrap();

    assert_eq!(written.ids.len(), 2, "one change, two objects");
    let read = block_on(opened.get(&space, "decisionsDecision", written.ids[0]))
        .unwrap()
        .expect("the decision is read back");
    assert_eq!(
        read.get::<&str>("handle").unwrap(),
        Some("2026-11-24-NEX-126-a")
    );
    let topic = block_on(opened.get(&space, "decisionsTopic", written.ids[1]))
        .unwrap()
        .unwrap();
    assert_eq!(
        topic.get::<ObjectId>("decision").unwrap(),
        Some(written.ids[0])
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_object_that_is_not_write_once_takes_a_set_a_delete_and_a_restore() {
    let (base, opened, space) = repository("write-change");
    let id = block_on(opened.write(&space, |w| {
        w.create("ticketsTicket", ticket("ka.1")).map(drop)
    }))
    .unwrap()
    .ids[0];

    block_on(opened.write(&space, |w| {
        w.update("ticketsTicket", id, [("key", Value::from("ka.2"))])
    }))
    .unwrap();
    let read = block_on(opened.get(&space, "ticketsTicket", id))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<&str>("key").unwrap(), Some("ka.2"));

    block_on(opened.write(&space, |w| w.delete("ticketsTicket", id))).unwrap();
    let read = block_on(opened.get(&space, "ticketsTicket", id))
        .unwrap()
        .unwrap();
    assert!(read.deleted());

    block_on(opened.write(&space, |w| w.restore("ticketsTicket", id))).unwrap();
    let read = block_on(opened.get(&space, "ticketsTicket", id))
        .unwrap()
        .unwrap();
    assert!(!read.deleted());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_collection_written_into_a_space_of_no_known_kind_is_refused_and_nothing_is_written() {
    let base = scratch("write-no-kind");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

    let err = block_on(opened.write(&space, |w| {
        w.create("decisionsDecision", decision("2026-11-24-NEX-126-b"))
            .map(drop)
    }))
    .unwrap_err();

    let WriteError::WrongSpace(err) = err else {
        panic!("a wrong-kind refusal, got {err:?}");
    };
    assert_eq!(err.collection, "decisionsDecision");
    assert_eq!(err.space, None);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_refused_write_writes_nothing_of_its_change() {
    let (base, opened, space) = repository("write-abort");
    let other = "00112233445566778899aabbccddeeff"
        .parse::<SpaceId>()
        .unwrap();
    let mut created = None;

    let err = block_on(opened.write(&space, |w| {
        created = Some(w.create("ticketsTicket", ticket("ka.3"))?);
        w.create("notACollection", [])?;
        Ok(())
    }))
    .unwrap_err();

    assert!(matches!(err, WriteError::WrongSpace(_)), "{err:?}");
    let created = created.expect("the first create was recorded");
    assert!(block_on(opened.get(&space, "ticketsTicket", created))
        .unwrap()
        .is_none());
    let err = block_on(opened.write(&other, |w| {
        w.create("ticketsTicket", ticket("ka.4")).map(drop)
    }))
    .unwrap_err();
    assert_eq!(err, WriteError::NotOpen(other));
    std::fs::remove_dir_all(&base).unwrap();
}
