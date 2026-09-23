//! `nxs sync daemon` — the machine-wide loop (E4 task 11, `fa65`). Unlike every other `nxs sync`
//! verb it must run from ANY directory, including one with no `.nxs/` anywhere up the tree: it
//! serves the WHOLE workspace registry, not "the workspace found by walking up from cwd". This
//! pins that `Command::Sync`'s dispatch does not resolve a workspace before calling
//! `sync::daemon::serve` — the arm the other `sync` verbs (`bind`/`run`) DO resolve one for.

mod common;

use assert_cmd::Command as AssertCmd;
use nxs_test_support::PinHome;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

/// The `nxs` binary path, via the gated resolver (nexus-flow-0yyw): this asserts the multicall
/// binary is fresh before handing back the path, so a stale `nxs` cannot make this test pass for
/// the wrong reason. `assert_cmd::Command` (what `nxs_test_support::cargo_bin` returns) has no
/// public `spawn`, and this test needs to spawn the daemon and kill it rather than wait for exit
/// — so it goes through a plain `std::process::Command` instead (mirrors
/// `crates/nxs/tests/sigpipe.rs`'s pattern for the same reason).
fn nxs_bin() -> PathBuf {
    nxs_test_support::assert_multicall_binary_fresh();
    assert_cmd::cargo::cargo_bin("nxs")
}

