//! The status module's collections and indexes (RFC 0007, "The collections", "Health" and
//! "Indexes"): `statusNote`, `statusItem` and `statusGoal`, every index but one in one space, and
//! `statusNote.byDateAcrossSpaces` across every space of the member.

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

/// A note of `date`, gathered at `until`.
fn note(space: &Space, date: &str, until: &str) -> ObjectId {
    create(
        space,
        "statusNote",
        vec![
            ("date", Value::from(date)),
            ("timeZone", Value::from("Europe/Berlin")),
            ("until", Value::from(until)),
            ("headline", Value::from(format!("The note of {date}"))),
        ],
    )
}

/// A line of `note` of `kind` at `position`, with `extra` fields.
fn item(
    space: &Space,
    note: ObjectId,
    kind: &str,
    position: i64,
    date: &str,
    extra: Vec<(&str, Value)>,
) -> ObjectId {
    let mut fields = vec![
        ("note", Value::from(note)),
        ("kind", Value::from(kind)),
        ("position", Value::from(position)),
        ("text", Value::from(format!("{kind} {position}"))),
        ("date", Value::from(date)),
    ];
    fields.extend(extra);
    create(space, "statusItem", fields)
}

fn open() -> (Db, Space) {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    (db, space)
}

#[test]
fn a_note_is_written_with_every_field_of_the_rfc_and_read_back() {
    let (_db, space) = open();
    let earlier = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let id = create(
        &space,
        "statusNote",
        vec![
            ("date", Value::from("2026-11-20")),
            ("timeZone", Value::from("Europe/Berlin")),
            ("since", Value::from("2026-11-19T05:00:00Z")),
            ("until", Value::from("2026-11-20T05:10:00Z")),
            ("repository", Value::from("nexus-flow")),
            ("health", Value::from("atRisk")),
            ("healthComputed", Value::from("onTrack")),
            ("healthWhy", Value::from("The relay is down")),
            ("healthBasis", Value::from("noGoalInFocus")),
            ("headline", Value::from("Two PRs merged, one waits")),
            ("body", Value::from("What **changed**.")),
            ("corrects", Value::from(earlier)),
            ("author", Value::from("9b2d")),
        ],
    );
    let read = block_on(space.get("statusNote", id)).unwrap().unwrap();
    for (field, want) in [
        ("date", "2026-11-20"),
        ("timeZone", "Europe/Berlin"),
        ("since", "2026-11-19T05:00:00Z"),
        ("until", "2026-11-20T05:10:00Z"),
        ("repository", "nexus-flow"),
        ("health", "atRisk"),
        ("healthComputed", "onTrack"),
        ("healthWhy", "The relay is down"),
        ("healthBasis", "noGoalInFocus"),
        ("headline", "Two PRs merged, one waits"),
        ("body", "What **changed**."),
        ("author", "9b2d"),
    ] {
        assert_eq!(read.get::<&str>(field).unwrap(), Some(want), "{field}");
    }
    assert_eq!(read.get::<ObjectId>("corrects").unwrap(), Some(earlier));
}

/// `health` and `healthComputed` take the three healths, `healthBasis` the three reasons the rule
/// computed none; any other value is refused.
#[test]
fn a_notes_health_takes_the_values_of_the_rfc_and_no_other() {
    let (_db, space) = open();
    for health in ["onTrack", "atRisk", "offTrack"] {
        for field in ["health", "healthComputed"] {
            create(&space, "statusNote", vec![(field, Value::from(health))]);
        }
    }
    for basis in ["noCoordinationSpace", "noGoalInFocus", "noWorkForGoals"] {
        create(
            &space,
            "statusNote",
            vec![("healthBasis", Value::from(basis))],
        );
    }
    for (field, value) in [
        ("health", "noWork"),
        ("healthComputed", "green"),
        ("healthBasis", "noWork"),
    ] {
        let refused = block_on(space.create("statusNote", vec![(field, Value::from(value))]));
        assert!(refused.is_err(), "{field} `{value}` must be refused");
    }
}

