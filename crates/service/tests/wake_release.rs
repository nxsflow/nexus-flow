//! **The assertion really is given back** (nxf 6j6v.7q3r) — against the machine's own power
//! management, not a spy.
//!
//! `wake.rs`'s unit tests prove the DECISION: when one is taken, when it is given back, that a
//! dropped keeper does not leak one. They prove it against a stub, so what they cannot prove is
//! that the FFI underneath does anything at all — an `IOPMAssertionCreateWithName` that silently
//! failed, or a `release` that released the wrong id, would pass every one of them.
//!
//! This suite asks `pmset` instead, because the failure it guards against is one nobody reports:
//! a machine that never sleeps again, noticed days later as a battery or a fan problem.
//!
//! Both directions, **and the second one first**, exactly as the ticket asks:
//!
//! 1. the assertion appears while it is held and is gone once it is released;
//! 2. **the assertion is gone when the process holding it is KILLED** — the guarantee no code can
//!    give itself, and the reason the service can crash without stranding the machine awake.
#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nxs_service::wake::{SystemWake, WakeKeeper};

/// The env var that turns this test binary into the CHILD of the kill test — it holds an assertion
/// under the name given and then sleeps until somebody kills it.
const HOLD_ENV: &str = "NXS_WAKE_HOLD_UNDER_NAME";

/// A name no other run can collide with, so grepping `pmset` for it is unambiguous.
fn unique_name(tag: &str) -> String {
    format!(
        "nxs-service test {tag} {} {:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// Whether `pmset` currently lists an assertion under `name`.
fn assertion_is_listed(name: &str) -> bool {
    let out = Command::new("/usr/bin/pmset")
        .args(["-g", "assertions"])
        .output()
        .expect("pmset runs on macOS");
    String::from_utf8_lossy(&out.stdout).contains(name)
}

/// Poll until `pmset` agrees with `want`, or give up after `secs`. Returns whether it agreed.
///
/// Polling rather than a fixed sleep: the power-management daemon is asked out of process, so the
/// answer is eventually-consistent by a few milliseconds and a fixed wait would either be flaky or
/// slow.
fn wait_until_listed(name: &str, want: bool, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if assertion_is_listed(name) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assertion_is_listed(name) == want
}

/// Direction one, and the whole reason the module exists: a real assertion is taken, the system
/// sees it, and giving it back really removes it.
#[test]
fn a_real_assertion_appears_while_held_and_is_gone_once_released() {
    let name = unique_name("held-then-released");
    assert!(
        !assertion_is_listed(&name),
        "the name must be unique to this run"
    );

    let wake = SystemWake;
    let mut keeper = WakeKeeper::new(&wake, name.clone());
    keeper.want(true);
    assert!(
        keeper.is_held(),
        "the system refused an assertion this test needs; run it on a Mac that allows one"
    );
    assert!(
        wait_until_listed(&name, true, 5),
        "the machine does not think anything is holding it awake, so nothing was really asserted"
    );

    keeper.want(false);
    assert!(
        wait_until_listed(&name, false, 5),
        "the assertion survived its release — this is the state that stops a Mac ever sleeping again"
    );
}

/// The same, with nobody calling `want(false)`: the keeper simply goes out of scope, which is every
/// early return out of the service loop.
#[test]
fn a_keeper_that_goes_out_of_scope_really_hands_the_assertion_back_to_the_system() {
    let name = unique_name("dropped");
    let wake = SystemWake;
    {
        let mut keeper = WakeKeeper::new(&wake, name.clone());
        keeper.want(true);
        assert!(wait_until_listed(&name, true, 5), "nothing was asserted");
    }
    assert!(
        wait_until_listed(&name, false, 5),
        "a keeper that went out of scope holding one left it with the system"
    );
}

/// **Direction two: the service dies holding one.**
///
/// A `SIGKILL` runs no destructor, so neither `want(false)` nor `Drop` happens — the only thing
/// left is the kernel, which owns the assertion on the process's behalf and reclaims it on exit.
/// That is the property the service's crash-restart loop rests on (`KeepAlive` restarts it, and a
/// stranded assertion from every previous life would accumulate), and it is asserted here rather
/// than assumed.
///
/// The child is THIS test binary, re-executed with [`HOLD_ENV`] set — the same trick the repo's
/// other "prove it against a real process" tests use, and the only way to get a second process
/// that runs our own FFI without shipping a helper binary.
#[test]
fn an_assertion_dies_with_the_process_that_held_it_even_on_a_hard_kill() {
    if let Ok(name) = std::env::var(HOLD_ENV) {
        // The CHILD role. Hold one and wait to be killed; the parent is the only thing that ends
        // this, and the harness's own timeout is the backstop if it never does.
        let wake = SystemWake;
        let mut keeper = WakeKeeper::new(&wake, name);
        assert!(matches!(
            keeper.want(true),
            nxs_service::wake::WakeChange::Held
        ));
        std::thread::sleep(Duration::from_secs(120));
        return;
    }

    let name = unique_name("killed");
    let mut child = Command::new(std::env::current_exe().expect("this test binary names itself"))
        .args([
            "--exact",
            "an_assertion_dies_with_the_process_that_held_it_even_on_a_hard_kill",
            "--nocapture",
        ])
        .env(HOLD_ENV, &name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the test binary re-executes");

    let appeared = wait_until_listed(&name, true, 30);
    if !appeared {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the child never took an assertion, so there is nothing to prove about killing it");
    }

    child.kill().expect("the child can be killed");
    child.wait().expect("and reaped");

    assert!(
        wait_until_listed(&name, false, 10),
        "a process killed while holding an assertion left it behind — the machine would never \
         sleep again, and no restart of the service could clear it"
    );
}
