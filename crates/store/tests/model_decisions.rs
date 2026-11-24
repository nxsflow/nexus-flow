//! The decisions module's collections and indexes (RFC 0003, "The collections"): decisions and
//! the topic, subject and supersession objects that belong to them are written with every field
//! the RFC declares, read back, and found through every index the RFC declares, in the order it
//! documents, in the one space the index runs in.

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

struct Decisions {
    db: Db,
    space: Space,
    /// A ticket of the space that two decisions are about.
    ticket: ObjectId,
    /// 2026-11-15, NEX-15.
    oldest: ObjectId,
    /// 2026-11-20, NEX-51; supersedes `oldest`.
    handles: ObjectId,
    /// 2026-11-20, NEX-52, the same day as `handles` and after it by handle.
    on_main: ObjectId,
}

fn decision(space: &Space, handle: &str, date: &str, tickets: &str, pr: Option<&str>) -> ObjectId {
    let mut fields = vec![
        ("handle", Value::from(handle)),
        ("date", Value::from(date)),
        ("tickets", Value::from(tickets)),
        ("title", Value::from(format!("The decision {handle}"))),
        ("decidedBy", Value::from("Carsten")),
        ("why", Value::from("Why it was decided")),
        ("insteadOf", Value::from("The options not taken")),
        ("body", Value::from("## The rule\n\nAs written.")),
        ("author", Value::from("9b2d")),
    ];
    if let Some(pr) = pr {
        fields.push(("pr", Value::from(pr)));
    }
    create(space, "decisionsDecision", fields)
}

fn decisions() -> Decisions {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    let ticket = create(
        &space,
        "ticketsTicket",
        vec![
            ("key", Value::from("ka.15")),
            ("type", Value::from("decision")),
        ],
    );
    let oldest = decision(
        &space,
        "2026-11-15-NEX-15-one-space-per-repository",
        "2026-11-15",
        "NEX-15",
        None,
    );
    let handles = decision(
        &space,
        "2026-11-20-NEX-51-a-decision-has-a-handle",
        "2026-11-20",
        "NEX-51",
        None,
    );
    let on_main = decision(
        &space,
        "2026-11-20-NEX-52-on-main-by-commit-or-ticket",
        "2026-11-20",
        "NEX-52",
        Some("https://github.com/nxsflow/nexus-flow/pull/120"),
    );
    Decisions {
        db,
        space,
        ticket,
        oldest,
        handles,
        on_main,
    }
}

#[test]
fn a_decision_is_written_with_every_field_of_the_rfc_and_read_back() {
    let d = decisions();
    let read = block_on(d.space.get("decisionsDecision", d.on_main))
        .unwrap()
        .unwrap();
    for (field, value) in [
        ("handle", "2026-11-20-NEX-52-on-main-by-commit-or-ticket"),
        ("date", "2026-11-20"),
        ("tickets", "NEX-52"),
        (
            "title",
            "The decision 2026-11-20-NEX-52-on-main-by-commit-or-ticket",
        ),
        ("decidedBy", "Carsten"),
        ("why", "Why it was decided"),
        ("insteadOf", "The options not taken"),
        ("body", "## The rule\n\nAs written."),
        ("pr", "https://github.com/nxsflow/nexus-flow/pull/120"),
        ("author", "9b2d"),
    ] {
        assert_eq!(read.get::<&str>(field).unwrap(), Some(value), "{field}");
    }
}

/// `byDate`: `date` descending, then `handle` descending; `byHandle`: `handle`.
#[test]
fn a_decision_is_found_by_date_newest_first_and_by_its_handle() {
    let d = decisions();
    let by =
        |index: &str, equal: Vec<Value>| found(&d.db, &d.space, "decisionsDecision", index, equal);
    assert_eq!(
        by("byDate", vec![]),
        [d.on_main, d.handles, d.oldest],
        "newest first, the same day by handle descending"
    );
    assert_eq!(
        by(
            "byHandle",
            vec!["2026-11-20-NEX-51-a-decision-has-a-handle".into()]
        ),
        [d.handles]
    );
}

