//! `nxf self-update` end-to-end through the real binary (nexus-flow-85y.18). The verify +
//! atomic-swap install path needs an embedded minisign key and is covered live against
//! staging in the PR; here we exercise the `--check` orchestration (query `/updater`, decide,
//! deterministic output, channel persistence) against an in-process updater — no network, no key.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use std::net::TcpListener;
use std::path::Path;
use tempfile::TempDir;

/// Spin a minimal `/updater` on a loopback port. `offer_update` ⇒ 200 with a strictly-newer
/// version, else 204. Returns the base URL. The body's sha256/signature are placeholders:
/// `--check` never downloads or verifies, so they are never read.
fn spawn_updater(offer_update: bool) -> String {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();

    async fn up(offer: bool) -> axum::response::Response {
        if offer {
            axum::Json(serde_json::json!({
                "version": "9.9.9",
                "pub_date": "2026-06-12T00:00:00Z",
                "notes": "newer build",
                "url": "http://127.0.0.1/never-fetched.tar.gz",
                "sha256": "00",
                "signature": "unused"
            }))
            .into_response()
        } else {
            StatusCode::NO_CONTENT.into_response()
        }
    }

    let router = axum::Router::new().route("/updater", get(move || up(offer_update)));
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

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// Run `self-update` with the updater + config dir pointed at the harness, return parsed stdout.
fn check_json(base: &str, cfg_dir: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["self-update", "--check", "--json"];
    args.extend_from_slice(extra);
    let out = nxf()
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
fn check_reports_an_available_update_without_installing() {
    let base = spawn_updater(true);
    let cfg = TempDir::new().unwrap();
    let v = check_json(&base, cfg.path(), &[]);
    assert_eq!(v["status"], "update-available");
    assert_eq!(v["latest_version"], "9.9.9");
    assert_eq!(v["channel"], "stable");
}

#[test]
fn check_reports_up_to_date_on_204() {
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();
    let v = check_json(&base, cfg.path(), &[]);
    assert_eq!(v["status"], "up-to-date");
    assert_eq!(v["channel"], "stable");
}

#[test]
fn channel_is_persisted_in_the_user_config_and_then_sticky() {
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();

    // First run pins the channel to beta.
    let v = check_json(&base, cfg.path(), &["--channel", "beta"]);
    assert_eq!(v["channel"], "beta");

    // It was written to the per-install config (channel + a minted machine_id).
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

#[test]
fn nxf_self_update_nudges_to_nxs_on_stderr_but_stays_silent_under_json() {
    // nexus-flow-gel: the hidden `nxf self-update` alias still works but is deprecated — a human
    // run nudges the user to the canonical `nxs self-update` on STDERR (never stdout, so the
    // result line and the `--json` machine contract stay clean). `--check` against a 204 updater
    // never downloads, so this exercises the alias path with no key and no network.
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();

    // Human mode: the deprecation note lands on stderr, pointing at `nxs self-update`.
    let human = nxf()
        .args(["self-update", "--check"])
        .env("NXF_BASE_URL", &base)
        .env("NXF_CONFIG_DIR", cfg.path())
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&human.get_output().stderr);
    assert!(
        stderr.contains("deprecated") && stderr.contains("nxs self-update"),
        "human `nxf self-update` nudges to `nxs self-update` on stderr: {stderr}"
    );

    // `--json` mode: the note is fully suppressed — absent from BOTH streams — and stdout is still
    // a single clean JSON line (the machine contract the nudge must never break).
    let json = nxf()
        .args(["self-update", "--check", "--json"])
        .env("NXF_BASE_URL", &base)
        .env("NXF_CONFIG_DIR", cfg.path())
        .assert()
        .success();
    let out_stdout = String::from_utf8_lossy(&json.get_output().stdout);
    let out_stderr = String::from_utf8_lossy(&json.get_output().stderr);
    assert!(
        !out_stderr.contains("deprecated") && !out_stdout.contains("deprecated"),
        "under --json the deprecation note is silent: stdout={out_stdout} stderr={out_stderr}"
    );
    serde_json::from_str::<serde_json::Value>(out_stdout.trim())
        .expect("under --json stdout is still a clean JSON line");
}

// ---- the service alias, through the REAL `self-update` seam ------------------------------------
//
// nxf 6j6v.dcpk (b) wires `service_alias_divergence` into `run()` at two call sites: the
// already-up-to-date self-heal branch and the post-install branch. The unit tests beside the
// function prove the DECISION; the review of PR #393 (Test Quality #1, High) found that nothing
// drove the WIRING — deleting both call sites left every test in the repo green, because the only
// `self-update` cases here used `--check`, which returns before either one is reached.
//
// The 204 (up-to-date) branch is the one that can be driven hermetically: it never downloads and
// never verifies, so it needs neither network nor an embedded key, and it runs the whole tail of
// `run()` — install dir, persona links, alias comparison, receipt.

/// Run a REAL `self-update` (no `--check`) against a 204 updater, with `$HOME` pointed at `home`,
/// and return the parsed `--json` receipt.
fn self_update_json(base: &str, cfg_dir: &Path, home: &Path) -> serde_json::Value {
    let out = nxf()
        .args(["self-update", "--json"])
        .env("NXF_BASE_URL", base)
        .env("NXF_CONFIG_DIR", cfg_dir)
        .pin_home(home)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("deterministic json on stdout")
}

/// A `~/.nexusflow/bin/nexus-flow` alias under `home`, pointing wherever the caller says.
fn alias_pointing_at(home: &Path, target: &Path) {
    let bin = home.join(".nexusflow").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(target, bin.join("nexus-flow")).unwrap();
}

#[test]
fn an_update_that_leaves_the_service_on_another_build_says_so_on_the_receipt() {
    // The measured case (6j6v.dcpk): three `self-update` runs on one day left the machine's only
    // clock on a `target/debug` build from four days earlier, and nobody was told.
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let elsewhere = home.path().join("some-other-build").join("nxs");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    std::fs::write(&elsewhere, b"#!/bin/sh\n").unwrap();
    alias_pointing_at(home.path(), &elsewhere);

    let v = self_update_json(&base, cfg.path(), home.path());
    let note = v["service_alias"]
        .as_str()
        .unwrap_or_else(|| panic!("the receipt must carry the divergence: {v}"));
    assert!(
        note.contains(&elsewhere.display().to_string()),
        "with the build the service actually runs: {note}"
    );
    assert!(
        note.contains("nxs sync daemon install"),
        "and the command that moves it: {note}"
    );
    assert_eq!(
        v["status"], "up-to-date",
        "the update itself is untouched by the note: {v}"
    );
}

#[test]
fn an_update_whose_service_already_runs_the_installed_binary_adds_nothing_to_the_receipt() {
    // The healthy machine, and the reason the key is omittable rather than `null`: an added field
    // on every ordinary update would change a byte contract for nothing.
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let installed = std::fs::canonicalize(assert_cmd::cargo::cargo_bin("nxs"))
        .expect("the binary under test exists");
    alias_pointing_at(home.path(), &installed);

    let v = self_update_json(&base, cfg.path(), home.path());
    assert!(
        v.get("service_alias").is_none(),
        "an agreeing alias is nothing to report: {v}"
    );
}

#[test]
fn a_machine_with_no_service_alias_is_never_nagged_by_an_update() {
    let base = spawn_updater(false);
    let cfg = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let v = self_update_json(&base, cfg.path(), home.path());
    assert!(
        v.get("service_alias").is_none(),
        "no alias means the background service was never installed here: {v}"
    );
}
