//! The move's collections and the ticket's v1 id (RFC 0008, "The migration records", "The
//! migration run", "Changes to the decided RFCs"): a run and its records are written with every
//! field the RFC declares, read back, and found through every index the RFC declares, in the order
//! it documents, in the one space the index runs in; a moved ticket is found by its v1 id.

mod common;

use common::block_on;
use nexus_flow_store::model;
use nocentraldb::{Db, ObjectId, OpenOptions, Space, Value};

/// The ids of the first page of `index` of `collection` in `space`, its equality prefix fixed to
/// `equal`.
fn found(
    db: &Db,
    space: &Space,
    collection: &str,
    index: &str,
    equal: Vec<Value>,
) -> Vec<ObjectId> {
    let query = db
        .index(collection, index)
        .unwrap_or_else(|e| panic!("{collection}.{index} is declared: {e}"))
        .equal(equal);
    let page = block_on(space.find(query))
        .unwrap_or_else(|e| panic!("{collection}.{index} runs in one space: {e}"));
    page.items.iter().map(|object| object.id()).collect()
}

fn create(space: &Space, collection: &str, fields: Vec<(&str, Value)>) -> ObjectId {
    block_on(space.create(collection, fields))
        .unwrap_or_else(|e| panic!("{collection} is a collection of the model: {e}"))
        .ids[0]
}

fn run(space: &Space, workspace: &str, started_at: &str) -> ObjectId {
    create(
        space,
        "migrationRun",
        vec![
            ("workspace", Value::from(workspace)),
            ("replica", Value::from("0b9e4c1a 6j6v")),
            ("member", Value::from("m-migration")),
            ("startedAt", Value::from(started_at)),
            ("endedAt", Value::from("2026-11-24T21:05:00Z")),
            ("state", Value::from("complete")),
            ("ops", Value::from(3)),
            ("last", Value::from(2)),
            ("report", Value::from("{\"state\":\"complete\"}")),
        ],
    )
}

/// A record of the op at `position` of `run`, about `item`, written to `ticket` when it is one.
fn record(
    space: &Space,
    run: ObjectId,
    origin: &str,
    position: i64,
    item: &str,
    ticket: Option<ObjectId>,
) -> ObjectId {
    let mut fields = vec![
        ("run", Value::from(run)),
        ("source", Value::from("op")),
        ("origin", Value::from(origin)),
        ("position", Value::from(position)),
        ("kind", Value::from("flow/item/set/title")),
        ("item", Value::from(item)),
        ("author", Value::from("carsten")),
        ("wallClock", Value::from("2026-03-01T10:00:00Z")),
        ("lamport", Value::from(position + 10)),
        ("site", Value::from(7)),
        ("keyId", Value::from("k1")),
        ("sig", Value::from("c2ln")),
        ("provenance", Value::from("own")),
        ("payload", Value::from("cGF5bG9hZA")),
        ("outcome", Value::from("applied")),
    ];
    if let Some(ticket) = ticket {
        fields.push(("ticket", Value::from(ticket)));
    }
    create(space, "migrationRecord", fields)
}

struct Move {
    db: Db,
    space: Space,
    older_run: ObjectId,
    run: ObjectId,
    ticket: ObjectId,
    /// The records of `run`, by position: on `ticket`, on a memory, on `ticket` again.
    records: [ObjectId; 3],
}

/// A space with an older run of the same workspace, a run with three records written out of
/// order, and the ticket two of them wrote to.
fn moved() -> Move {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    let older_run = run(&space, "stream-1", "2026-11-20T21:00:00Z");
    let run = run(&space, "stream-1", "2026-11-24T21:00:00Z");
    let ticket = create(
        &space,
        "ticketsTicket",
        vec![
            ("key", Value::from("mg.1")),
            ("v1Id", Value::from("6j6v.pzkb")),
        ],
    );
    let third = record(&space, run, "op-3", 2, "6j6v.pzkb", Some(ticket));
    let memory = record(&space, run, "op-2", 1, "conventions", None);
    let first = record(&space, run, "op-1", 0, "6j6v.pzkb", Some(ticket));
    Move {
        db,
        space,
        older_run,
        run,
        ticket,
        records: [first, memory, third],
    }
}

#[test]
fn a_run_is_written_with_every_field_of_the_rfc_and_read_back() {
    let m = moved();
    let object = block_on(m.space.get("migrationRun", m.run))
        .unwrap()
        .unwrap();
    for (field, value) in [
        ("workspace", "stream-1"),
        ("replica", "0b9e4c1a 6j6v"),
        ("member", "m-migration"),
        ("startedAt", "2026-11-24T21:00:00Z"),
        ("endedAt", "2026-11-24T21:05:00Z"),
        ("state", "complete"),
        ("report", "{\"state\":\"complete\"}"),
    ] {
        assert_eq!(object.get::<&str>(field).unwrap(), Some(value), "{field}");
    }
    assert_eq!(object.get::<i64>("ops").unwrap(), Some(3));
    assert_eq!(object.get::<i64>("last").unwrap(), Some(2));
}