/// `byTopic`: `topic`, then `date` descending.
#[test]
fn a_topic_is_found_by_its_tag_newest_decision_first() {
    let d = decisions();
    let topic = |decision: ObjectId, tag: &str, date: &str| {
        create(
            &d.space,
            "decisionsTopic",
            vec![
                ("decision", Value::from(decision)),
                ("topic", Value::from(tag)),
                ("date", Value::from(date)),
            ],
        )
    };
    let oldest = topic(d.oldest, "spaces", "2026-11-15");
    let handles = topic(d.handles, "spaces", "2026-11-20");
    topic(d.handles, "handles", "2026-11-20");

    assert_eq!(
        found(
            &d.db,
            &d.space,
            "decisionsTopic",
            "byTopic",
            vec!["spaces".into()]
        ),
        [handles, oldest]
    );
    let read = block_on(d.space.get("decisionsTopic", handles))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<ObjectId>("decision").unwrap(), Some(d.handles));
    assert_eq!(read.get::<&str>("topic").unwrap(), Some("spaces"));
    assert_eq!(read.get::<&str>("date").unwrap(), Some("2026-11-20"));
}

/// `byLabel`: `label`, then `date` descending; `byTicket`: `ticket`. A subject in another space
/// is a reference in `elsewhere`.
#[test]
fn a_subject_is_found_by_its_label_newest_decision_first_and_by_its_ticket() {
    let d = decisions();
    let other = block_on(d.db.spaces().create()).unwrap();
    let cited = create(
        &other,
        "ticketsTicket",
        vec![
            ("key", Value::from("nc.9")),
            ("type", Value::from("feature")),
        ],
    );
    let elsewhere = Reference::new(other.id(), cited);
    let oldest = create(
        &d.space,
        "decisionsSubject",
        vec![
            ("decision", Value::from(d.oldest)),
            ("ticket", Value::from(d.ticket)),
            ("label", Value::from("NEX-15")),
            ("date", Value::from("2026-11-15")),
        ],
    );
    let handles = create(
        &d.space,
        "decisionsSubject",
        vec![
            ("decision", Value::from(d.handles)),
            ("label", Value::from("NEX-15")),
            ("date", Value::from("2026-11-20")),
        ],
    );
    let remote = create(
        &d.space,
        "decisionsSubject",
        vec![
            ("decision", Value::from(d.on_main)),
            ("elsewhere", Value::from(elsewhere)),
            ("label", Value::from("nocentraldb nc.9")),
            ("date", Value::from("2026-11-20")),
        ],
    );

    let by =
        |index: &str, equal: Vec<Value>| found(&d.db, &d.space, "decisionsSubject", index, equal);
    assert_eq!(by("byLabel", vec!["NEX-15".into()]), [handles, oldest]);
    assert_eq!(by("byTicket", vec![d.ticket.into()]), [oldest]);
    let read = block_on(d.space.get("decisionsSubject", remote))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<ObjectId>("decision").unwrap(), Some(d.on_main));
    assert_eq!(read.get::<Reference>("elsewhere").unwrap(), Some(elsewhere));
    assert_eq!(read.get::<&str>("label").unwrap(), Some("nocentraldb nc.9"));
}

/// `bySuperseded`: `supersedes`; `byDate`: `date` descending.
#[test]
fn a_supersession_is_found_by_the_decision_it_supersedes_and_by_date_newest_first() {
    let d = decisions();
    let first = create(
        &d.space,
        "decisionsSupersession",
        vec![
            ("decision", Value::from(d.handles)),
            ("supersedes", Value::from(d.oldest)),
            ("date", Value::from("2026-11-20")),
        ],
    );
    let later = decision(
        &d.space,
        "2026-11-24-NEX-125-a-later-rule",
        "2026-11-24",
        "NEX-125",
        None,
    );
    let second = create(
        &d.space,
        "decisionsSupersession",
        vec![
            ("decision", Value::from(later)),
            ("supersedes", Value::from(d.handles)),
            ("date", Value::from("2026-11-24")),
        ],
    );

    let by = |index: &str, equal: Vec<Value>| {
        found(&d.db, &d.space, "decisionsSupersession", index, equal)
    };
    assert_eq!(by("bySuperseded", vec![d.oldest.into()]), [first]);
    assert_eq!(by("bySuperseded", vec![d.handles.into()]), [second]);
    assert_eq!(by("byDate", vec![]), [second, first], "newest first");
    let read = block_on(d.space.get("decisionsSupersession", first))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<ObjectId>("decision").unwrap(), Some(d.handles));
    assert_eq!(read.get::<ObjectId>("supersedes").unwrap(), Some(d.oldest));
    assert_eq!(read.get::<&str>("date").unwrap(), Some("2026-11-20"));
}
