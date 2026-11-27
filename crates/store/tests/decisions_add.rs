//! RFC 0003, "Writing a decision": `decisions::add` checks a new decision, then writes it with
//! its topics, subjects and supersessions in one change of the repository space. Each refusal
//! names its problem and writes nothing. `decidedBy` defaults to the writer's role record, and
//! `author` is always the writer's member id. A handle already taken in the local copy is
//! written all the same, and `with_handle` finds both decisions.

mod common;

use std::path::PathBuf;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::decisions::{self, DecisionError, NewDecision};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::members;
use nexus_flow_store::opened::{self, Opened};
use nexus_flow_store::space::init;
use nocentraldb::{ObjectId, Reference, Value};

/// A repository space, freshly made, open on the device's database.
fn repository(test: &str) -> (PathBuf, Opened, SpaceId) {
    let base = scratch(test);
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();
    (base, opened, report.space.parse().unwrap())
}

/// Gives the device a role record naming `role` and, when `key` is given, a ticket of the space
/// with that key: what `nxs member add` writes for a member it adds.
fn role_record(opened: &Opened, space: &SpaceId, role: &str, key: Option<&str>) {
    let me = block_on(members::list(opened)).unwrap().remove(0).member;
    block_on(opened.write(space, |w| {
        let ticket = match key {
            Some(key) => {
                let id = w.create(
                    "ticketsTicket",
                    [("key", Value::from(key)), ("type", Value::from("feature"))],
                )?;
                Value::from(Reference::new(space.to_string().parse().unwrap(), id))
            }
            None => Value::clear(),
        };
        w.create(
            "spaceMember",
            [
                ("member", Value::from(me.clone())),
                ("role", Value::from(role)),
                ("ticket", ticket),
            ],
        )
        .map(drop)
    }))
    .unwrap();
}

/// A decision with everything required, decided by the owner, on 2026-11-24.
fn new(title: &str) -> NewDecision {
    NewDecision {
        title: title.to_owned(),
        why: "Because it holds".to_owned(),
        instead_of: "The option not taken".to_owned(),
        topics: vec!["decisions".to_owned()],
        date: Some("2026-11-24".to_owned()),
        decided_by: Some("Carsten".to_owned()),
        ..NewDecision::default()
    }
}

/// The ids of the objects of `collection` the index `index` finds for `equal`.
fn found(opened: &Opened, space: &SpaceId, collection: &str, index: &str, equal: &str) -> Vec<ObjectId> {
    let equal = if equal.is_empty() { vec![] } else { vec![Value::from(equal)] };
    block_on(opened.find(space, collection, index, equal))
        .unwrap()
        .iter()
        .map(|o| o.id())
        .collect()
}