/// Isolate HOME so the daemon reads an EMPTY registry/global config rather than the developer's
/// real `~/.nexusflow/` (mirrors `sync_bind.rs`/`sync_run.rs`'s isolation — `bind` writes there,
/// and the daemon reads the very same files on every tick).
///
/// Through [`PinHome`] rather than a bare `.env`: a raw `std::process::Command` inherits the
/// service INSTANCE this repo's `.envrc` names, and the daemon would then read
/// `<the pinned home>/.nexusflow-dev` — a directory these tests never write to (nxf 6j6v.9bjv).
fn nxs(dir: &Path) -> Command {
    let mut c = Command::new(nxs_bin());
    c.current_dir(dir)
        .pin_home(dir.join(".fake-home"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// `nxf` (the plugin-init verb). Its own isolation is [`nxs_test_support::cargo_bin`]'s: since nxf
/// 6j6v.y12q `nxf init` registers the workspace it creates with the background service, so it DOES
/// touch `~/.nexusflow/` — the structural pin is what keeps that off the developer's real one,
/// rather than anything this file adds (mirrors `sync_bind.rs`/`sync_run.rs`'s own `nxf()`).
fn nxf() -> AssertCmd {
    nxs_test_support::cargo_bin("nxf")
}

/// A one-shot `nxs` invocation (`bind`, not `daemon`) via `assert_cmd`, HOME-isolated exactly
/// like [`nxs`] above — the two exist side by side because [`nxs`] returns a plain
/// `std::process::Command` (needed to `spawn`+`kill` the daemon rather than wait for exit), while
/// the one-shot verbs below just want `.assert()`.
fn nxs_oneshot(dir: &Path) -> AssertCmd {
    let mut c = nxs_test_support::cargo_bin("nxs");
    c.current_dir(dir).pin_home(dir.join(".fake-home"));
    c
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

#[test]
fn daemon_starts_from_a_directory_with_no_workspace_anywhere_up_the_tree() {
    // A bare tmp dir: no `.nxs/` here, and (being under the system tmp root) none at any
    // ancestor either. If `Command::Sync`'s dispatch mistakenly resolved a workspace for
    // `Daemon` the way it does for `Bind`/`Run`, this would fail fast with a `no_workspace`
    // error within milliseconds — a real workspace here would let that bug hide.
    let tmp = TempDir::new().unwrap();
    let mut child = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "1", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");

    // Long enough to clear any startup work (registry read, global-config read, one tick) but
    // far short of the loop ever exiting on its own — the daemon runs forever until killed.
    std::thread::sleep(Duration::from_millis(500));
    let status = child.try_wait().expect("try_wait");

    // Always kill before asserting, so a failing assertion never strands a background process.
    let _ = child.kill();
    let output = child
        .wait_with_output()
        .expect("wait for the killed daemon");

    assert!(
        status.is_none(),
        "the daemon must still be running 500ms in — a workspace-resolution error would have \
         exited almost immediately instead; stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.to_lowercase().contains("panic"),
        "no panic on the no-workspace-anywhere startup path; stderr was:\n{stderr}"
    );
}

/// nxf 6j6v.d43g, the writing half, through the running loop rather than beside it.
///
/// `recorded_start` is unit-tested on its own, but the thing that broke was the WIRING: `serve`
/// stamped the moment it wrote its first heartbeat. Swapping the call back for `SystemTime::now()`
/// leaves the unit suite green, so this is the only level that can hold it — and it holds it by
/// exact string equality, because both sides format the same kernel-reported instant with the same
/// formatter, while a `now()` stamp lands however many milliseconds `execve` → first write took.
#[test]
fn a_running_service_stamps_the_start_of_its_process_not_the_moment_it_first_wrote() {
    let tmp = TempDir::new().unwrap();
    let mut daemon = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "1", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");
    let pid = daemon.id();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stamped = String::new();
    while Instant::now() < deadline {
        let v = run_status_json(tmp.path());
        if let Some(at) = v["heartbeat"]["started_at"].as_str() {
            stamped = at.to_string();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Asked while the daemon is still alive — a dead pid has no start time to compare against.
    let os_start = nxs_service::heartbeat::recorded_start(pid, SystemTime::UNIX_EPOCH);
    let _ = daemon.kill();
    let _ = daemon.wait();

    assert!(!stamped.is_empty(), "the service never wrote a heartbeat");
    assert_eq!(
        stamped,
        nxs_service::heartbeat::rfc3339(os_start),
        "the service must record when its PROCESS began, not when it got round to saying so"
    );
}

// ---- `nxs sync daemon status --json` (task 12, `4z4f`): required test #2 from the task-12 -----
// audit — the brief's DoD says this is deterministic; nothing in the brief's own test list checks
// it. HOME-isolated exactly like the test above, via the SAME `nxs()` helper — no daemon process
// is spawned here, `status` only ever reads a heartbeat file.

fn run_status_json(dir: &Path) -> serde_json::Value {
    let output = nxs(dir)
        .args(["sync", "daemon", "status", "--json"])
        .output()
        .expect("run nxs sync daemon status");
    assert!(
        output.status.success(),
        "status must succeed even with no heartbeat (a missing one is a state, not an error); \
         stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json output")
}

/// The `program` block every `status --json` carries since nxf 6j6v.dcpk: the alias, its resolved
/// target, whether that exists, and the one note worth making about the pair.
///
/// Rendered ALWAYS, `null`s included, exactly as `running`/`heartbeat` already are — a reader must
/// never have to tell "the key is absent" from "there is nothing to report". Under these tests'
/// fake `$HOME` no alias has ever been installed, so every arm below is the empty one.
fn absent_program(dir: &Path) -> serde_json::Value {
    serde_json::json!({
        "link": dir
            .join(".fake-home")
            .join(".nexusflow")
            .join("bin")
            .join("nexus-flow")
            .display()
            .to_string(),
        "target": null,
        "exists": false,
        "note": null,
    })
}

/// **Why `registration` is `null` in every case here** (nxf 6j6v.kvda). `status` compares what
/// launchd holds under this instance's label against this instance's own plist — and it asks that
/// question ONLY when the process's `$HOME` is the login session's. These tests pin `$HOME` at a
/// `TempDir`, which is exactly the condition that refuses; reading the developer's real `gui/<uid>`
/// from here would compare their production registration against a plist under a temporary
/// directory and report a machine-dependent "FOREIGN" on a perfectly healthy Mac.
///
/// So `null` here is the tri-state's third arm — *nobody looked* — and not "there is nothing
/// registered". The four answers themselves are proved where they can be proved deterministically:
/// against `launchctl print`'s own bytes, in `crates/service/src/launchd.rs` and
/// `crates/nxs/src/sync/daemon.rs`.
const NOBODY_LOOKED_AT_LAUNCHD: serde_json::Value = serde_json::Value::Null;

/// The service home of the instance these tests run as — the DEFAULT one, since `nxs()` clears
/// `NXS_SERVICE_INSTANCE` (nxf 6j6v.gd9p). Named here because `status --json` now says which
/// service it is answering for, and a report whose subject a reader has to guess is what a machine
/// running several instances would leave them with.
fn default_home(dir: &Path) -> String {
    dir.join(".fake-home")
        .join(".nexusflow")
        .display()
        .to_string()
}

#[test]
fn status_with_no_heartbeat_reports_not_running_without_erroring() {
    let tmp = TempDir::new().unwrap();
    let v = run_status_json(tmp.path());
    assert_eq!(
        v,
        serde_json::json!({
            "running": false,
            // nxf 6j6v.kcan: the four answers by name, beside the tri-state bool that cannot tell
            // its own two `null`s apart.
            "state": "not_running",
            "instance": "nexus-flow",
            "home": default_home(tmp.path()),
            "heartbeat": null,
            "program": absent_program(tmp.path()),
            "registration": NOBODY_LOOKED_AT_LAUNCHD,
            "shared_workspaces": [],
        })
    );
}

/// The instant a live service would have recorded for itself, in the shape it records it.
fn now_rfc3339() -> String {
    time::OffsetDateTime::from(SystemTime::now())
        .format(&time::format_description::well_known::Rfc3339)
        .expect("an instant formats")
}

#[test]
fn status_with_a_heartbeat_reports_the_full_deterministic_shape() {
    let tmp = TempDir::new().unwrap();
    // Plant the heartbeat by hand — the daemon loop itself is task 11's concern, this test is
    // purely `status`'s read side of the file-based idiom. The pid must name a process that is
    // ACTUALLY alive for the whole test (task-12 review Finding 3: `running` now probes it, not
    // just the heartbeat's presence) — this very test binary's own pid is exactly that: it is,
    // by construction, alive for as long as this test is running.
    //
    // And since 6j6v.0wvp the pid is not enough: `running` also compares the recorded start instant
    // against the process's REAL one, so the stamp has to be this process's own. A fixed one would
    // (correctly) read as a pid that now belongs to somebody else — which is what the test below
    // this one is about.
    let hb_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&hb_dir).unwrap();
    let pid = std::process::id();
    let started_at = now_rfc3339();
    std::fs::write(
        hb_dir.join("sync-daemon.json"),
        format!(
            r#"{{"pid":{pid},"started_at":"{started_at}","last_pass_at":"2026-07-30T09:05:00Z",
            "workspaces":[{{"path":"/repo","last_ok":"2026-07-30T09:05:00Z","last_error":null,
            "pushed":3,"pulled":7}}]}}"#
        ),
    )
    .unwrap();

    let v = run_status_json(tmp.path());
    assert_eq!(
        v,
        serde_json::json!({
            "running": true,
            "state": "running",
            "instance": "nexus-flow",
            "home": default_home(tmp.path()),
            "registration": NOBODY_LOOKED_AT_LAUNCHD,
            "shared_workspaces": [],
            "heartbeat": {
                "pid": pid,
                "started_at": started_at,
                "last_pass_at": "2026-07-30T09:05:00Z",
                "workspaces": [{
                    "path": "/repo",
                    "last_ok": "2026-07-30T09:05:00Z",
                    "last_error": null,
                    "pushed": 3,
                    "pulled": 7,
                }],
                // Stamped by a real service since nxf 6j6v.dcpk (a); `null` here because this test
                // plants the file by hand, in the shape a service from before the field wrote it —
                // which is exactly the on-disk state every existing machine has.
                "program": null,
                "version": null,
                // Same `Option`, same reason: a hand-planted heartbeat is exactly the shape a
                // service from before 6j6v.gd9p wrote, and `status` must read it rather than
                // refuse it.
                "instance": null,
                // nxf 6j6v.kcan. Rendered `null` rather than omitted, exactly like the three
                // above it: a reader must never have to tell "the key is absent" from "there is
                // nothing to report".
                "write_failures": null,
            },
            "program": absent_program(tmp.path()),
        })
    );

    // Determinism (the CLI contract): the very same on-disk heartbeat renders byte-identically
    // on a second, independent invocation.
    assert_eq!(v, run_status_json(tmp.path()));
}

// Task-12 review Finding 3: a heartbeat's mere PRESENCE is not proof the daemon is alive — a
// `SIGKILL`ed process leaves its last heartbeat behind forever, since nothing survives to
// overwrite it. `running` must reflect the recorded pid's actual liveness; the rest of the
// heartbeat (last pass, per-workspace state) must still render regardless — it stays useful
// diagnostics even once the process itself is confirmed gone.
/// The defect 6j6v.0wvp names, at the seam a human reads: a service that died long ago, whose pid
/// the operating system has since handed to something else.
///
/// The pid here is THIS test process — alive, and emphatically not the thing that wrote a heartbeat
/// claiming to have started on 2026-07-30. Before the fix, `kill(pid, 0)` answered yes and `status`
/// said `running: true` forever.
#[test]
fn status_does_not_report_a_reused_pid_as_running() {
    let tmp = TempDir::new().unwrap();
    let hb_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&hb_dir).unwrap();
    let pid = std::process::id();
    std::fs::write(
        hb_dir.join("sync-daemon.json"),
        format!(
            r#"{{"pid":{pid},"started_at":"2026-07-30T09:00:00Z","last_pass_at":"2026-07-30T09:05:00Z",
            "workspaces":[]}}"#
        ),
    )
    .unwrap();

    let v = run_status_json(tmp.path());
    assert_ne!(
        v["running"],
        serde_json::json!(true),
        "a live pid whose start time disagrees with the heartbeat is not a running service: {v}"
    );
    // nxf 6j6v.kcan: and it is not a STOPPED one either. `running` cannot carry that — it is a
    // tri-state bool and this is a fourth answer — so the state travels beside it by name.
    assert_eq!(
        v["state"], "unconfirmed",
        "a process holds the id and did not write this heartbeat: neither running nor gone: {v}"
    );
    assert_eq!(
        v["running"],
        serde_json::Value::Null,
        "which means `running` is a `null`, not the `false` that used to send people to \
         reinstall a service that was working: {v}"
    );
    assert!(
        !v["heartbeat"].is_null(),
        "and the heartbeat still renders — it stays useful diagnostics: {v}"
    );
}

