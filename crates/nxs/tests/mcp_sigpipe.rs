//! `nxs mcp serve` must survive a host closing its stdio pipe WITHOUT dying by SIGPIPE
//! (mitigation of the PR #224 review's Integrity finding). The one-shot CLI filters reset SIGPIPE
//! to SIG_DFL (4yt4) so `nxf … | head` dies quietly; the long-running MCP server scopes that back
//! to SIG_IGN at its entry, so a broken stdout surfaces as an EPIPE its transport error path can
//! handle — not a signal death that skips the graceful `NxfError::io` + clean-exit path. Verified
//! discriminating: with the SIG_IGN restore removed this test observes the SIGPIPE signal death.
#![cfg(unix)]

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

/// POSIX SIGPIPE signal number (13 on Linux and macOS). Hard-coded to avoid pulling `libc` into
/// the integration-test crate for a single constant.
const SIGPIPE: i32 = 13;

fn init_workspace(dir: &Path) {
    // **`init` writes to the real service registry unless a home is pinned** (nxf 6j6v.y12q,
    // review of PR #421): since y12q every `nxf init` puts the workspace on the background
    // service's list, and this file builds its commands from a bare `cargo_bin` path rather than
    // through `nxs_test_support::cargo_bin`, so it carries none of that helper's isolation.
    let ok = Command::new(cargo_bin("nxf"))
        .pin_home(dir.join(".fake-home"))
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("nxf init")
        .success();
    assert!(ok, "nxf init succeeded");
}

fn send(stdin: &mut ChildStdin, msg: &str) {
    // Ignore write errors: once the server exits, its stdin closes and the write fails — expected.
    let _ = writeln!(stdin, "{msg}");
    let _ = stdin.flush();
}

#[test]
fn mcp_serve_survives_a_broken_stdout_without_a_sigpipe_death() {
    let ws = TempDir::new().unwrap();
    init_workspace(ws.path());

    let mut child = Command::new(cargo_bin("nxs"))
        .pin_home(ws.path().join(".fake-home"))
        .args(["mcp", "serve", "--workspace", ws.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn nxs mcp serve");

    let mut stdin = child.stdin.take().expect("stdin piped");
    let mut out = BufReader::new(child.stdout.take().expect("stdout piped"));

    // Handshake — proves the server writes its first stdout bytes (so the pipe is live).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}"#,
    );
    let mut line = String::new();
    out.read_line(&mut line).expect("initialize response");
    assert!(
        line.contains("\"result\""),
        "server answered initialize on stdout: {line}"
    );
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Close OUR read end → the server's next response write breaks its stdout pipe.
    drop(out);

    // Force response writes onto the now-broken pipe.
    for _ in 0..4 {
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        );
        std::thread::sleep(Duration::from_millis(150));
    }

    // The regression is a SIGPIPE signal death (immediate, on the broken write). Under the fix the
    // server never dies by signal — it handles the EPIPE and stays under its own control. Poll
    // briefly for an early signal death, then tear the server down.
    let mut sigpipe_death = false;
    for _ in 0..10 {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                sigpipe_death = status.signal() == Some(SIGPIPE);
                break;
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        !sigpipe_death,
        "nxs mcp serve was killed by SIGPIPE on a broken stdout — its graceful stdio error path \
         was skipped (the 4yt4 SIG_DFL reset leaked into the server persona)"
    );
}
