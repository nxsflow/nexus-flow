//! The idempotent-append contract every `OpStore` keeps (spec §4.4, 6j6v.tm4k), as ONE script the
//! SQLite, Postgres and DynamoDB suites each run against their backend. The backends may differ in
//! one documented way only: SQLite and Postgres consume no seq for a re-push (`dense`), DynamoDB
//! burns the one it allocated.

use nxs_server::store::OpStore;
use nxs_sync::protocol::{Cursor, StreamId};
use nxs_sync::wire::WireOp;

pub fn assert_idempotent_append(store: &dyn OpStore, wire: impl Fn(&str) -> WireOp, dense: bool) {
    let s = StreamId("idempotence".into());

    let o1 = store.append(&s, &wire("o1")).unwrap();
    assert_eq!(
        store.append(&s, &wire("o1")).unwrap(),
        o1,
        "a re-push answers its own seq"
    );
    let o2 = store.append(&s, &wire("o2")).unwrap();
    assert!(o2 > o1, "a distinct op still advances the stream");
    if dense {
        assert_eq!(
            o2,
            Cursor(o1.0 + 1),
            "this backend consumes no seq for a re-push"
        );
    }
    assert_eq!(
        store.append(&s, &wire("o1")).unwrap(),
        o1,
        "an older op's answer is its own position, below the stream's end"
    );
    assert_eq!(store.append(&s, &wire("o2")).unwrap(), o2);

    let (ops, end) = store.read_since(&s, Cursor::BEGINNING, 100).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
        ["o1", "o2"],
        "each op is stored once"
    );
    assert_eq!(end, o2);
}
