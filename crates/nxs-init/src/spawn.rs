//! Shell-out to the standalone module binaries (TB-5). `nxs` reaches each active module by running
//! its CLI (`<binary> agent-manifest --json` for the assembler, `<binary> prime` for the fan-out),
//! never by linking it — that is what keeps the products release-independent.
//!
//! Binary resolution prefers a **sibling** next to the running executable: an installed suite ships
//! `nxs`/`nxf`/`nxm` in one directory, and the cargo test harness builds them all into the same
//! `target/<profile>/` — so a sibling lookup finds the right peer in both. It falls back to the
//! bare name (PATH) when there is no sibling.

use nxs_foundation::error::{NxfError, Result};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
fn bin_name(name: &str) -> String {
    format!("{name}.exe")
}
#[cfg(not(windows))]
fn bin_name(name: &str) -> String {
    name.to_string()
}

/// Resolve a suite binary: a sibling of the current executable if present, else the bare name.
///
/// This deliberately uses `current_exe()` — the OPPOSITE of the multicall ROUTER (`nxs::cli::run`),
/// which dispatches on `argv[0]` so a symlink keeps its persona. The two needs are distinct:
/// *routing* asks "which persona was I invoked as?" (must survive the symlink → `argv[0]`);
/// *resolution* asks "where on disk do my siblings live?" (must follow the symlink to the real
/// install dir → `current_exe()`). With multicall, `current_exe()` lands in the directory holding
/// `nxs` and the `nxf`/`nxm` symlinks, so the sibling lookup returns the persona symlink, which then
/// re-routes by its own `argv[0]` when spawned. Both choices are correct precisely because they
/// differ.
pub fn resolve_binary(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(bin_name(name));
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from(name)
}

/// Resource ceilings for ONE shell-out (aye.35): a wall-clock budget and a stdout byte cap. A
/// child that breaches either is killed and the call fails loud, so a hung or runaway sibling can
/// never wedge a SessionStart hook nor OOM it by buffering unbounded stdout. The
/// blast radius is bounded (first-party sibling binaries running `prime` / `agent-manifest`), so
/// the defaults are generous; both are overridable so the breach paths are unit-testable.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Kill the child (and fail loud) if it has not exited within this wall-clock budget.
    pub wall_clock: Duration,
    /// Kill the child (and fail loud) if its stdout grows past this many bytes.
    pub max_stdout: usize,
}

/// Wall-clock budget for one child: a `prime`/`agent-manifest` shell-out is a fast, bounded call,
/// so 30s is far above any healthy run yet still rescues a wedged session start.
pub const DEFAULT_WALL_CLOCK: Duration = Duration::from_secs(30);
/// stdout cap for one child: a module's `prime`/manifest is small; 16 MiB is generous headroom
/// while still bounding the memory a runaway child can make `nxs` buffer.
pub const DEFAULT_MAX_STDOUT: usize = 16 * 1024 * 1024;
/// How often the wait loop polls for child exit / a stdout-cap breach. Small enough that a healthy
/// child's exit is observed promptly (negligible added latency on the prime fan-out).
const POLL_INTERVAL: Duration = Duration::from_millis(10);

impl Default for Limits {
    fn default() -> Self {
        Limits {
            wall_clock: DEFAULT_WALL_CLOCK,
            max_stdout: DEFAULT_MAX_STDOUT,
        }
    }
}

/// Run `<binary> <args…>`, capture stdout, and error (with stderr) on a non-zero exit or a spawn
/// failure (e.g. the binary is not installed). The captured stdout is returned verbatim.
pub fn run_capture(binary: &str, args: &[&str]) -> Result<String> {
    run_capture_env(binary, args, &[])
}

/// [`run_capture`] with extra environment variables set on the child — used by `nxs prime` to inject
/// the shared `now` via each module's pin-clock env var. Runs under the [`Limits::default`] ceilings.
pub fn run_capture_env(binary: &str, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    run_capture_env_limited(binary, args, env, Limits::default())
}