#[test]
fn a_line_is_written_with_every_field_of_the_rfc_and_read_back() {
    let (db, space) = open();
    let note = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let ticket = create(&space, "ticketsTicket", vec![("key", Value::from("ka.42"))]);
    let decision = create(
        &space,
        "decisionsDecision",
        vec![("handle", Value::from("2026-11-20-NEX-1-a-call"))],
    );
    let coordination = block_on(db.spaces().create()).unwrap();
    let goal = create(
        &coordination,
        "goalsGoal",
        vec![("key", Value::from("cb.1"))],
    );
    let elsewhere = Reference::new(coordination.id(), goal);
    let id = item(
        &space,
        note,
        "waits",
        2,
        "2026-11-20",
        vec![
            ("ticket", Value::from(ticket)),
            ("decision", Value::from(decision)),
            ("pr", Value::from("https://github.com/o/r/pull/160")),
            ("elsewhere", Value::from(elsewhere)),
            ("label", Value::from("ka.42")),
        ],
    );
    let read = block_on(space.get("statusItem", id)).unwrap().unwrap();
    assert_eq!(read.get::<ObjectId>("note").unwrap(), Some(note));
    assert_eq!(read.get::<&str>("kind").unwrap(), Some("waits"));
    assert_eq!(read.get::<i64>("position").unwrap(), Some(2));
    assert_eq!(read.get::<&str>("text").unwrap(), Some("waits 2"));
    assert_eq!(read.get::<ObjectId>("ticket").unwrap(), Some(ticket));
    assert_eq!(read.get::<ObjectId>("decision").unwrap(), Some(decision));
    assert_eq!(
        read.get::<&str>("pr").unwrap(),
        Some("https://github.com/o/r/pull/160")
    );
    assert_eq!(read.get::<Reference>("elsewhere").unwrap(), Some(elsewhere));
    assert_eq!(read.get::<&str>("label").unwrap(), Some("ka.42"));
    assert_eq!(read.get::<&str>("date").unwrap(), Some("2026-11-20"));
}

/// The five kinds of RFC 0007, "statusItem"; any other is refused.
#[test]
fn a_line_takes_the_five_kinds_and_no_other() {
    let (_db, space) = open();
    let note = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    for kind in ["changed", "works", "waits", "decided", "blocked"] {
        item(&space, note, kind, 1, "2026-11-20", vec![]);
    }
    let refused = block_on(space.create("statusItem", vec![("kind", Value::from("shipped"))]));
    assert!(refused.is_err(), "kind `shipped` must be refused");
}

/// `elsewhere` names the collections that exist of RFC 0007's three: a goal in the coordination
/// space and another repository's ticket. `requestsRequest` is added when RFC 0009 declares it.
#[test]
fn a_line_refers_to_a_goal_or_another_repositorys_ticket() {
    let (db, space) = open();
    let note = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let coordination = block_on(db.spaces().create()).unwrap();
    let other = block_on(db.spaces().create()).unwrap();
    let goal = create(
        &coordination,
        "goalsGoal",
        vec![("key", Value::from("cb.1"))],
    );
    let ticket = create(&other, "ticketsTicket", vec![("key", Value::from("ab.7"))]);
    for target in [
        Reference::new(coordination.id(), goal),
        Reference::new(other.id(), ticket),
    ] {
        let id = item(
            &space,
            note,
            "changed",
            1,
            "2026-11-20",
            vec![("elsewhere", Value::from(target))],
        );
        let read = block_on(space.get("statusItem", id)).unwrap().unwrap();
        assert_eq!(read.get::<Reference>("elsewhere").unwrap(), Some(target));
    }
}