/// nxf 6j6v.kcan, at the surface a person actually reads: the same heartbeat, without `--json`.
///
/// The measured incident was somebody reading the human line and acting on it — so the human line
/// is where the claim has to stop being made, not only the machine one.
#[test]
fn the_human_status_of_an_unconfirmable_service_never_says_the_process_is_gone() {
    let tmp = TempDir::new().unwrap();
    let hb_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&hb_dir).unwrap();
    let pid = std::process::id();
    std::fs::write(
        hb_dir.join("sync-daemon.json"),
        format!(
            r#"{{"pid":{pid},"started_at":"2026-07-30T09:00:00Z","last_pass_at":"2026-07-30T09:05:00Z",
            "workspaces":[],
            "write_failures":{{"count":2,"last_at":"2026-08-31T15:06:00Z",
            "last_error":"writing sync-daemon.json: No space left on device (os error 28)"}}}}"#
        ),
    )
    .unwrap();

    let out = nxs(tmp.path())
        .args(["sync", "daemon", "status"])
        .output()
        .expect("run nxs sync daemon status");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("the process is gone"),
        "the claim this ticket exists to stop making: {stdout}"
    );
    assert!(
        stdout.contains("UNCONFIRMED"),
        "and it says which answer it is giving instead: {stdout}"
    );
    // The second half of the ticket: a failed heartbeat write went only into a log nobody opens.
    assert!(
        stdout.contains("No space left on device"),
        "a gap in the heartbeat now carries the reason it had: {stdout}"
    );
}

