//! `nxs sync` (RFC 0001, "Sync"; "Public surfaces this RFC brings"): `--relay <url>` stores the
//! device's relay in the data directory, never in `nexus-flow.toml`; `--status` prints `member`,
//! `relay`, `unconfirmed` and `synced_at` from the local database without the network; a plain
//! command never waits for sync; `--watch` keeps the database open.
//!
//! No NoCentralDB relay is published for an embedder's tests (NEX-227): the relay here is a TCP
//! listener that counts connections and never answers. A write reaching another member waits for
//! NEX-228.

mod common;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{json, nxs, scratch, stdout};

/// A relay that accepts every connection, counts it and never answers.
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

/// A repository given its space by `nxs init`, in island mode.
fn repository(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch(test);
    let repo = base.join("repo");
    let out = nxs(&base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    (base, repo)
}

/// Sets the relay with `nxs sync --relay <url> --status`, which stays off the network.
fn set_relay(base: &Path, repo: &Path, url: &str) {
    let out = nxs(base, repo, &["sync", "--relay", url, "--status"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
}

#[test]
fn relay_is_stored_in_the_data_directory_and_not_in_nexus_flow_toml() {
    let (base, repo) = repository("sync-relay");
    let file = fs::read(repo.join("nexus-flow.toml")).unwrap();

    let out = nxs(
        &base,
        &repo,
        &[
            "sync",
            "--relay",
            "wss://relay.example.org/team",
            "--status",
        ],
    );

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["relay"], "wss://relay.example.org/team");
    assert_eq!(fs::read(repo.join("nexus-flow.toml")).unwrap(), file);
    let settings = fs::read_to_string(base.join("data").join("device.toml")).unwrap();
    assert!(
        settings.contains("wss://relay.example.org/team"),
        "{settings}"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_relay_url_that_is_not_ws_or_wss_is_refused_and_not_stored() {
    let (base, repo) = repository("sync-relay-bad");

    let out = nxs(
        &base,
        &repo,
        &["sync", "--relay", "https://relay.example.org", "--status"],
    );

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "validation");
    let msg = json(&out)["error"]["msg"].as_str().unwrap().to_owned();
    assert!(msg.contains("ws://"), "{msg}");
    assert!(!base.join("data").join("device.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn status_prints_member_relay_unconfirmed_and_synced_at_without_the_network() {
    let (base, repo) = repository("sync-status");
    let relay = SilentRelay::start();
    set_relay(&base, &repo, &relay.url);

    let out = nxs(&base, &repo, &["sync", "--status"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let status = json(&out);
    let text = stdout(&out);
    let at = |key: &str| text.find(&format!("\"{key}\":")).expect(key);
    assert!(
        at("member") < at("relay")
            && at("relay") < at("unconfirmed")
            && at("unconfirmed") < at("synced_at"),
        "{text}"
    );
    assert_eq!(status.as_object().unwrap().len(), 4, "{text}");
    assert_eq!(status["member"].as_str().map(str::len), Some(64));
    assert_eq!(status["relay"], relay.url.as_str());
    assert!(status["unconfirmed"].as_u64().unwrap() > 0, "{status}");
    assert!(status["synced_at"].is_null(), "{status}");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(relay.connections(), 0, "--status went online");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_reports_the_relay_mode_once_a_relay_is_set() {
    let (base, repo) = repository("sync-init-relay");
    let relay = SilentRelay::start();
    set_relay(&base, &repo, &relay.url);

    let out = nxs(&base, &repo, &["init"]);
    let md = nxs(&base, &repo, &["init", "--md"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["sync"], "relay");
    assert_eq!(md.code, 0, "{}", stdout(&md));
    let md = stdout(&md);
    assert!(!md.contains("island mode"), "{md}");
    assert!(md.contains("relay"), "{md}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_without_a_relay_reports_the_island_mode() {
    let (base, repo) = repository("sync-init-island");

    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["sync"], "island");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn status_without_a_relay_reports_relay_null() {
    let (base, repo) = repository("sync-status-island");

    let out = nxs(&base, &repo, &["sync", "--status"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(json(&out)["relay"].is_null());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_plain_command_never_waits_and_its_write_stays_queued() {
    let (base, repo) = repository("sync-plain");
    let relay = SilentRelay::start();
    set_relay(&base, &repo, &relay.url);

    let started = Instant::now();
    let out = nxs(&base, &repo, &["init"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a command does not wait for sync: {:?}",
        started.elapsed()
    );
    let status = json(&nxs(&base, &repo, &["sync", "--status"]));
    assert!(
        status["unconfirmed"].as_u64().unwrap() > 0,
        "the write no relay confirmed stays queued for the next open: {status}"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn watch_keeps_the_database_open_and_syncing() {
    let (base, repo) = repository("sync-watch");
    let relay = SilentRelay::start();
    set_relay(&base, &repo, &relay.url);

    let watching = {
        let (base, repo) = (base.clone(), repo.clone());
        thread::spawn(move || nxs(&base, &repo, &["sync", "--watch"]))
    };

    assert!(relay.connected_within(Duration::from_secs(10)));
    thread::sleep(Duration::from_secs(2));
    assert!(!watching.is_finished(), "--watch returned");
    // The watching thread ends with the test process.
}

#[test]
fn status_takes_neither_watch_nor_timeout() {
    let (base, repo) = repository("sync-status-flags");

    for flags in [["--status", "--watch"], ["--status", "--timeout=1"]] {
        let args: Vec<&str> = std::iter::once("sync").chain(flags).collect();
        let out = nxs(&base, &repo, &args);
        assert_eq!(out.code, 1, "{flags:?}");
        assert_eq!(json(&out)["error"]["kind"], "validation", "{flags:?}");
        let msg = json(&out)["error"]["msg"].as_str().unwrap().to_owned();
        assert!(msg.contains("cannot be used with"), "{msg}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn status_prints_the_last_sync_as_an_rfc_3339_time_in_utc() {
    let (base, repo) = repository("sync-status-synced-at");
    // No relay is published for tests (NEX-227), so the time a sync recorded is written directly.
    fs::write(
        base.join("data").join("device.toml"),
        "relay = \"wss://relay.example.org\"\nsynced_at = 1795000000\n",
    )
    .unwrap();

    let out = nxs(&base, &repo, &["sync", "--status"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["synced_at"], "2026-11-18T11:06:40Z");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn timeout_exits_non_zero_and_lists_what_is_still_behind() {
    let (base, repo) = repository("sync-timeout");
    let relay = SilentRelay::start();
    set_relay(&base, &repo, &relay.url);
    let space = json(&nxs(&base, &repo, &["init"]))["space"]
        .as_str()
        .unwrap()
        .to_owned();

    let started = Instant::now();
    let out = nxs(&base, &repo, &["sync", "--timeout", "1"]);

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "timeout");
    let msg = json(&out)["error"]["msg"].as_str().unwrap().to_owned();
    assert!(msg.contains("timed out"), "{msg}");
    assert!(msg.contains("unconfirmed"), "{msg}");
    assert!(msg.contains(&space), "{msg}");
    assert!(json(&nxs(&base, &repo, &["sync", "--status"]))["synced_at"].is_null());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn status_in_markdown_names_each_field_for_a_person() {
    let (base, repo) = repository("sync-status-md");
    fs::write(
        base.join("data").join("device.toml"),
        "relay = \"wss://relay.example.org\"\nsynced_at = 1795000000\n",
    )
    .unwrap();

    let out = nxs(&base, &repo, &["sync", "--status", "--md"]);

    assert_eq!(out.code, 0);
    let text = stdout(&out);
    assert!(text.starts_with("- member `"), "{text}");
    assert!(
        text.contains("- relay `wss://relay.example.org`\n"),
        "{text}"
    );
    assert!(text.contains(" unconfirmed\n"), "{text}");
    assert!(
        text.ends_with("- last synced at 2026-11-18T11:06:40Z\n"),
        "{text}"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn sync_without_a_relay_is_a_validation_error_that_names_relay() {
    let (base, repo) = repository("sync-no-relay");

    let out = nxs(&base, &repo, &["sync"]);

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "validation");
    let msg = json(&out)["error"]["msg"].as_str().unwrap().to_owned();
    assert!(msg.contains("nxs sync --relay <url>"), "{msg}");
    fs::remove_dir_all(&base).unwrap();
}
