//! The Postgres backend must work **where the relay actually runs it**: inside a Tokio
//! runtime.
//!
//! `postgres_parity.rs` proves the backend is observably identical to SQLite, but it calls
//! the trait from plain `#[test]` functions — threads with no async runtime anywhere. The
//! shipped `nxf-relay` is the opposite: `main` is `#[tokio::main]`, so `backends()` runs in
//! an async context and every axum handler calls the store from one too.
//!
//! That gap hid a total failure. The synchronous `postgres` driver owns an internal runtime
//! and `block_on`s it on the calling thread; Tokio panics with *"Cannot start a runtime from
//! within a runtime"* when that happens inside async context. So
//! `NXF_RELAY_BACKEND=postgres` aborted at boot — not on a rare path, on the only path —
//! while the parity suite stayed green, because the suite never entered a runtime
//! (nexus-flow-6j6v.94gm, found while making the backend shippable for 6j6v.7nee).
//!
//! These tests pin the calling contexts that exist in production, so the fix cannot silently
//! regress:
//!
//!   * a **multi-thread** runtime — what `#[tokio::main]` builds, i.e. the real binary;
//!   * a **current-thread** runtime — what an embedder may build, and the flavor where
//!     `block_in_place` is itself illegal, so the guard has to do something else;
//!   * an **embedded relay serving real HTTP**, which is the handler path end to end.
//!
//! Gated exactly like the parity suite: the `postgres` feature plus `DATABASE_URL`, loud SKIP
//! locally when unset, hard failure under `CI` so a dropped database can never pass green.
#![cfg(feature = "postgres")]

use std::sync::Arc;
use std::thread;

use nxs_server::app::app;
use nxs_server::registry::PrefixRegistry;
use nxs_server::registry_pg::PostgresPrefixRegistry;
use nxs_server::store::OpStore;
use nxs_server::store_pg::PostgresOpStore;
use nxs_sync::protocol::{Cursor, StreamId};

mod common;

use common::{base_url, isolated_url, wire};

// ----- the calling contexts the relay really uses ------------------------------------

#[test]
fn connect_and_append_inside_a_multi_thread_runtime() {
    // This is `nxf-relay` itself: `#[tokio::main]` is multi-thread, `backends()` runs in its
    // async body, and the handlers run on its workers. Before the fix this panicked with
    // "Cannot start a runtime from within a runtime" and aborted the process.
    let Some(base) = base_url("multi_thread") else {
        return;
    };
    let url = isolated_url(&base, "rt");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("multi-thread runtime");
    rt.block_on(async move {
        let store = PostgresOpStore::connect(&url).expect("connect op store in async context");
        let registry = PostgresPrefixRegistry::connect(&url).expect("connect registry");
        let stream = StreamId("rt-multi".into());
        let cursor = store.append(&stream, &wire("m1")).expect("append");
        assert_eq!(cursor.0, 1, "first append is seq 1, exactly as on SQLite");
        let (ops, _next) = store
            .read_since(&stream, Cursor::BEGINNING, 10)
            .expect("read back");
        assert_eq!(ops.len(), 1);
        // Registry writes take the same path through the driver, so prove it too rather
        // than assuming the op store's fix covers it.
        registry
            .register(&stream, "ab12", "11111111-1111-1111-1111-111111111111")
            .expect("register in async context");
    });
}

#[test]
fn connect_and_append_inside_a_current_thread_runtime() {
    // An embedder may build a current-thread runtime (the DynamoDB smoke test does). It is
    // the flavor where `block_in_place` PANICS, so a guard that only reaches for
    // `block_in_place` would trade one panic for another — this test is what makes the
    // flavor distinction load-bearing rather than decorative.
    let Some(base) = base_url("current_thread") else {
        return;
    };
    let url = isolated_url(&base, "rt");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    rt.block_on(async move {
        let store = PostgresOpStore::connect(&url).expect("connect op store on current-thread rt");
        let registry = PostgresPrefixRegistry::connect(&url).expect("connect registry");
        let stream = StreamId("rt-current".into());
        store.append(&stream, &wire("c1")).expect("append");
        let (ops, _next) = store
            .read_since(&stream, Cursor::BEGINNING, 10)
            .expect("read back");
        assert_eq!(ops.len(), 1);
        // The registry travels the same guarded path, and this flavor is the one where
        // `block_in_place` is itself illegal — so cover it here too rather than leaving the
        // current-thread case asymmetric with the multi-thread one (PR #305 review, Test
        // Quality #2).
        registry
            .register(&stream, "ab12", "22222222-2222-2222-2222-222222222222")
            .expect("register on current-thread rt");
    });
}

#[test]
fn an_embedded_relay_serves_the_postgres_backend_over_http() {
    // The handler path end to end — the shape the shipped binary has: HTTP in, store call
    // on a runtime worker, HTTP out. Nothing else in the suite crosses `app.rs` with a
    // Postgres backend behind it.
    use nxs_server::app::{SharedRegistry, SharedStore};

    let Some(base) = base_url("embedded_relay") else {
        return;
    };
    let url = isolated_url(&base, "rt");
    let ops: SharedStore = Arc::new(PostgresOpStore::connect(&url).expect("op store"));
    let registry: SharedRegistry =
        Arc::new(PostgresPrefixRegistry::connect(&url).expect("registry"));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    listener.set_nonblocking(true).expect("nonblocking");
    let router = app(ops, registry);
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("relay runtime");
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("adopt listener");
            axum::serve(listener, router).await.expect("serve");
        });
    });

    let base = format!("http://{addr}");
    let push: serde_json::Value = ureq::post(&format!("{base}/streams/rt-http/ops"))
        .send_json(serde_json::json!({ "ops": [wire("h1")] }))
        .expect("push over HTTP")
        .into_json()
        .expect("push json");
    assert_eq!(
        push["appended"], 1,
        "the relay must serve the append, not abort the worker: {push}"
    );

    let pull: serde_json::Value = ureq::get(&format!("{base}/streams/rt-http/ops"))
        .query("since", "0")
        .query("limit", "10")
        .call()
        .expect("pull over HTTP")
        .into_json()
        .expect("pull json");
    assert_eq!(
        pull["ops"].as_array().map(|a| a.len()),
        Some(1),
        "the op must come back through the same backend: {pull}"
    );

    // Presence (6j6v.f0b5) is the third store call a handler makes, and the same off-runtime
    // guard has to cover it: an announcement and a read, over HTTP, on a runtime worker.
    let announced = ureq::post(&format!("{base}/streams/rt-http/machines"))
        .send_json(serde_json::json!({
            "machine_id": "m1", "name": "MacBook", "interval_secs": 300
        }))
        .expect("announce over HTTP");
    assert_eq!(announced.status(), 204);
    let machines: serde_json::Value = ureq::get(&format!("{base}/streams/rt-http/machines"))
        .call()
        .expect("machines over HTTP")
        .into_json()
        .expect("machines json");
    assert_eq!(
        machines["machines"][0]["machine_id"], "m1",
        "the announcement must come back through the same backend: {machines}"
    );
}
