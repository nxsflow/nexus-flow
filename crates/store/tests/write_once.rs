//! RFC 0003, "Append-only", and RFC 0008, "The migration records": the store refuses every change
//! to an existing object of the four decision collections and of the move's records, a field set,
//! a delete or a restore, for every caller. The guard is generic over a table of write-once
//! collections, so status notes can be declared write-once later without a guard of their own.

use nexus_flow_store::model;
use nexus_flow_store::write_once::{self, Change};

const DECISIONS: [&str; 4] = [
    "decisionsDecision",
    "decisionsTopic",
    "decisionsSubject",
    "decisionsSupersession",
];

const RECORDS: [&str; 1] = ["migrationRecord"];

const CHANGES: [Change; 3] = [Change::Set, Change::Delete, Change::Restore];

#[test]
fn every_change_to_an_existing_decision_object_is_refused() {
    for collection in DECISIONS {
        for change in CHANGES {
            let err = write_once::check(collection, change).unwrap_err();
            assert_eq!(err.collection, collection);
            assert_eq!(err.change, change);
            assert!(
                err.to_string().contains(collection),
                "the refusal names the collection: {err}"
            );
        }
    }
}

#[test]
fn every_change_to_an_existing_migration_record_is_refused() {
    for collection in RECORDS {
        for change in CHANGES {
            let err = write_once::check(collection, change).unwrap_err();
            assert_eq!(err.collection, collection);
            assert_eq!(err.change, change);
        }
    }
}

/// A run's `state`, `last`, `endedAt` and `report` change as the run goes on (RFC 0008, "The
/// migration records").
#[test]
fn a_collection_that_is_not_write_once_takes_every_change() {
    for change in CHANGES {
        assert_eq!(write_once::check("ticketsTicket", change), Ok(()));
        assert_eq!(write_once::check("spaceMember", change), Ok(()));
        assert_eq!(write_once::check("migrationRun", change), Ok(()));
    }
}

/// The guard a later module reuses: a table of its own collections, checked the same way.
#[test]
fn the_guard_checks_any_table_of_write_once_collections() {
    let table: &[&[&str]] = &[&["statusNote"]];
    for change in CHANGES {
        let err = write_once::check_in(table, "statusNote", change).unwrap_err();
        assert_eq!(err.collection, "statusNote");
        assert_eq!(
            write_once::check_in(table, "decisionsDecision", change),
            Ok(())
        );
    }
}

#[test]
fn every_write_once_collection_is_a_collection_of_the_model() {
    let declared: Vec<&str> = write_once::WRITE_ONCE
        .iter()
        .flat_map(|t| t.iter().copied())
        .collect();
    let expected: Vec<&str> = DECISIONS.iter().chain(&RECORDS).copied().collect();
    assert_eq!(declared, expected, "the write-once collections");
    for name in declared {
        assert!(
            model::COLLECTIONS.iter().any(|&(c, _)| c == name),
            "{name} is not a collection of the model"
        );
    }
}
