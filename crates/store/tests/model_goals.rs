//! The goals module's collection and indexes (RFC 0005, "The goal"), and the two places other
//! modules refer to a goal: RFC 0002's `ticketsReference`, whose `target` takes a goal and whose
//! `kind` takes `proposed` ("A ticket serves a goal", "What the re-evaluation stores"), and RFC
//! 0003's `decisionsSubject.elsewhere` ("decisionsSubject").

mod common;

use common::block_on;
use nexus_flow_store::model;
use nocentraldb::{Db, ObjectId, OpenOptions, Reference, Space, Value};

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

/// A goal with `key`, `status`, `statusAt` and, for a goal in focus, `rank`.
fn goal(space: &Space, key: &str, status: &str, rank: Option<&str>, status_at: &str) -> ObjectId {
    let mut fields = vec![
        ("key", Value::from(key)),
        ("title", Value::from(format!("The goal {key}"))),
        ("status", Value::from(status)),
        ("statusAt", Value::from(status_at)),
        ("createdAt", Value::from("2026-11-20T09:00:00Z")),
        ("createdBy", Value::from("m1")),
        ("statusBy", Value::from("m1")),
    ];
    if let Some(rank) = rank {
        fields.push(("rank", Value::from(rank)));
    }
    create(space, "goalsGoal", fields)
}

fn open() -> (Db, Space) {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    (db, space)
}

#[test]
fn a_goal_is_written_with_every_field_of_the_rfc_and_read_back() {
    let (_db, space) = open();
    let id = create(
        &space,
        "goalsGoal",
        vec![
            ("key", Value::from("cb.3")),
            (
                "title",
                Value::from("Win 20 paying customers for nexflow.it"),
            ),
            ("description", Value::from("Why it **matters**.")),
            (
                "reachedWhen",
                Value::from("20 customers pay a monthly plan"),
            ),
            ("status", Value::from("focus")),
            ("rank", Value::from("i")),
            ("statusAt", Value::from("2026-11-20T10:00:00Z")),
            ("statusComment", Value::from("The owner put it first")),
            ("owner", Value::from("member:9b2d")),
            ("createdAt", Value::from("2026-11-20T09:00:00Z")),
            ("createdBy", Value::from("9b2d")),
            ("statusBy", Value::from("9b2d")),
        ],
    );
    let read = block_on(space.get("goalsGoal", id)).unwrap().unwrap();
    for (field, want) in [
        ("key", "cb.3"),
        ("title", "Win 20 paying customers for nexflow.it"),
        ("description", "Why it **matters**."),
        ("reachedWhen", "20 customers pay a monthly plan"),
        ("status", "focus"),
        ("rank", "i"),
        ("statusAt", "2026-11-20T10:00:00Z"),
        ("statusComment", "The owner put it first"),
        ("owner", "member:9b2d"),
        ("createdAt", "2026-11-20T09:00:00Z"),
        ("createdBy", "9b2d"),
        ("statusBy", "9b2d"),
    ] {
        assert_eq!(read.get::<&str>(field).unwrap(), Some(want), "{field}");
    }
}

/// Every status of RFC 0005, "Status", is a value of `status`; any other is refused.
#[test]
fn a_goal_takes_the_five_statuses_and_no_other() {
    let (_db, space) = open();
    for status in ["proposed", "focus", "later", "reached", "dropped"] {
        goal(&space, "cb.1", status, None, "2026-11-20T10:00:00Z");
    }
    let refused = block_on(space.create(
        "goalsGoal",
        vec![
            ("key", Value::from("cb.2")),
            ("status", Value::from("open")),
        ],
    ));
    assert!(refused.is_err(), "status `open` must be refused");
}

