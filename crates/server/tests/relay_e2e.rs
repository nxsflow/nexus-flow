//! The end-to-end proof (nexus-flow-6j6v.6hbd): an item created in ONE workspace shows up in a
//! SECOND one, carried by the **shipped `nxf-relay` binary** over real HTTP, with the op log
//! durable in a real Postgres — and it survives the relay being restarted mid-flow.
//!
//! ## The gap this closes
//!
//! Every other automated test in this repo drives an in-process relay: `relay_http.rs` and
//! `crates/nxs/tests/sync_e2e.rs` both build the axum router in the test binary and serve it on a
//! background thread. That proves the routing and the data path, and it is worth having — but it
//! never touches the artifact people download. No `nxf-relay` process, no `NXF_RELAY_BACKEND`
//! selection, no boot path, no restart, no second workspace on disk. "All tests green" therefore
//! did not mean sync works, and the only thing that did cover it was a runbook someone had to
//! walk by hand (`6j6v.be0v`).
//!
//! So everything here is the real thing:
//!
//! * the relay is `nxf-relay`, spawned as a child process, reached over TCP;
//! * its backend is `NXF_RELAY_BACKEND=postgres` — the durable, managed-Postgres path that makes
//!   "sync to a Postgres, e.g. Supabase" true of the shipped binary;
//! * the clients are the real `nxf`/`nxs` binary in two real workspaces, driven the way a human
//!   drives them (`init`, `create`, `sync bind`, `sync run`), not through the library;
//! * the restart is a real kill and re-spawn against the same database.
//!
//! ## Where it runs
//!
//! Keyed on `DATABASE_URL` exactly like the parity suite, so it runs in **both** Postgres jobs
//! with no per-job knob: against the service container on every change (deterministic, no foreign
//! dependency) and against the managed Supabase endpoint (the outward proof). And it inherits
//! that suite's fail-loud rule — under `CI` a missing `DATABASE_URL` is a hard failure, never a
//! silent skip, because a skipped proof reported as green is the exact trap this ticket exists to
//! close.
//!
//! Each run gets its own schema for the relay's tables AND its own freshly minted `stream_id`, so
//! two CI runs against one shared managed project cannot see each other's ops.
#![cfg(feature = "postgres")]

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

mod common;

use common::{base_url, drop_schema, isolated_schema};
use nxs_test_support::PinHome;

/// How long the relay gets to bind its port. Generous: booting the Postgres backend opens a real
/// connection first, and against a managed endpoint that is a TLS handshake across the internet.
const RELAY_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

// ----- the relay, as a real child process --------------------------------------------------

/// A running `nxf-relay` process. Killed on drop, so a panicking assertion never leaves one
/// listening on a port for the rest of the CI job.
struct Relay {
    child: Child,
    addr: SocketAddr,
    pg_url: String,
    log: PathBuf,
}

impl Relay {
    /// Spawn the shipped binary against `pg_url` on a port nothing else is using, and wait for it
    /// to accept connections.
    fn start(pg_url: &str, addr: SocketAddr, log: PathBuf) -> Relay {
        let mut relay = Relay {
            child: spawn_relay(pg_url, addr, &log),
            addr,
            pg_url: pg_url.to_string(),
            log,
        };
        relay.wait_until_listening();
        relay
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Block until the relay accepts a connection, or fail with everything it said on the way
    /// down. A relay that dies at boot is the single most likely failure here — a wrong URL, a
    /// database that refuses TLS, a backend the binary was not built with — and the reason is
    /// always in its log, so the log is what the failure prints.
    fn wait_until_listening(&mut self) {
        let deadline = Instant::now() + RELAY_BOOT_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "nxf-relay exited before it listened ({status}). Its output:\n{}",
                    self.output()
                );
            }
            if TcpStream::connect_timeout(&self.addr, Duration::from_millis(500)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "nxf-relay did not listen on {} within {:?}. Its output:\n{}",
            self.addr,
            RELAY_BOOT_TIMEOUT,
            self.output()
        );
    }

    /// Everything the relay has written so far — the diagnostic every failure in this file
    /// carries, because a bare "connection refused" says nothing about why.
    fn output(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_else(|e| format!("<no relay log: {e}>"))
    }

    /// Kill it and bring it back on the same port, against the same database.
    ///
    /// The point of the exercise: the relay is a *dumb durable carrier*. Its truth lives in
    /// Postgres, so a restart must be invisible to the replicas — no re-bind, no re-push, no
    /// lost ops. Nothing in the automated suite covered this before; the runbook asked a human to
    /// do it by hand.
    fn restart(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The port only frees once the process is reaped, and the OS may hold it briefly after.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline
            && TcpStream::connect_timeout(&self.addr, Duration::from_millis(200)).is_ok()
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        self.child = spawn_relay(&self.pg_url, self.addr, &self.log);
        self.wait_until_listening();
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Launch `nxf-relay` with the Postgres backend, appending to `log`.
///
/// Appending, not truncating: [`Relay::restart`] reuses the same file, and a failure after the
/// restart is much easier to read when the first process's boot line is still above it.
fn spawn_relay(pg_url: &str, addr: SocketAddr, log: &Path) -> Child {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("open relay log");
    Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"))
        .env("NXF_RELAY_BACKEND", "postgres")
        .env("NXF_RELAY_PG_URL", pg_url)
        .env("NXF_RELAY_ADDR", addr.to_string())
        // `main.rs` falls back to `DATABASE_URL` when `NXF_RELAY_PG_URL` is unset. Removing it
        // means a refactor that dropped the variable we DO set cannot quietly succeed against the
        // unscoped, shared database — it fails here instead.
        .env_remove("DATABASE_URL")
        .stdout(Stdio::from(file.try_clone().expect("dup relay log")))
        .stderr(Stdio::from(file))
        .spawn()
        .expect("spawn nxf-relay")
}

/// A loopback port nothing is listening on. Bound and released rather than guessed, so two test
/// binaries running in parallel cannot pick the same one.
fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read the bound port")
}