#[test]
fn a_goals_verdict_is_written_with_every_field_of_the_rfc_and_read_back() {
    let (db, space) = open();
    let note = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let coordination = block_on(db.spaces().create()).unwrap();
    let goal = create(
        &coordination,
        "goalsGoal",
        vec![("key", Value::from("cb.1"))],
    );
    let target = Reference::new(coordination.id(), goal);
    const COUNTS: [&str; 8] = [
        "open",
        "inProgress",
        "ready",
        "blocked",
        "doneInPeriod",
        "overdue",
        "flagged",
        "waitingForOwner",
    ];
    let mut fields = vec![
        ("note", Value::from(note)),
        ("goal", Value::from(target)),
        (
            "label",
            Value::from("cb.1 Win 20 paying customers for nexflow.it"),
        ),
        ("position", Value::from(1i64)),
        ("verdict", Value::from("atRisk")),
        ("date", Value::from("2026-11-20")),
    ];
    for (n, count) in COUNTS.iter().enumerate() {
        fields.push((count, Value::from(n as i64 + 1)));
    }
    let id = create(&space, "statusGoal", fields);
    let read = block_on(space.get("statusGoal", id)).unwrap().unwrap();
    assert_eq!(read.get::<ObjectId>("note").unwrap(), Some(note));
    assert_eq!(read.get::<Reference>("goal").unwrap(), Some(target));
    assert_eq!(
        read.get::<&str>("label").unwrap(),
        Some("cb.1 Win 20 paying customers for nexflow.it")
    );
    assert_eq!(read.get::<i64>("position").unwrap(), Some(1));
    assert_eq!(read.get::<&str>("verdict").unwrap(), Some("atRisk"));
    assert_eq!(read.get::<&str>("date").unwrap(), Some("2026-11-20"));
    for (n, count) in COUNTS.iter().enumerate() {
        assert_eq!(
            read.get::<i64>(count).unwrap(),
            Some(n as i64 + 1),
            "{count}"
        );
    }
}

/// The four verdicts of RFC 0007, "Health"; any other is refused.
#[test]
fn a_goals_verdict_takes_the_four_verdicts_and_no_other() {
    let (_db, space) = open();
    for verdict in ["onTrack", "atRisk", "offTrack", "noWork"] {
        create(
            &space,
            "statusGoal",
            vec![("verdict", Value::from(verdict))],
        );
    }
    let refused = block_on(space.create("statusGoal", vec![("verdict", Value::from("stalled"))]));
    assert!(refused.is_err(), "verdict `stalled` must be refused");
}

/// `statusNote.byDate`: `date` desc, then `until` desc, in one space.
#[test]
fn the_notes_of_a_space_are_found_newest_day_first() {
    let (db, space) = open();
    let older = note(&space, "2026-11-19", "2026-11-19T05:00:00Z");
    let first = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let correction = note(&space, "2026-11-20", "2026-11-20T09:00:00Z");
    let newest = note(&space, "2026-11-21", "2026-11-21T05:00:00Z");

    assert_eq!(
        found(&db, &space, "statusNote", "byDate", vec![]),
        [newest, correction, first, older]
    );
}

/// `statusNote.byDateAcrossSpaces`: `date` desc, then `until` desc, across every space of the
/// member: the notes of two repository spaces come back in one ordered page, each with its space.
#[test]
fn the_notes_of_two_repository_spaces_are_found_in_one_ordered_page() {
    let (db, one) = open();
    let two = block_on(db.spaces().create()).unwrap();
    let one_older = note(&one, "2026-11-19", "2026-11-19T05:00:00Z");
    let two_older = note(&two, "2026-11-19", "2026-11-19T06:00:00Z");
    let one_newer = note(&one, "2026-11-20", "2026-11-20T05:00:00Z");
    let two_newer = note(&two, "2026-11-20", "2026-11-20T04:00:00Z");

    let query = db.index("statusNote", "byDateAcrossSpaces").unwrap();
    let page = block_on(db.find(query)).expect("byDateAcrossSpaces spans every space");
    let read: Vec<(ObjectId, _)> = page
        .items
        .iter()
        .map(|object| (object.id(), object.space()))
        .collect();
    assert_eq!(
        read,
        [
            (one_newer, one.id()),
            (two_newer, two.id()),
            (two_older, two.id()),
            (one_older, one.id()),
        ]
    );

    let in_one_space = db.index("statusNote", "byDateAcrossSpaces").unwrap();
    assert!(
        block_on(one.find(in_one_space)).is_err(),
        "byDateAcrossSpaces is declared without scope: \"space\""
    );
}

/// `statusNote.byCorrects`: `corrects`, which says whether a note is corrected and by which.
#[test]
fn the_note_that_corrects_a_note_is_found() {
    let (db, space) = open();
    let corrected = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let correction = create(
        &space,
        "statusNote",
        vec![
            ("date", Value::from("2026-11-20")),
            ("until", Value::from("2026-11-20T09:00:00Z")),
            ("corrects", Value::from(corrected)),
        ],
    );
    let by = |id: ObjectId| found(&db, &space, "statusNote", "byCorrects", vec![id.into()]);
    assert_eq!(by(corrected), [correction]);
    assert_eq!(by(correction), []);
}

