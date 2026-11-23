//! Sync with a relay (RFC 0001, "Sync"): the device's database syncs while it is open, through
//! the relay its settings name. A command opens it and never waits; `sync` waits until nothing is
//! behind or unconfirmed, within a timeout; `status` reads the local database alone.
//!
//! No NoCentralDB relay is published for an embedder's tests (NEX-227), so the relay here is a
//! TCP listener that counts the connections it is offered and never answers: enough to see that a
//! command connects, that it does not wait, and that `status` makes no network call. A write
//! reaching another member through a real relay waits for NEX-228.

mod common;

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::data_dir::DataDir;
use nexus_flow_store::device::DeviceSettings;
use nexus_flow_store::opened;
use nexus_flow_store::space::init;
use nexus_flow_store::sync::{self, SyncError};

/// A relay that never answers: it accepts every connection, counts it and holds it open.
struct SilentRelay {
    url: String,
    connections: Arc<AtomicUsize>,
}

impl SilentRelay {
    fn start() -> SilentRelay {
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
        SilentRelay { url, connections }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Whether a connection arrives within `wait`.
    fn connected_within(&self, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if self.connections() > 0 {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

/// A repository given its space in island mode, with the relay `url` set afterwards.
fn repository_with_relay(base: &Path, url: &str) -> DataDir {
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    sync::set_relay(&data, &url.parse().unwrap()).unwrap();
    data
}

#[test]
fn setting_the_relay_keeps_the_last_sync_and_leaves_nexus_flow_toml_alone() {
    let base = scratch("sync-set-relay");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let file = fs::read(base.join("repo").join("nexus-flow.toml")).unwrap();
    DeviceSettings {
        relay: None,
        synced_at: Some(7),
    }
    .write(&data)
    .unwrap();

    sync::set_relay(&data, &"wss://relay.example.org".parse().unwrap()).unwrap();

    let settings = DeviceSettings::read(&data).unwrap();
    assert_eq!(settings.relay.unwrap().as_str(), "wss://relay.example.org");
    assert_eq!(settings.synced_at, Some(7));
    assert_eq!(
        fs::read(base.join("repo").join("nexus-flow.toml")).unwrap(),
        file
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn opening_with_a_relay_set_connects_to_it_and_does_not_wait() {
    let base = scratch("sync-open-connects");
    let relay = SilentRelay::start();
    let data = repository_with_relay(&base, &relay.url);

    let started = Instant::now();
    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a command does not wait for sync: {:?}",
        started.elapsed()
    );
    assert!(
        relay.connected_within(Duration::from_secs(10)),
        "the open database syncs with the relay its settings name"
    );
    drop(opened);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn status_reads_the_local_database_and_makes_no_network_call() {
    let base = scratch("sync-status-offline");
    let relay = SilentRelay::start();
    let data = repository_with_relay(&base, &relay.url);

    let status = block_on(sync::status(&base.join("repo"), &data, keys(&data))).unwrap();

    assert_eq!(status.relay.unwrap().as_str(), relay.url);
    let member = status.member.expect("the device is a member of its space");
    assert_eq!(member.len(), 64, "{member}");
    assert!(member.chars().all(|c| c.is_ascii_hexdigit()), "{member}");
    assert_eq!(status.synced_at, None);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(relay.connections(), 0, "status touched the network");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_write_made_in_island_mode_stays_queued_as_unconfirmed() {
    let base = scratch("sync-queued");
    let relay = SilentRelay::start();
    let data = repository_with_relay(&base, &relay.url);

    let status = block_on(sync::status(&base.join("repo"), &data, keys(&data))).unwrap();

    assert!(
        status.unconfirmed > 0,
        "creating the space is a write no relay confirmed yet"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn sync_that_times_out_lists_what_is_still_behind() {
    let base = scratch("sync-timeout");
    let relay = SilentRelay::start();
    let data = repository_with_relay(&base, &relay.url);

    let started = Instant::now();
    let result = block_on(sync::sync(
        &base.join("repo"),
        &data,
        keys(&data),
        Some(Duration::from_millis(500)),
    ));

    assert!(started.elapsed() < Duration::from_secs(10));
    let behind = match result {
        Err(SyncError::TimedOut(behind)) => behind,
        other => panic!("expected a timeout, got {other:?}"),
    };
    assert!(behind.unconfirmed > 0);
    assert_eq!(behind.spaces.len(), 1);
    assert!(!behind.spaces[0].initial_sync_complete);
    let text = behind.to_string();
    assert!(text.contains("unconfirmed"), "{text}");
    assert!(text.contains(&behind.spaces[0].space.to_string()), "{text}");
    assert_eq!(DeviceSettings::read(&data).unwrap().synced_at, None);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn sync_without_a_relay_is_refused() {
    let base = scratch("sync-island");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();

    let result = block_on(sync::sync(&base.join("repo"), &data, keys(&data), None));

    assert!(matches!(result, Err(SyncError::NoRelay)), "{result:?}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn watch_keeps_the_database_open_until_stopped() {
    let base = scratch("sync-watch");
    let relay = SilentRelay::start();
    let data = repository_with_relay(&base, &relay.url);
    let polls = AtomicUsize::new(0);

    let result = block_on(sync::watch(&base.join("repo"), &data, keys(&data), || {
        polls.fetch_add(1, Ordering::SeqCst) >= 10
    }));

    assert!(result.is_ok(), "{result:?}");
    assert!(polls.load(Ordering::SeqCst) > 10);
    assert!(
        relay.connections() > 0,
        "the database synced while it was open"
    );
    fs::remove_dir_all(&base).unwrap();
}