/// `byStatus`: `status`, then `rank`, then `statusAt`, equal on `status`. The goals in focus come
/// out in their rank order, compared by bytes; the others longest-standing first.
#[test]
fn the_goals_of_one_status_are_found_in_their_order() {
    let (db, space) = open();
    let second = goal(&space, "cb.1", "focus", Some("r"), "2026-11-20T08:00:00Z");
    let first = goal(&space, "cb.2", "focus", Some("i"), "2026-11-21T08:00:00Z");
    let third = goal(&space, "cb.3", "focus", Some("ri"), "2026-11-19T08:00:00Z");
    let newer = goal(&space, "cb.4", "later", None, "2026-11-22T08:00:00Z");
    let older = goal(&space, "cb.5", "later", None, "2026-11-18T08:00:00Z");
    let proposed = goal(&space, "cb.6", "proposed", None, "2026-11-23T08:00:00Z");

    let by = |status: &str| found(&db, &space, "goalsGoal", "byStatus", vec![status.into()]);
    assert_eq!(by("focus"), [first, second, third]);
    assert_eq!(by("later"), [older, newer]);
    assert_eq!(by("proposed"), [proposed]);
    assert_eq!(by("reached"), []);
}

/// `byKey`: `key`, which resolves a key.
#[test]
fn a_goal_is_found_by_its_key() {
    let (db, space) = open();
    let wanted = goal(&space, "cb.3", "later", None, "2026-11-20T08:00:00Z");
    goal(&space, "cb.4", "later", None, "2026-11-20T08:00:00Z");

    let by = |key: &str| found(&db, &space, "goalsGoal", "byKey", vec![key.into()]);
    assert_eq!(by("cb.3"), [wanted]);
    assert_eq!(by("cb.9"), []);
}

/// RFC 0005, "A ticket serves a goal" and "What the re-evaluation stores": a ticket names a goal
/// in another space through a `ticketsReference` whose target is the goal, of kind `serves` or
/// `proposed`, and pm finds both through `byTarget`, ordered by kind.
#[test]
fn a_ticket_serves_or_is_proposed_for_a_goal_in_another_space() {
    let (db, repository) = open();
    let coordination = block_on(db.spaces().create()).unwrap();
    let id = goal(
        &coordination,
        "cb.3",
        "focus",
        Some("i"),
        "2026-11-20T08:00:00Z",
    );
    let target = Reference::new(coordination.id(), id);
    let ticket = create(
        &repository,
        "ticketsTicket",
        vec![
            ("key", Value::from("ka.7")),
            ("type", Value::from("feature")),
        ],
    );
    let reference = |kind: &str| {
        create(
            &repository,
            "ticketsReference",
            vec![
                ("ticket", Value::from(ticket)),
                ("kind", Value::from(kind)),
                ("target", Value::from(target)),
                ("label", Value::from("cb.3 Win 20 paying customers")),
            ],
        )
    };
    let serves = reference("serves");
    let proposed = reference("proposed");

    // `byTarget` is equal on `target` and then ordered by `kind`.
    assert_eq!(
        found(
            &db,
            &repository,
            "ticketsReference",
            "byTarget",
            vec![target.into()]
        ),
        [proposed, serves]
    );
    let read = block_on(repository.get("ticketsReference", proposed))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<Reference>("target").unwrap(), Some(target));
    assert_eq!(read.get::<&str>("kind").unwrap(), Some("proposed"));
}

/// RFC 0003, "decisionsSubject": a decision may be about a goal, which is in another space.
#[test]
fn a_decision_is_about_a_goal_in_another_space() {
    let (db, repository) = open();
    let coordination = block_on(db.spaces().create()).unwrap();
    let id = goal(
        &coordination,
        "cb.3",
        "focus",
        Some("i"),
        "2026-11-20T08:00:00Z",
    );
    let elsewhere = Reference::new(coordination.id(), id);
    let decision = create(
        &repository,
        "decisionsDecision",
        vec![(
            "handle",
            Value::from("2026-11-20-NEX-58-only-a-person-sets-focus"),
        )],
    );
    let subject = create(
        &repository,
        "decisionsSubject",
        vec![
            ("decision", Value::from(decision)),
            ("elsewhere", Value::from(elsewhere)),
            ("label", Value::from("cb.3")),
            ("date", Value::from("2026-11-20")),
        ],
    );
    let read = block_on(repository.get("decisionsSubject", subject))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<Reference>("elsewhere").unwrap(), Some(elsewhere));
}
