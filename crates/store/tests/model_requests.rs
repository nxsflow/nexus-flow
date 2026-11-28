//! The requests module's repository record (RFC 0009, "Repositories in the space"): one
//! `requestsRepository` object per repository of a group, in the coordination space, with its
//! `name`, `space` and `left`, and the indexes that list the records by name and find a
//! repository's own record by its space.

mod common;

use common::block_on;
use nexus_flow_store::model::{self, Module};
use nexus_flow_store::space_kind::{self, SpaceKind};
use nocentraldb::{Db, ObjectId, OpenOptions, Space, Value};

fn open() -> (Db, Space) {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    (db, space)
}

fn record(space: &Space, name: &str, of: &str) -> ObjectId {
    block_on(space.create(
        "requestsRepository",
        vec![("name", Value::from(name)), ("space", Value::from(of))],
    ))
    .unwrap_or_else(|e| panic!("requestsRepository is a collection of the model: {e}"))
    .ids[0]
}

fn found(db: &Db, space: &Space, index: &str, equal: Vec<Value>) -> Vec<ObjectId> {
    let query = db
        .index("requestsRepository", index)
        .unwrap_or_else(|e| panic!("requestsRepository.{index} is declared: {e}"))
        .equal(equal);
    let page = block_on(space.find(query)).unwrap();
    page.items.iter().map(|object| object.id()).collect()
}

#[test]
fn a_repository_record_holds_its_name_space_and_left() {
    let (_db, space) = open();
    let id = record(&space, "central-infra", "0123456789abcdef0123456789abcdef");
    block_on(space.update("requestsRepository", id, vec![("left", Value::from(true))])).unwrap();

    let read = block_on(space.get("requestsRepository", id))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<&str>("name").unwrap(), Some("central-infra"));
    assert_eq!(
        read.get::<&str>("space").unwrap(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(read.get::<bool>("left").unwrap(), Some(true));
}

#[test]
fn records_are_listed_by_name_and_found_by_their_space() {
    let (db, space) = open();
    let infra = record(&space, "central-infra", "11111111111111111111111111111111");
    let app = record(&space, "app-foundations", "22222222222222222222222222222222");

    assert_eq!(found(&db, &space, "byName", Vec::new()), vec![app, infra]);
    assert_eq!(
        found(
            &db,
            &space,
            "bySpace",
            vec!["11111111111111111111111111111111".into()]
        ),
        vec![infra]
    );
}

#[test]
fn the_record_belongs_to_the_requests_module_and_the_coordination_space() {
    assert!(model::COLLECTIONS.contains(&("requestsRepository", Module::Requests)));
    let kinds = space_kind::KINDS
        .iter()
        .find(|(name, _)| *name == "requestsRepository")
        .map(|(_, kinds)| *kinds);
    assert_eq!(kinds, Some(&[SpaceKind::Coordination][..]));
}
