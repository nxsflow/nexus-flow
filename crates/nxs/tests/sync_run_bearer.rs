//! **A pass run as a person, with a bearer on stdin, that survives the token running out**
//! (nxf 6j6v.2tn0) — on the REAL binary.
//!
//! An importer pushes a person's whole stream with `nxs sync run --bearer-stdin` under a 15-minute
//! token, then writes `nxs sync snapshot --based` — which needs the watermarks in `.nxs/sync.toml`
//! to be right. So this file proves, in this order:
//!
//! 1. the watermarks are on DISK after every push batch and every pull page, not only when the
//!    pass returns — a pass KILLED while a request is in flight leaves exactly the last batch or
//!    page that made it;
//! 2. a pass at first-import scale whose token runs out mid-push stops with its own exit code, and
//!    a second pass under a fresh token finishes it without re-pushing anything acknowledged —
//!    after which a based snapshot is taken;
//! 3. the refusals (401/403, a userinfo beside the bearer, an empty stdin), and that the token is
//!    in no output, no file and no process.
//!
//! The relay is the real axum app behind a gate that can hold a request (so the test can kill the
//! pass while it waits), expire a token after a number of requests, and record what it accepted.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

/// Shaped like a JWT, each segment distinctive on its own.
const TOKEN: &str = "hdr2tn0Qx.pay2tn0Zq.sig2tn0Xw";
/// The second token of a two-token import.
const FRESH: &str = "hdr2tn0Fr.pay2tn0Fe.sig2tn0Fs";

const OPEN: u8 = 0;
/// `TOKEN` until its allowance is spent, then 401 for it; `FRESH` always.
const TOKENS: u8 = 1;
const ALWAYS_401: u8 = 2;
const ALWAYS_403: u8 = 3;
/// `TOKEN` until its allowance is spent, then 403 for it: valid, and no longer allowed.
const TOKEN_THEN_403: u8 = 4;

/// The exit code of a pass whose token ran out (EX_TEMPFAIL: run it again).
const EXIT_TOKEN_EXPIRED: i32 = 75;

#[derive(Default)]
struct Gate {
    mode: AtomicU8,
    /// How many more requests `TOKEN` is accepted for in [`TOKENS`] mode.
    token_left: Mutex<usize>,
    /// Hold the n-th push (1-based) and the n-th pull from now on, forever; 0 = never.
    hold_push: AtomicUsize,
    hold_pull: AtomicUsize,
    pushes: AtomicUsize,
    pulls: AtomicUsize,
    /// What the held request was: the first op id of a held push, or the `since` of a held pull.
    held: Mutex<Option<String>>,
    /// The op ids of every push the relay accepted, in order, across every pass.
    acked: Mutex<Vec<String>>,
    /// The `Authorization` header of every request while the gate was not open.
    seen: Mutex<Vec<Option<String>>>,
    /// Every process of this user, argv and environment, listed during the first gated request.
    processes: Mutex<Option<String>>,
}

impl Gate {
    fn set(&self, mode: u8) {
        self.seen.lock().unwrap().clear();
        *self.processes.lock().unwrap() = None;
        self.mode.store(mode, Ordering::SeqCst);
    }

    fn hold_push(&self, n: usize) {
        *self.held.lock().unwrap() = None;
        self.pushes.store(0, Ordering::SeqCst);
        self.hold_push.store(n, Ordering::SeqCst);
    }

    fn hold_pull(&self, n: usize) {
        *self.held.lock().unwrap() = None;
        self.pulls.store(0, Ordering::SeqCst);
        self.hold_pull.store(n, Ordering::SeqCst);
    }

    fn held(&self) -> Option<String> {
        self.held.lock().unwrap().clone()
    }

    fn acked(&self) -> Vec<String> {
        self.acked.lock().unwrap().clone()
    }

    fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

fn process_table() -> String {
    #[cfg(target_os = "macos")]
    let args = ["-A", "-ww", "-E", "-o", "command="];
    #[cfg(not(target_os = "macos"))]
    let args = ["axeww", "-o", "args="];
    let out = Command::new("ps").args(args).output().expect("ps runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn query_since(uri: &axum::http::Uri) -> Option<String> {
    uri.query()?
        .split('&')
        .find_map(|kv| kv.strip_prefix("since=").map(str::to_string))
}

fn gated_relay() -> (String, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let g = Arc::clone(&gate);
    let router = nxs_server::app::app(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
    )
    .layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let g = Arc::clone(&g);
            async move {
                let is_ops = req.uri().path().ends_with("/ops");
                let is_push = is_ops && req.method() == axum::http::Method::POST;
                let is_pull = is_ops && req.method() == axum::http::Method::GET;

                // The body of a push, read here so the gate knows which ops it carries.
                let (parts, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
                let op_ids: Vec<String> = if is_push {
                    let v: Value = serde_json::from_slice(&bytes).unwrap();
                    v["ops"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|op| op["op_id"].as_str().unwrap().to_string())
                        .collect()
                } else {
                    Vec::new()
                };

                let hold = if is_push {
                    let n = g.pushes.fetch_add(1, Ordering::SeqCst) + 1;
                    (n == g.hold_push.load(Ordering::SeqCst)).then(|| op_ids[0].clone())
                } else if is_pull {
                    let n = g.pulls.fetch_add(1, Ordering::SeqCst) + 1;
                    (n == g.hold_pull.load(Ordering::SeqCst))
                        .then(|| query_since(&parts.uri).unwrap_or_default())
                } else {
                    None
                };
                if let Some(what) = hold {
                    *g.held.lock().unwrap() = Some(what);
                    // Never answered: the test kills the pass while it waits here.
                    std::future::pending::<()>().await;
                }

                let mode = g.mode.load(Ordering::SeqCst);
                if mode != OPEN {
                    let auth = parts
                        .headers
                        .get(axum::http::header::AUTHORIZATION)
                        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
                    g.seen.lock().unwrap().push(auth.clone());
                    if g.processes.lock().unwrap().is_none() {
                        let table = tokio::task::spawn_blocking(process_table).await.unwrap();
                        *g.processes.lock().unwrap() = Some(table);
                    }
                    let accepted = match (mode, auth.as_deref()) {
                        (TOKENS, Some(a)) if a == format!("Bearer {FRESH}") => true,
                        (TOKENS | TOKEN_THEN_403, Some(a)) if a == format!("Bearer {TOKEN}") => {
                            let mut left = g.token_left.lock().unwrap();
                            let accepted = *left > 0;
                            *left = left.saturating_sub(1);
                            accepted
                        }
                        _ => false,
                    };
                    if !accepted {
                        let status = if matches!(mode, ALWAYS_403 | TOKEN_THEN_403) {
                            axum::http::StatusCode::FORBIDDEN
                        } else {
                            axum::http::StatusCode::UNAUTHORIZED
                        };
                        return axum::response::IntoResponse::into_response(status);
                    }
                }

                let req = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
                let response = next.run(req).await;
                if is_push && response.status().is_success() {
                    g.acked.lock().unwrap().extend(op_ids);
                }
                response
            }
        },
    ));
    (serve(router), gate)
}

