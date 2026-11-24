//! `Engine::sync` and `Engine::sync_status` (RFC 0001, "The library seam", "Sync"): an open
//! engine syncs through the relay the device's settings name; `sync` waits until nothing is
//! behind or unconfirmed, within a timeout, and `sync_status` reads the local database without
//! the network. `set_relay` sets the device's relay in the data directory.
//!
//! No NoCentralDB relay is published for an embedder's tests (NEX-227): the relay here is a TCP
//! listener that counts connections and never answers.

mod common;

use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::{block_on, options, scratch};
use nexus_flow_seam::{set_relay, Engine, Error, RelayUrl, SyncError};
use nexus_flow_store::space::init;

/// A relay that accepts every connection, counts it and never answers.
fn silent_relay() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&connections);
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            count.fetch_add(1, Ordering::SeqCst);
            held.push(stream);
        }
    });
    (url, connections)
}

#[test]
fn an_engine_on_a_silent_relay_times_out_with_what_is_behind() {
    let base = scratch("engine-sync-timeout");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let (url, connections) = silent_relay();
    let relay: RelayUrl = url.parse().unwrap();
    set_relay(&setup, &relay).unwrap();

    let mut engine = block_on(Engine::open_with(&base.join("repo"), setup.clone())).unwrap();
    let err = block_on(engine.sync(Some(Duration::from_millis(300)))).unwrap_err();

    match err {
        Error::Sync(SyncError::TimedOut(behind)) => assert!(behind.unconfirmed > 0),
        other => panic!("expected a timeout, got {other:?}"),
    }
    assert!(connections.load(Ordering::SeqCst) > 0);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_engine_without_a_relay_has_nothing_to_sync_with() {
    let base = scratch("engine-sync-island");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();

    let mut engine = block_on(Engine::open_with(&base.join("repo"), setup)).unwrap();
    let err = block_on(engine.sync(None)).unwrap_err();

    assert!(matches!(err, Error::Sync(SyncError::NoRelay)), "{err:?}");
    assert!(err.to_string().contains("--relay"), "{err}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn sync_status_reads_the_local_database_without_the_network() {
    let base = scratch("engine-sync-status");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let (url, connections) = silent_relay();
    set_relay(&setup, &url.parse().unwrap()).unwrap();

    let status = block_on(Engine::sync_status_with(&base.join("repo"), setup)).unwrap();

    assert_eq!(status.relay.map(|r| r.to_string()), Some(url));
    assert!(status.member.is_some());
    assert!(status.unconfirmed > 0);
    assert_eq!(status.synced_at, None);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(connections.load(Ordering::SeqCst), 0, "status went online");
    fs::remove_dir_all(&base).unwrap();
}
