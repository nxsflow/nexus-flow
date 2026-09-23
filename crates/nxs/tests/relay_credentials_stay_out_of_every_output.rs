//! **A relay URL's credentials never reach an output** (nxf 6j6v.q3kk) — on the REAL binary, at
//! every place a failed sync pass is reported.
//!
//! A relay behind a gateway takes its key as userinfo in the URL
//! (`https://<user>:<password>@relay.example`), because that is the only place the `ureq` transport
//! has for credentials. A failed pass used to quote that URL whole, and the message travelled on to
//! `service.log` (0644), the heartbeat's `last_error`, and from there to `service_attendance` —
//! every app and every agent session that reads it. manufakt.io measured exactly that line with its
//! CURRENT staging key in it.
//!
//! So this provokes the failure for real — a workspace bound to a URL with userinfo whose port
//! nobody listens on — and reads every exit: the service's own output (what launchd writes to
//! `service.log`), the heartbeat, `ServiceHome::attendance`, `nxs sync daemon status` in both forms,
//! and the two one-shot verbs a person runs by hand. Host and path must survive in each of them:
//! a masked message that no longer says WHICH relay failed would be switched off again.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

const USER: &str = "relayuser7";
/// Percent-encoded as a URL carries an `@` in a password, so the decoded form (`hunter2@…`) is a
/// second spelling that must not appear either — both contain `hunter2`.
const PASSWORD: &str = "hunter2%40staging-key";
/// base64("relayuser7:hunter2%40staging-key") — the Basic header's value, which a gateway can echo.
const BASIC: &str = "cmVsYXl1c2VyNzpodW50ZXIyJTQwc3RhZ2luZy1rZXk=";
const STREAM: &str = "stream-credential-e2e";

fn nxs(home: &Path, cwd: &Path) -> Command {
    nxs_test_support::assert_multicall_binary_fresh();
    let mut c = Command::new(assert_cmd::cargo::cargo_bin("nxs"));
    c.current_dir(cwd).pin_home(home).env("NXC_TIMER", "dry");
    c
}

/// A port on the loopback nobody listens on: bound, read, released. A connect to it is refused at
/// once, which is the transport failure this test needs, without waiting out a timeout.
fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Kill and reap on the way out, whatever the test did.
struct Reaped(Option<Child>);

