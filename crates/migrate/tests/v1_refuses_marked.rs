//! NEX-180: a workspace the move has marked is refused by v1 itself: v1's `assess_open`, which every
//! v1 open runs before it writes, finds the raised header and refuses the database (RFC 0008, "The
//! marker").

mod fixture;

use fixture::Fixture;
use nexus_flow_migrate::marker::{self, Mark};
use nexus_flow_migrate::v1_reader::V1Workspace;
use nxs_foundation::schema::{assess_open, SchemaCompat};
use rusqlite::Connection;

fn mark(f: &Fixture) {
    let mut ws = V1Workspace::open(&f.nxs()).unwrap();
    ws.mark(&Mark {
        space: "space-0001".into(),
        run: "run-0001".into(),
        migrated_at: "2026-11-20T21:00:00Z".into(),
        by: "nxs 2.0.0".into(),
    })
    .unwrap();
}

fn assess(f: &Fixture) -> SchemaCompat {
    let conn = Connection::open(f.db()).unwrap();
    assess_open(&conn).unwrap()
}

#[test]
fn v1_opens_the_workspace_before_the_mark() {
    let f = Fixture::v1();
    assert_eq!(assess(&f), SchemaCompat::Current);
}

#[test]
fn v1_refuses_the_marked_workspace_as_written_by_a_newer_nxs() {
    let f = Fixture::v1();
    mark(&f);
    let verdict = assess(&f);
    assert!(
        matches!(
            verdict,
            SchemaCompat::Incompatible {
                db_version: 10,
                db_min_compatible: 10,
                ..
            }
        ),
        "v1 refuses the database, got {verdict:?}"
    );
    let refusal = verdict.refusal().expect("v1 says why");
    assert!(
        refusal.contains("written by a newer nxs"),
        "v1's own message: {refusal}"
    );
}

#[test]
fn v1_opens_the_workspace_again_after_undo() {
    let f = Fixture::v1();
    mark(&f);
    marker::undo(&f.nxs()).unwrap();
    assert_eq!(assess(&f), SchemaCompat::Current);
}