/// nxf 6j6v.d43g at the surface the incident was read on: the counterpart of the test above.
///
/// A live pid whose heartbeat is stamped LATER than the process began is the service itself,
/// stamping late — no handed-on id can produce it. It used to read as "NOT running (the process is
/// gone)", then as `unconfirmed`; both are answers about a service that is working.
#[test]
fn status_reports_a_service_that_stamped_itself_late_as_running() {
    let tmp = TempDir::new().unwrap();
    let hb_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&hb_dir).unwrap();
    let pid = std::process::id();
    // This test process began seconds ago, so the only stamp that can sit more than the slack
    // AFTER its start is one ahead of the clock. The measured case — all three instants in the
    // past, 69 minutes apart — is pinned on synthetic instants in `born_together`'s own tests.
    let late =
        nxs_service::heartbeat::rfc3339(SystemTime::now() + Duration::from_secs(2 * 60 * 60));
    std::fs::write(
        hb_dir.join("sync-daemon.json"),
        format!(
            r#"{{"pid":{pid},"started_at":"{late}","last_pass_at":"2026-07-30T09:05:00Z",
            "workspaces":[]}}"#
        ),
    )
    .unwrap();

    let v = run_status_json(tmp.path());
    assert_eq!(
        v["state"], "running",
        "a service that stamped itself late is running, and saying anything else about it is \
         what sent somebody to reinstall a working service: {v}"
    );
    assert_eq!(v["running"], serde_json::json!(true), "{v}");

    let out = nxs(tmp.path())
        .args(["sync", "daemon", "status"])
        .output()
        .expect("run nxs sync daemon status");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("running — pid"),
        "and the human line says so too: {stdout}"
    );
}

