//! RFC 0003, "Append-only": no change can delete or rewrite a decision through nexus-flow. The
//! store's one write path refuses a field set, a delete and a restore of an existing object of
//! each of the four decision collections, for every caller, and the object stays as it was.

mod common;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::init;
use nexus_flow_store::write::WriteError;
use nexus_flow_store::write_once::Change;
use nocentraldb::{ObjectId, Value};

/// One decision of each collection, written in one change, with the field the test tries to set.
struct Log {
    objects: Vec<(&'static str, ObjectId, &'static str)>,
}

fn decision(handle: &'static str) -> Vec<(&'static str, Value)> {
    vec![
        ("handle", Value::from(handle)),
        ("date", Value::from("2026-11-24")),
        ("tickets", Value::from("NEX-126")),
        ("title", Value::from(handle)),
        ("decidedBy", Value::from("coder on NEX-126")),
    ]
}

fn write_log(opened: &Opened, space: &SpaceId) -> Log {
    let mut objects = Vec::new();
    block_on(opened.write(space, |w| {
        let older = w.create("decisionsDecision", decision("2026-11-24-NEX-126-older"))?;
        let newer = w.create("decisionsDecision", decision("2026-11-24-NEX-126-newer"))?;
        let topic = w.create(
            "decisionsTopic",
            [
                ("decision", Value::from(newer)),
                ("topic", Value::from("store")),
                ("date", Value::from("2026-11-24")),
            ],
        )?;
        let subject = w.create(
            "decisionsSubject",
            [
                ("decision", Value::from(newer)),
                ("label", Value::from("NEX-126")),
                ("date", Value::from("2026-11-24")),
            ],
        )?;
        let supersession = w.create(
            "decisionsSupersession",
            [
                ("decision", Value::from(newer)),
                ("supersedes", Value::from(older)),
                ("date", Value::from("2026-11-24")),
            ],
        )?;
        objects = vec![
            ("decisionsDecision", newer, "title"),
            ("decisionsTopic", topic, "topic"),
            ("decisionsSubject", subject, "label"),
            ("decisionsSupersession", supersession, "date"),
        ];
        Ok(())
    }))
    .unwrap();
    Log { objects }
}

/// The object's field `field` and whether it is deleted.
fn state(
    opened: &Opened,
    space: &SpaceId,
    collection: &str,
    id: ObjectId,
    field: &str,
) -> (String, bool) {
    let object = block_on(opened.get(space, collection, id))
        .unwrap()
        .unwrap_or_else(|| panic!("{collection} {id:?} is held"));
    let value = object.get::<&str>(field).unwrap().unwrap().to_owned();
    (value, object.deleted())
}

#[test]
fn a_set_a_delete_and_a_restore_of_every_decision_object_are_refused_and_change_nothing() {
    let base = scratch("append-only");
    let repo = base.join("repo");
    let data = data_dir(&base.join("data"));
    let space: SpaceId = block_on(init(&repo, &data, keys(&data)))
        .unwrap()
        .space
        .parse()
        .unwrap();
    let opened = block_on(opened::repository(&repo, &data, keys(&data))).unwrap();
    let log = write_log(&opened, &space);

    for &(collection, id, field) in &log.objects {
        let before = state(&opened, &space, collection, id, field);
        let attempts: [(Change, Result<_, WriteError>); 3] = [
            (
                Change::Set,
                block_on(opened.write(&space, |w| {
                    w.update(collection, id, [(field, Value::from("rewritten"))])
                })),
            ),
            (
                Change::Delete,
                block_on(opened.write(&space, |w| w.delete(collection, id))),
            ),
            (
                Change::Restore,
                block_on(opened.write(&space, |w| w.restore(collection, id))),
            ),
        ];
        for (change, result) in attempts {
            let Err(WriteError::WriteOnce(err)) = result else {
                panic!("{change:?} of {collection} is refused as write-once, got {result:?}");
            };
            assert_eq!(err.collection, collection);
            assert_eq!(err.change, change);
            assert_eq!(
                state(&opened, &space, collection, id, field),
                before,
                "{change:?} left {collection} unchanged"
            );
        }
    }
    std::fs::remove_dir_all(&base).unwrap();
}
