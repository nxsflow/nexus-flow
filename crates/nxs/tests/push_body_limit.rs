//! The push body ceiling as the REAL relay enforces it and the real transport meets it (6j6v.3gq0):
//! a body of exactly `MAX_PUSH_BYTES` goes through, one byte more is refused — and the transport
//! hands that refusal to the engine as the one failure it splits on, not as an opaque error.

use std::sync::Arc;

use nxs_sync::engine::{HttpTransport, Transport};
use nxs_sync::protocol::{PushRequest, StreamId, MAX_PUSH_BYTES};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

fn serve(router: axum::Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
    });
    format!("http://{addr}")
}

fn op(op_id: &str, value: String) -> WireOp {
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
        value: Some(value),
        author: "a".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

/// An op whose push body, sent alone, is exactly `bytes` long.
fn op_with_body_of(op_id: &str, bytes: usize) -> WireOp {
    let body = |o: &WireOp| {
        serde_json::to_vec(&PushRequest {
            ops: vec![o.clone()],
        })
        .unwrap()
        .len()
    };
    let empty = op(op_id, String::new());
    let o = op(op_id, "x".repeat(bytes - body(&empty)));
    assert_eq!(body(&o), bytes);
    o
}

#[test]
fn the_relay_takes_a_body_of_exactly_the_ceiling_and_refuses_one_byte_more_as_too_large() {
    let base = serve(nxs_server::app::app(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    ));
    let transport = HttpTransport::new(base);
    let stream = StreamId("s".into());

    transport
        .push(&stream, &[op_with_body_of("at", MAX_PUSH_BYTES)])
        .expect("a body of exactly the ceiling is accepted");
    let refused = transport
        .push(&stream, &[op_with_body_of("over", MAX_PUSH_BYTES + 1)])
        .expect_err("one byte more is refused");
    assert!(
        refused.is_too_large(),
        "the refusal reaches the engine as too large: {refused}"
    );
}
