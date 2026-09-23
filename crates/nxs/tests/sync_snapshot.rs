//! **A new machine starts from a snapshot and pulls only the rest** (nxf 6j6v.mxt2) — on the REAL
//! binary, the way a person does it: `nxs sync snapshot <file>` on a machine that syncs the board,
//! `nxs sync bind --snapshot <file>` on the new one, then an ordinary pass. The board the new
//! machine shows afterwards must be the source's — `nxf next`, `nxf blocked` and the full list —
//! although it never folded the history the snapshot covers.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

/// `nxs` (or a persona of it) as one machine: its own `$HOME`, standing in `cwd`.
fn run(bin: &str, home: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    nxs_test_support::assert_multicall_binary_fresh();
    Command::new(assert_cmd::cargo::cargo_bin(bin))
        .args(args)
        .current_dir(cwd)
        .pin_home(home)
        .env("NXC_TIMER", "dry")
        .env("NXF_ACTOR", "tester")
        .output()
        .expect("the binary runs")
}

fn json(bin: &str, home: &Path, cwd: &Path, args: &[&str]) -> Value {
    let out = run(bin, home, cwd, args);
    assert!(
        out.status.success(),
        "{bin} {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{bin} {args:?}: not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// What an agent reads off the board: the command's JSON, whole.
fn board(home: &Path, cwd: &Path, args: &[&str]) -> Value {
    json("nxf", home, cwd, args)
}

fn relay() -> String {
    serve(nxs_server::app::app(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    ))
}

/// A relay that also watches for the snapshot's position probe (a one-op pull) and records, at
/// that moment, whether `watched/.nxs/sync.toml` already exists — the ordering `bind --snapshot`
/// promises, observed from outside the process.
fn relay_watching(watched: PathBuf) -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let probes = Arc::new(AtomicUsize::new(0));
    let bound_at_probe = Arc::new(AtomicBool::new(false));
    let (p, b) = (Arc::clone(&probes), Arc::clone(&bound_at_probe));
    let router = nxs_server::app::app(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    )
    .layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let (p, b, watched) = (Arc::clone(&p), Arc::clone(&b), watched.clone());
            async move {
                let probe = req.method() == axum::http::Method::GET
                    && req.uri().path().ends_with("/ops")
                    && req
                        .uri()
                        .query()
                        .is_some_and(|q| q.split('&').any(|kv| kv == "limit=1"));
                if probe {
                    p.fetch_add(1, Ordering::SeqCst);
                    if watched.join(".nxs").join("sync.toml").exists() {
                        b.store(true, Ordering::SeqCst);
                    }
                }
                next.run(req).await
            }
        },
    ));
    (serve(router), probes, bound_at_probe)
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

struct Machine {
    home: TempDir,
    ws: PathBuf,
    _dir: TempDir,
}

fn machine() -> Machine {
    let home = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let ws = std::fs::canonicalize(dir.path()).unwrap();
    json(
        "nxs",
        home.path(),
        &ws,
        &["init", "--module", "flow", "--json"],
    );
    Machine {
        home,
        ws,
        _dir: dir,
    }
}