/// [`run_capture_env`] under explicit [`Limits`] — the String entry over the hardened
/// [`capture_inner`] core (aye.35): stdout verbatim on success (UTF-8 checked), a stderr-bearing
/// error on a non-zero exit, a loud error on a timeout / stdout-cap breach / spawn failure.
pub fn run_capture_env_limited(
    binary: &str,
    args: &[&str],
    env: &[(&str, &str)],
    limits: Limits,
) -> Result<String> {
    let bytes = capture_inner(binary, args, env, limits, false)?
        .expect("capture_inner returns None only when notfound_is_none is set");
    String::from_utf8(bytes)
        .map_err(|e| NxfError::io(format!("`{binary}` output was not UTF-8: {e}")))
}

/// Run `<binary> <args…>` under [`Limits::default`] and return its RAW stdout bytes, or `Ok(None)`
/// when the binary is **not installed** — the seam a caller uses to fall back (the beads migration
/// when `bd` is absent). Same hardening as [`run_capture_env_limited`] (timeout + stdout cap, child
/// killed on breach); bytes are returned verbatim so a byte-faithful artifact (a backup) is
/// possible without a lossy round-trip. A non-zero exit (a `bd` that runs but fails) is still loud.
pub fn run_capture_optional_bytes(
    binary: &str,
    args: &[&str],
    limits: Limits,
) -> Result<Option<Vec<u8>>> {
    capture_inner(binary, args, &[], limits, true)
}

/// The hardened spawn/drain/poll core (aye.35) shared by the String and bytes/optional entries: pipe
/// stdout/stderr (stdin `/dev/null`), drain both in threads (stdout capped, signalling an overflow),
/// poll for exit against `limits.wall_clock`, and kill + fail loud on a timeout or cap breach.
/// Returns the raw stdout on a clean exit. When `notfound_is_none`, a "binary not installed" spawn
/// error becomes `Ok(None)` (the fall-back seam) instead of a loud error.
fn capture_inner(
    binary: &str,
    args: &[&str],
    env: &[(&str, &str)],
    limits: Limits,
    notfound_is_none: bool,
) -> Result<Option<Vec<u8>>> {
    let path = resolve_binary(binary);
    let mut cmd = Command::new(&path);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if notfound_is_none && e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(NxfError::io(format!(
                "running `{binary} {}`: {e} (is `{binary}` installed?)",
                args.join(" ")
            )))
        }
    };

    // Drain both pipes in their own threads so the child can never deadlock filling one pipe while
    // we wait on the other. stdout is capped (and signals an overflow); stderr is read to a generous
    // bound purely so a stderr flood cannot OOM us either.
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");
    let overflow = Arc::new(AtomicBool::new(false));
    let cap = limits.max_stdout;
    let stdout_flag = Arc::clone(&overflow);
    let stdout_handle = thread::spawn(move || read_capped(stdout_pipe, cap, Some(&stdout_flag)));
    let stderr_handle = thread::spawn(move || read_capped(stderr_pipe, cap, None).0);

    // Wait for exit, killing on a timeout or once the stdout reader signals it blew the cap (a
    // runaway that never exits would otherwise sit until the wall-clock; the flag fails it fast).
    let deadline = Instant::now() + limits.wall_clock;
    let breach = loop {
        if overflow.load(Ordering::Relaxed) {
            break Some(cap_breach_msg(binary, args, cap));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // Exited on its own — but it may have exited *because* it overran the cap (the
                // reader closed its pipe → SIGPIPE), so re-check the flag before trusting status.
                if overflow.load(Ordering::Relaxed) {
                    break Some(cap_breach_msg(binary, args, cap));
                }
                return finish_bytes(binary, args, status, stdout_handle, stderr_handle).map(Some);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    break Some(format!(
                        "`{binary} {}` exceeded the {}s wall-clock timeout and was killed",
                        args.join(" "),
                        limits.wall_clock.as_secs()
                    ));
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(NxfError::io(format!(
                    "waiting on `{binary} {}`: {e}",
                    args.join(" ")
                )));
            }
        }
    };

    // A breach: kill, reap, drain the reader threads (so they cannot leak), then fail loud.
    let _ = child.kill();
    let _ = child.wait();
    let _ = stdout_handle.join();
    let _ = stderr_handle.join();
    Err(NxfError::io(breach.expect("breach path sets a message")))
}