#[test]
fn a_run_takes_every_state_of_the_rfc() {
    let m = moved();
    for state in [
        "replaying",
        "verifying",
        "complete",
        "verificationFailed",
        "undone",
    ] {
        block_on(
            m.space
                .update("migrationRun", m.run, [("state", Value::from(state))]),
        )
        .unwrap_or_else(|e| panic!("{state} is a state of a run: {e}"));
    }
    assert!(
        block_on(
            m.space
                .update("migrationRun", m.run, [("state", Value::from("paused"))])
        )
        .is_err(),
        "a state the RFC does not name is refused"
    );
}

#[test]
fn a_record_is_written_with_every_field_of_the_rfc_and_read_back() {
    let m = moved();
    let object = block_on(m.space.get("migrationRecord", m.records[0]))
        .unwrap()
        .unwrap();
    for (field, value) in [
        ("source", "op"),
        ("origin", "op-1"),
        ("kind", "flow/item/set/title"),
        ("item", "6j6v.pzkb"),
        ("author", "carsten"),
        ("wallClock", "2026-03-01T10:00:00Z"),
        ("keyId", "k1"),
        ("sig", "c2ln"),
        ("provenance", "own"),
        ("payload", "cGF5bG9hZA"),
        ("outcome", "applied"),
    ] {
        assert_eq!(object.get::<&str>(field).unwrap(), Some(value), "{field}");
    }
    assert_eq!(object.get::<i64>("position").unwrap(), Some(0));
    assert_eq!(object.get::<i64>("lamport").unwrap(), Some(10));
    assert_eq!(object.get::<i64>("site").unwrap(), Some(7));
    assert_eq!(object.get::<ObjectId>("run").unwrap(), Some(m.run));
    assert_eq!(object.get::<ObjectId>("ticket").unwrap(), Some(m.ticket));
}

#[test]
fn a_record_takes_every_source_provenance_and_outcome_of_the_rfc() {
    let m = moved();
    let choices = [
        ("source", &["op", "file"][..]),
        (
            "provenance",
            &["own", "verified", "invalid", "unsigned"][..],
        ),
        (
            "outcome",
            &["applied", "noEffect", "dropped", "withheld", "lateConflict"][..],
        ),
    ];
    for (field, options) in choices {
        for option in options {
            block_on(m.space.create(
                "migrationRecord",
                [
                    ("run", Value::from(m.run)),
                    ("origin", Value::from(format!("{field}-{option}"))),
                    (field, Value::from(*option)),
                ],
            ))
            .unwrap_or_else(|e| panic!("{option} is a {field} of a record: {e}"));
        }
        assert!(
            block_on(m.space.create(
                "migrationRecord",
                [("run", Value::from(m.run)), (field, Value::from("other"))],
            ))
            .is_err(),
            "a {field} the RFC does not name is refused"
        );
    }
}

#[test]
fn a_record_is_found_by_its_origin() {
    let m = moved();
    let found = found(
        &m.db,
        &m.space,
        "migrationRecord",
        "byOrigin",
        vec![Value::from("op-2")],
    );
    assert_eq!(found, [m.records[1]]);
}

#[test]
fn a_tickets_records_are_found_in_v1_order() {
    let m = moved();
    let found = found(
        &m.db,
        &m.space,
        "migrationRecord",
        "byTicket",
        vec![Value::from(m.ticket)],
    );
    assert_eq!(found, [m.records[0], m.records[2]]);
}

#[test]
fn an_items_records_are_found_in_v1_order() {
    let m = moved();
    let ticket = found(
        &m.db,
        &m.space,
        "migrationRecord",
        "byItem",
        vec![Value::from("6j6v.pzkb")],
    );
    assert_eq!(ticket, [m.records[0], m.records[2]]);
    let memory = found(
        &m.db,
        &m.space,
        "migrationRecord",
        "byItem",
        vec![Value::from("conventions")],
    );
    assert_eq!(memory, [m.records[1]]);
}

#[test]
fn a_runs_records_are_found_in_v1_order() {
    let m = moved();
    let found = found(
        &m.db,
        &m.space,
        "migrationRecord",
        "byRun",
        vec![Value::from(m.run)],
    );
    assert_eq!(found, m.records);
}

#[test]
fn a_workspaces_runs_are_found_oldest_first() {
    let m = moved();
    let found = found(
        &m.db,
        &m.space,
        "migrationRun",
        "byWorkspace",
        vec![Value::from("stream-1")],
    );
    assert_eq!(found, [m.older_run, m.run]);
}

#[test]
fn a_moved_ticket_keeps_its_v1_id_and_is_found_by_it() {
    let m = moved();
    let object = block_on(m.space.get("ticketsTicket", m.ticket))
        .unwrap()
        .unwrap();
    assert_eq!(object.get::<&str>("v1Id").unwrap(), Some("6j6v.pzkb"));
    let found = found(
        &m.db,
        &m.space,
        "ticketsTicket",
        "byV1Id",
        vec![Value::from("6j6v.pzkb")],
    );
    assert_eq!(found, [m.ticket]);
}