// ----- the two workspaces, driven through the shipped CLI ------------------------------------

/// One replica: a workspace directory plus a HOME of its own.
///
/// The private HOME is not hygiene, it is correctness: `nxs sync bind` registers the workspace in
/// `~/.nexusflow/workspaces.toml`, and a test that inherited the developer's HOME would write into
/// their real registry. `crates/nxs/tests/sync_e2e.rs` carries a note about that exact accident
/// happening twice on a real machine.
struct Replica {
    dir: PathBuf,
    home: PathBuf,
}

impl Replica {
    fn new(root: &Path, name: &str) -> Replica {
        let dir = root.join(name);
        let home = root.join(format!("home-{name}"));
        std::fs::create_dir_all(&dir).expect("workspace dir");
        std::fs::create_dir_all(&home).expect("home dir");
        let replica = Replica { dir, home };
        replica.nxf(&["init", "--plugin", "issue-tracker", "--json"]);
        replica
    }

    /// Run the shipped binary under the `nxf` persona and return its stdout.
    fn nxf(&self, args: &[&str]) -> String {
        self.run("nxf", args)
    }

    /// …and under the `nxs` persona, which is where `sync` lives (it is a platform operation, not
    /// a flow one).
    fn nxs(&self, args: &[&str]) -> String {
        self.run("nxs", args)
    }