#[test]
fn status_with_a_heartbeat_naming_a_dead_pid_reports_not_running_but_still_renders_it() {
    let tmp = TempDir::new().unwrap();
    let hb_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&hb_dir).unwrap();
    std::fs::write(
        hb_dir.join("sync-daemon.json"),
        r#"{"pid":999999999,"started_at":"2026-07-30T09:00:00Z","last_pass_at":"2026-07-30T09:05:00Z",
            "workspaces":[{"path":"/repo","last_ok":"2026-07-30T09:05:00Z","last_error":null,
            "pushed":3,"pulled":7}]}"#,
    )
    .unwrap();

    let v = run_status_json(tmp.path());
    assert_eq!(
        v,
        serde_json::json!({
            "running": false,
            "state": "not_running",
            "instance": "nexus-flow",
            "home": default_home(tmp.path()),
            "registration": NOBODY_LOOKED_AT_LAUNCHD,
            "shared_workspaces": [],
            "heartbeat": {
                "pid": 999_999_999,
                "started_at": "2026-07-30T09:00:00Z",
                "last_pass_at": "2026-07-30T09:05:00Z",
                "workspaces": [{
                    "path": "/repo",
                    "last_ok": "2026-07-30T09:05:00Z",
                    "last_error": null,
                    "pushed": 3,
                    "pulled": 7,
                }],
                "program": null,
                "version": null,
                "instance": null,
                // nxf 6j6v.kcan. Rendered `null` rather than omitted, exactly like the three
                // above it: a reader must never have to tell "the key is absent" from "there is
                // nothing to report".
                "write_failures": null,
            },
            "program": absent_program(tmp.path()),
        }),
        "a stale heartbeat still renders in full — only `running` reflects the dead pid"
    );
}

// ---- final-review fix wave (E4 continuous sync client) ----------------------------------------
//
// `serve()`'s real-clock loop has no injection seam to unit-test directly (unlike
// `Scheduler`/`DaemonState`/`tick`, which the daemon-module unit tests drive with a synthetic
// clock) — these three pin the OBSERVABLE, process-level effect at the CLI boundary instead, the
// way `daemon_starts_from_a_directory_with_no_workspace_anywhere_up_the_tree` above already does.

