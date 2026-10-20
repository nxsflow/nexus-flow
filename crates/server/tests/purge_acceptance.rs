//! The purge's real-process acceptance (6j6v.f6yb DoD, first lines): on each backend a real
//! `nxf-relay` SERVER process holds the target stream `n-<h>-1` — written by two real workspaces
//! (`nxf`/`nxs` binaries, `sync bind` + `sync run`: both register their prefix, one creates an item
//! carrying unique byte markers and then closes and archives it, which is how an item is "deleted"
//! today), padded to 5,000 ops through the relay's own push route, with 30 machines announced
//! through its presence route — beside the neighbours `n-<h>-10` and `n-<h>-1-x`. The purge runs
//! as a separate `nxf-relay purge` process under the C9 log key and with `--expect-identity`.
//!
//! Then: the dry run deletes nothing and reports what was seeded; the bound `--yes` empties the
//! target and nothing else (full dumps), no marker is left (and on SQLite not in the raw files
//! either); the confirm passes. And the racing-pusher smoke test: a workspace keeps pushing while
//! the test loops a dry run and a bound `--yes` while the exit is 4, at most five times.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};
use nxs_test_support::PinHome;

#[path = "support/purge.rs"]
mod support;

#[cfg(feature = "postgres")]
mod common;

#[cfg(feature = "dynamodb")]
#[path = "support/ddb.rs"]
mod ddb;
use support::{file_contains, purge, Purged, C9_KEY};

const H: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const TARGET_OPS: usize = 5_000;

fn target() -> String {
    format!("n-{H}-1")
}
fn neighbour() -> String {
    format!("n-{H}-10")
}
fn prefix_neighbour() -> String {
    format!("n-{H}-1-x")
}

// ----- the relay, as a real server process -------------------------------------------------------

struct Relay {
    child: Child,
    addr: SocketAddr,
    log: PathBuf,
}

