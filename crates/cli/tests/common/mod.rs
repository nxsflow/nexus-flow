//! Shared test harness: spin up a real relay on an ephemeral loopback port so the `nxf`
//! binary (a child process) can sync against it over HTTP.
//!
//! This module is `include`d per integration-test binary, so not every helper is used by
//! every binary — allow the resulting dead-code noise rather than annotate each item.
#![allow(dead_code)]

use std::sync::Arc;

use nxs_server::app::app;
use nxs_server::registry::SqlitePrefixRegistry;
use nxs_server::store::{OpStore, SqliteOpStore, StoreError, StoreResult};
use nxs_sync::protocol::{Cursor, StreamId};
use nxs_sync::wire::WireOp;

/// Start an in-memory relay on a background thread; return its base URL (`http://127…`).
pub fn spawn_relay() -> String {
    serve(Arc::new(
        SqliteOpStore::open_in_memory().expect("relay store"),
    ))
}

/// An [`OpStore`] that durably appends (push succeeds) but always fails `read_since` (pull
/// fails). Drives `nxs sync run` to a mid-pass error AFTER the push watermark has advanced,
/// so a test can prove the CLI persists that progress on the error path.
struct PullFailsStore(SqliteOpStore);

impl OpStore for PullFailsStore {
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        self.0.append(stream, op)
    }
    fn read_since(&self, _: &StreamId, _: Cursor, _: usize) -> StoreResult<(Vec<WireOp>, Cursor)> {
        Err(StoreError("injected pull failure".into()))
    }
}

/// Like [`spawn_relay`] but the relay accepts pushes and fails every pull.
pub fn spawn_relay_pull_fails() -> String {
    serve(Arc::new(PullFailsStore(
        SqliteOpStore::open_in_memory().expect("relay store"),
    )))
}

/// Serve a relay over `ops` on an ephemeral loopback port; return its base URL.
fn serve(ops: Arc<dyn OpStore + Send + Sync>) -> String {
    let registry = Arc::new(SqlitePrefixRegistry::open_in_memory().expect("relay registry"));
    let router = app(ops, registry);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
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
