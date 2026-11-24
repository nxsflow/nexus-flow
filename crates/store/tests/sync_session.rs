//! Sync as a session's member (RFC 0001, "Sync"; NEX-98's member a command runs as): the relay in
//! the device's settings is the device member's, and a session's member has no relay setting yet.
//! So a session's database opens in island mode, a sync on it has no relay to wait for, and its
//! status reports neither the device's relay nor the device's last sync.

mod common;

use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::data_dir::DataDir;
use nexus_flow_store::device::DeviceSettings;
use nexus_flow_store::id::SessionId;
use nexus_flow_store::member_select::{RunAs, SessionFiles};
use nexus_flow_store::model;
use nexus_flow_store::opened;
use nexus_flow_store::space::init;
use nexus_flow_store::sync::{self, SyncError};
use nocentraldb::{Db, OpenOptions};

/// A relay that accepts every connection, counts it and never answers; its URL and its count.
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

/// Starts the session `id` as `nxs session start` leaves it: a sub-agent's database and its keys
/// in the session's folder.
fn start_session(data: &DataDir, id: &SessionId) {
    let files = SessionFiles::of(data, id);
    let options = OpenOptions::new(model::LOCK)
        .sub_agent()
        .path(&files.database)
        .key_file(&files.keys);
    drop(block_on(Db::open(options)).unwrap());
}

#[test]
fn a_session_member_has_no_relay_to_sync_with_and_never_reaches_the_devices() {
    let base = scratch("sync-session");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let (url, connections) = silent_relay();
    sync::set_relay(&data, &url.parse().unwrap()).unwrap();
    DeviceSettings {
        relay: Some(url.parse().unwrap()),
        synced_at: Some(7),
    }
    .write(&data)
    .unwrap();
    let session: SessionId = "coder-nex-102".parse().unwrap();
    start_session(&data, &session);

    let mut opened = block_on(opened::repository_as(
        &base.join("repo"),
        &data,
        RunAs::Session(session),
    ))
    .unwrap();
    let waited = block_on(sync::wait(&mut opened, Some(Duration::from_secs(5))));
    let status = block_on(sync::report(&opened)).unwrap();

    assert!(matches!(waited, Err(SyncError::NoRelay)), "{waited:?}");
    assert_eq!(
        status.relay, None,
        "the device's relay is not the session's"
    );
    assert_eq!(
        status.synced_at, None,
        "the device's last sync is not the session's"
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "the session went online"
    );
    fs::remove_dir_all(&base).unwrap();
}
