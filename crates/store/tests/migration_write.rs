//! RFC 0008, "The migration records" and "The migration run": only the move writes a run and its
//! records, through the store's write path for the move (`Opened::write_as_move`); every other
//! write that names either collection is refused and writes nothing of its change. A record takes
//! no write after the change that creates it, for the move too; a run takes the writes the move
//! makes as it goes on.

mod common;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::init;
use nexus_flow_store::write::WriteError;
use nexus_flow_store::write_once::Change;
use nocentraldb::{ObjectId, Value};

/// A repository space, freshly made, open on the device's database.
fn repository(test: &str) -> (std::path::PathBuf, Opened, SpaceId) {
    let base = scratch(test);
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();
    (base, opened, report.space.parse().unwrap())
}

fn run_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("workspace", Value::from("stream-1")),
        ("startedAt", Value::from("2026-11-24T21:00:00Z")),
        ("state", Value::from("replaying")),
    ]
}

fn record(run: ObjectId, ticket: ObjectId) -> Vec<(&'static str, Value)> {
    vec![
        ("run", Value::from(run)),
        ("source", Value::from("op")),
        ("origin", Value::from("op-1")),
        ("position", Value::from(0)),
        ("ticket", Value::from(ticket)),
        ("outcome", Value::from("applied")),
    ]
}

/// A run, a moved ticket and the record of its first op, written by the move in one change.
fn moved(opened: &Opened, space: &SpaceId) -> (ObjectId, ObjectId, ObjectId) {
    let mut ids = None;
    block_on(opened.write_as_move(space, |w| {
        let run = w.create("migrationRun", run_fields())?;
        let ticket = w.create(
            "ticketsTicket",
            [
                ("key", Value::from("mg.1")),
                ("v1Id", Value::from("6j6v.pzkb")),
            ],
        )?;
        let record = w.create("migrationRecord", record(run, ticket))?;
        ids = Some((run, ticket, record));
        Ok(())
    }))
    .unwrap();
    ids.unwrap()
}

fn field(opened: &Opened, space: &SpaceId, collection: &str, id: ObjectId, name: &str) -> String {
    let object = block_on(opened.get(space, collection, id))
        .unwrap()
        .unwrap_or_else(|| panic!("{collection} {id:?} is held"));
    object.get::<&str>(name).unwrap().unwrap().to_owned()
}

#[test]
fn the_move_writes_a_run_a_ticket_and_its_record_in_one_change() {
    let (base, opened, space) = repository("move-write");
    let (run, ticket, record) = moved(&opened, &space);

    assert_eq!(
        field(&opened, &space, "migrationRun", run, "state"),
        "replaying"
    );
    assert_eq!(
        field(&opened, &space, "ticketsTicket", ticket, "v1Id"),
        "6j6v.pzkb"
    );
    assert_eq!(
        field(&opened, &space, "migrationRecord", record, "origin"),
        "op-1"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_write_that_is_not_the_moves_is_refused_for_a_run_or_a_record_and_writes_nothing() {
    let (base, opened, space) = repository("move-only");
    let (run, ticket, _) = moved(&opened, &space);

    for collection in ["migrationRun", "migrationRecord"] {
        let mut created = None;
        let err = block_on(opened.write(&space, |w| {
            created = Some(w.create("ticketsTicket", [("key", Value::from("ka.1"))])?);
            let fields = match collection {
                "migrationRun" => run_fields(),
                _ => record(run, ticket),
            };
            w.create(collection, fields)?;
            Ok(())
        }))
        .unwrap_err();
        let WriteError::MoveOnly(err) = err else {
            panic!("a create of {collection} outside the move is refused, got {err:?}");
        };
        assert_eq!(err.collection, collection);
        assert!(
            err.to_string().contains(collection),
            "the refusal names the collection: {err}"
        );
        let ticket = created.expect("the ticket was recorded before the refusal");
        assert!(
            block_on(opened.get(&space, "ticketsTicket", ticket))
                .unwrap()
                .is_none(),
            "nothing of the refused change is written"
        );
    }

    let err = block_on(opened.write(&space, |w| {
        w.update("migrationRun", run, [("state", Value::from("undone"))])
    }))
    .unwrap_err();
    assert!(
        matches!(err, WriteError::MoveOnly(ref e) if e.collection == "migrationRun"),
        "a set of a run outside the move is refused, got {err:?}"
    );
    assert_eq!(
        field(&opened, &space, "migrationRun", run, "state"),
        "replaying"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_move_sets_a_run_as_it_goes_on() {
    let (base, opened, space) = repository("move-run");
    let (run, _, _) = moved(&opened, &space);

    block_on(opened.write_as_move(&space, |w| {
        w.update(
            "migrationRun",
            run,
            [
                ("state", Value::from("complete")),
                ("last", Value::from(0)),
                ("endedAt", Value::from("2026-11-24T21:05:00Z")),
                ("report", Value::from("{}")),
            ],
        )
    }))
    .unwrap();

    assert_eq!(
        field(&opened, &space, "migrationRun", run, "state"),
        "complete"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_second_write_to_a_record_is_refused_for_the_move_too_and_changes_nothing() {
    let (base, opened, space) = repository("move-record");
    let (_, _, record) = moved(&opened, &space);

    let attempts: [(Change, Result<_, WriteError>); 3] = [
        (
            Change::Set,
            block_on(opened.write_as_move(&space, |w| {
                w.update(
                    "migrationRecord",
                    record,
                    [("origin", Value::from("rewritten"))],
                )
            })),
        ),
        (
            Change::Delete,
            block_on(opened.write_as_move(&space, |w| w.delete("migrationRecord", record))),
        ),
        (
            Change::Restore,
            block_on(opened.write_as_move(&space, |w| w.restore("migrationRecord", record))),
        ),
    ];
    for (change, result) in attempts {
        let Err(WriteError::WriteOnce(err)) = result else {
            panic!("{change:?} of a record is refused as write-once, got {result:?}");
        };
        assert_eq!(err.collection, "migrationRecord");
        assert_eq!(err.change, change);
    }
    let object = block_on(opened.get(&space, "migrationRecord", record))
        .unwrap()
        .unwrap();
    assert_eq!(object.get::<&str>("origin").unwrap(), Some("op-1"));
    assert!(!object.deleted());
    std::fs::remove_dir_all(&base).unwrap();
}
