//! The update hint (nexus-flow-85y.21) must never pollute a command's real output. These
//! drive the actual `nxf` binary with its stderr piped (i.e. non-interactive, exactly like an
//! agent or a CI step): the hint is suppressed, stdout stays clean, and an explicit
//! `NXF_NO_UPDATE_CHECK=1` is honored too. The positive "the nudge appears" path is unit-tested
//! (`compute_update_hint`) and confirmed live; it cannot surface here because a piped stderr is
//! deliberately not a terminal. The updater here would offer an update if probed — silence is
//! therefore the suppression working, not the absence of an offer.

use std::net::TcpListener;
use tempfile::TempDir;

fn spawn_offering_updater() -> String {
    use axum::routing::get;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    async fn up() -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({
            "version": "9.9.9", "pub_date": "2026-06-12T00:00:00Z", "notes": "n",
            "url": "http://127.0.0.1/x.tar.gz", "sha256": "00", "signature": "s"
        }))
    }
    let router = axum::Router::new().route("/updater", get(up));
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(l, router).await.unwrap();
        });
    });
    format!("http://{addr}")
}

fn init(dir: &std::path::Path) {
    nxs_test_support::cargo_bin("nxf")
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

#[test]
fn no_update_hint_leaks_into_human_output_when_non_interactive() {
    let base = spawn_offering_updater();
    let ws = TempDir::new().unwrap();
    let cfg = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    init(ws.path());

    let out = nxs_test_support::cargo_bin("nxf")
        .arg("next")
        .current_dir(ws.path())
        .env("NXF_BASE_URL", &base)
        .env("NXF_CONFIG_DIR", cfg.path())
        .env("NXF_CACHE_DIR", cache.path())
        .env_remove("CI")
        .assert()
        .success()
        .get_output()
        .clone();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("note: nxf"),
        "hint leaked to stderr: {stderr}"
    );
}

#[test]
fn json_stdout_is_never_polluted_by_the_hint() {
    let base = spawn_offering_updater();
    let ws = TempDir::new().unwrap();
    let cfg = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    init(ws.path());

    let out = nxs_test_support::cargo_bin("nxf")
        .args(["next", "--json"])
        .current_dir(ws.path())
        .env("NXF_BASE_URL", &base)
        .env("NXF_CONFIG_DIR", cfg.path())
        .env("NXF_CACHE_DIR", cache.path())
        .assert()
        .success()
        .get_output()
        .clone();

    // stdout parses as JSON (the hint never reaches it).
    serde_json::from_slice::<serde_json::Value>(&out.stdout).expect("clean json stdout");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("note: nxf"));
}

#[test]
fn nxf_no_update_check_silences_the_hint() {
    let base = spawn_offering_updater();
    let ws = TempDir::new().unwrap();
    let cfg = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    init(ws.path());

    let out = nxs_test_support::cargo_bin("nxf")
        .arg("next")
        .current_dir(ws.path())
        .env("NXF_BASE_URL", &base)
        .env("NXF_CONFIG_DIR", cfg.path())
        .env("NXF_CACHE_DIR", cache.path())
        .env("NXF_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(!String::from_utf8_lossy(&out.stderr).contains("note: nxf"));
    // Disabled means no probe happened — no cache file was written.
    assert!(!cache.path().join("update-check.toml").exists());
}