/// `statusItem.byNote`: `note`, then `kind`, then `position`.
#[test]
fn a_notes_lines_are_found_by_kind_and_position() {
    let (db, space) = open();
    let wanted = note(&space, "2026-11-20", "2026-11-20T05:00:00Z");
    let other = note(&space, "2026-11-21", "2026-11-21T05:00:00Z");
    let waits_2 = item(&space, wanted, "waits", 2, "2026-11-20", vec![]);
    let changed_1 = item(&space, wanted, "changed", 1, "2026-11-20", vec![]);
    let waits_1 = item(&space, wanted, "waits", 1, "2026-11-20", vec![]);
    item(&space, other, "changed", 1, "2026-11-21", vec![]);

    assert_eq!(
        found(&db, &space, "statusItem", "byNote", vec![wanted.into()]),
        [changed_1, waits_1, waits_2]
    );
}

/// `statusItem.byTicket` and `statusItem.byDecision`: the lines about one record, newest first.
#[test]
fn the_lines_about_a_ticket_or_a_decision_are_found_newest_first() {
    let (db, space) = open();
    let ticket = create(&space, "ticketsTicket", vec![("key", Value::from("ka.42"))]);
    let decision = create(
        &space,
        "decisionsDecision",
        vec![("handle", Value::from("2026-11-20-NEX-1-a-call"))],
    );
    let monday = note(&space, "2026-11-23", "2026-11-23T05:00:00Z");
    let tuesday = note(&space, "2026-11-24", "2026-11-24T05:00:00Z");
    let about = |note, date, field: &'static str, target: ObjectId| {
        item(
            &space,
            note,
            "changed",
            1,
            date,
            vec![(field, Value::from(target))],
        )
    };
    let ticket_monday = about(monday, "2026-11-23", "ticket", ticket);
    let ticket_tuesday = about(tuesday, "2026-11-24", "ticket", ticket);
    let decision_monday = about(monday, "2026-11-23", "decision", decision);

    assert_eq!(
        found(&db, &space, "statusItem", "byTicket", vec![ticket.into()]),
        [ticket_tuesday, ticket_monday]
    );
    assert_eq!(
        found(
            &db,
            &space,
            "statusItem",
            "byDecision",
            vec![decision.into()]
        ),
        [decision_monday]
    );
}

/// `statusGoal.byNote` (`note`, then `position`) and `statusGoal.byGoal` (`goal`, then `date`
/// desc).
#[test]
fn a_notes_goals_and_a_goals_verdicts_are_found_in_their_order() {
    let (db, space) = open();
    let coordination = block_on(db.spaces().create()).unwrap();
    let goal = |key: &str| {
        let id = create(&coordination, "goalsGoal", vec![("key", Value::from(key))]);
        Reference::new(coordination.id(), id)
    };
    let (first, second) = (goal("cb.1"), goal("cb.2"));
    let monday = note(&space, "2026-11-23", "2026-11-23T05:00:00Z");
    let tuesday = note(&space, "2026-11-24", "2026-11-24T05:00:00Z");
    let verdict = |note: ObjectId, goal: Reference, position: i64, date: &str| {
        create(
            &space,
            "statusGoal",
            vec![
                ("note", Value::from(note)),
                ("goal", Value::from(goal)),
                ("position", Value::from(position)),
                ("verdict", Value::from("onTrack")),
                ("date", Value::from(date)),
            ],
        )
    };
    let monday_second = verdict(monday, second, 2, "2026-11-23");
    let monday_first = verdict(monday, first, 1, "2026-11-23");
    let tuesday_first = verdict(tuesday, first, 1, "2026-11-24");

    assert_eq!(
        found(&db, &space, "statusGoal", "byNote", vec![monday.into()]),
        [monday_first, monday_second]
    );
    assert_eq!(
        found(&db, &space, "statusGoal", "byGoal", vec![first.into()]),
        [tuesday_first, monday_first]
    );
}