    fn run(&self, persona: &str, args: &[&str]) -> String {
        // Through the gated resolver, not `Command::cargo_bin`: `cargo test -p nxs-server` never
        // rebuilds `nxs`, and `nxf` is an argv[0] symlink to it — so without the freshness gate
        // this test would happily assert against an arbitrarily old binary and pass (0yyw).
        let output = nxs_test_support::cargo_bin(persona)
            .args(args)
            .current_dir(&self.dir)
            .pin_home(&self.home)
            .output()
            .unwrap_or_else(|e| panic!("run {persona} {args:?}: {e}"));
        assert!(
            output.status.success(),
            "{persona} {args:?} failed ({}):\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).expect("utf-8 output")
    }

    /// Create one item and return its id.
    fn create(&self, title: &str) -> String {
        let out = self.nxf(&[
            "create",
            "--type",
            "chore",
            "--title",
            title,
            "--description",
            "relay e2e",
            "--priority",
            "P2",
            "--json",
        ]);
        json(&out)["id"].as_str().expect("created id").to_string()
    }

    /// One push/pull pass against `endpoint`, returning the pass's own report so a caller can
    /// assert ops actually moved rather than that the command exited 0.
    fn sync(&self, endpoint: &str) -> Value {
        json(&self.nxs(&["sync", "run", "--remote", endpoint, "--json"]))
    }

    /// Every item title in this replica's store right now.
    fn titles(&self) -> Vec<String> {
        json(&self.nxf(&["list", "--json"]))
            .as_array()
            .expect("list returns an array")
            .iter()
            .filter_map(|item| item["title"].as_str().map(str::to_string))
            .collect()
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn pushed(pass: &Value) -> u64 {
    pass["pushed"].as_u64().expect("pass reports pushed")
}

fn pulled(pass: &Value) -> u64 {
    pass["pulled"].as_u64().expect("pass reports pulled")
}

// ----- the proof ------------------------------------------------------------------------------

#[test]
fn an_item_travels_from_one_workspace_to_another_through_the_shipped_relay() {
    let Some(base) = base_url("relay_e2e") else {
        return;
    };
    // The relay's own tables get a schema of their own, so a run against a SHARED managed project
    // never reads or writes another run's rows.
    let (schema, pg_url) = isolated_schema(&base, "e2e");
    let root = TempDir::new().expect("temp root");
    let relay = Relay::start(&pg_url, free_addr(), root.path().join("relay.log"));
    let endpoint = relay.base_url();

    let a = Replica::new(root.path(), "a");
    let b = Replica::new(root.path(), "b");

    // A mints the stream; B joins the id A minted. A fresh id per run is what keeps concurrent CI
    // runs on one managed project from crossing over.
    let bound = json(&a.nxs(&[
        "sync",
        "bind",
        "--create",
        "--no-daemon",
        "--endpoint",
        &endpoint,
        "--json",
    ]));
    let stream = bound["stream_id"]
        .as_str()
        .expect("a stream id")
        .to_string();
    b.nxs(&[
        "sync",
        "bind",
        "--join",
        &stream,
        "--no-daemon",
        "--endpoint",
        &endpoint,
        "--json",
    ]);

    a.create("from replica A");
    let a_push = a.sync(&endpoint);
    assert!(
        pushed(&a_push) > 0,
        "A's ops must actually reach the relay, not just exit 0. Relay said:\n{}",
        relay.output()
    );

    let b_pull = b.sync(&endpoint);
    assert!(
        pulled(&b_pull) > 0,
        "B must actually receive ops. Relay said:\n{}",
        relay.output()
    );
    assert!(
        b.titles().contains(&"from replica A".to_string()),
        "an item created in workspace A must be readable BY TITLE in workspace B, carried by the \
         shipped nxf-relay binary over a real Postgres. B has: {:?}\nRelay said:\n{}",
        b.titles(),
        relay.output()
    );

    drop(relay);
    drop_schema(&base, &schema);
}

#[test]
fn the_op_log_survives_a_relay_restart_mid_flow() {
    let Some(base) = base_url("relay_e2e_restart") else {
        return;
    };
    let (schema, pg_url) = isolated_schema(&base, "e2e_restart");
    let root = TempDir::new().expect("temp root");
    let mut relay = Relay::start(&pg_url, free_addr(), root.path().join("relay.log"));
    let endpoint = relay.base_url();

    let a = Replica::new(root.path(), "a");
    let b = Replica::new(root.path(), "b");
    let bound = json(&a.nxs(&[
        "sync",
        "bind",
        "--create",
        "--no-daemon",
        "--endpoint",
        &endpoint,
        "--json",
    ]));
    let stream = bound["stream_id"]
        .as_str()
        .expect("a stream id")
        .to_string();
    b.nxs(&[
        "sync",
        "bind",
        "--join",
        &stream,
        "--no-daemon",
        "--endpoint",
        &endpoint,
        "--json",
    ]);

    a.create("written before the restart");
    a.sync(&endpoint);
    b.sync(&endpoint);

    // The relay process is destroyed and rebuilt. Nothing in either workspace is told about it.
    relay.restart();

    // What the restart has to prove is not that the new process starts — it is that the log the
    // OLD process accepted is still there, and that a replica which never saw the old process can
    // still be caught up by the new one.
    b.create("written after the restart");
    let b_push = b.sync(&endpoint);
    assert!(
        pushed(&b_push) > 0,
        "B must be able to push to the restarted relay. Relay said:\n{}",
        relay.output()
    );

    let a_pull = a.sync(&endpoint);
    assert!(
        pulled(&a_pull) > 0,
        "A must receive B's post-restart ops. Relay said:\n{}",
        relay.output()
    );

    let titles = a.titles();
    for expected in ["written before the restart", "written after the restart"] {
        assert!(
            titles.contains(&expected.to_string()),
            "the durable log must span the restart: {expected:?} missing from {titles:?}\n\
             Relay said:\n{}",
            relay.output()
        );
    }

    // A third replica that has never spoken to the relay before joins AFTER the restart and is
    // caught up from the durable log alone — the case the runbook called scenario 3.
    let c = Replica::new(root.path(), "c");
    c.nxs(&[
        "sync",
        "bind",
        "--join",
        &stream,
        "--no-daemon",
        "--endpoint",
        &endpoint,
        "--json",
    ]);
    c.sync(&endpoint);
    let c_titles = c.titles();
    for expected in ["written before the restart", "written after the restart"] {
        assert!(
            c_titles.contains(&expected.to_string()),
            "a replica bound after the restart must receive the whole log: {expected:?} missing \
             from {c_titles:?}\nRelay said:\n{}",
            relay.output()
        );
    }

    drop(relay);
    drop_schema(&base, &schema);
}
