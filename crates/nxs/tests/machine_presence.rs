//! **Which machines sync this workspace, and which are online** (nxf 6j6v.f0b5) — at the surface a
//! person uses, on the REAL binary.
//!
//! Two machines are two service homes (the unit `nxs_service::machine` documents as a machine),
//! each a pinned `$HOME` running its own `nxs sync daemon`, each with its own clone bound to one
//! relay. The relay runs in this process so its clock can be turned: presence is judged by AGE, and
//! nobody waits out a window in a test. Everything else is the shipped path — `sync bind`, the
//! service's own pass announcing the machine, `sync machines` reading it back.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::IntoResponse;
use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

const STREAM: &str = "stream-presence-e2e";

/// `nxs` as one machine: its own `$HOME`, standing in `cwd`. Built on the gated binary path — the
/// freshness check first, so a stale `nxs` can never answer — but as a plain `Command`, because the
/// service has to be spawned and left running.
fn nxs(home: &Path, cwd: &Path) -> Command {
    nxs_test_support::assert_multicall_binary_fresh();
    let mut c = Command::new(assert_cmd::cargo::cargo_bin("nxs"));
    c.current_dir(cwd).pin_home(home).env("NXC_TIMER", "dry");
    c
}

fn json_of(out: std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

fn serve(router: axum::Router) -> String {
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

fn relay_with_clock() -> (String, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000_000));
    let clock_now = Arc::clone(&now);
    let router = nxs_server::app::app_with_clock(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
        Arc::new(move || clock_now.load(Ordering::SeqCst)),
    );
    (serve(router), now)
}

/// What a relay from before 6j6v.f0b5 answers: every route as today, `404` on `/machines`.
fn relay_from_before_presence() -> String {
    let router = nxs_server::app::app(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    )
    .layer(axum::middleware::from_fn(
        |req: axum::extract::Request, next: axum::middleware::Next| async move {
            if req.uri().path().ends_with("/machines") {
                axum::http::StatusCode::NOT_FOUND.into_response()
            } else {
                next.run(req).await
            }
        },
    ));
    serve(router)
}

/// One machine: a pinned home, and in it a workspace bound to `relay`'s shared stream.
struct Machine {
    home: TempDir,
    ws: PathBuf,
    _ws_dir: TempDir,
}