fn serve(router: axum::Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
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

fn spawn(m: &Machine, bin: &str, args: &[&str], stdin: Option<&str>) -> Child {
    nxs_test_support::assert_multicall_binary_fresh();
    let mut child = Command::new(assert_cmd::cargo::cargo_bin(bin))
        .args(args)
        .current_dir(&m.ws)
        .pin_home(m.home.path())
        .env("NXC_TIMER", "dry")
        .env("NXF_ACTOR", "tester")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    if let Some(text) = stdin {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    child
}

fn run_with(m: &Machine, bin: &str, args: &[&str], stdin: Option<&str>) -> Output {
    spawn(m, bin, args, stdin)
        .wait_with_output()
        .expect("the binary ends")
}

fn json(m: &Machine, bin: &str, args: &[&str]) -> Value {
    let out = run_with(m, bin, args, None);
    assert!(
        out.status.success(),
        "{bin} {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn machine() -> Machine {
    let home = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let ws = std::fs::canonicalize(dir.path()).unwrap();
    let m = Machine {
        home,
        ws,
        _dir: dir,
    };
    json(&m, "nxs", &["init", "--module", "flow", "--json"]);
    m
}

/// A machine bound to `endpoint` (nothing pushed yet).
fn bound(endpoint: &str) -> Machine {
    let m = machine();
    json(
        &m,
        "nxs",
        &[
            "sync",
            "bind",
            "--create",
            "--endpoint",
            endpoint,
            "--no-daemon",
            "--json",
        ],
    );
    m
}

/// A second machine joining `source`'s stream at `endpoint`.
fn joined(source: &Machine, endpoint: &str) -> Machine {
    let m = machine();
    let stream = meta(source)["stream_id"].as_str().unwrap().to_string();
    json(
        &m,
        "nxs",
        &[
            "sync",
            "bind",
            "--join",
            &stream,
            "--endpoint",
            endpoint,
            "--no-daemon",
            "--json",
        ],
    );
    m
}

/// `items` items written straight into the workspace's store — three ops each — so an import-sized
/// log is built in one process rather than one process per item.
fn fill(m: &Machine, items: usize) -> usize {
    let ws = nxs_foundation::workspace::Workspace::resolve(None, &m.ws).unwrap();
    let mut store =
        nexus_flow_core::store::Store::open(&ws.db_path_str().unwrap(), ws.replica.site_id)
            .unwrap();
    // Every op is its own transaction; a fixture need not wait for the disk after each.
    store
        .connection()
        .execute_batch("PRAGMA synchronous = OFF")
        .unwrap();
    for i in 0..items {
        store.create_item(
            &format!("{}.f{i:06}", ws.replica.prefix),
            "chore",
            "t",
            "tester",
        );
    }
    drop(store);
    local_ops(m).len()
}

/// This machine's own ops, `(rowid, op_id)` in rowid order.
fn local_ops(m: &Machine) -> Vec<(i64, String)> {
    let ws = nxs_foundation::workspace::Workspace::resolve(None, &m.ws).unwrap();
    let conn = rusqlite::Connection::open(ws.db_path_str().unwrap()).unwrap();
    let mut stmt = conn
        .prepare("SELECT rowid, op_id FROM ops WHERE site = ?1 ORDER BY rowid")
        .unwrap();
    let rows = stmt
        .query_map([ws.replica.site_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

fn rowid_of(m: &Machine, op_id: &str) -> i64 {
    local_ops(m)
        .into_iter()
        .find(|(_, id)| id == op_id)
        .unwrap_or_else(|| panic!("{op_id} is a local op"))
        .0
}

/// `.nxs/sync.toml`, as a person would read it.
fn meta(m: &Machine) -> Value {
    let raw = std::fs::read_to_string(m.ws.join(".nxs").join("sync.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&raw).unwrap();
    serde_json::to_value(parsed).unwrap()
}

fn marks(m: &Machine) -> (i64, i64) {
    let v = meta(m);
    (
        v["pushed_through"].as_i64().unwrap_or(0),
        v["pulled_through"].as_i64().unwrap_or(0),
    )
}

/// Wait until the gate holds a request, at most a minute.
fn until_held(gate: &Gate, child: &mut Child) -> String {
    let started = Instant::now();
    loop {
        if let Some(held) = gate.held() {
            return held;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the pass ended ({status}) before a request was held");
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no request was held"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn kill(mut child: Child) {
    child.kill().unwrap();
    child.wait().unwrap();
}

fn assert_nowhere(what: &str, text: &str) {
    for token in [TOKEN, FRESH] {
        for form in std::iter::once(token).chain(token.split('.')) {
            assert!(
                !text.contains(form),
                "the token ({form}) is in {what}:\n{text}"
            );
        }
    }
}

fn assert_no_file_holds_a_token(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let bytes = std::fs::read(&path).unwrap_or_default();
                for token in [TOKEN, FRESH] {
                    for form in std::iter::once(token).chain(token.split('.')) {
                        assert!(
                            !bytes.windows(form.len()).any(|w| w == form.as_bytes()),
                            "the token ({form}) is in {}",
                            path.display()
                        );
                    }
                }
            }
        }
    }
}

// ---- 1. the watermarks are on disk after every batch and every page ---------------------------

/// A pass killed while its THIRD push is in flight leaves `pushed_through` on the last op of the
/// SECOND — what the relay acknowledged, and nothing it did not.
#[test]
fn a_pass_killed_mid_push_leaves_the_last_acknowledged_batch_in_sync_toml() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    let total = fill(&source, 600); // 1800 ops: four push batches of 500 and a short one.
    assert!(total > 1500, "{total}");

    gate.hold_push(3);
    let mut child = spawn(&source, "nxs", &["sync", "run", "--json"], None);
    let first_held = until_held(&gate, &mut child);
    let on_disk_while_held = marks(&source);
    kill(child);

    let acked = gate.acked();
    assert_eq!(acked.len(), 1000, "two batches of 500 were acknowledged");
    let last_acked = rowid_of(&source, acked.last().unwrap());
    assert_eq!(
        on_disk_while_held.0, last_acked,
        "while batch 3 was in flight, sync.toml already showed batch 2"
    );
    assert!(
        rowid_of(&source, &first_held) > last_acked,
        "the held batch starts after what is recorded"
    );
    assert_eq!(
        marks(&source).0,
        last_acked,
        "killing the pass changes nothing on disk"
    );

    // The next pass starts there: it pushes exactly what was never acknowledged.
    gate.hold_push(0);
    let resumed = json(&source, "nxs", &["sync", "run", "--json"]);
    assert_eq!(resumed["pushed"], total - 1000, "{resumed}");
    let acked = gate.acked();
    let mut unique = acked.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), acked.len(), "no op was pushed twice");
    assert_eq!(acked.len(), total);
}

/// A pass killed while it waits for its THIRD pull page leaves `pulled_through` on the cursor the
/// SECOND page reached — the `since` the third one asked from.
#[test]
fn a_pass_killed_mid_pull_leaves_the_last_applied_page_in_sync_toml() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    let total = fill(&source, 600);
    json(&source, "nxs", &["sync", "run", "--json"]);

    let reader = joined(&source, &relay);
    gate.hold_pull(3);
    let mut child = spawn(&reader, "nxs", &["sync", "run", "--json"], None);
    let since: i64 = until_held(&gate, &mut child).parse().unwrap();
    let on_disk_while_held = marks(&reader);
    kill(child);

    assert!(
        since > 0,
        "two pages were applied before the third was asked"
    );
    assert_eq!(
        on_disk_while_held.1, since,
        "while page 3 was in flight, sync.toml already showed page 2's cursor"
    );
    assert_eq!(marks(&reader).1, since, "killing the pass changes nothing");

    gate.hold_pull(0);
    let resumed = json(&reader, "nxs", &["sync", "run", "--json"]);
    assert_eq!(
        resumed["pulled"].as_u64().unwrap() as i64,
        total as i64 - since,
        "the next pass pulls only what came after: {resumed}"
    );
}

// ---- 2. a first import over two tokens ---------------------------------------------------------

/// The size of the first nexflow.it import's main stream.
const IMPORT_OPS: usize = 174_000;

/// 174k ops, one token that runs out mid-push, a fresh one that finishes, then a based snapshot.
#[test]
fn an_import_whose_token_runs_out_mid_push_finishes_under_a_fresh_one_without_repushing() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    let total = fill(&source, IMPORT_OPS / 3);
    assert!(total >= IMPORT_OPS, "{total}");

    // The first token is good for registration and 120 pushes, then the relay answers 401.
    *gate.token_left.lock().unwrap() = 1 + 120;
    gate.set(TOKENS);
    let first = run_with(
        &source,
        "nxs",
        &["sync", "run", "--bearer-stdin", "--json"],
        Some(&format!("{TOKEN}\n")),
    );
    assert_eq!(
        first.status.code(),
        Some(EXIT_TOKEN_EXPIRED),
        "{}",
        said(&first)
    );
    let error: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "token_expired", "{error}");
    assert!(
        error["error"]["msg"]
            .as_str()
            .unwrap()
            .contains("token expired, run again with a fresh one"),
        "{error}"
    );
    assert_nowhere("the first pass's output", &said(&first));

    let acked_first = gate.acked();
    assert_eq!(acked_first.len(), 120 * 500, "120 batches landed");
    assert_eq!(
        marks(&source).0,
        rowid_of(&source, acked_first.last().unwrap()),
        "sync.toml shows the last acknowledged batch"
    );

    let second = run_with(
        &source,
        "nxs",
        &["sync", "run", "--bearer-stdin", "--json"],
        Some(FRESH),
    );
    assert!(second.status.success(), "{}", said(&second));
    let done: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(done["pushed"], total - acked_first.len(), "{done}");
    let acked = gate.acked();
    assert_eq!(acked.len(), total, "every op acknowledged exactly once");
    let mut unique = acked.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), total, "nothing acknowledged was pushed again");
    for auth in gate.seen() {
        let auth = auth.unwrap();
        assert!(auth.starts_with("Bearer "), "never a Basic header");
    }

    let file = source.ws.join("import.snapshot");
    let snap = run_with(
        &source,
        "nxs",
        &[
            "sync",
            "snapshot",
            file.to_str().unwrap(),
            "--based",
            "--bearer-stdin",
            "--json",
        ],
        Some(FRESH),
    );
    assert!(snap.status.success(), "{}", said(&snap));
    let taken: Value = serde_json::from_slice(&snap.stdout).unwrap();
    assert_eq!(taken["format"], 2, "{taken}");

    assert_no_file_holds_a_token(source.home.path());
    assert_no_file_holds_a_token(&source.ws);
}