/// The loud message for a stdout-cap breach.
fn cap_breach_msg(binary: &str, args: &[&str], cap: usize) -> String {
    format!(
        "`{binary} {}` exceeded the {cap}-byte stdout cap and was killed",
        args.join(" ")
    )
}

/// Validate a cleanly-exited child and return its RAW stdout bytes: a non-zero exit is a loud error
/// carrying the child's stderr; success yields stdout verbatim (the caller decides UTF-8 handling).
fn finish_bytes(
    binary: &str,
    args: &[&str],
    status: std::process::ExitStatus,
    stdout_handle: thread::JoinHandle<(Vec<u8>, bool)>,
    stderr_handle: thread::JoinHandle<Vec<u8>>,
) -> Result<Vec<u8>> {
    let (stdout, _) = stdout_handle.join().expect("stdout reader thread");
    let stderr = stderr_handle.join().expect("stderr reader thread");
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(NxfError::io(format!(
            "`{binary} {}` failed ({status}): {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// Read `r` to EOF into a buffer, stopping once `cap` bytes are stored. Returns `(bytes, overflow)`
/// where `overflow` is true iff the source had more than `cap` bytes. When `flag` is given it is set
/// the moment the cap is breached, so a concurrent waiter can kill a runaway child promptly.
fn read_capped<R: Read>(mut r: R, cap: usize, flag: Option<&AtomicBool>) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk) {
            Ok(0) => return (buf, false),
            Ok(n) => {
                if buf.len() + n > cap {
                    let room = cap.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..room]);
                    if let Some(f) = flag {
                        f.store(true, Ordering::Relaxed);
                    }
                    return (buf, true);
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            // The pipe was closed (e.g. the child was killed) — stop with what we have.
            Err(_) => return (buf, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // The hardened paths (aye.35) need a real child that hangs / floods, so these are gated to
    // unix where `sh` is always present; the cross-platform spawn/resolve logic is covered by the
    // e2e suite against the real sibling binaries.
    #[cfg(unix)]
    fn tiny(wall_ms: u64, cap: usize) -> Limits {
        Limits {
            wall_clock: Duration::from_millis(wall_ms),
            max_stdout: cap,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_outruns_the_wall_clock_is_killed_and_fails_loud() {
        // A child that never exits within the budget is killed; the call fails loud (it does NOT
        // hang the caller — that is the whole point for the SessionStart hook).
        let err = run_capture_env_limited("sh", &["-c", "sleep 30"], &[], tiny(150, 1 << 20))
            .unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Io);
        assert!(
            err.msg.contains("timeout") || err.msg.contains("timed out"),
            "names the timeout: {}",
            err.msg
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_floods_stdout_past_the_cap_is_killed_and_fails_loud() {
        // `yes` emits unboundedly and never exits on its own — so this also proves the cap breach
        // *kills* the child (rather than waiting for it to finish) and fails loud.
        let err =
            run_capture_env_limited("sh", &["-c", "yes"], &[], tiny(5_000, 4096)).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Io);
        assert!(
            err.msg.contains("cap") || err.msg.contains("stdout"),
            "names the stdout cap: {}",
            err.msg
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_under_both_limits_is_returned_verbatim() {
        let out = run_capture_env_limited("sh", &["-c", "printf hello"], &[], tiny(5_000, 1 << 20))
            .unwrap();
        assert_eq!(out, "hello");
    }

    #[cfg(unix)]
    #[test]
    fn a_nonzero_exit_still_surfaces_stderr() {
        // The hardened path preserves the original contract: a non-zero exit is a loud error that
        // carries the child's stderr.
        let err = run_capture_env_limited(
            "sh",
            &["-c", "echo boom >&2; exit 3"],
            &[],
            tiny(5_000, 1 << 20),
        )
        .unwrap_err();
        assert!(
            err.msg.contains("boom"),
            "carries child stderr: {}",
            err.msg
        );
    }
}