impl Relay {
    fn start(env: &[(String, String)], log: PathBuf) -> Relay {
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let file = std::fs::File::create(&log).unwrap();
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"));
        cmd.env("NXF_RELAY_ADDR", addr.to_string())
            .env_remove("DATABASE_URL")
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut relay = Relay {
            child: cmd.spawn().expect("spawn nxf-relay"),
            addr,
            log,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect_timeout(&relay.addr, Duration::from_millis(300)).is_err() {
            if let Ok(Some(status)) = relay.child.try_wait() {
                panic!("nxf-relay exited ({status}): {}", relay.output());
            }
            assert!(
                Instant::now() < deadline,
                "relay did not listen: {}",
                relay.output()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        relay
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn output(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ----- a workspace, through the shipped CLI ---------------------------------------------------------

struct Replica {
    dir: PathBuf,
    home: PathBuf,
}

impl Replica {
    fn new(root: &Path, name: &str) -> Replica {
        let (dir, home) = (root.join(name), root.join(format!("home-{name}")));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let r = Replica { dir, home };
        r.run("nxf", &["init", "--plugin", "issue-tracker", "--json"]);
        r
    }

    fn run(&self, persona: &str, args: &[&str]) -> String {
        let out = nxs_test_support::cargo_bin(persona)
            .args(args)
            .current_dir(&self.dir)
            .pin_home(&self.home)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{persona} {args:?}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn join(&self, stream: &str, endpoint: &str) {
        self.run(
            "nxs",
            &[
                "sync",
                "bind",
                "--join",
                stream,
                "--no-daemon",
                "--endpoint",
                endpoint,
                "--json",
            ],
        );
    }

    fn create(&self, title: &str, description: &str) -> String {
        let out = self.run(
            "nxf",
            &[
                "create",
                "--type",
                "chore",
                "--title",
                title,
                "--description",
                description,
                "--priority",
                "P2",
                "--json",
            ],
        );
        serde_json::from_str::<Value>(&out).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn sync(&self, endpoint: &str) -> Value {
        serde_json::from_str(&self.run("nxs", &["sync", "run", "--remote", endpoint, "--json"]))
            .unwrap()
    }
}

// ----- seeding through the relay's own routes ---------------------------------------------------

fn wire(op_id: &str, value: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "pad.0001".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some(value.into()),
        author: "padding".into(),
        wall_clock: "2026-10-18T20:00:00Z".into(),
        extra: Default::default(),
    }
}

fn push(relay: &str, stream: &str, ops: &[WireOp]) {
    for chunk in ops.chunks(400) {
        ureq::post(&format!("{relay}/streams/{stream}/ops"))
            .send_json(json!({ "ops": chunk }))
            .expect("push");
    }
}

fn op_count(relay: &str, stream: &str) -> usize {
    let mut since = 0i64;
    let mut n = 0;
    loop {
        let page: Value = ureq::get(&format!("{relay}/streams/{stream}/ops"))
            .query("since", &since.to_string())
            .query("limit", "5000")
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        let ops = page["ops"].as_array().unwrap().len();
        n += ops;
        let next = page["next"].as_i64().unwrap();
        if ops == 0 || next == since {
            return n;
        }
        since = next;
    }
}

fn announce(relay: &str, stream: &str, machine: &str, name: &str) {
    ureq::post(&format!("{relay}/streams/{stream}/machines"))
        .send_json(json!({"machine_id": machine, "name": name, "interval_secs": 300}))
        .expect("announce");
}

fn register(relay: &str, stream: &str, prefix: &str, uuid: &str) {
    ureq::post(&format!("{relay}/streams/{stream}/register"))
        .send_json(json!({"prefix": prefix, "replica_uuid": uuid}))
        .expect("register");
}

/// What the seeding wrote that must never appear in a report or in the store afterwards.
struct Seeded {
    markers: Vec<String>,
    ids: Vec<String>,
}

/// Seed the target through two real workspaces and the relay's routes, and the neighbours through
/// the routes; returns the markers and ids.
fn seed(root: &Path, relay: &str, tag: &str) -> Seeded {
    let a = Replica::new(root, "a");
    let b = Replica::new(root, "b");
    a.join(&target(), relay);
    b.join(&target(), relay);
    let marker_title = format!("MARKER{tag}title");
    let marker_body = format!("MARKER{tag}body");
    let deleted = a.create(&marker_title, &marker_body);
    a.run(
        "nxf",
        &[
            "close",
            &deleted,
            "--reason",
            &format!("MARKER{tag}reason"),
            "--json",
        ],
    );
    a.run("nxf", &["archive", &deleted, "--json"]);
    for i in 0..5 {
        a.create(&format!("item a{i}"), "seeded by workspace a");
        b.create(&format!("item b{i}"), "seeded by workspace b");
    }
    assert!(a.sync(relay)["pushed"].as_u64().unwrap() > 0);
    assert!(b.sync(relay)["pushed"].as_u64().unwrap() > 0);
    a.sync(relay);
    let real = op_count(relay, &target());
    assert!(
        real > 20 && real < TARGET_OPS,
        "{real} ops from the workspaces"
    );
    let pad: Vec<WireOp> = (0..TARGET_OPS - real)
        .map(|i| wire(&format!("pad-{tag}-{i:05}"), &format!("padding {i}")))
        .collect();
    push(relay, &target(), &pad);
    assert_eq!(op_count(relay, &target()), TARGET_OPS);
    let mut ids = vec![target(), neighbour(), prefix_neighbour()];
    for i in 0..30 {
        let machine = format!("m-{tag}-{i}");
        announce(
            relay,
            &target(),
            &machine,
            &format!("MARKER{tag}machine {i}"),
        );
        ids.push(machine);
    }
    for stream in [neighbour(), prefix_neighbour()] {
        let ops: Vec<WireOp> = (0..120)
            .map(|i| wire(&format!("n-{tag}-{stream}-{i}"), &format!("neighbour {i}")))
            .collect();
        push(relay, &stream, &ops);
        register(relay, &stream, "ab12", &format!("uuid-{tag}-{stream}-1"));
        register(relay, &stream, "cd34", &format!("uuid-{tag}-{stream}-2"));
        announce(
            relay,
            &stream,
            &format!("m-{tag}-{stream}"),
            "neighbour machine",
        );
    }
    Seeded {
        markers: vec![
            marker_title,
            marker_body,
            format!("MARKER{tag}reason"),
            format!("MARKER{tag}machine"),
        ],
        ids,
    }
}

/// The acceptance on one backend: `dump` reads every table in full, `env` points the purge at the
/// store, `files` (SQLite) are searched raw for the markers.
fn acceptance(
    backend: &str,
    relay: &Relay,
    root: &Path,
    env: &[(String, String)],
    dump: &dyn Fn() -> Vec<String>,
    files: &[PathBuf],
    extra: &[(&str, &str)],
) {
    let tag = format!("{:x}", std::process::id());
    let seeded = seed(root, &relay.url(), &tag);
    let mut env: Vec<(String, String)> = env.to_vec();
    env.push(("NXS_LOG_HMAC_KEY".into(), C9_KEY.into()));
    for (k, v) in extra {
        env.push((k.to_string(), v.to_string()));
    }
    let run = |args: &[&str]| -> Purged {
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        purge(args, &env)
    };
    // The pin for this store, as the operator takes it once.
    let pin = run(&[&target(), "--dry-run"]).identity_digest();
    let pinned = |mode: &[&str]| -> Purged {
        let t = target();
        let mut args: Vec<&str> = vec![t.as_str()];
        args.extend_from_slice(mode);
        args.extend(["--expect-identity", pin.as_str()]);
        run(&args)
    };
    let without_target = |rows: Vec<String>| -> Vec<String> {
        let t = target();
        rows.into_iter()
            .filter(|r| {
                !r.contains(&format!("\"{t}\""))
                    && !r.contains(&format!("S:{t}\""))
                    && !r.contains(&format!("stream#{t}"))
            })
            .collect()
    };

    let before = dump();
    let dry = pinned(&["--dry-run"]);
    dry.expect(0, &format!("{backend}: dry run"));
    assert_eq!(dump(), before, "{backend}: a dry run deletes nothing");
    assert_eq!(dry.found("ops"), TARGET_OPS as u64, "{backend}");
    let registry = dry.kind("registry");
    let by_prefix = &registry["by_prefix"];
    if backend == "dynamodb" {
        assert_eq!(by_prefix["prefix#"], 2, "{backend}: {by_prefix}");
        assert_eq!(by_prefix["uuid#"], 2, "{backend}");
        assert_eq!(by_prefix["machine#"], 30, "{backend}");
        assert_eq!(dry.found("claims"), TARGET_OPS as u64);
        assert_eq!(dry.found("counter"), 1);
    } else {
        assert_eq!(by_prefix["prefix_claims"], 2, "{backend}: {by_prefix}");
        assert_eq!(by_prefix["machine_presence"], 30, "{backend}");
        assert!(dry.kind("ops")["value_bytes"].as_u64().unwrap() > 0);
    }
    assert_eq!(dry.kind("ops")["max_seq"], TARGET_OPS as u64);
    let digest = dry.digest();
    assert_eq!(pinned(&["--dry-run"]).digest(), digest, "{backend}: stable");

    let started = Instant::now();
    let yes = pinned(&["--yes", "--plan", &digest]);
    yes.expect(0, &format!("{backend}: --yes"));
    if backend == "dynamodb" {
        // The shipped ttl (900 s) renews at most once per 450 s.
        let renewals = yes.report["lease"]["renewals"].as_u64().unwrap();
        assert!(
            renewals <= started.elapsed().as_secs() / 450,
            "{renewals} renewals"
        );
    }
    for kind in ["ops", "registry"] {
        assert_eq!(yes.deleted(kind), dry.found(kind), "{backend}: {kind}");
    }
    let after = dump();
    assert_eq!(
        after,
        without_target(before.clone()),
        "{backend}: only the target went"
    );
    for m in &seeded.markers {
        assert!(
            !after.iter().any(|r| r.contains(m.as_str())),
            "{backend}: {m} in the dump"
        );
        for file in files {
            assert!(
                !file_contains(file, m),
                "{backend}: {m} in {}",
                file.display()
            );
        }
    }
    pinned(&["--dry-run", "--expect-empty"]).expect(0, &format!("{backend}: confirm"));
    // No id, marker or key in clear in anything the purge printed.
    for out in [&dry, &yes] {
        for needle in seeded.ids.iter().chain(seeded.markers.iter()) {
            assert!(
                !out.stdout.contains(needle.as_str()),
                "{backend}: {needle} in stdout"
            );
            assert!(
                !out.stderr.contains(needle.as_str()),
                "{backend}: {needle} in stderr"
            );
        }
    }
    assert_eq!(yes.report["sid"], "h1:t1:".to_string() + &sid_hex());
}

/// The C9 token of the target, from `nxf-relay log-hash` (C8).
fn sid_hex() -> String {
    let out = Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"))
        .args(["log-hash", "sid", &target()])
        .env("NXS_LOG_HMAC_KEY", C9_KEY)
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .trim_start_matches("sid=h1:t1:")
        .to_string()
}

// ----- SQLite --------------------------------------------------------------------------------------

fn sqlite_dump(files: &[PathBuf]) -> Vec<String> {
    let mut rows = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let conn = rusqlite::Connection::open(f).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for t in tables {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
            let n = stmt.column_count();
            let mut q = stmt.query([]).unwrap();
            while let Some(row) = q.next().unwrap() {
                let cells: Vec<String> = (0..n)
                    .map(|c| match row.get_ref(c).unwrap() {
                        rusqlite::types::ValueRef::Text(t) => {
                            format!("{:?}", String::from_utf8_lossy(t))
                        }
                        other => format!("{other:?}"),
                    })
                    .collect();
                rows.push(format!("{i}|{t}|{}", cells.join("|")));
            }
        }
    }
    rows.sort();
    rows
}

fn sqlite_acceptance(two_files: bool) {
    let root = TempDir::new().unwrap();
    let ops = root.path().join("relay.sqlite");
    let reg = if two_files {
        root.path().join("registry.sqlite")
    } else {
        ops.clone()
    };
    let env = vec![
        ("NXF_RELAY_BACKEND".to_string(), "sqlite".to_string()),
        (
            "NXF_RELAY_DB".to_string(),
            ops.to_str().unwrap().to_string(),
        ),
        (
            "NXF_RELAY_REGISTRY_DB".to_string(),
            reg.to_str().unwrap().to_string(),
        ),
    ];
    let relay = Relay::start(&env, root.path().join("relay.log"));
    let files: Vec<PathBuf> = if two_files {
        vec![ops.clone(), reg.clone()]
    } else {
        vec![ops.clone()]
    };
    let raw: Vec<PathBuf> = files
        .iter()
        .flat_map(|f| [f.clone(), PathBuf::from(format!("{}-wal", f.display()))])
        .collect();
    let dump = || sqlite_dump(&files);
    acceptance(
        if two_files {
            "sqlite (two files)"
        } else {
            "sqlite"
        },
        &relay,
        root.path(),
        &env,
        &dump,
        &raw,
        &[],
    );
}

#[test]
fn sqlite_one_file_a_real_relay_and_real_workspaces() {
    sqlite_acceptance(false);
}

#[test]
fn sqlite_two_files_a_real_relay_and_real_workspaces() {
    sqlite_acceptance(true);
}

#[test]
fn a_racing_pusher_never_makes_the_neighbours_change_and_the_loop_ends() {
    // CQ 5: loop a dry run and a bound --yes while the exit is 4, at most five times. The final
    // exit must be 0 or 3; five 4s fail ("never reached the delete"), and the neighbours stay
    // byte-identical whatever the pusher did.
    let root = TempDir::new().unwrap();
    let ops = root.path().join("relay.sqlite");
    let env = vec![
        ("NXF_RELAY_BACKEND".to_string(), "sqlite".to_string()),
        (
            "NXF_RELAY_DB".to_string(),
            ops.to_str().unwrap().to_string(),
        ),
        (
            "NXF_RELAY_REGISTRY_DB".to_string(),
            ops.to_str().unwrap().to_string(),
        ),
    ];
    let relay = Relay::start(&env, root.path().join("relay.log"));
    let url = relay.url();
    let pusher = Replica::new(root.path(), "pusher");
    pusher.join(&target(), &url);
    pusher.create("first", "racing");
    pusher.sync(&url);
    let ops_n: Vec<WireOp> = (0..50).map(|i| wire(&format!("nb-{i}"), "n")).collect();
    push(&url, &neighbour(), &ops_n);
    register(&url, &neighbour(), "ab12", "uuid-racing-n");
    let files = vec![ops.clone()];
    let outside = || -> Vec<String> {
        let t = target();
        sqlite_dump(&files)
            .into_iter()
            .filter(|r| !r.contains(&format!("\"{t}\"")))
            .collect()
    };
    let before = outside();
    let stop = Arc::new(AtomicBool::new(false));
    let racing = {
        let stop = stop.clone();
        let url = url.clone();
        std::thread::spawn(move || {
            let mut i = 0;
            while !stop.load(Ordering::SeqCst) {
                pusher.create(&format!("racing {i}"), "racing");
                pusher.sync(&url);
                i += 1;
            }
        })
    };
    let penv: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut last = None;
    for _ in 0..5 {
        let dry = purge(&[&target(), "--dry-run"], &penv);
        dry.expect(0, "dry run under a racing pusher");
        let yes = purge(&[&target(), "--yes", "--plan", &dry.digest()], &penv);
        last = yes.code;
        if yes.code != Some(4) {
            break;
        }
    }
    stop.store(true, Ordering::SeqCst);
    racing.join().unwrap();
    assert!(
        matches!(last, Some(0) | Some(3)),
        "the loop never reached the delete (last exit {last:?})"
    );
    assert_eq!(outside(), before, "the neighbours are byte-identical");
}

// ----- Postgres -------------------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod pg {
    use super::*;

    #[test]
    fn postgres_a_real_relay_and_real_workspaces() {
        let Some(base) = common::base_url("purge_acceptance_pg") else {
            return;
        };
        let (schema, url) = common::isolated_schema(&base, "purge_acc");
        let root = TempDir::new().unwrap();
        let env = vec![
            ("NXF_RELAY_BACKEND".to_string(), "postgres".to_string()),
            ("NXF_RELAY_PG_URL".to_string(), url.clone()),
        ];
        let relay = Relay::start(&env, root.path().join("relay.log"));
        let (u, s) = (url.clone(), schema.clone());
        let dump = move || -> Vec<String> {
            let (u, s) = (u.clone(), s.clone());
            std::thread::spawn(move || {
                let mut c =
                    postgres::Client::connect(&u, nxs_server::tls_pg::make_tls().unwrap()).unwrap();
                let tables: Vec<String> = c
                    .query(
                        "SELECT tablename FROM pg_tables WHERE schemaname = $1",
                        &[&s],
                    )
                    .unwrap()
                    .iter()
                    .map(|r| r.get(0))
                    .collect();
                let mut rows = Vec::new();
                for t in tables {
                    for r in c
                        .query(
                            &format!("SELECT row_to_json(x)::text FROM \"{s}\".\"{t}\" x"),
                            &[],
                        )
                        .unwrap()
                    {
                        rows.push(format!("{t}|{}", r.get::<_, String>(0)));
                    }
                }
                rows.sort();
                rows
            })
            .join()
            .unwrap()
        };
        acceptance("postgres", &relay, root.path(), &env, &dump, &[], &[]);
        drop(relay);
        common::drop_schema(&base, &schema);
    }
}

// ----- DynamoDB -------------------------------------------------------------------------------------

#[cfg(feature = "dynamodb")]
mod dynamodb {
    use super::*;

    #[test]
    fn dynamodb_a_real_relay_and_real_workspaces_at_page_seven() {
        let Some(endpoint) = ddb::endpoint("purge_acceptance_ddb") else {
            return;
        };
        let client = ddb::client(&endpoint);
        let ops = ddb::ops_table(&client);
        let reg = ddb::registry_table(&client);
        ddb::put_raw(&client, &reg, &[("stream_id", "#due"), ("sk", &target())]);
        ddb::put_raw(
            &client,
            &reg,
            &[("stream_id", "#due"), ("sk", &neighbour())],
        );
        ddb::put_raw(
            &client,
            &reg,
            &[
                ("stream_id", "#sweep"),
                ("sk", "lease"),
                ("holder", "sweep"),
            ],
        );
        let root = TempDir::new().unwrap();
        let env = vec![
            ("NXF_RELAY_BACKEND".to_string(), "dynamodb".to_string()),
            ("NXF_RELAY_DDB_TABLE".to_string(), ops.clone()),
            ("NXF_RELAY_DDB_REGISTRY_TABLE".to_string(), reg.clone()),
            ("NXF_RELAY_DDB_REGION".to_string(), ddb::REGION.to_string()),
            ("NXF_RELAY_DDB_ENDPOINT".to_string(), endpoint.clone()),
            ("AWS_ACCESS_KEY_ID".to_string(), "test".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "test".to_string()),
            ("AWS_REGION".to_string(), ddb::REGION.to_string()),
        ];
        let relay = Relay::start(&env, root.path().join("relay.log"));
        let (c, o, r) = (client.clone(), ops.clone(), reg.clone());
        let dump = move || -> Vec<String> {
            let mut rows: Vec<String> = ddb::dump(&c, &o)
                .into_iter()
                .map(|x| format!("ops|{x}"))
                .collect();
            rows.extend(ddb::dump(&c, &r).into_iter().map(|x| format!("reg|{x}")));
            rows
        };
        acceptance(
            "dynamodb",
            &relay,
            root.path(),
            &env,
            &dump,
            &[],
            &[("NXF_RELAY_TEST_PURGE_PAGE", "7")],
        );
        drop(relay);
        ddb::delete_table(&client, &ops);
        ddb::delete_table(&client, &reg);
    }
}