fn create(m: &Machine, title: &str) -> String {
    json(
        "nxf",
        m.home.path(),
        &m.ws,
        &[
            "create",
            "--type",
            "chore",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            "P2",
            "--json",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_new_machine_started_from_a_snapshot_shows_the_sources_board_after_pulling_only_the_rest() {
    let relay = relay();
    let source = machine();
    let (h, w) = (source.home.path(), source.ws.as_path());
    json(
        "nxs",
        h,
        w,
        &[
            "sync",
            "bind",
            "--create",
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ],
    );

    // A board with shape: a chain of dependencies, a closed item, a claimed one.
    let a = create(&source, "design the snapshot");
    let b = create(&source, "build the snapshot");
    let c = create(&source, "measure the snapshot");
    json("nxf", h, w, &["dep", "add", &b, &a, "--json"]);
    json("nxf", h, w, &["dep", "add", &c, &b, "--json"]);
    json("nxs", h, w, &["sync", "run", "--json"]);

    let file = source.ws.join("board.snapshot");
    let file = file.to_str().unwrap();
    let taken = json("nxs", h, w, &["sync", "snapshot", file, "--json"]);
    assert_eq!(taken["ok"], true, "{taken}");
    let through = taken["snapshot_through"].as_i64().unwrap();
    assert!(
        through > 0 && taken["bytes"].as_u64().unwrap() > 0,
        "{taken}"
    );

    // The rest: history written after the snapshot, which the new machine must pull.
    json(
        "nxf",
        h,
        w,
        &["close", &a, "--reason", "designed", "--json"],
    );
    let d = create(&source, "write the guide");
    json("nxf", h, w, &["claim", &d, "--json"]);
    let after = json("nxs", h, w, &["sync", "run", "--json"]);
    let rest = after["pushed"].as_u64().unwrap();
    assert!(rest > 0, "{after}");

    // The new machine.
    let fresh = machine();
    let (fh, fw) = (fresh.home.path(), fresh.ws.as_path());
    let bound = json(
        "nxs",
        fh,
        fw,
        &[
            "sync",
            "bind",
            "--snapshot",
            file,
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ],
    );
    assert_eq!(bound["mode"], "snapshot", "{bound}");
    assert_eq!(
        bound["stream_id"], taken["stream_id"],
        "it joined the snapshot's stream"
    );
    assert_eq!(bound["snapshot"]["views"], "taken", "{bound}");
    assert_eq!(bound["snapshot"]["position"], "confirmed", "{bound}");
    assert_eq!(bound["snapshot"]["resumes_from"], through, "{bound}");

    let pass = json("nxs", fh, fw, &["sync", "run", "--json"]);
    assert_eq!(
        pass["pulled"].as_u64().unwrap(),
        rest,
        "it pulled ONLY what the source wrote after the snapshot: {pass}"
    );

    for args in [
        &["next", "--json"][..],
        &["blocked", "--json"][..],
        &["list", "--json"][..],
    ] {
        assert_eq!(
            board(fh, fw, args),
            board(h, w, args),
            "`nxf {}` differs between the source and the machine that started from its snapshot",
            args.join(" ")
        );
    }
}

/// The refusals, each by name and each leaving the workspace as it found it.
#[test]
fn a_snapshot_is_refused_where_it_cannot_be_what_it_promises() {
    let relay = relay();
    let source = machine();
    let (h, w) = (source.home.path(), source.ws.as_path());
    json(
        "nxs",
        h,
        w,
        &[
            "sync",
            "bind",
            "--create",
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ],
    );
    create(&source, "one");
    json("nxs", h, w, &["sync", "run", "--json"]);
    let file = source.ws.join("board.snapshot");
    let file = file.to_str().unwrap();
    json("nxs", h, w, &["sync", "snapshot", file, "--json"]);

    // Ops the relay has not seen yet: no snapshot until they are pushed.
    create(&source, "not pushed yet");
    let out = run("nxs", h, w, &["sync", "snapshot", file]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("sync first"), "{stderr}");

    // A workspace that already holds ops syncs the ordinary way — and stays unbound.
    let busy = machine();
    create(&busy, "mine");
    let out = run(
        "nxs",
        busy.home.path(),
        &busy.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            file,
            "--endpoint",
            &relay,
            "--no-daemon",
        ],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("fresh replica"), "{stderr}");
    assert!(!busy.ws.join(".nxs/sync.toml").exists(), "left unbound");

    // A --join that names another stream than the snapshot's.
    let other = machine();
    let out = run(
        "nxs",
        other.home.path(),
        &other.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            file,
            "--join",
            "stream-other",
            "--endpoint",
            &relay,
            "--no-daemon",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("belongs to"));
    assert!(!other.ws.join(".nxs/sync.toml").exists(), "left unbound");

    // No relay to confirm the position against.
    let out = run(
        "nxs",
        other.home.path(),
        &other.ws,
        &["sync", "bind", "--snapshot", file, "--no-daemon"],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--endpoint"));
    assert!(!other.ws.join(".nxs/sync.toml").exists(), "left unbound");
}

/// A synced source with a board and its snapshot file — the start of every test below.
fn source_with_snapshot(relay: &str) -> (Machine, String, Value) {
    let source = machine();
    let (h, w) = (source.home.path(), source.ws.as_path());
    json(
        "nxs",
        h,
        w,
        &[
            "sync",
            "bind",
            "--create",
            "--endpoint",
            relay,
            "--no-daemon",
            "--json",
        ],
    );
    let a = create(&source, "one");
    let b = create(&source, "two");
    json("nxf", h, w, &["dep", "add", &b, &a, "--json"]);
    json("nxs", h, w, &["sync", "run", "--json"]);
    let file = source
        .ws
        .join("board.snapshot")
        .to_str()
        .unwrap()
        .to_string();
    let taken = json("nxs", h, w, &["sync", "snapshot", &file, "--json"]);
    (source, file, taken)
}

/// Rewrite a snapshot file the way a newer nxs could have written it.
fn rewritten(file: &str, change: impl FnOnce(&mut Value)) -> String {
    use std::io::{Read, Write};
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(file).unwrap())
        .read_to_end(&mut json)
        .unwrap();
    let mut envelope: Value = serde_json::from_slice(&json).unwrap();
    change(&mut envelope);
    let out = format!("{file}.rewritten");
    let mut gz = flate2::write::GzEncoder::new(
        std::fs::File::create(&out).unwrap(),
        flate2::Compression::default(),
    );
    gz.write_all(&serde_json::to_vec(&envelope).unwrap())
        .unwrap();
    gz.finish().unwrap();
    out
}

/// A workspace a refused `bind --snapshot` left behind is exactly the one it found: unbound,
/// empty, and bindable the ordinary way.
fn assert_left_as_found(m: &Machine, relay: &str, stream: &str) {
    assert!(
        !m.ws.join(".nxs/sync.toml").exists(),
        "a refused bind leaves the workspace unbound"
    );
    assert_eq!(
        board(m.home.path(), &m.ws, &["list", "--json"]),
        serde_json::json!([]),
        "…and its log empty"
    );
    json(
        "nxs",
        m.home.path(),
        &m.ws,
        &[
            "sync",
            "bind",
            "--join",
            stream,
            "--endpoint",
            relay,
            "--no-daemon",
            "--json",
        ],
    );
    json("nxs", m.home.path(), &m.ws, &["sync", "run", "--json"]);
    assert_eq!(
        board(m.home.path(), &m.ws, &["list", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2,
        "…and a plain bind afterwards syncs the board the ordinary way"
    );
}

/// **The import comes before the binding exists** (review of PR #487, Test Quality #1). Watched
/// from the relay's side: when the snapshot's position probe arrives, the new workspace has no
/// `sync.toml` yet — so a background service, which skips unbound workspaces, cannot have started a
/// pass into it. And every way the import can fail after the network was reached leaves the
/// workspace as it found it.
#[test]
fn bind_from_a_snapshot_imports_before_the_binding_exists_and_a_failed_one_leaves_nothing() {
    let fresh = machine();
    let (relay, probes, bound_at_probe) = relay_watching(fresh.ws.clone());
    let (_source, file, taken) = source_with_snapshot(&relay);
    let stream = taken["stream_id"].as_str().unwrap().to_string();

    let before = probes.load(Ordering::SeqCst);
    let bound = json(
        "nxs",
        fresh.home.path(),
        &fresh.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            &file,
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ],
    );
    assert_eq!(bound["snapshot"]["position"], "confirmed", "{bound}");
    assert!(
        probes.load(Ordering::SeqCst) > before,
        "the position was probed at the relay"
    );
    assert!(
        !bound_at_probe.load(Ordering::SeqCst),
        "sync.toml existed when the position probe arrived — the binding was written before the \
         import"
    );

    // Refused after the relay was reached: a snapshot whose floor this nxs is below.
    let locked = machine();
    let crafted = rewritten(&file, |e| {
        e["image"]["min_compatible"] = serde_json::json!(999)
    });
    let out = run(
        "nxs",
        locked.home.path(),
        &locked.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            &crafted,
            "--endpoint",
            &relay,
            "--no-daemon",
        ],
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("upgrade nxs")
            || String::from_utf8_lossy(&out.stdout).contains("upgrade nxs"),
        "{out:?}"
    );
    assert_left_as_found(&locked, &relay, &stream);

    // Refused because the relay cannot be reached at all.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let offline = machine();
    let out = run(
        "nxs",
        offline.home.path(),
        &offline.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            &file,
            "--endpoint",
            &format!("http://127.0.0.1:{port}"),
            "--no-daemon",
        ],
    );
    assert!(!out.status.success(), "{out:?}");
    assert_left_as_found(&offline, &relay, &stream);
}

/// A new machine whose id prefix another replica on the stream already holds is reassigned BEFORE
/// the import (review of PR #487, Integrity #5): the remap then touches an empty store, and the
/// source's ids — which carry that prefix — arrive untouched.
#[test]
fn a_new_machine_whose_prefix_is_taken_is_reassigned_before_the_import_and_the_sources_ids_stay() {
    let relay = relay();
    let (source, file, _) = source_with_snapshot(&relay);
    let taken_prefix = std::fs::read_to_string(source.ws.join(".nxs/replica.toml"))
        .unwrap()
        .lines()
        .find_map(|l| {
            l.strip_prefix("prefix = ")
                .map(|p| p.trim_matches('"').to_string())
        })
        .unwrap();

    let fresh = machine();
    let replica = fresh.ws.join(".nxs/replica.toml");
    let own = std::fs::read_to_string(&replica).unwrap();
    let collided: String = own
        .lines()
        .map(|l| {
            if l.starts_with("prefix = ") {
                format!("prefix = \"{taken_prefix}\"")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&replica, collided + "\n").unwrap();

    let bound = json(
        "nxs",
        fresh.home.path(),
        &fresh.ws,
        &[
            "sync",
            "bind",
            "--snapshot",
            &file,
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ],
    );
    let reassigned = bound["snapshot"]["reassigned_prefix"]
        .as_str()
        .unwrap_or_else(|| panic!("the taken prefix was reassigned: {bound}"));
    assert_ne!(reassigned, taken_prefix);
    json(
        "nxs",
        fresh.home.path(),
        &fresh.ws,
        &["sync", "run", "--json"],
    );
    assert_eq!(
        board(fresh.home.path(), &fresh.ws, &["list", "--json"]),
        board(source.home.path(), &source.ws, &["list", "--json"]),
        "the source's items keep their ids"
    );
}
