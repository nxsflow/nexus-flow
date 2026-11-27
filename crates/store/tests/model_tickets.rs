//! The tickets module's collections and indexes (RFC 0002, "Collections", "The ticket",
//! "Indexes", "Work links", "Notes", "References out of the space"): one object of each
//! collection is written and read back through every index the RFC declares for it, in the one
//! space the index runs in.
//!
//! An equality prefix here fixes `archivedAt` to a value: the Rust SDK at 0.1.0-beta.1 takes no
//! unset value in `Query::equal`, so the reads of unarchived tickets wait for NoCentralDB.

mod common;

use common::block_on;
use nexus_flow_store::model;
use nocentraldb::{Db, ObjectId, OpenOptions, Reference, Space, Value};

const ARCHIVED: &str = "2026-11-24T12:00:00Z";

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

struct Tickets {
    db: Db,
    space: Space,
    /// An open epic, unarchived, updated first.
    epic: ObjectId,
    /// A closed and archived feature in `epic`, updated last.
    feature: ObjectId,
}

/// A space with two tickets: an epic, and a feature in it that depends on it.
fn tickets() -> Tickets {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    let epic = block_on(space.create(
        "ticketsTicket",
        [
            ("key", Value::from("ka.1")),
            ("type", Value::from("epic")),
            ("title", Value::from("Tickets in NoCentralDB")),
            ("status", Value::from("open")),
            ("priority", Value::from(1)),
            ("assignee", Value::from("account:owner")),
            ("createdAt", Value::from("2026-11-24T09:00:00Z")),
            ("createdBy", Value::from("m1")),
            ("updatedAt", Value::from("2026-11-24T09:00:00Z")),
            ("updatedBy", Value::from("m1")),
        ],
    ))
    .expect("ticketsTicket is a collection of the model")
    .ids[0];
    let feature = block_on(space.create(
        "ticketsTicket",
        [
            ("key", Value::from("ka.2")),
            ("type", Value::from("feature")),
            ("title", Value::from("Declare the ticket collections")),
            ("description", Value::from("Why")),
            ("design", Value::from("How")),
            ("completionCriterion", Value::from("Done when")),
            ("status", Value::from("closed")),
            ("resolution", Value::from("done")),
            ("closingComment", Value::from("Merged")),
            ("closedAt", Value::from("2026-11-24T11:00:00Z")),
            ("archivedAt", Value::from(ARCHIVED)),
            ("due", Value::from("2026-12-01T00:00:00Z")),
            ("deferUntil", Value::from("2026-11-25T00:00:00Z")),
            ("assignee", Value::from("member:m2")),
            ("parent", Value::from(epic)),
            ("dependsOn", Value::add([epic])),
            ("contributesTo", Value::add([epic])),
            ("mentions", Value::add([epic])),
            ("labels", Value::add(["rfc"])),
            ("createdAt", Value::from("2026-11-24T10:00:00Z")),
            ("createdBy", Value::from("m2")),
            ("updatedAt", Value::from("2026-11-24T12:00:00Z")),
            ("updatedBy", Value::from("m2")),
        ],
    ))
    .unwrap()
    .ids[0];
    Tickets {
        db,
        space,
        epic,
        feature,
    }
}

#[test]
fn a_ticket_is_written_with_every_field_of_the_rfc_and_read_back() {
    let t = tickets();
    let replacement = block_on(t.space.create(
        "ticketsTicket",
        [
            ("key", Value::from("ka.3")),
            ("type", Value::from("question")),
            ("status", Value::from("closed")),
            ("resolution", Value::from("replaced")),
            ("replacedBy", Value::from(t.feature)),
        ],
    ))
    .unwrap()
    .ids[0];
    let feature = block_on(t.space.get("ticketsTicket", t.feature))
        .unwrap()
        .unwrap();
    assert_eq!(feature.get::<&str>("key").unwrap(), Some("ka.2"));
    assert_eq!(feature.get::<&str>("resolution").unwrap(), Some("done"));
    assert_eq!(feature.get::<ObjectId>("parent").unwrap(), Some(t.epic));
    for field in ["dependsOn", "contributesTo", "mentions"] {
        assert_eq!(
            feature.get::<&[Value]>(field).unwrap(),
            Some([Value::from(t.epic)].as_slice()),
            "{field}"
        );
    }
    assert_eq!(
        feature.get::<&[Value]>("labels").unwrap(),
        Some([Value::from("rfc")].as_slice())
    );
    let epic = block_on(t.space.get("ticketsTicket", t.epic))
        .unwrap()
        .unwrap();
    assert_eq!(epic.get::<i64>("priority").unwrap(), Some(1));
    assert_eq!(epic.get::<&str>("archivedAt").unwrap(), None);
    let replaced = block_on(t.space.get("ticketsTicket", replacement))
        .unwrap()
        .unwrap();
    assert_eq!(
        replaced.get::<ObjectId>("replacedBy").unwrap(),
        Some(t.feature)
    );
}

