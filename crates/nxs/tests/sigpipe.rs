//! SIGPIPE handling (4yt4): piping nxf output into a reader that closes early — the everyday
//! `nxf blocked | head`, `nxf list | grep -m1 …` — must NOT panic with
//! "failed printing to stdout: Broken pipe". The Rust runtime installs SIGPIPE as SIG_IGN at
//! startup, which turns a closed-pipe write into an `EPIPE` error that `println!` unwraps into a
//! panic. `nxs` main restores the default disposition (SIG_DFL) so the process is instead killed
//! quietly by the signal (exit 128+SIGPIPE), exactly like `cat`/`ls`/every other Unix filter.

use assert_cmd::Command as AssertCommand;
use nxs_test_support::PinHome;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

fn nxf_path() -> PathBuf {
    assert_cmd::cargo::cargo_bin("nxf")
}

/// init + one item whose description is well over the OS pipe buffer (64 KiB), so reading it back
/// is guaranteed to keep the writer busy long enough for a downstream close to interrupt an
/// in-flight write — rather than the whole output slipping into the buffer and the close being a
/// no-op (which is why a tiny command like `guide` can't reproduce the panic).
fn workspace_with_big_item() -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    // **`init` writes to the real service registry unless a home is pinned** (nxf 6j6v.y12q,
    // review of PR #421). This file builds its commands from a bare path rather than through
    // `nxs_test_support::cargo_bin`, so it inherits none of that helper's isolation and has to say
    // it itself — and since y12q every `nxf init` puts the workspace on the background service's
    // list, which without this line is the developer's own `~/.nexusflow/workspaces.toml` gaining
    // an entry for a `TempDir` that is deleted moments later.
    AssertCommand::new(nxf_path())
        .pin_home(tmp.path().join(".fake-home"))
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let big = "x".repeat(200_000); // all `x` → no JSON escaping needed
    let payload =
        format!(r#"{{"type":"feature","title":"big","description":"{big}","priority":"P1"}}"#);
    AssertCommand::new(nxf_path())
        .pin_home(tmp.path().join(".fake-home"))
        .args(["create", "--json", "-"])
        .current_dir(tmp.path())
        .write_stdin(payload)
        .assert()
        .success();
    tmp
}

#[test]
fn stdout_broken_pipe_does_not_panic() {
    let tmp = workspace_with_big_item();

    // `nxf list --json` emits the >64 KiB record. Spawn it, let it fill the pipe buffer and block
    // on the large write, THEN close the read end — reproducing `nxf list --json | head`.
    let mut child = Command::new(nxf_path())
        .pin_home(tmp.path().join(".fake-home"))
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nxf");
    std::thread::sleep(Duration::from_millis(300));
    drop(child.stdout.take()); // close the read end → the blocked write breaks the pipe

    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr piped")
        .read_to_string(&mut err)
        .expect("read child stderr"); // surface a read failure rather than masking a real panic
    let status = child.wait().expect("wait");

    assert!(
        !err.contains("panicked") && !err.to_lowercase().contains("broken pipe"),
        "nxf must not panic on a broken pipe; stderr was:\n{err}"
    );
    // A Rust panic exits 101; SIGPIPE under SIG_DFL kills the process by signal (code None).
    assert_ne!(
        status.code(),
        Some(101),
        "a broken pipe must not surface as a panic exit"
    );
}