#[test]
fn a_decision_is_written_with_its_topics_subjects_and_supersessions_in_one_change() {
    let (base, opened, space) = repository("decision-add");
    let older = block_on(decisions::add(&opened, new("The older rule"))).unwrap();

    let mut decision = new("The newer rule replaces the older one");
    decision.topics = vec!["decisions".to_owned(), "store".to_owned()];
    decision.tickets = vec!["NEX-127".to_owned(), "NEX-126".to_owned()];
    decision.supersedes = vec![older.handle.clone()];
    decision.pr = Some("https://github.com/nxsflow/nexus-flow/pull/222".to_owned());
    decision.body = Some("## The rule\n\nAs written.".to_owned());
    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.handle, "2026-11-24-NEX-127-the-newer-rule-replaces-the-older-one");
    assert_eq!(added.tickets, "NEX-127, NEX-126");
    assert_eq!(added.topics, ["decisions", "store"]);
    assert_eq!(added.supersedes.len(), 1);
    assert_eq!(added.supersedes[0].handle, older.handle);
    assert_eq!(added.supersedes[0].id, older.id);
    assert_eq!(added.pr.as_deref(), Some("https://github.com/nxsflow/nexus-flow/pull/222"));
    assert_eq!(added.body, "## The rule\n\nAs written.");
    let me = block_on(members::list(&opened)).unwrap().remove(0).member;
    assert_eq!(added.author, me, "the writer's member id");

    let id: ObjectId = added.id.parse().unwrap();
    let read = block_on(opened.get(&space, "decisionsDecision", id)).unwrap().unwrap();
    for (field, value) in [
        ("handle", added.handle.as_str()),
        ("date", "2026-11-24"),
        ("tickets", "NEX-127, NEX-126"),
        ("title", "The newer rule replaces the older one"),
        ("decidedBy", "Carsten"),
        ("why", "Because it holds"),
        ("insteadOf", "The option not taken"),
        ("body", "## The rule\n\nAs written."),
        ("pr", "https://github.com/nxsflow/nexus-flow/pull/222"),
        ("author", me.as_str()),
    ] {
        assert_eq!(read.get::<&str>(field).unwrap(), Some(value), "{field}");
    }
    assert_eq!(found(&opened, &space, "decisionsTopic", "byTopic", "store").len(), 1);
    assert_eq!(found(&opened, &space, "decisionsTopic", "byTopic", "decisions").len(), 2);
    assert_eq!(found(&opened, &space, "decisionsSubject", "byLabel", "NEX-126").len(), 1);
    let superseded = found(
        &opened,
        &space,
        "decisionsSupersession",
        "bySuperseded",
        "",
    );
    assert!(superseded.is_empty(), "bySuperseded takes the older decision");
    let supersessions = block_on(opened.find(
        &space,
        "decisionsSupersession",
        "bySuperseded",
        vec![Value::from(older.id.parse::<ObjectId>().unwrap())],
    ))
    .unwrap();
    assert_eq!(supersessions.len(), 1);
    assert_eq!(supersessions[0].get::<ObjectId>("decision").unwrap(), Some(id));
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_range_label_becomes_one_subject_per_ticket_and_tickets_keeps_the_text() {
    let (base, opened, space) = repository("decision-range");
    let mut decision = new("A range of tickets");
    decision.tickets = vec!["NEX-6 to NEX-13".to_owned()];

    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.tickets, "NEX-6 to NEX-13");
    assert_eq!(added.handle, "2026-11-24-NEX-6-a-range-of-tickets");
    let labels: Vec<&str> = added.subjects.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["NEX-6", "NEX-7", "NEX-8", "NEX-9", "NEX-10", "NEX-11", "NEX-12", "NEX-13"]
    );
    for label in labels {
        assert_eq!(
            found(&opened, &space, "decisionsSubject", "byLabel", label).len(),
            1,
            "{label} is a subject"
        );
    }
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_slug_given_replaces_the_one_from_the_title() {
    let (base, opened, _) = repository("decision-slug");
    let mut decision = new("A title far longer than the slug the writer wants");
    decision.slug = Some("short".to_owned());

    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.handle, "2026-11-24-no-ticket-short");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn decided_by_defaults_to_the_writers_role_and_ticket() {
    let (base, opened, space) = repository("decision-role");
    role_record(&opened, &space, "coder", Some("ka.42"));
    let mut decision = new("A coder's call");
    decision.decided_by = None;

    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.decided_by, "coder on ka.42");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn decided_by_defaults_to_the_role_alone_without_a_ticket() {
    let (base, opened, space) = repository("decision-role-pm");
    role_record(&opened, &space, "pm", None);
    let mut decision = new("A pm's call");
    decision.decided_by = None;

    assert_eq!(block_on(decisions::add(&opened, decision)).unwrap().decided_by, "pm");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_taken_handle_is_written_all_the_same_and_both_are_found_by_it() {
    let (base, opened, _) = repository("decision-taken");
    let first = block_on(decisions::add(&opened, new("The same title"))).unwrap();

    let second = block_on(decisions::add(&opened, new("The same title"))).unwrap();

    assert_eq!(second.handle, first.handle);
    assert_ne!(second.id, first.id);
    let mut taken = block_on(decisions::with_handle(&opened, &first.handle)).unwrap();
    taken.sort();
    let mut both = vec![first.id, second.id];
    both.sort();
    assert_eq!(taken, both);
    std::fs::remove_dir_all(&base).unwrap();
}

/// Each refusal, with the decision that causes it.
fn refusals(older: &str) -> Vec<(NewDecision, DecisionError)> {
    let with = |change: &dyn Fn(&mut NewDecision)| {
        let mut d = new("A decision");
        change(&mut d);
        d
    };
    let long = "x".repeat(201);
    vec![
        (with(&|d| d.title = "  ".to_owned()), DecisionError::Missing { field: "title" }),
        (with(&|d| d.why = String::new()), DecisionError::Missing { field: "why" }),
        (
            with(&|d| d.instead_of = String::new()),
            DecisionError::Missing { field: "instead of" },
        ),
        (with(&|d| d.topics.clear()), DecisionError::NoTopic),
        (
            with(&|d| d.title = "Two\nlines".to_owned()),
            DecisionError::TitleNotOneLine,
        ),
        (
            with(&|d| d.title = long.clone()),
            DecisionError::TitleTooLong { chars: 201 },
        ),
        (
            with(&|d| d.topics = vec!["Store".to_owned()]),
            DecisionError::NotATopic { text: "Store".to_owned() },
        ),
        (
            with(&|d| d.date = Some("2026-02-30".to_owned())),
            DecisionError::NotADate { text: "2026-02-30".to_owned() },
        ),
        (
            with(&|d| d.tickets = vec!["NEX-13 to NEX-6".to_owned()]),
            DecisionError::NotATicket { text: "NEX-13 to NEX-6".to_owned() },
        ),
        (
            with(&|d| d.pr = Some("http://example.com/pull/1".to_owned())),
            DecisionError::NotAnHttpsUrl { text: "http://example.com/pull/1".to_owned() },
        ),
        (
            with(&|d| d.slug = Some("Not A Slug".to_owned())),
            DecisionError::NotASlug { text: "Not A Slug".to_owned() },
        ),
        (with(&|d| d.title = "?!".to_owned()), DecisionError::NoSlug),
        (
            with(&|d| d.decided_by = Some(" ".to_owned())),
            DecisionError::Missing { field: "decided by" },
        ),
        (
            with(&|d| d.supersedes = vec!["2026-11-01-no-ticket-nothing".to_owned()]),
            DecisionError::SupersedesUnknown { key: "2026-11-01-no-ticket-nothing".to_owned() },
        ),
        (
            with(&|d| d.supersedes = vec!["00112233445566778899aabbccddeeff".to_owned()]),
            DecisionError::SupersedesUnknown { key: "00112233445566778899aabbccddeeff".to_owned() },
        ),
        (
            with(&|d| {
                d.date = Some("2026-11-23".to_owned());
                d.supersedes = vec![older.to_owned()];
            }),
            DecisionError::SupersedesLater {
                key: older.to_owned(),
                date: "2026-11-24".to_owned(),
            },
        ),
        (
            with(&|d| d.supersedes = vec!["2026-11-24-no-ticket-a-decision".to_owned()]),
            DecisionError::SupersedesItself,
        ),
        (with(&|d| d.decided_by = None), DecisionError::NoRole),
    ]
}

#[test]
fn each_refusal_names_its_problem_and_writes_nothing() {
    let (base, opened, space) = repository("decision-refused");
    let older = block_on(decisions::add(&opened, new("The older rule"))).unwrap();
    let before = found(&opened, &space, "decisionsDecision", "byDate", "");

    for (decision, expected) in refusals(&older.handle) {
        let err = block_on(decisions::add(&opened, decision)).unwrap_err();
        assert_eq!(err, expected);
        assert!(!err.to_string().is_empty());
        assert_eq!(
            found(&opened, &space, "decisionsDecision", "byDate", ""),
            before,
            "{expected:?} wrote nothing"
        );
        assert_eq!(found(&opened, &space, "decisionsTopic", "byTopic", "decisions").len(), 1);
    }
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_ambiguous_superseded_handle_is_refused_with_the_matching_ids() {
    let (base, opened, _) = repository("decision-ambiguous");
    let first = block_on(decisions::add(&opened, new("The same title"))).unwrap();
    let second = block_on(decisions::add(&opened, new("The same title"))).unwrap();
    let mut decision = new("A later one");
    decision.supersedes = vec![first.handle.clone()];

    let err = block_on(decisions::add(&opened, decision)).unwrap_err();

    let DecisionError::SupersedesAmbiguous { handle, mut ids } = err.clone() else {
        panic!("{err:?}");
    };
    ids.sort();
    let mut both = vec![first.id.clone(), second.id.clone()];
    both.sort();
    assert_eq!((handle, ids), (first.handle.clone(), both));
    let text = err.to_string();
    assert!(text.contains(&first.id) && text.contains(&second.id), "{text}");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_decision_supersedes_another_by_its_object_id() {
    let (base, opened, _) = repository("decision-by-id");
    let older = block_on(decisions::add(&opened, new("The older rule"))).unwrap();
    let mut decision = new("The newer rule");
    decision.supersedes = vec![older.id.clone()];

    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.supersedes[0].handle, older.handle);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_date_defaults_to_today_in_utc() {
    let (base, opened, _) = repository("decision-today");
    let mut decision = new("Made today");
    decision.date = None;

    let added = block_on(decisions::add(&opened, decision)).unwrap();

    assert_eq!(added.date, decisions::date::today());
    assert!(added.handle.starts_with(&added.date));
    std::fs::remove_dir_all(&base).unwrap();
}