#[test]
fn a_ticket_is_read_through_each_of_its_indexes() {
    let t = tickets();
    let (db, space, epic, feature) = (&t.db, &t.space, t.epic, t.feature);
    let ticket = |index: &str, equal: Vec<Value>| found(db, space, "ticketsTicket", index, equal);

    assert_eq!(ticket("byKey", vec!["ka.2".into()]), [feature]);
    assert_eq!(
        ticket("byStatus", vec!["closed".into(), ARCHIVED.into()]),
        [feature]
    );
    assert_eq!(
        ticket("byClosedAt", vec!["closed".into(), ARCHIVED.into()]),
        [feature]
    );
    assert_eq!(ticket("byClosedAtAll", vec!["closed".into()]), [feature]);
    assert_eq!(
        ticket("byArchivedAt", vec![]).first(),
        Some(&feature),
        "archived tickets first, newest first"
    );
    assert_eq!(ticket("byParent", vec![epic.into()]), [feature]);
    assert_eq!(
        ticket("byAssignee", vec!["member:m2".into(), "closed".into()]),
        [feature]
    );
    assert_eq!(
        ticket("byAssignee", vec!["account:owner".into(), "open".into()]),
        [epic]
    );
    assert_eq!(
        ticket(
            "byType",
            vec!["feature".into(), "closed".into(), ARCHIVED.into()]
        ),
        [feature]
    );
    assert_eq!(
        ticket("byUpdatedAt", vec![]),
        [feature, epic],
        "the last written first"
    );
}

#[test]
fn a_note_is_read_through_its_ticket() {
    let t = tickets();
    let note = block_on(t.space.create(
        "ticketsNote",
        [
            ("ticket", Value::from(t.feature)),
            ("body", Value::from("Answered.")),
            ("createdAt", Value::from("2026-11-24T10:30:00Z")),
            ("createdBy", Value::from("m2")),
        ],
    ))
    .expect("ticketsNote is a collection of the model")
    .ids[0];
    let reply = block_on(t.space.create(
        "ticketsNote",
        [
            ("ticket", Value::from(t.feature)),
            ("body", Value::from("Thanks.")),
            ("replyTo", Value::from(note)),
            ("createdAt", Value::from("2026-11-24T10:40:00Z")),
            ("createdBy", Value::from("m1")),
        ],
    ))
    .unwrap()
    .ids[0];

    assert_eq!(
        found(
            &t.db,
            &t.space,
            "ticketsNote",
            "byTicket",
            vec![t.feature.into()]
        ),
        [note, reply],
        "oldest first"
    );
    let read = block_on(t.space.get("ticketsNote", reply))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<ObjectId>("replyTo").unwrap(), Some(note));
}

#[test]
fn a_work_link_is_read_through_its_ticket_branch_and_url() {
    let t = tickets();
    let url = "https://github.com/nxsflow/nexus-flow/pull/1";
    let work = block_on(t.space.create(
        "ticketsWork",
        [
            ("ticket", Value::from(t.feature)),
            ("kind", Value::from("pr")),
            ("url", Value::from(url)),
            ("branch", Value::from("ka.2-declare-the-ticket-collections")),
            ("state", Value::from("merged")),
            ("commit", Value::from("c0afa2f")),
            ("createdAt", Value::from("2026-11-24T10:50:00Z")),
            ("createdBy", Value::from("m2")),
        ],
    ))
    .expect("ticketsWork is a collection of the model")
    .ids[0];

    let work_by =
        |index: &str, equal: Vec<Value>| found(&t.db, &t.space, "ticketsWork", index, equal);
    assert_eq!(work_by("byTicket", vec![t.feature.into()]), [work]);
    assert_eq!(
        work_by(
            "byBranch",
            vec!["ka.2-declare-the-ticket-collections".into()]
        ),
        [work]
    );
    assert_eq!(work_by("byUrl", vec![url.into()]), [work]);
}

#[test]
fn a_reference_is_read_through_its_ticket_and_its_target() {
    let t = tickets();
    let elsewhere = block_on(t.db.spaces().create()).unwrap();
    let cited = block_on(elsewhere.create(
        "ticketsTicket",
        [("key", Value::from("ka.12")), ("type", Value::from("bug"))],
    ))
    .unwrap()
    .ids[0];
    let target = Reference::new(elsewhere.id(), cited);
    let reference = block_on(t.space.create(
        "ticketsReference",
        [
            ("ticket", Value::from(t.feature)),
            ("kind", Value::from("cites")),
            ("target", Value::from(target)),
            ("label", Value::from("nocentraldb ka.12")),
            ("createdAt", Value::from("2026-11-24T10:55:00Z")),
            ("createdBy", Value::from("m2")),
        ],
    ))
    .expect("ticketsReference is a collection of the model")
    .ids[0];

    let refs =
        |index: &str, equal: Vec<Value>| found(&t.db, &t.space, "ticketsReference", index, equal);
    assert_eq!(refs("byTicket", vec![t.feature.into()]), [reference]);
    assert_eq!(refs("byTarget", vec![target.into()]), [reference]);
    let read = block_on(t.space.get("ticketsReference", reference))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<Reference>("target").unwrap(), Some(target));
}

/// RFC 0002, "Ticket ids": a member's ticket tag is a field of its role record.
#[test]
fn a_role_record_carries_the_members_ticket_tag() {
    let t = tickets();
    let record = block_on(t.space.create(
        "spaceMember",
        [
            ("member", Value::from("m2")),
            ("role", Value::from("coder")),
            ("tag", Value::from("ka")),
            (
                "ticket",
                Value::from(Reference::new(t.space.id(), t.feature)),
            ),
        ],
    ))
    .unwrap()
    .ids[0];
    let read = block_on(t.space.get("spaceMember", record))
        .unwrap()
        .unwrap();
    assert_eq!(read.get::<&str>("tag").unwrap(), Some("ka"));
}
