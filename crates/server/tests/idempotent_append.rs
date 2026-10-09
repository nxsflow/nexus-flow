//! The SQLite run of the shared idempotent-append contract (`support/idempotence.rs`); Postgres runs
//! it in `postgres_parity.rs`, DynamoDB in `dynamodb_local.rs`.

use nxs_server::store::SqliteOpStore;
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

#[path = "support/idempotence.rs"]
mod idempotence;

fn wire(op_id: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "ab12.0001".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("t".into()),
        author: "a".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

#[test]
fn sqlite_appends_idempotently_by_op_id() {
    let store = SqliteOpStore::open_in_memory().unwrap();
    idempotence::assert_idempotent_append(&store, wire, true);
}