// ---- 3. the refusals, and the token nowhere ----------------------------------------------------

/// A relay that refuses the token from the first request — 401 and 403 — is `forbidden`, exit 1,
/// the status named and the token not echoed; nothing moved, nothing recorded.
#[test]
fn a_token_refused_from_the_start_is_forbidden_and_echoed_nowhere() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    fill(&source, 10);
    for (mode, status) in [(ALWAYS_401, "401"), (ALWAYS_403, "403")] {
        gate.set(mode);
        let text = run_with(
            &source,
            "nxs",
            &["sync", "run", "--bearer-stdin"],
            Some(TOKEN),
        );
        let json_out = run_with(
            &source,
            "nxs",
            &["sync", "run", "--bearer-stdin", "--json"],
            Some(TOKEN),
        );
        gate.set(OPEN);

        assert_eq!(text.status.code(), Some(1), "{}", said(&text));
        let stderr = String::from_utf8_lossy(&text.stderr);
        assert!(stderr.contains("refused"), "{stderr}");
        assert!(stderr.contains(status), "{stderr}");
        assert_nowhere("the text refusal", &said(&text));

        assert_eq!(json_out.status.code(), Some(1), "{}", said(&json_out));
        let error: Value = serde_json::from_slice(&json_out.stdout).unwrap();
        assert_eq!(error["error"]["kind"], "forbidden", "{error}");
        assert!(
            error["error"]["msg"].as_str().unwrap().contains(status),
            "{error}"
        );
        assert_nowhere("the --json refusal", &said(&json_out));
    }
    assert!(gate.acked().is_empty());
    assert_eq!(marks(&source), (0, 0));
    assert_no_file_holds_a_token(source.home.path());
    assert_no_file_holds_a_token(&source.ws);
}

