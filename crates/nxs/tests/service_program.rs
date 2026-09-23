//! **A service whose program is gone says so, from every entrance** (nxf 6j6v.dcpk (c)).
//!
//! Black-box and macOS-only, because the state under test is a launchd plist plus a dead alias.
//! `$HOME` is pointed at a `TempDir`, so the plist and the alias are real files in a real layout —
//! the same one `nxs_service::launchd` writes — and nothing here can touch the developer's own
//! `~/Library/LaunchAgents` or `~/.nexusflow`.
//!
//! This is the case the ticket was cut for: today the ONLY report of a dead alias is a service that
//! never starts.
#![cfg(target_os = "macos")]

use std::path::Path;

use assert_cmd::Command;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// A `$HOME` with the launchd job installed and the program alias pointing at `target` — which the
/// caller creates, or does not.
fn home_with_alias_to(target: &Path) -> TempDir {
    home_for_instance_with_alias_to(&nxs_service::Instance::production(), target)
}

/// The same, for a named instance: its own plist under its own label, its own home directory, and
/// its alias under its own name (nxf 6j6v.gd9p).
fn home_for_instance_with_alias_to(instance: &nxs_service::Instance, target: &Path) -> TempDir {
    let home = TempDir::new().unwrap();
    let agents = home.path().join("Library").join("LaunchAgents");
    std::fs::create_dir_all(&agents).unwrap();
    let root = home.path().join(instance.home_dir());
    let alias = root.join("bin").join(instance.name());
    std::fs::write(
        agents.join(format!("{}.plist", instance.label())),
        nxs_service::launchd::render_plist(
            &instance.label(),
            &alias,
            &root.join("logs"),
            "/usr/bin:/bin",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::os::unix::fs::symlink(target, &alias).unwrap();
    home
}

/// Every entrance the multicall binary has — the four personas plus the SERVICE ALIAS itself, which
/// `nxs` routes on `argv[0]` exactly like the others (review of PR #393, Test Quality #4).
///
/// `nexus-flow` gets its own argv because it is not a fifth surface: everything after the program
/// name is forwarded to `nxs sync daemon`, so `--version` there would be an argument to the daemon
/// verb rather than to the binary. `status` is that alias's own cheapest invocation.
const ENTRANCES: &[(&str, &[&str])] = &[
    ("nxs", &["--version"]),
    ("nxf", &["--version"]),
    ("nxm", &["--version"]),
    ("nxc", &["--version"]),
    ("nexus-flow", &["status"]),
];

/// Run one entrance under `home` and return stderr. The arguments are deliberately the cheapest
/// each surface has — the report must reach an invocation that asks the product for nothing at all,
/// so a probe that did real work would prove less, not more.
fn stderr_of(persona: &str, args: &[&str], home: &TempDir) -> String {
    let out = Command::cargo_bin(persona)
        .expect("the persona binary")
        .args(args)
        // The developer's own shell names an instance for this repo (`.envrc`), and it must not
        // reach a subprocess whose whole subject is which home the service reads (nxf 6j6v.gd9p);
        // [`PinHome`] clears it with the home it pins.
        .pin_home(home.path())
        // This suite builds its own command, so it inherits no default from
        // `nxs_test_support::cargo_bin` — see `no_test_arms_the_real_scheduler.rs`. None of these
        // invocations runs a verb that arms anything, and the timer is pinned anyway: "it happens
        // not to schedule today" is the reasoning that gate exists to stop relying on.
        .env("NXC_TIMER", "dry")
        .output()
        .expect("the binary runs");
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// [`stderr_of`] for a named instance: the caller is not an alias, so it is `NXS_SERVICE_INSTANCE`
/// that tells it which service to look at.
fn stderr_of_instance(persona: &str, args: &[&str], home: &TempDir, instance: &str) -> String {
    let out = Command::cargo_bin(persona)
        .expect("the persona binary")
        .args(args)
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .env("NXS_SERVICE_INSTANCE", instance)
        .output()
        .expect("the binary runs");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn a_dead_program_alias_is_named_by_every_invocation_of_every_persona() {
    let tmp = TempDir::new().unwrap();
    let gone = tmp.path().join("target").join("debug").join("nxs");
    let home = home_with_alias_to(&gone);

    for (persona, args) in ENTRANCES {
        let err = stderr_of(persona, args, &home);
        assert!(
            err.contains("the nexus-flow background service cannot start"),
            "`{persona} {args:?}` said nothing about a dead service: {err:?}"
        );
        assert!(
            err.contains(&gone.display().to_string()),
            "and it must name the dead target: {err:?}"
        );
        assert!(
            err.contains("nxs sync daemon install"),
            "and the command that fixes it: {err:?}"
        );
    }
}

#[test]
fn a_healthy_alias_leaves_every_invocation_silent() {
    // Every entrance, not just `nxs` — the name of this test is a claim about all of them, and it
    // was checking one (review of PR #393, Code Quality #3).
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("nxs");
    std::fs::write(&real, b"#!/bin/sh\n").unwrap();
    let home = home_with_alias_to(&real);

    for (persona, args) in ENTRANCES {
        let err = stderr_of(persona, args, &home);
        assert!(
            !err.contains("background service cannot start"),
            "a healthy machine must stay quiet or the warning becomes noise nobody reads; \
             `{persona}` said: {err:?}"
        );
    }
}

#[test]
fn a_dangling_alias_with_no_job_installed_is_left_alone() {
    // The uninstall leftover: `nxs sync daemon uninstall` removes the plist and leaves the alias.
    // Nothing is supposed to be running, so nothing is broken.
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let bin = home.path().join(".nexusflow").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("gone"), bin.join("nexus-flow")).unwrap();

    let err = stderr_of("nxs", &["--version"], &home);
    assert!(
        !err.contains("background service cannot start"),
        "no job installed means nothing was promised: {err:?}"
    );
}

/// The `--version` probe above proves the report reaches an invocation that runs no verb. This one
/// proves the same for a real verb with a `--json` contract: the line goes to stderr and stdout
/// stays machine-readable.
#[test]
fn the_report_never_lands_in_a_json_stdout() {
    let tmp = TempDir::new().unwrap();
    let home = home_with_alias_to(&tmp.path().join("gone"));
    let out = Command::cargo_bin("nxs")
        .expect("nxs")
        .args(["sync", "daemon", "status", "--json"])
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .output()
        .expect("the binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("background service cannot start"),
        "the warning is on stderr"
    );
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is JSON: {e}\n{stdout}"));
    assert_eq!(parsed["running"], serde_json::Value::Bool(false));
    assert_eq!(parsed["program"]["exists"], serde_json::Value::Bool(false));
    assert_eq!(
        parsed["program"]["target"],
        serde_json::Value::String(tmp.path().join("gone").display().to_string()),
        "`status --json` names the alias's target, so an app never has to open the file by hand"
    );
}

/// `nxs sync daemon status` in HUMAN mode under `home` — stdout.
///
/// The `--json` branch is covered above and by `crates/cli/tests/sync_daemon.rs`; this is the other
/// half, which nothing drove end to end (review of PR #393, Test Quality #3). It is the branch a
/// person actually reads, and it is where the `program:` and `alias:` lines live.
fn human_status(home: &TempDir) -> String {
    let out = Command::cargo_bin("nxs")
        .expect("nxs")
        .args(["sync", "daemon", "status"])
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .output()
        .expect("the binary runs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Plant a heartbeat under `home` claiming a dead pid and the given resolved program.
fn heartbeat_naming(home: &TempDir, program: Option<&Path>) {
    let dir = home.path().join(".nexusflow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sync-daemon.json"),
        serde_json::json!({
            "pid": u32::MAX,
            "started_at": "2026-08-29T07:13:41Z",
            "last_pass_at": "2026-08-29T07:13:41Z",
            "workspaces": [],
            "program": program.map(|p| p.display().to_string()),
            "version": "0.72.0",
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn the_human_status_names_the_binary_the_alias_and_the_dead_target() {
    let tmp = TempDir::new().unwrap();
    let gone = tmp.path().join("target").join("debug").join("nxs");
    let home = home_with_alias_to(&gone);
    heartbeat_naming(&home, Some(&gone));

    let out = human_status(&home);
    assert!(
        out.contains("program: ") && out.contains("(nxs 0.72.0)"),
        "the human branch says which binary the service runs, with its version:\n{out}"
    );
    assert!(
        out.contains(&format!(
            "alias: {}",
            home.path()
                .join(".nexusflow")
                .join("bin")
                .join("nexus-flow")
                .display()
        )),
        "and names the alias:\n{out}"
    );
    assert!(
        out.contains("(MISSING)") && out.contains(&gone.display().to_string()),
        "and marks a dead target as dead:\n{out}"
    );
    assert!(
        out.contains("note: ") && out.contains("nxs sync daemon install"),
        "and closes with the note that says what to do:\n{out}"
    );
}

#[test]
fn a_heartbeat_from_before_the_field_says_unrecorded_rather_than_printing_nothing() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("nxs");
    std::fs::write(&real, b"#!/bin/sh\n").unwrap();
    let home = home_with_alias_to(&real);
    heartbeat_naming(&home, None);

    let out = human_status(&home);
    assert!(
        out.contains("program: unrecorded"),
        "every existing machine has such a heartbeat on disk, and an empty line would read as a \
         bug rather than as history:\n{out}"
    );
}

/// **A second symlink hop is not two builds** (review of PR #393, Code Quality #4).
///
/// The heartbeat's `program` is fully canonical (the service resolves `current_exe()` because under
/// launchd that IS the alias), while the alias's own target is one `read_link` hop. An install
/// reached through a shim — Homebrew's `bin`, a versioned directory — therefore looked like a
/// version drift on every single `status`, while `self-update`'s equivalent check, which
/// canonicalises both sides, correctly said nothing about the same machine.
#[test]
fn an_alias_reached_through_a_further_symlink_is_not_reported_as_a_second_build() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real").join("nxs");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, b"#!/bin/sh\n").unwrap();
    let shim = tmp.path().join("shim");
    std::fs::create_dir_all(&shim).unwrap();
    let shim_nxs = shim.join("nxs");
    std::os::unix::fs::symlink(&real, &shim_nxs).unwrap();

    // The alias names the SHIM; the running service recorded the canonical binary behind it. One
    // machine, one build, two spellings.
    let home = home_with_alias_to(&shim_nxs);
    heartbeat_naming(&home, Some(&std::fs::canonicalize(&real).unwrap()));

    let out = human_status(&home);
    assert!(
        !out.contains("two different builds"),
        "one build reached two ways must not read as a version drift:\n{out}"
    );
    // …and the line a reader has to act on still shows what the link LITERALLY says, not the
    // resolved path, because that is the file they would go and change.
    assert!(
        out.contains(&shim_nxs.display().to_string()),
        "the alias line still shows the raw target:\n{out}"
    );
}

/// **The report follows the INSTANCE, not a constant** (nxf 6j6v.gd9p).
///
/// The dead-alias warning reads three things off `$HOME`: whether a plist is installed, where the
/// alias is, and what it points at. All three used to be fixed strings. This drives the whole path
/// for `nexus-flow-dev` — its own plist, its own `~/.nexusflow-dev`, its own alias name — and then
/// checks the direction that would go wrong first: production, standing beside it, must stay quiet,
/// because none of that is its.
#[test]
fn a_named_instance_reports_its_own_dead_alias_and_leaves_production_alone() {
    let dev = nxs_service::Instance::named("nexus-flow-dev").expect("a legal name");
    let tmp = TempDir::new().unwrap();
    let gone = tmp.path().join("target").join("debug").join("nxs");
    let home = home_for_instance_with_alias_to(&dev, &gone);

    let err = stderr_of_instance("nxs", &["--version"], &home, "nexus-flow-dev");
    assert!(
        err.contains("the nexus-flow background service cannot start")
            && err.contains(&gone.display().to_string()),
        "the dev instance must report its own dead alias: {err:?}"
    );
    assert!(
        err.contains(".nexusflow-dev"),
        "and it must name ITS home, not the shared one: {err:?}"
    );

    // Production shares the machine and has nothing installed here: silence, not somebody else's
    // fault reported as its own.
    let quiet = stderr_of("nxs", &["--version"], &home);
    assert!(
        !quiet.contains("background service cannot start"),
        "production must not inherit a sister instance's broken alias: {quiet:?}"
    );

    // And the alias the dev instance runs is its own file, under its own name.
    assert!(home
        .path()
        .join(".nexusflow-dev")
        .join("bin")
        .join("nexus-flow-dev")
        .symlink_metadata()
        .is_ok());
}

/// The running service's own channel: started as its alias, with NO environment naming an
/// instance, `nexus-flow-dev status` must still answer for `~/.nexusflow-dev`.
///
/// This is the hard constraint from the ticket exercised end to end — `ProgramArguments` carries
/// one element, so `argv[0]` is all a launchd-started service ever gets.
#[test]
fn a_service_started_as_its_alias_needs_no_environment_to_know_which_it_is() {
    let dev = nxs_service::Instance::named("nexus-flow-dev").expect("a legal name");
    let tmp = TempDir::new().unwrap();
    let gone = tmp.path().join("target").join("debug").join("nxs");
    let home = home_for_instance_with_alias_to(&dev, &gone);

    // `nexus-flow-dev` is not a shipped symlink, so it is made here — which is exactly what
    // `link_program` does on install, and exactly how launchd starts the service.
    let alias_dir = tmp.path().join("aliases");
    std::fs::create_dir_all(&alias_dir).unwrap();
    let alias = alias_dir.join("nexus-flow-dev");
    let real = assert_cmd::cargo::cargo_bin("nxs");
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    let out = std::process::Command::new(&alias)
        .arg("status")
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .output()
        .expect("the alias runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(".nexusflow-dev"),
        "started as `nexus-flow-dev` with nothing in the environment, the service must answer for \
         its own home:\n{stdout}"
    );
    assert!(
        !stdout.contains(".nexusflow/"),
        "and never for the production one:\n{stdout}"
    );
}
