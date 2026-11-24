//! The store refuses to write a collection into a space of the wrong kind (RFC 0001, "The model",
//! **One model, both kinds of space.**): a ticket or a status note into a coordination space, a
//! goal or a request into a repository space. The module RFCs name the error codes: RFC 0002's
//! `not_a_ticket_space`, RFC 0007's `not_a_repository_space`.

use nexus_flow_store::space_kind::{self, SpaceKind, WrongSpace};

use SpaceKind::{AdHoc, Coordination, Repository};

/// The collections the module RFCs declare, with their kinds: the rule must already hold for the
/// collections of modules the lock does not hold yet.
const MODULES: &[(&str, &[SpaceKind])] = &[
    ("spaceMember", &[Repository, Coordination]),
    ("ticketsTicket", &[Repository]),
    ("decisionsDecision", &[Repository]),
    ("statusNote", &[Repository]),
    ("goalsGoal", &[Coordination]),
    ("requestsRequest", &[Coordination, AdHoc]),
    ("everywhere", &[Repository, Coordination, AdHoc]),
];

fn refused(collection: &str, space: Option<SpaceKind>) -> WrongSpace {
    space_kind::check_in(MODULES, collection, space)
        .expect_err(&format!("{collection} in {space:?} must be refused"))
}

#[test]
fn a_repository_collection_in_a_coordination_space_is_refused_with_the_rfcs_code() {
    let ticket = refused("ticketsTicket", Some(Coordination));
    assert_eq!(ticket.code(), Some("not_a_ticket_space"));
    assert_eq!(ticket.collection, "ticketsTicket");
    assert_eq!(ticket.space, Some(Coordination));

    let note = refused("statusNote", Some(Coordination));
    assert_eq!(note.code(), Some("not_a_repository_space"));

    let decision = refused("decisionsDecision", Some(Coordination));
    assert_eq!(decision.code(), None, "RFC 0003 names no code");
}

#[test]
fn a_coordination_collection_in_a_repository_space_is_refused() {
    let goal = refused("goalsGoal", Some(Repository));
    assert_eq!(goal.space, Some(Repository));
    assert_eq!(goal.code(), None, "RFC 0005 names no code");
    let request = refused("requestsRequest", Some(Repository));
    assert_eq!(request.code(), None, "RFC 0009 names no code");
}

#[test]
fn a_collection_in_a_kind_the_table_names_is_written() {
    for &(collection, kinds) in MODULES {
        for &kind in kinds {
            assert_eq!(
                space_kind::check_in(MODULES, collection, Some(kind)),
                Ok(()),
                "{collection} in {kind:?}"
            );
        }
    }
}

#[test]
fn an_ad_hoc_space_takes_only_copies_of_requests() {
    for collection in ["spaceMember", "ticketsTicket", "goalsGoal", "statusNote"] {
        refused(collection, Some(AdHoc));
    }
}

#[test]
fn a_space_of_unknown_kind_takes_no_kind_restricted_write() {
    for collection in [
        "spaceMember",
        "ticketsTicket",
        "goalsGoal",
        "requestsRequest",
    ] {
        let err = refused(collection, None);
        assert_eq!(err.space, None);
        assert_eq!(err.code(), None, "{collection}: no RFC names this code");
    }
    assert_eq!(space_kind::check_in(MODULES, "everywhere", None), Ok(()));
}

#[test]
fn a_collection_the_table_does_not_name_is_refused_everywhere() {
    for kind in [Some(Repository), Some(Coordination), Some(AdHoc), None] {
        refused("ticketsComment", kind);
    }
}

#[test]
fn the_space_member_record_goes_into_repository_and_coordination_spaces() {
    assert_eq!(space_kind::check("spaceMember", Some(Repository)), Ok(()));
    assert_eq!(space_kind::check("spaceMember", Some(Coordination)), Ok(()));
    assert!(space_kind::check("spaceMember", Some(AdHoc)).is_err());
    assert!(space_kind::check("spaceMember", None).is_err());
}

#[test]
fn a_refusal_names_the_collection_and_both_kinds() {
    let text = refused("goalsGoal", Some(Repository)).to_string();
    assert!(text.contains("goalsGoal"), "{text}");
    assert!(text.contains("coordination space"), "{text}");
    assert!(text.contains("repository space"), "{text}");
    let unknown = refused("ticketsTicket", None).to_string();
    assert!(unknown.contains("kind"), "{unknown}");
}

/// RFC 0002, "Collections": the four ticket collections are in the repository space, and the
/// storage crate refuses them in a coordination or ad-hoc space.
#[test]
fn the_ticket_collections_go_into_repository_spaces_only() {
    for collection in [
        "ticketsTicket",
        "ticketsNote",
        "ticketsWork",
        "ticketsReference",
    ] {
        assert_eq!(
            space_kind::check(collection, Some(Repository)),
            Ok(()),
            "{collection}"
        );
        for kind in [Some(Coordination), Some(AdHoc), None] {
            let err = space_kind::check(collection, kind)
                .expect_err(&format!("{collection} in {kind:?} must be refused"));
            assert_eq!(err.belongs_in, [Repository], "{collection}");
        }
        let err = space_kind::check(collection, Some(Coordination)).unwrap_err();
        assert_eq!(err.code(), Some("not_a_ticket_space"), "{collection}");
    }
}

/// RFC 0003, "Where decisions live": the four decision collections are in the repository space,
/// and the storage crate refuses them in a coordination or ad-hoc space. RFC 0003 names no code.
#[test]
fn the_decision_collections_go_into_repository_spaces_only() {
    for collection in [
        "decisionsDecision",
        "decisionsTopic",
        "decisionsSubject",
        "decisionsSupersession",
    ] {
        assert_eq!(
            space_kind::check(collection, Some(Repository)),
            Ok(()),
            "{collection}"
        );
        for kind in [Some(Coordination), Some(AdHoc), None] {
            let err = space_kind::check(collection, kind)
                .expect_err(&format!("{collection} in {kind:?} must be refused"));
            assert_eq!(err.belongs_in, [Repository], "{collection}");
            assert_eq!(err.code(), None, "{collection}");
        }
    }
}

/// RFC 0005, "Where goals live": a goal is in the coordination space, and the storage crate
/// refuses it in a repository or ad-hoc space. RFC 0005 names no code.
#[test]
fn a_goal_goes_into_coordination_spaces_only() {
    assert_eq!(space_kind::check("goalsGoal", Some(Coordination)), Ok(()));
    for kind in [Some(Repository), Some(AdHoc), None] {
        let err = space_kind::check("goalsGoal", kind)
            .expect_err(&format!("goalsGoal in {kind:?} must be refused"));
        assert_eq!(err.belongs_in, [Coordination]);
        assert_eq!(err.code(), None);
    }
}