fn machine(relay: &str, name: &str) -> Machine {
    let home = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let ws = std::fs::canonicalize(ws_dir.path()).unwrap();
    json_of(
        nxs(home.path(), &ws)
            .args(["init", "--module", "flow", "--json"])
            .output()
            .unwrap(),
    );
    json_of(
        nxs(home.path(), &ws)
            .args(["sync", "bind", "--join", STREAM, "--endpoint", relay])
            .args(["--no-daemon", "--json"])
            .output()
            .unwrap(),
    );
    let named = json_of(
        nxs(home.path(), &ws)
            .args(["sync", "machine", name, "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(named["name"], name, "{named}");
    Machine {
        home,
        ws,
        _ws_dir: ws_dir,
    }
}

/// Kill and reap on the way out, so a failing assertion never leaves a service running. Its output
/// goes to a log in its home, so a service that fails to start shows WHY in the failure, not only
/// "timed out".
struct Service {
    child: Child,
    log: PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Service {
    fn log(&self) -> String {
        format!(
            "--- {} ---\n{}",
            self.log.display(),
            std::fs::read_to_string(&self.log).unwrap_or_default()
        )
    }
}

fn start_service(m: &Machine, interval_secs: u64) -> Service {
    let log = m.home.path().join("service.log");
    let out = std::fs::File::create(&log).unwrap();
    let err = out.try_clone().unwrap();
    Service {
        child: nxs(m.home.path(), m.home.path())
            .args(["sync", "daemon", "--interval", &interval_secs.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("the service starts"),
        log,
    }
}

fn machines(m: &Machine) -> Value {
    json_of(
        nxs(m.home.path(), &m.ws)
            .args(["sync", "machines", "--json"])
            .output()
            .unwrap(),
    )
}

/// `(name, online, this_machine)` per listed machine, sorted by name so a poll does not depend on
/// which service happened to pass last.
fn roster(reading: &Value) -> Vec<(String, bool, bool)> {
    let mut rows: Vec<(String, bool, bool)> = reading["machines"]
        .as_array()
        .unwrap_or_else(|| panic!("no machines array: {reading}"))
        .iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap().to_string(),
                m["online"].as_bool().unwrap(),
                m["this_machine"].as_bool().unwrap(),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn wait_for(secs: u64, what: &str, services: &[&Service], mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let logs: Vec<String> = services.iter().map(|s| s.log()).collect();
    assert!(f(), "timed out waiting for {what}\n{}", logs.join("\n"));
}

#[test]
fn two_machines_see_each_other_and_a_stopped_one_goes_offline_after_its_window() {
    let (relay, now) = relay_with_clock();
    let laptop = machine(&relay, "MacBook");
    let mini = machine(&relay, "Mac mini");
    let laptop_service = start_service(&laptop, 2);
    let mini_service = start_service(&mini, 2);

    wait_for(
        60,
        "both services to announce",
        &[&laptop_service, &mini_service],
        || roster(&machines(&laptop)).len() == 2,
    );
    assert_eq!(
        roster(&machines(&laptop)),
        [
            ("Mac mini".to_string(), true, false),
            ("MacBook".to_string(), true, true)
        ],
        "the laptop lists both, and knows which one it is"
    );
    assert_eq!(
        roster(&machines(&mini)),
        [
            ("Mac mini".to_string(), true, true),
            ("MacBook".to_string(), true, false)
        ],
        "and so does the mini"
    );
    let reading = machines(&laptop);
    assert_eq!(reading["stream_id"], STREAM);
    assert_eq!(reading["endpoint"], relay.as_str());
    assert_eq!(reading["presence"], "reported");
    assert_eq!(reading["truncated"], false);
    let first = &reading["machines"][0];
    // A 2-second interval is announced as the 300 s retry backoff: a promise shorter than that
    // could not absorb one failed pass (review of PR #485, Code Quality #6).
    assert_eq!(first["interval_secs"], 300, "{first}");
    assert_eq!(first["online_within_secs"], 2 * 300 + 60, "{first}");
    assert_eq!(
        first["last_seen"], 1_000_000,
        "the relay's clock, not the machine's: {first}"
    );
    assert_eq!(first["age_secs"], 0, "{first}");
    assert_eq!(
        first["machine_id"].as_str().map(str::len),
        Some(26),
        "{first}"
    );
    assert_eq!(first["last_seen_at"], "1970-01-12T13:46:40Z", "{first}");

    // The mini stops. On the relay's clock, a whole window passes; the laptop's service keeps
    // announcing (its next pass is stamped with the moved clock), the mini's last sighting ages out.
    drop(mini_service);
    now.fetch_add(2 * 300 + 60 + 1, Ordering::SeqCst);
    let expected = [
        ("Mac mini".to_string(), false, false),
        ("MacBook".to_string(), true, true),
    ];
    wait_for(
        30,
        "the laptop's next pass after the clock moved",
        &[&laptop_service],
        || roster(&machines(&laptop)) == expected,
    );

    let human = nxs(laptop.home.path(), &laptop.ws)
        .args(["sync", "machines"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("2 machines, 1 online"), "{human}");
    let mini_line = human.lines().find(|l| l.contains("Mac mini")).unwrap();
    assert!(mini_line.contains("offline"), "{human}");
    assert!(mini_line.contains("last seen 11m ago"), "{human}");
    let laptop_line = human.lines().find(|l| l.contains("MacBook")).unwrap();
    assert!(laptop_line.contains("online"), "{human}");
    assert!(laptop_line.contains("(this machine)"), "{human}");
    assert!(
        !human.contains("note:"),
        "an honest stream is not flagged as partial: {human}"
    );
}

#[test]
fn binding_says_under_which_name_the_relay_will_list_this_machine() {
    let (relay, _now) = relay_with_clock();
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    json_of(
        nxs(home.path(), ws.path())
            .args(["init", "--module", "flow", "--json"])
            .output()
            .unwrap(),
    );
    let bound = nxs(home.path(), ws.path())
        .args([
            "sync",
            "bind",
            "--join",
            STREAM,
            "--endpoint",
            &relay,
            "--no-daemon",
        ])
        .output()
        .unwrap();
    assert!(bound.status.success());
    let name = json_of(
        nxs(home.path(), ws.path())
            .args(["sync", "machine", "--json"])
            .output()
            .unwrap(),
    )["name"]
        .as_str()
        .unwrap()
        .to_string();
    let stderr = String::from_utf8_lossy(&bound.stderr);
    assert!(
        stderr.contains(&format!("this machine is listed as \"{name}\"")),
        "{stderr}"
    );
    assert!(stderr.contains("nxs sync machine <name>"), "{stderr}");
}

#[test]
fn listing_the_machines_leaves_no_trace_on_a_machine_that_has_no_identity() {
    let (relay, _now) = relay_with_clock();
    let m = machine(&relay, "MacBook");
    let file = m.home.path().join(".nexusflow").join("machine.toml");
    std::fs::remove_file(&file).unwrap();
    machines(&m);
    assert!(!file.exists(), "a read minted an identity");
}

#[test]
fn the_global_endpoint_and_a_one_shot_remote_are_honoured_like_run_honours_them() {
    // The guide's own flow: a global default, and a workspace bound without `--endpoint`.
    let (relay, _now) = relay_with_clock();
    let old_relay = relay_from_before_presence();
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    json_of(
        nxs(home.path(), ws.path())
            .args(["init", "--module", "flow", "--json"])
            .output()
            .unwrap(),
    );
    json_of(
        nxs(home.path(), ws.path())
            .args(["sync", "endpoint", &relay, "--json"])
            .output()
            .unwrap(),
    );
    json_of(
        nxs(home.path(), ws.path())
            .args(["sync", "bind", "--join", STREAM, "--no-daemon", "--json"])
            .output()
            .unwrap(),
    );
    let reading = json_of(
        nxs(home.path(), ws.path())
            .args(["sync", "machines", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(reading["endpoint"], relay.as_str(), "the global default");
    assert_eq!(reading["presence"], "reported");

    let reading = json_of(
        nxs(home.path(), ws.path())
            .args(["sync", "machines", "--remote", &old_relay, "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        reading["endpoint"],
        old_relay.as_str(),
        "the one-shot override"
    );
    assert_eq!(reading["presence"], "unsupported");
}

#[test]
fn an_unreachable_relay_is_an_error_that_names_it() {
    let (relay, _now) = relay_with_clock();
    let m = machine(&relay, "MacBook");
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let out = nxs(m.home.path(), &m.ws)
        .args(["sync", "machines", "--remote", &closed, "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["error"]["kind"], "io", "{err}");
    assert!(
        err["error"]["msg"].as_str().unwrap().contains(&closed),
        "{err}"
    );
}

#[test]
fn a_hand_edited_machine_file_no_relay_would_accept_is_refused_with_the_way_out() {
    let (relay, _now) = relay_with_clock();
    let m = machine(&relay, "MacBook");
    std::fs::write(
        m.home.path().join(".nexusflow").join("machine.toml"),
        "id = \"01j8zqk7m2n4p6r8s0t2v4w6x8\"\nname = \"Mac\u{202E}mini\"\n",
    )
    .unwrap();
    let out = nxs(m.home.path(), &m.ws)
        .args(["sync", "machine", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["error"]["kind"], "validation", "{err}");
    let msg = err["error"]["msg"].as_str().unwrap();
    assert!(
        msg.contains("machine.toml") && msg.contains("delete"),
        "{msg}"
    );

    // Listing still works: the broken local file costs only the "(this machine)" marker.
    let listing = nxs(m.home.path(), &m.ws)
        .args(["sync", "machines", "--json"])
        .output()
        .unwrap();
    assert!(listing.status.success());
    assert!(
        String::from_utf8_lossy(&listing.stderr)
            .contains("cannot tell which of these is this machine"),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
}

#[test]
fn a_manual_sync_run_syncs_but_never_lists_the_machine() {
    // Only the service keeps the promise "online" makes, so only the service announces.
    let (relay, _now) = relay_with_clock();
    let m = machine(&relay, "MacBook");
    json_of(
        nxs(m.home.path(), &m.ws)
            .args(["sync", "run", "--json"])
            .output()
            .unwrap(),
    );
    let reading = machines(&m);
    assert_eq!(reading["presence"], "reported");
    assert_eq!(reading["machines"], serde_json::json!([]));

    let human = nxs(m.home.path(), &m.ws)
        .args(["sync", "machines"])
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("no machine has announced itself"), "{human}");
}

#[test]
fn a_relay_from_before_presence_is_named_not_an_error() {
    let relay = relay_from_before_presence();
    let m = machine(&relay, "MacBook");
    let reading = machines(&m);
    assert_eq!(reading["ok"], true);
    assert_eq!(reading["presence"], "unsupported");
    assert_eq!(reading["machines"], serde_json::json!([]));

    let human = nxs(m.home.path(), &m.ws)
        .args(["sync", "machines"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(
        human.contains("does not record which machines sync a stream"),
        "{human}"
    );
}

#[test]
fn this_machine_is_shown_and_renamed_and_its_id_survives_the_rename() {
    let home = TempDir::new().unwrap();
    let shown = json_of(
        nxs(home.path(), home.path())
            .args(["sync", "machine", "--json"])
            .output()
            .unwrap(),
    );
    let id = shown["machine_id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 26, "{shown}");
    assert_eq!(shown["instance"], "nexus-flow", "{shown}");
    assert_eq!(shown["renamed"], false);

    let renamed = json_of(
        nxs(home.path(), home.path())
            .args(["sync", "machine", "Mac mini", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(renamed["machine_id"], id.as_str());
    assert_eq!(renamed["name"], "Mac mini");
    assert_eq!(renamed["renamed"], true);

    let human = nxs(home.path(), home.path())
        .args(["sync", "machine"])
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("Mac mini"), "{human}");
    assert!(human.contains(&id), "{human}");

    let refused = nxs(home.path(), home.path())
        .args(["sync", "machine", "   ", "--json"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    let err: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(err["error"]["kind"], "validation", "{err}");
}

#[test]
fn listing_the_machines_of_a_workspace_that_syncs_nowhere_says_how_to_bind_it() {
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    json_of(
        nxs(home.path(), ws.path())
            .args(["init", "--module", "flow", "--json"])
            .output()
            .unwrap(),
    );
    let out = nxs(home.path(), ws.path())
        .args(["sync", "machines", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["error"]["kind"], "validation");
    assert!(
        err["error"]["msg"]
            .as_str()
            .unwrap()
            .contains("nxs sync bind"),
        "{err}"
    );
}
