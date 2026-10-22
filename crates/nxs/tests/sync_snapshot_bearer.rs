//! **A snapshot taken as a person, with a bearer on stdin** (nxf 6j6v.47vy) — on the REAL binary.
//!
//! An operator whose relay takes no shared key any more reaches it with the token a token exchange
//! gave them. `nxs sync snapshot <file> --bearer-stdin` (format 1, and `--based` format 2) reads
//! that token from STDIN and sends it as `Authorization: Bearer`, never as Basic. The token is a
//! credential, so this file reads EVERY place it could end up for its bytes: stdout and stderr in
//! both forms, every file under the machine's `$HOME` and workspace — the log among them — and the
//! argv and environment of every process of this user WHILE the request is in flight, the child
//! processes included.
//!
//! The relay is the real axum app behind a gate: open while the test sets a machine up with an
//! ordinary bind and pass, then accepting only that bearer — or answering every request 401 or 403.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

/// Three `.`-separated segments, as a JWT is, each distinctive on its own: a gateway that quotes
/// only one segment must not get it out either.
const TOKEN: &str = "hdr47vyQx.pay47vyZq.sig47vyXw";

/// What the gate does with a request.
const OPEN: u8 = 0;
const BEARER_ONLY: u8 = 1;
const ALWAYS_401: u8 = 2;
const ALWAYS_403: u8 = 3;

/// The relay's gate, and what it saw while it was closed.
#[derive(Default)]
struct Gate {
    mode: AtomicU8,
    /// The `Authorization` header of every request that reached the gate while it was not open.
    seen: Mutex<Vec<Option<String>>>,
    /// Every process of this user, with its argv and environment, listed while the FIRST request
    /// behind the gate was held — the snapshot verb was running and waiting for the answer.
    processes: Mutex<Option<String>>,
}

impl Gate {
    fn set(&self, mode: u8) {
        self.seen.lock().unwrap().clear();
        *self.processes.lock().unwrap() = None;
        self.mode.store(mode, Ordering::SeqCst);
    }

    fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

/// Every process of this user, argv and environment, one line each.
fn process_table() -> String {
    #[cfg(target_os = "macos")]
    let args = ["-A", "-ww", "-E", "-o", "command="];
    #[cfg(not(target_os = "macos"))]
    let args = ["axeww", "-o", "args="];
    let out = Command::new("ps").args(args).output().expect("ps runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
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
                let mode = g.mode.load(Ordering::SeqCst);
                if mode == OPEN {
                    return next.run(req).await;
                }
                let auth = req
                    .headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
                g.seen.lock().unwrap().push(auth.clone());
                if g.processes.lock().unwrap().is_none() {
                    let table = tokio::task::spawn_blocking(process_table).await.unwrap();
                    *g.processes.lock().unwrap() = Some(table);
                }
                let status = match mode {
                    ALWAYS_401 => axum::http::StatusCode::UNAUTHORIZED,
                    ALWAYS_403 => axum::http::StatusCode::FORBIDDEN,
                    _ if auth.as_deref() == Some(&format!("Bearer {TOKEN}")) => {
                        return next.run(req).await
                    }
                    _ => axum::http::StatusCode::UNAUTHORIZED,
                };
                axum::response::IntoResponse::into_response(status)
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

/// `bin` as this machine, with `stdin` piped in (or none at all).
fn run_with(m: &Machine, bin: &str, args: &[&str], stdin: Option<&str>) -> Output {
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
    child.wait_with_output().expect("the binary ends")
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

/// A machine bound to `endpoint` that holds two items, both pushed — through the OPEN gate.
fn synced_source(endpoint: &str) -> Machine {
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
    for title in ["one", "two"] {
        json(
            &m,
            "nxf",
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
        );
    }
    json(&m, "nxs", &["sync", "run", "--json"]);
    m
}

/// Each place the token's bytes could be found, and the bytes looked for: the token whole and each
/// of its segments.
fn assert_nowhere(what: &str, text: &str) {
    for form in std::iter::once(TOKEN).chain(TOKEN.split('.')) {
        assert!(
            !text.contains(form),
            "the token ({form}) is in {what}:\n{text}"
        );
    }
}

/// Every file under `root`, read as bytes — the log, the database, the snapshot, the config.
fn assert_no_file_holds_the_token(root: &Path) {
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
                for form in std::iter::once(TOKEN).chain(TOKEN.split('.')) {
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

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The process table taken while the verb waited: it must show the verb itself, with the pinned
/// `$HOME` in its environment — else the table proves nothing — and the token nowhere.
fn assert_processes_clean(gate: &Gate, m: &Machine) {
    let table = gate
        .processes
        .lock()
        .unwrap()
        .clone()
        .expect("a request reached the gate");
    let verb = table
        .lines()
        .find(|l| l.contains("sync snapshot"))
        .expect("the verb is in the process table");
    assert!(
        verb.contains(&m.home.path().display().to_string()),
        "the table does not show the verb's environment, so it proves nothing about it"
    );
    // A failure names the program only: the table holds every process's environment, and a CI
    // log is no place for the rest of it.
    for line in table.lines() {
        for form in std::iter::once(TOKEN).chain(TOKEN.split('.')) {
            assert!(
                !line.contains(form),
                "the token ({form}) is in the argv or environment of {}",
                line.split_whitespace().next().unwrap_or("?")
            );
        }
    }
}

/// The real binary, both formats: the snapshot is taken with the bearer alone and starts a new
/// machine; the token is in no output, no file and no process.
#[test]
fn a_snapshot_with_the_bearer_on_stdin_reaches_the_relay_as_that_bearer_alone() {
    let (relay, gate) = gated_relay();
    let source = synced_source(&relay);

    for (based, name) in [(false, "with-log.snapshot"), (true, "based.snapshot")] {
        let file = source.ws.join(name);
        let file = file.to_str().unwrap();
        let mut args = vec!["sync", "snapshot", file, "--bearer-stdin"];
        if based {
            args.push("--based");
        }
        for json_form in [false, true] {
            let mut args = args.clone();
            if json_form {
                args.push("--json");
            }
            gate.set(BEARER_ONLY);
            let out = run_with(&source, "nxs", &args, Some(&format!("{TOKEN}\n")));
            let seen = gate.seen();
            gate.set(OPEN);
            assert!(out.status.success(), "{args:?}: {}", said(&out));
            assert!(!seen.is_empty(), "the relay was asked");
            for auth in &seen {
                assert_eq!(
                    auth.as_deref(),
                    Some(format!("Bearer {TOKEN}").as_str()),
                    "every request carries the bearer, and no Basic header: {seen:?}"
                );
            }
            assert_nowhere(&format!("the output of {args:?}"), &said(&out));
            if json_form {
                let taken: Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(taken["format"], if based { 2 } else { 1 }, "{taken}");
            }
        }

        // It reads back: a new machine starts from it.
        let fresh = machine();
        let mut bind = vec![
            "sync",
            "bind",
            "--snapshot",
            file,
            "--endpoint",
            &relay,
            "--no-daemon",
            "--json",
        ];
        let signer;
        if based {
            signer = json(&source, "nxs", &["sync", "key", "--json"])["key_id"]
                .as_str()
                .unwrap()
                .to_string();
            bind.extend(["--trust-signer", signer.as_str()]);
        }
        let bound = json(&fresh, "nxs", &bind);
        assert_eq!(bound["mode"], "snapshot", "{bound}");
        assert_eq!(bound["snapshot"]["views"], "taken", "{bound}");
        assert_eq!(
            json(&fresh, "nxf", &["list", "--json"]),
            json(&source, "nxf", &["list", "--json"]),
            "the new machine shows the source's board"
        );
    }

    // While a request was held, every process of this user, children included.
    gate.set(BEARER_ONLY);
    let file = source.ws.join("watched.snapshot");
    let out = run_with(
        &source,
        "nxs",
        &[
            "sync",
            "snapshot",
            file.to_str().unwrap(),
            "--based",
            "--bearer-stdin",
        ],
        Some(TOKEN),
    );
    assert!(out.status.success(), "{}", said(&out));
    assert_processes_clean(&gate, &source);
    gate.set(OPEN);

    assert_no_file_holds_the_token(source.home.path());
    assert_no_file_holds_the_token(&source.ws);
}

/// A relay URL that carries a userinfo AND a bearer is refused before any request: the relay sees
/// nothing, and the refusal is a validation error.
#[test]
fn a_userinfo_in_the_relay_url_with_the_bearer_is_refused_before_any_request() {
    let (relay, gate) = gated_relay();
    let with_userinfo = relay.replace("http://", "http://relay-user:s3cret-key@");
    let source = synced_source(&with_userinfo);
    let file = source.ws.join("never.snapshot");
    gate.set(BEARER_ONLY);
    let out = run_with(
        &source,
        "nxs",
        &[
            "sync",
            "snapshot",
            file.to_str().unwrap(),
            "--bearer-stdin",
            "--json",
        ],
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
    assert!(!file.exists());
}

/// Nothing on stdin is a validation error, and nothing is asked of the relay.
#[test]
fn an_empty_stdin_is_refused_as_validation_before_any_request() {
    let (relay, gate) = gated_relay();
    let source = synced_source(&relay);
    gate.set(BEARER_ONLY);
    for stdin in [Some(""), Some("\n"), None] {
        let out = run_with(
            &source,
            "nxs",
            &[
                "sync",
                "snapshot",
                source.ws.join("never.snapshot").to_str().unwrap(),
                "--bearer-stdin",
                "--json",
            ],
            stdin,
        );
        assert!(!out.status.success(), "{stdin:?}");
        let error: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(error["error"]["kind"], "validation", "{stdin:?}: {error}");
        assert!(
            error["error"]["msg"].as_str().unwrap().contains("empty"),
            "{error}"
        );
    }
    assert_eq!(gate.seen(), Vec::<Option<String>>::new(), "no request");
}

/// The relay refusing the token — 401, and 403 — is the typed refusal: `forbidden`, exit 1, the
/// status named, the token not echoed. Text and `--json`.
#[test]
fn a_relay_refusing_the_bearer_is_a_typed_refusal_that_does_not_echo_it() {
    let (relay, gate) = gated_relay();
    let source = synced_source(&relay);
    let file = source.ws.join("refused.snapshot");
    let file = file.to_str().unwrap();
    for (mode, status) in [(ALWAYS_401, "401"), (ALWAYS_403, "403")] {
        for based in [false, true] {
            let mut args = vec!["sync", "snapshot", file, "--bearer-stdin"];
            if based {
                args.push("--based");
            }
            gate.set(mode);
            let text = run_with(&source, "nxs", &args, Some(TOKEN));
            args.push("--json");
            let json_out = run_with(&source, "nxs", &args, Some(TOKEN));
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
    }
    assert!(!Path::new(file).exists(), "no snapshot was written");
    assert_no_file_holds_the_token(source.home.path());
    assert_no_file_holds_the_token(&source.ws);
}

/// The library seam the verb sits on (`nxs::sync::take_snapshot`), called in-process the way a
/// host would: the bearer reaches the relay alone, a userinfo beside it is refused before any
/// request, and a refusal is `Forbidden`.
#[test]
fn take_snapshot_at_the_library_seam_sends_the_bearer_and_refuses_what_the_verb_refuses() {
    use nxs_foundation::error::ErrorKind;
    use nxs_foundation::workspace::Workspace;

    let (relay, gate) = gated_relay();
    let source = synced_source(&relay);
    let ws = Workspace::resolve(None, &source.ws).unwrap();
    let bearer = nxs_sync::engine::read_bearer(TOKEN.as_bytes()).unwrap();

    for based in [false, true] {
        gate.set(BEARER_ONLY);
        let bytes = nxs::sync::take_snapshot(&ws, based, Some(&bearer)).unwrap();
        let seen = gate.seen();
        gate.set(OPEN);
        let header = nxs_sync::snapshot::peek(&bytes).unwrap();
        assert_eq!(header.format, if based { 2 } else { 1 });
        assert!(!seen.is_empty());
        assert!(
            seen.iter()
                .all(|a| a.as_deref() == Some(format!("Bearer {TOKEN}").as_str())),
            "{seen:?}"
        );
    }

    for (mode, status) in [(ALWAYS_401, "401"), (ALWAYS_403, "403")] {
        gate.set(mode);
        let refused = nxs::sync::take_snapshot(&ws, true, Some(&bearer)).unwrap_err();
        gate.set(OPEN);
        assert_eq!(refused.kind, ErrorKind::Forbidden, "{}", refused.msg);
        assert!(refused.msg.contains(status), "{}", refused.msg);
        assert_nowhere("the refusal", &refused.msg);
    }

    let with_userinfo = synced_source(&relay.replace("http://", "http://relay-user:s3cret@"));
    let ws = Workspace::resolve(None, &with_userinfo.ws).unwrap();
    gate.set(BEARER_ONLY);
    let refused = nxs::sync::take_snapshot(&ws, false, Some(&bearer)).unwrap_err();
    assert_eq!(gate.seen(), Vec::<Option<String>>::new(), "no request");
    assert_eq!(refused.kind, ErrorKind::Validation, "{}", refused.msg);
}
