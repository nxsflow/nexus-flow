//! The service's clock, from the outside (6j6v.8see) — the one property no unit test can hold,
//! because it is about `argv[0]`.
//!
//! `nxs` routes by the name it was invoked as. Under launchd the service is `exec`ed through the
//! `nexus-flow` link, whose whole job is to put a NAME in the user's background items — and on
//! macOS `current_exe()` gives that link's path back, symlink and all. So a job spawned with
//! `Command::new(current_exe())` inherits `argv[0] = nexus-flow`, which routes every argument to
//! `nxs sync daemon`, and a due deadline came back as `error: unrecognized subcommand 'chat'`.
//! Measured on the first live run of the service; this is what keeps it fixed.

use std::os::unix::process::CommandExt as _;
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// Run the `nexus-flow`-named binary with an explicit `argv[0]`, returning combined output.
fn run_as(arg0: &str, args: &[&str], cwd: &TempDir) -> String {
    let out = Command::new(cargo_bin("nexus-flow"))
        .arg0(arg0)
        .args(args)
        .current_dir(cwd.path())
        .pin_home(cwd.path())
        .env("NXC_TIMER", "dry")
        .output()
        .expect("the binary runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_tick_a_due_deadline_runs_reaches_chat_and_not_the_daemon_verb() {
    let tmp = TempDir::new().unwrap();
    // Exactly the argv the service builds for a `chat_tick` job, with the `argv[0]` it forces.
    let forced = run_as("nxs", &["chat", "tick", "--thread", "m-01ABC"], &tmp);
    assert!(
        !forced.contains("unrecognized subcommand"),
        "the job must reach chat's `tick`: {forced}"
    );
    // It gets as far as resolving a workspace and finds none here — which is proof it reached the
    // verb, since `sync daemon` never resolves one.
    assert!(
        forced.contains("no workspace") || forced.contains("nxc init") || forced.contains("init"),
        "and it got as far as needing a workspace: {forced}"
    );
}

#[test]
fn the_same_argv_under_the_service_name_is_the_trap_this_guards_against() {
    let tmp = TempDir::new().unwrap();
    // The bug, pinned so the guard above cannot quietly stop meaning anything: reached with
    // `argv[0] = nexus-flow`, the identical arguments are forwarded to `nxs sync daemon`, which has
    // no `chat` subcommand.
    let trapped = run_as("nexus-flow", &["chat", "tick", "--thread", "m-01ABC"], &tmp);
    assert!(
        trapped.contains("unrecognized subcommand"),
        "the alias forwards to `nxs sync daemon`, so this is what an unforced argv[0] costs: \
         {trapped}"
    );
}

#[test]
fn the_service_alias_still_answers_the_daemon_question_it_is_for() {
    let tmp = TempDir::new().unwrap();
    let out = run_as("nexus-flow", &["--json", "status"], &tmp);
    assert!(
        out.contains("\"running\":false"),
        "`nexus-flow status` is `nxs sync daemon status`: {out}"
    );
}
