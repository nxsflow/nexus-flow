//! **A running service really holds the machine awake, and really lets go** (nxf 6j6v.7q3r).
//!
//! Two things are proven apart from each other elsewhere: `nxs_service::wake` proves the assertion
//! against a real `pmset`, and `daemon::a_run_is_alive` proves the decision against real pid files.
//! What neither can prove is the fifteen lines of the loop that JOIN them — and that is exactly
//! where the expensive mistake lives, because inverting the polarity there produces a Mac that
//! never sleeps and a machine nobody suspects for days.
//!
//! So this drives the real binary: a real service process, a real workspace it attends, a real
//! session claim held by a real child, and `pmset` asked between each step.
//!
//! **Its own INSTANCE**, and not only for tidiness (nxf 6j6v.gd9p): the assertion is found by NAME,
//! and the name carries the instance — so a test named after the production service would find the
//! developer's own running one and pass without proving anything. A per-run instance makes the
//! name unique, which is also the first live evidence that the reason string says WHICH service is
//! keeping the machine up.
#![cfg(target_os = "macos")]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// This run's own service instance — legal (lowercase alphanumeric segments) and unique per test
/// process, so nothing here can be confused with the developer's real service.
fn instance_name() -> String {
    format!("nexus-flow-t{}", std::process::id())
}

fn assertions() -> String {
    let out = Command::new("/usr/bin/pmset")
        .args(["-g", "assertions"])
        .output()
        .expect("pmset runs on macOS");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Poll until the machine agrees, or give up. Returns whether it agreed.
fn wait_for_assertion(name: &str, want: bool, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if assertions().contains(name) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assertions().contains(name) == want
}

/// Kill and reap on the way out, whatever the test did — a panicking assertion must not leave a
/// service or a stand-in session running on the developer's machine.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_service_holds_the_machine_awake_while_a_run_lives_and_releases_when_it_ends() {
    let instance = instance_name();
    let reason = format!("{instance} is running an agent session");
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    // A real workspace…
    let init = Command::new(cargo_bin("nxs"))
        .args(["init", "--module", "chat", "--json"])
        .current_dir(project.path())
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .env("NXS_SERVICE_INSTANCE", &instance)
        .output()
        .expect("nxs runs");
    assert!(
        project.path().join(".nxs").join("replica.toml").is_file(),
        "the workspace was not created: {}{}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );

    // …that this instance attends. Written directly rather than through `sync bind`, which would
    // also want a relay: attendance is a line in the registry, and that is all the service reads.
    let service_home = home
        .path()
        .join(format!(".nexusflow-t{}", std::process::id()));
    std::fs::create_dir_all(&service_home).unwrap();
    std::fs::write(
        service_home.join("workspaces.toml"),
        format!(
            "[[workspace]]\nname = \"under-test\"\npath = \"{}\"\n",
            project.path().display()
        ),
    )
    .unwrap();

    assert!(
        !assertions().contains(&reason),
        "this instance's name must be unique to this run"
    );

    let service = Reaped(
        Command::new(cargo_bin("nxs"))
            .args(["sync", "daemon"])
            .current_dir(home.path())
            .pin_home(home.path())
            .env("NXC_TIMER", "dry")
            .env("NXS_SERVICE_INSTANCE", &instance)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the service starts"),
    );

    // Nothing is running yet, so the machine must be free to sleep. Given a few seconds so this is
    // a real observation of a live service rather than of one that has not started.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !assertions().contains(&reason),
        "a service with nothing to do must not be holding the machine awake"
    );

    // A stand-in for a run: a real live process, and its claim written exactly where a session
    // writes one.
    let session = Reaped(
        Command::new("/bin/sleep")
            .arg("300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep starts"),
    );
    let logs = project.path().join(".nxs").join("agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    let claim = logs.join("m-under-test.pid");
    std::fs::write(&claim, session.0.id().to_string()).unwrap();

    assert!(
        wait_for_assertion(&reason, true, 20),
        "a live run in an attended workspace did not keep the machine awake:\n{}",
        assertions()
    );

    // **The direction that matters.** The run ends; the claim it leaves behind names a dead pid,
    // exactly as a finished session's does. If the assertion survives this, the machine never
    // sleeps again and nobody connects it to us.
    drop(session);
    assert!(
        wait_for_assertion(&reason, false, 20),
        "the assertion outlived the run that justified it — this is the state a person meets days \
         later as a battery problem:\n{}",
        assertions()
    );

    drop(service);
}

/// The third guarantee, at the level the service actually runs at: it is killed outright while
/// holding the assertion, and the machine is free again anyway.
#[test]
fn a_service_killed_mid_run_leaves_no_assertion_behind() {
    let instance = format!("{}k", instance_name());
    let reason = format!("{instance} is running an agent session");
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    Command::new(cargo_bin("nxs"))
        .args(["init", "--module", "chat", "--json"])
        .current_dir(project.path())
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .env("NXS_SERVICE_INSTANCE", &instance)
        .output()
        .expect("nxs runs");
    assert!(project.path().join(".nxs").join("replica.toml").is_file());

    let service_home = home
        .path()
        .join(format!(".nexusflow-t{}k", std::process::id()));
    std::fs::create_dir_all(&service_home).unwrap();
    std::fs::write(
        service_home.join("workspaces.toml"),
        format!(
            "[[workspace]]\nname = \"under-test\"\npath = \"{}\"\n",
            project.path().display()
        ),
    )
    .unwrap();

    let session = Reaped(
        Command::new("/bin/sleep")
            .arg("300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep starts"),
    );
    let logs = project.path().join(".nxs").join("agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("m-under-test.pid"), session.0.id().to_string()).unwrap();

    let mut service = Command::new(cargo_bin("nxs"))
        .args(["sync", "daemon"])
        .current_dir(home.path())
        .pin_home(home.path())
        .env("NXC_TIMER", "dry")
        .env("NXS_SERVICE_INSTANCE", &instance)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the service starts");

    assert!(
        wait_for_assertion(&reason, true, 20),
        "the service never took one, so there is nothing to prove about killing it:\n{}",
        assertions()
    );

    // `kill` is SIGKILL: no destructor runs, so neither `want(false)` nor the keeper's `Drop`
    // happens. What is left is the kernel — and under `KeepAlive` a service that crashed and left
    // an assertion behind would accumulate one per life.
    service.kill().expect("the service can be killed");
    service.wait().expect("and reaped");
    assert!(
        wait_for_assertion(&reason, false, 20),
        "a killed service stranded the machine awake, and no restart could clear it:\n{}",
        assertions()
    );
}