// Finding 2 (final review): a freshly started daemon used to leave `Scheduler::due()` false for a
// full `interval` (300s in production), during which no heartbeat existed and `status` answered
// `running: false, heartbeat: null` — the exact question the verb exists to answer, unanswerable
// for up to five minutes after start. The scheduling half is pinned directly in
// `crates/nxs/src/sync/daemon.rs`'s `a_fresh_scheduler_is_due_immediately_at_startup`; this pins
// the combined effect (immediate-due first tick + the explicit pre-loop heartbeat write) as it
// actually matters to a caller of `status`: a `--interval 300` daemon (the production default) must
// still answer `running: true` within a couple of seconds, not up to the full interval.
/// nxf 6j6v.kcan, through the running loop rather than beside it (review of PR #419, Test
/// Quality #4).
///
/// `WriteFailureLog` is unit-tested on its own, and `status` is unit-tested on its rendering — but
/// the thing the ticket is actually about is the WIRING between them: that `serve` records a
/// failed heartbeat write and that the next write which SUCCEEDS carries it out. Swapping the two
/// calls for no-ops leaves both unit suites green, so this is the only level that can hold it.
///
/// Staged the way the incident happened, minus the full disk: the heartbeat PATH is occupied by a
/// directory, so every write onto it fails; removing the directory is the disk freeing up again.
#[test]
fn a_heartbeat_write_that_failed_is_carried_out_by_the_next_one_that_succeeds() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&home).unwrap();
    // A DIRECTORY where the heartbeat file belongs: the atomic writer's rename onto it cannot
    // succeed, which is a write failure the service must survive rather than exit on.
    let heartbeat = home.join("sync-daemon.json");
    std::fs::create_dir(&heartbeat).unwrap();

    let mut daemon = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "1", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");

    // Let the startup write and at least one pass write hit the blocked path.
    let blocked_until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < blocked_until {
        std::thread::sleep(Duration::from_millis(100));
    }
    let still_alive = daemon.try_wait().expect("try_wait").is_none();
    std::fs::remove_dir(&heartbeat).expect("the path frees up again");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut carried = serde_json::Value::Null;
    while Instant::now() < deadline {
        let v = run_status_json(tmp.path());
        if !v["heartbeat"]["write_failures"].is_null() {
            carried = v["heartbeat"]["write_failures"].clone();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = daemon.kill();
    let _ = daemon.wait();

    assert!(
        still_alive,
        "a service that cannot write its heartbeat keeps running — that is the decision this \
         ticket wrote down, and exiting instead would take the machine's only clock with it"
    );
    assert!(
        !carried.is_null(),
        "the next successful heartbeat must say the gap had a cause; it carried nothing"
    );
    assert!(
        carried["count"].as_u64().unwrap_or(0) >= 1,
        "how many writes were missed: {carried}"
    );
    assert!(
        !carried["last_at"].as_str().unwrap_or_default().is_empty(),
        "when: {carried}"
    );
    assert!(
        carried["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("sync-daemon.json"),
        "and why, naming the file that could not be written: {carried}"
    );
}

#[test]
fn status_reports_running_within_a_couple_seconds_of_a_fresh_start() {
    let tmp = TempDir::new().unwrap();
    let mut daemon = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "300", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut running = false;
    while Instant::now() < deadline {
        if run_status_json(tmp.path())["running"] == serde_json::json!(true) {
            running = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let heartbeat = run_status_json(tmp.path())["heartbeat"].clone();
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(
        running,
        "status must report running:true within a few seconds of a fresh start, not up to the \
         full 300s interval a stale-by-design scheduler would have required"
    );

    // **And it says WHICH binary it is** (nxf 6j6v.dcpk (a)). Until this, a machine could not
    // answer that at all: the launchd job names an alias, two installs do not collide, the last
    // one silently wins, and `sync-daemon.json` recorded pid/started_at/last_pass_at and nothing
    // about the program. Measured on the owner's machine on 2026-08-29, the running service and
    // what the alias pointed at were already two different builds with nowhere to notice it.
    let program = heartbeat["program"]
        .as_str()
        .unwrap_or_else(|| panic!("a live service records its own program: {heartbeat}"));
    assert!(
        Path::new(program).is_absolute() && Path::new(program).exists(),
        "and it is a real, resolved path — never the alias it may have been exec'd through: \
         {program}"
    );
    assert_eq!(
        heartbeat["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "with the version of that binary beside it: {heartbeat}"
    );
}

// Finding 3 (final review): `serve()` used to load the global default endpoint ONCE, before the
// loop, while the workspace registry is re-read every tick. A workspace with no endpoint would
// then never recover even after running `nxs sync endpoint <url>` — the exact remedy the daemon's
// own log line names — because the running process never looked at the config file again. This
// proves the opposite: an endpoint that appears in `config.toml` AFTER the daemon has already
// started (and already ticked past the workspace once with no endpoint at all) is picked up on a
// LATER tick, with no restart. Against the OLD (pre-fix) code this test would time out: the
// workspace's `health` entry never gets created at all, because a `skipped_no_endpoint` workspace
// never reaches the `report.synced`/`report.errors` bucket that feeds the heartbeat.
#[test]
fn a_global_endpoint_written_after_the_daemon_starts_is_picked_up_without_restart() {
    let relay = common::spawn_relay();
    let tmp = TempDir::new().unwrap();

    // A workspace bound to a stream but with NO endpoint anywhere yet — `sync bind` never sets
    // one by itself — then registered (via `bind`'s own upsert) into the isolated registry the
    // daemon reads every tick.
    init(tmp.path());
    nxs_oneshot(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--create", "--json"])
        .assert()
        .success();

    let mut daemon = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "1", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");

    // Give the daemon a couple of ticks to see the freshly-registered workspace and find it has
    // no resolvable endpoint yet (finding 2's immediate-first-tick fix makes this fast — no need
    // to wait out a real interval).
    std::thread::sleep(Duration::from_millis(1500));

    // Plant the global default AFTER the daemon is already running — written directly rather
    // than through a second `nxs sync endpoint` invocation, so this assertion is isolated from
    // that command's own correctness and tests only the daemon's own re-read.
    let config_path = tmp
        .path()
        .join(".fake-home")
        .join(".nexusflow")
        .join("config.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
        &config_path,
        format!("[sync]\ndefault_endpoint = \"{relay}\"\n"),
    )
    .unwrap();

    // Poll the heartbeat file directly (not the daemon's own `status`, to avoid coordinating two
    // processes' notion of time) for a SUCCESSFUL pass, up to a generous ceiling.
    let hb_path = tmp
        .path()
        .join(".fake-home")
        .join(".nexusflow")
        .join("sync-daemon.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut synced = false;
    while Instant::now() < deadline {
        if let Ok(raw) = std::fs::read_to_string(&hb_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                synced = v["workspaces"]
                    .as_array()
                    .map(|ws| ws.iter().any(|w| w["last_ok"].is_string()))
                    .unwrap_or(false);
                if synced {
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let _ = daemon.kill();
    let output = daemon
        .wait_with_output()
        .expect("wait for the killed daemon");
    assert!(
        synced,
        "the daemon never synced after the global endpoint was written mid-run — the config \
         load looks cached from before the daemon started; stdout was:\n{}\nstderr was:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// Finding 4 (final review): `serve()` used to propagate a malformed `workspaces.toml` with `?`,
// which killed the whole daemon process on the very next tick. Under launchd's `KeepAlive` (task
// 12, spec §5) that means a respawn every ~10s forever, into an unrotated log — for a file the
// registry's own module doc calls hand-editable. §5.6's "a broken workspace never stops the
// sweep" contract must extend to the sweep's own input. This proves the daemon survives instead.
#[test]
fn a_malformed_registry_is_logged_and_skipped_not_a_fatal_exit() {
    let tmp = TempDir::new().unwrap();
    let reg_dir = tmp.path().join(".fake-home").join(".nexusflow");
    std::fs::create_dir_all(&reg_dir).unwrap();
    std::fs::write(
        reg_dir.join("workspaces.toml"),
        "this is not = = valid toml [[[",
    )
    .unwrap();

    let mut daemon = nxs(tmp.path())
        .args(["sync", "daemon", "--interval", "1", "--json"])
        .spawn()
        .expect("spawn nxs sync daemon");

    // Long enough to clear several ticks over the malformed file — a propagating `?` would have
    // exited on the very first one.
    std::thread::sleep(Duration::from_millis(1500));
    let status = daemon.try_wait().expect("try_wait");

    // Always kill before asserting, so a failing assertion never strands a background process.
    let _ = daemon.kill();
    let output = daemon
        .wait_with_output()
        .expect("wait for the killed daemon");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        status.is_none(),
        "a malformed registry must not exit the daemon; stdout was:\n{stdout}\nstderr was:\n{stderr}"
    );
    assert!(
        stdout.contains("workspace registry"),
        "the malformed registry is logged, not silently swallowed; stdout was:\n{stdout}"
    );
}

/// **A black-box invocation never reads the developer's own `~/.nexusflow`** (review of PR #393,
/// Code Quality #1).
///
/// Since nxf 6j6v.dcpk every invocation of this binary resolves `$HOME` to look at the service's
/// program alias, whatever verb it was given — so the host's own service state became an input to
/// every black-box test in this repo. `nxs_test_support::cargo_bin` pins a throwaway `HOME` for
/// exactly that reason, the same way it already pins `NXC_TIMER`.
///
/// Asserted through the field that made the question askable: `status --json` reports the alias
/// path it resolved, and that path is derived from `$HOME`. This test deliberately sets NO `HOME`
/// of its own — the point is what a suite gets when it says nothing.
#[test]
fn a_helper_built_invocation_resolves_a_throwaway_home_not_the_developers() {
    let out = nxs_test_support::cargo_bin("nxs")
        .args(["sync", "daemon", "status", "--json"])
        .output()
        .expect("nxs runs");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("--json on stdout");
    let link = v["program"]["link"].as_str().expect("the alias path");

    let real_home = std::env::var("HOME").expect("the test runner has a HOME");
    let real_alias = Path::new(&real_home)
        .join(".nexusflow")
        .join("bin")
        .join("nexus-flow");
    assert_ne!(
        link,
        real_alias.display().to_string(),
        "a suite that says nothing must not be handed the developer's own service state"
    );
    assert!(
        link.contains("test-home"),
        "and what it IS handed is the throwaway home under target/: {link}"
    );
}