/// A 403 in the middle of a pass is not an expiry — the token is valid and may not do this.
#[test]
fn a_403_mid_pass_is_forbidden_not_expired() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    fill(&source, 400); // three push batches
                        // Registration and one push under the token, then the relay forbids.
    *gate.token_left.lock().unwrap() = 2;
    gate.set(TOKEN_THEN_403);
    let out = run_with(
        &source,
        "nxs",
        &["sync", "run", "--bearer-stdin", "--json"],
        Some(TOKEN),
    );
    assert_eq!(out.status.code(), Some(1), "{}", said(&out));
    let error: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "forbidden", "{error}");
    assert_nowhere("the refusal", &said(&out));
}

/// A userinfo in the relay URL beside the bearer is refused before any request.
#[test]
fn a_userinfo_in_the_relay_url_with_the_bearer_is_refused_before_any_request() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay.replace("http://", "http://relay-user:s3cret-key@"));
    fill(&source, 10);
    gate.set(TOKENS);
    let out = run_with(
        &source,
        "nxs",
        &["sync", "run", "--bearer-stdin", "--json"],
        Some(TOKEN),
    );
    assert!(!out.status.success());
    assert_eq!(gate.seen(), Vec::<Option<String>>::new(), "no request");
    let error: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "validation", "{error}");
    let msg = error["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("userinfo"), "{msg}");
    assert!(!msg.contains("s3cret-key"), "{msg}");
    assert_nowhere("the refusal", &said(&out));
    assert_eq!(marks(&source), (0, 0));
}