impl Reaped {
    fn finish(mut self) -> String {
        let mut child = self.0.take().expect("still running");
        let _ = child.kill();
        let out = child.wait_with_output().expect("reap the service");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn output_of(cmd: &mut Command) -> String {
    let out = cmd.output().expect("nxs runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The one assertion, said once: no credential, and the relay still named by host and path.
fn assert_masked(surface: &str, text: &str, host: &str) {
    assert!(
        !text.contains("hunter2"),
        "{surface} carries the relay PASSWORD (encoded or decoded):\n{text}"
    );
    assert!(
        !text.contains(BASIC),
        "{surface} carries the Basic credential:\n{text}"
    );
    assert!(
        !text.contains(USER),
        "{surface} carries the relay USER:\n{text}"
    );
    assert!(
        text.contains(host),
        "{surface} no longer says which relay failed — host and port must stay readable:\n{text}"
    );
}

#[test]
fn a_failed_pass_against_a_relay_url_with_userinfo_names_the_relay_but_never_its_credentials() {
    let home = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let ws: PathBuf = std::fs::canonicalize(ws_dir.path()).unwrap();
    let host = format!("127.0.0.1:{}", closed_port());
    let endpoint = format!("http://{USER}:{PASSWORD}@{host}");

    let init = nxs(home.path(), &ws)
        .args(["init", "--module", "flow", "--json"])
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    let bind = nxs(home.path(), &ws)
        .args(["sync", "bind", "--join", STREAM, "--endpoint", &endpoint])
        .args(["--no-daemon", "--json"])
        .output()
        .unwrap();
    assert!(bind.status.success(), "{bind:?}");
    // Its echo too — an idempotent re-run prints the STORED endpoint (review of PR #487).
    assert_masked(
        "`nxs sync bind --json`",
        &String::from_utf8_lossy(&bind.stdout),
        &host,
    );

    // ---- the service: a real pass, failing for real ----
    let service = Reaped(Some(
        nxs(home.path(), home.path())
            .args(["sync", "daemon", "--interval", "1", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the service starts"),
    ));
    let service_home = nxs_service::ServiceHome::at(home.path().join(".nexusflow"));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_error = None;
    while Instant::now() < deadline && last_error.is_none() {
        std::thread::sleep(Duration::from_millis(200));
        last_error = service_home
            .attendance(&ws, &ws)
            .ok()
            .and_then(|a| a.workspace)
            .and_then(|w| w.last_error);
    }
    let service_output = service.finish();
    let last_error = last_error.unwrap_or_else(|| {
        panic!("the service never recorded the failed pass; its output:\n{service_output}")
    });

    // What launchd writes to `service.log` is exactly this process's stdout and stderr.
    assert_masked("the service's log output", &service_output, &host);
    assert!(
        service_output.contains("/streams/"),
        "the log line keeps the PATH that failed, not only the host:\n{service_output}"
    );
    assert_masked("service_attendance's last_error", &last_error, &host);
    let heartbeat = std::fs::read_to_string(service_home.heartbeat()).unwrap();
    assert_masked("the heartbeat file", &heartbeat, &host);
    assert_masked(
        "`nxs sync daemon status --json`",
        &output_of(nxs(home.path(), &ws).args(["sync", "daemon", "status", "--json"])),
        &host,
    );
    assert_masked(
        "`nxs sync daemon status`",
        &output_of(nxs(home.path(), &ws).args(["sync", "daemon", "status"])),
        &host,
    );

    // ---- the one-shot verbs a person runs by hand ----
    assert_masked(
        "`nxs sync run` (the bound endpoint)",
        &output_of(nxs(home.path(), &ws).args(["sync", "run", "--json"])),
        &host,
    );
    assert_masked(
        "`nxs sync run --remote`",
        &output_of(nxs(home.path(), &ws).args(["sync", "run", "--remote", &endpoint])),
        &host,
    );
    assert_masked(
        "`nxs sync machines`",
        &output_of(nxs(home.path(), &ws).args(["sync", "machines"])),
        &host,
    );
}

/// `nxs sync machines` names the relay it asked on SUCCESS too — and that line reaches an agent's
/// transcript as readily as a failure does.
#[test]
fn a_machines_listing_names_the_relay_it_asked_without_its_credentials() {
    let relay = common_relay();
    let home = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let ws: PathBuf = std::fs::canonicalize(ws_dir.path()).unwrap();
    let host = relay.trim_start_matches("http://").to_string();
    let endpoint = format!("http://{USER}:{PASSWORD}@{host}");

    let init = nxs(home.path(), &ws)
        .args(["init", "--module", "flow", "--json"])
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    let bind = nxs(home.path(), &ws)
        .args(["sync", "bind", "--join", STREAM, "--endpoint", &endpoint])
        .args(["--no-daemon", "--json"])
        .output()
        .unwrap();
    assert!(bind.status.success(), "{bind:?}");

    let human = output_of(nxs(home.path(), &ws).args(["sync", "machines"]));
    assert_masked("`nxs sync machines`", &human, &host);
    let json: Value = serde_json::from_str(&output_of(
        nxs(home.path(), &ws).args(["sync", "machines", "--json"]),
    ))
    .unwrap();
    assert_masked(
        "`nxs sync machines --json`",
        &json["endpoint"].to_string(),
        &host,
    );
}

/// An in-process relay, the shape `machine_presence.rs` uses.
fn common_relay() -> String {
    let router = nxs_server::app::app(
        std::sync::Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        std::sync::Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    );
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

/// The configured endpoint is shown masked wherever nxs repeats it, and the files that store it are
/// their owner's alone (review of PR #487, Integrity #2).
#[test]
fn the_stored_endpoint_is_shown_masked_and_kept_in_owner_only_files() {
    let home = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let ws: PathBuf = std::fs::canonicalize(ws_dir.path()).unwrap();
    let host = "relay.example:8787";
    let endpoint = format!("https://{USER}:{PASSWORD}@{host}");

    assert_masked(
        "`nxs sync endpoint <url>`",
        &output_of(nxs(home.path(), &ws).args(["sync", "endpoint", &endpoint])),
        host,
    );
    assert_masked(
        "`nxs sync endpoint`",
        &output_of(nxs(home.path(), &ws).args(["sync", "endpoint"])),
        host,
    );
    assert_masked(
        "`nxs sync endpoint --json`",
        &output_of(nxs(home.path(), &ws).args(["sync", "endpoint", "--json"])),
        host,
    );

    let init = nxs(home.path(), &ws)
        .args(["init", "--module", "flow", "--json"])
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    let bind = nxs(home.path(), &ws)
        .args(["sync", "bind", "--join", STREAM, "--endpoint", &endpoint])
        .args(["--no-daemon", "--json"])
        .output()
        .unwrap();
    assert!(bind.status.success(), "{bind:?}");

    #[cfg(unix)]
    for file in [
        home.path().join(".nexusflow/config.toml"),
        ws.join(".nxs/sync.toml"),
    ] {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{} is {mode:o}", file.display());
    }
}
