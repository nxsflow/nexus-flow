//! NEX-177: the op log, read in v1's order without changing a byte (RFC 0008, "How the ops are
//! replayed").

mod fixture;

use fixture::{Fixture, Op};
use nexus_flow_migrate::v1_reader::V1Workspace;

fn ids(ws: &V1Workspace) -> Vec<String> {
    ws.ops().unwrap().into_iter().map(|op| op.op_id).collect()
}

#[test]
fn ops_come_in_lamport_then_site_order() {
    let f = Fixture::v1();
    f.insert(&[
        Op::title("c", 3, 1),
        Op::title("b", 2, 2),
        Op::title("a", 2, 1),
        Op::title("d", 1, 9),
    ]);
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ids(&ws), ["d", "a", "b", "c"]);
}

/// Three of twenty real workspaces hold ops with the same `(lamport, site)`; the tie is broken by
/// `op_id`, compared bytewise, whatever order the rows were written in.
#[test]
fn duplicate_lamport_and_site_order_by_op_id() {
    let f = Fixture::v1();
    f.insert(&[
        Op::title("op-b", 5, 1),
        Op::title("op-B", 5, 1),
        Op::title("op-a", 5, 1),
        Op::title("op-c", 4, 2),
    ]);
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ids(&ws), ["op-c", "op-B", "op-a", "op-b"]);
}

#[test]
fn an_op_carries_every_column_of_the_log() {
    let f = Fixture::v1();
    f.insert(&[
        Op::title("full", 7, 3),
        Op {
            value: None,
            author: None,
            wall_clock: None,
            key_id: None,
            sig: None,
            provenance: "unsigned",
            domain: "fact",
            target_kind: "fact",
            target_id: "f-1",
            field: "body",
            op_type: "forget",
            ..Op::title("bare", 8, 3)
        },
    ]);
    let ops = V1Workspace::open(&f.nxs()).unwrap().ops().unwrap();
    let full = &ops[0];
    assert_eq!(full.op_id, "full");
    assert_eq!((full.lamport, full.site), (7, 3));
    assert_eq!(full.domain, "task");
    assert_eq!(full.target_kind, "item");
    assert_eq!(full.target_id, "6j6v.pzkb");
    assert_eq!(full.field, "title");
    assert_eq!(full.op_type, "set");
    assert_eq!(full.value.as_deref(), Some("\"A title\""));
    assert_eq!(full.author.as_deref(), Some("carsten"));
    assert_eq!(full.wall_clock.as_deref(), Some("2026-11-20T21:00:00Z"));
    assert_eq!(full.key_id.as_deref(), Some("k1"));
    assert_eq!(full.sig.as_deref(), Some("c2ln"));
    assert_eq!(full.provenance, "own");

    let bare = &ops[1];
    assert_eq!(bare.domain, "fact");
    assert_eq!(bare.op_type, "forget");
    assert_eq!(bare.value, None);
    assert_eq!(bare.author, None);
    assert_eq!(bare.wall_clock, None);
    assert_eq!(bare.key_id, None);
    assert_eq!(bare.sig, None);
    assert_eq!(bare.provenance, "unsigned");
}

/// Opening and reading every op leaves the workspace as it was: the database byte for byte, and
/// no file added beside it (no `-wal`, `-shm` or `-journal`).
#[test]
fn reading_leaves_every_byte_of_the_workspace_as_it_was() {
    let f = Fixture::v1();
    f.insert(&[Op::title("a", 1, 1), Op::title("b", 2, 1)]);
    let before = f.snapshot();
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ws.ops().unwrap().len(), 2);
    drop(ws);
    assert_eq!(f.snapshot(), before);
}