/// Nothing on stdin is a validation error, and nothing is asked of the relay.
#[test]
fn an_empty_stdin_is_refused_as_validation_before_any_request() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    gate.set(TOKENS);
    for stdin in [Some(""), Some("\n"), None] {
        let out = run_with(
            &source,
            "nxs",
            &["sync", "run", "--bearer-stdin", "--json"],
            stdin,
        );
        assert!(!out.status.success(), "{stdin:?}");
        let error: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(error["error"]["kind"], "validation", "{stdin:?}: {error}");
    }
    assert_eq!(gate.seen(), Vec::<Option<String>>::new(), "no request");
}

/// While a request is held, no process of this user — the pass included — shows the token in its
/// argv or environment; and every request carried the bearer alone.
#[test]
fn the_token_is_in_no_process_and_every_request_carries_it_alone() {
    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    fill(&source, 10);
    *gate.token_left.lock().unwrap() = usize::MAX;
    gate.set(TOKENS);
    let out = run_with(
        &source,
        "nxs",
        &["sync", "run", "--bearer-stdin"],
        Some(TOKEN),
    );
    assert!(out.status.success(), "{}", said(&out));
    assert_nowhere("the output", &said(&out));
    let seen = gate.seen();
    assert!(!seen.is_empty());
    for auth in &seen {
        assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    }
    let table = gate.processes.lock().unwrap().clone().unwrap();
    // Other tests run `nxs sync run` in parallel: this one is the line with this test's own HOME,
    // and finding it at all proves the table shows environments — else it would prove nothing.
    let home = source.home.path().display().to_string();
    assert!(
        table
            .lines()
            .any(|l| l.contains("sync run") && l.contains(&home)),
        "the verb, with its environment, is in the process table"
    );
    for line in table.lines() {
        for form in std::iter::once(TOKEN).chain(TOKEN.split('.')) {
            assert!(
                !line.contains(form),
                "the token ({form}) is in the argv or environment of {}",
                line.split_whitespace().next().unwrap_or("?")
            );
        }
    }
    assert_no_file_holds_a_token(source.home.path());
    assert_no_file_holds_a_token(&source.ws);
}

// ---- the library seam --------------------------------------------------------------------------

/// `nxs::sync::run_pass_with_bearer`, in-process as a host would call it: the pass runs under the
/// bearer alone, an expiry mid-pass is `TokenExpired` with the progress on disk, a refusal from the
/// start is `Forbidden`, a userinfo beside it `Validation` before any request.
#[test]
fn run_pass_with_bearer_at_the_library_seam() {
    use nxs_foundation::error::ErrorKind;
    use nxs_foundation::workspace::Workspace;

    let (relay, gate) = gated_relay();
    let source = bound(&relay);
    let total = fill(&source, 400);
    let ws = Workspace::resolve(None, &source.ws).unwrap();
    let token = nxs_sync::engine::read_bearer(TOKEN.as_bytes()).unwrap();
    let fresh = nxs_sync::engine::read_bearer(FRESH.as_bytes()).unwrap();

    *gate.token_left.lock().unwrap() = 1 + 1;
    gate.set(TOKENS);
    let expired = nxs::sync::run_pass_with_bearer(&ws, &relay, &token).unwrap_err();
    assert_eq!(expired.kind, ErrorKind::TokenExpired, "{}", expired.msg);
    assert_nowhere("the expiry", &expired.msg);
    assert_eq!(
        marks(&source).0,
        rowid_of(&source, gate.acked().last().unwrap())
    );

    let done = nxs::sync::run_pass_with_bearer(&ws, &relay, &fresh).unwrap();
    assert_eq!(done.pushed, total - 500);

    gate.set(ALWAYS_401);
    let refused = nxs::sync::run_pass_with_bearer(&ws, &relay, &token).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Forbidden, "{}", refused.msg);

    gate.set(TOKENS);
    let with_userinfo = relay.replace("http://", "http://relay-user:s3cret@");
    let refused = nxs::sync::run_pass_with_bearer(&ws, &with_userinfo, &token).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Validation, "{}", refused.msg);
    assert_eq!(gate.seen(), Vec::<Option<String>>::new(), "no request");
}
