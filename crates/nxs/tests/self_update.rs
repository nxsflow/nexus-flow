//! `nxs self-update --check` end-to-end through the real `nxs` binary (nexus-flow-gel) — the
//! canonical, suite-wide self-update verb. We exercise the `--check` orchestration (query
//! `/updater`, decide, deterministic `--json`, channel persistence) against a hermetic raw-socket
//! updater on loopback — no network, no embedded key. The verify + atomic-swap install path is
//! covered in the cli crate's unit tests and the live install-update smoke workflow.

use assert_cmd::Command;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use tempfile::TempDir;

/// A throwaway `/updater` on a loopback port that always answers HTTP 204 (up to date). Std-only
/// (no axum) so the `nxs` test crate needs no extra deps. It serves connections until the listener
/// is dropped at process exit; `--check` makes exactly one request.
fn spawn_204_updater() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            // Drain the request (a small GET) enough that the client can finish writing, then
            // answer 204 with no body. We don't need the bytes — only a well-formed response.
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
            let _ = s.flush();
        }
    });
    format!("http://{addr}")
}

/// Run `nxs self-update --check --json` with the updater + per-install config dir pointed at the
/// harness (so the minted channel/machine_id never touch the caller's real home), parse stdout.
fn check_json(base: &str, cfg_dir: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["self-update", "--check", "--json"];
    args.extend_from_slice(extra);
    let out = Command::cargo_bin("nxs")
        .expect("nxs binary")
        .args(&args)
        .env("NXF_BASE_URL", base)
        .env("NXF_CONFIG_DIR", cfg_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("deterministic json on stdout")
}

#[test]
fn self_update_check_reports_up_to_date_with_deterministic_json() {
    let base = spawn_204_updater();
    let cfg = TempDir::new().unwrap();
    let v = check_json(&base, cfg.path(), &[]);
    assert_eq!(v["status"], "up-to-date", "204 ⇒ up to date: {v}");
    assert_eq!(v["channel"], "stable", "default channel: {v}");
}

#[test]
fn self_update_persists_the_channel_like_nxf() {
    // The umbrella verb shares the cli crate's per-install config, so `--channel` sticks exactly as
    // it does for `nxf self-update` — one updater, one config (nexus-flow-gel).
    let base = spawn_204_updater();
    let cfg = TempDir::new().unwrap();

    let v = check_json(&base, cfg.path(), &["--channel", "beta"]);
    assert_eq!(v["channel"], "beta");

    let toml = std::fs::read_to_string(cfg.path().join("config.toml")).unwrap();
    assert!(
        toml.contains("channel = \"beta\""),
        "channel persisted: {toml}"
    );
    assert!(toml.contains("machine_id"), "machine_id minted: {toml}");

    // A later run with no --channel uses the persisted value.
    let again = check_json(&base, cfg.path(), &[]);
    assert_eq!(again["channel"], "beta");
}
