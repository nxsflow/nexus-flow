//! **The gate of nxf 6j6v.kvda, and the counter-proof it is measured by.**
//!
//! On 2026-08-30 a black-box test with a pinned `$HOME` ran the real installer. A pinned home
//! isolates FILES; it does not isolate launchd, whose `gui/<uid>` domain is the developer's own
//! login session whatever `$HOME` says. So launchd took the registration — with the test's TempDir
//! paths in it — the test deleted the TempDir, and the machine's production service could not load
//! for five days behind a registration nothing could see. Nothing was red; nothing said a word.
//!
//! The memory `launchd-install-is-not-home-isolated` has warned about this in prose since. Prose
//! warns the person who WRITES the next test. This file is the part that does not depend on anyone
//! having read it: `nxs_service::launchd::RealCtl` is constructible only for the login session
//! whose domain it addresses, so a process whose home has been pointed elsewhere is refused before
//! it writes anything at all.
//!
//! # This test builds the dangerous call in on purpose
//!
//! That is what a counter-proof is: the acceptance criterion is "from inside a test, `launchctl
//! bootstrap` is not reachable", and the only honest way to check it is to reach for it. Two things
//! keep that safe rather than reckless:
//!
//!   1. It runs under a **service instance no machine has** (`nexus-flow-guardtest`), so even a
//!      total regression of the gate would register `com.nxsflow.nexus-flow-guardtest` and could not
//!      touch `com.nxsflow.nexus-flow`, `-dev` or `-foundations-dev`.
//!   2. It then **asks the real launchd** whether that label exists — a read, never a write. If the
//!      gate ever regresses, this assertion is what goes red, in the same run, naming the job that
//!      was created.
#![cfg(target_os = "macos")]

use nxs_test_support::PinHome;
use std::process::Command;
use tempfile::TempDir;

/// An instance name no machine runs, so a regression of the gate cannot reach a real service. It is
/// still a LEGAL name (`nxs_service::Instance::named`), because a name the product refuses would
/// stop the install before the gate and prove nothing.
const HARMLESS_INSTANCE: &str = "nexus-flow-guardtest";

fn label() -> String {
    format!("com.nxsflow.{HARMLESS_INSTANCE}")
}

/// Run one verb of the real binary with `$HOME` pinned at `home` — the exact shape that did the
/// damage on 2026-08-30, whichever verb is handed to it.
fn nxs_from_a_pinned_home(home: &TempDir, args: &[&str]) -> std::process::Output {
    let bin = {
        nxs_test_support::assert_multicall_binary_fresh();
        assert_cmd::cargo::cargo_bin("nxs")
    };
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .pin_home(home.path())
        // Named AFTER `pin_home`, which clears it: this suite's whole point is to aim the install
        // at an instance no machine has.
        .env("NXS_SERVICE_INSTANCE", HARMLESS_INSTANCE)
        .env("NXC_TIMER", "dry");
    cmd.output().expect("nxs runs")
}

/// Whether the real login session holds a job under `label`. A READ of the machine's own launchd —
/// the only way to check that nothing was created, and the assertion that would have caught the
/// 2026-08-30 poisoning on the day it happened.
fn the_real_session_holds(label: &str) -> bool {
    let uid = unsafe { libc::getuid() };
    Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/{label}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// **The counter-proof.** `nxs sync daemon install` under a pinned `$HOME` is refused, it says why
/// in the vocabulary of the incident, it writes nothing, and — the assertion that actually guards
/// the machine — the real login session holds no job for the label afterwards.
#[test]
fn the_real_installer_refuses_to_run_from_a_home_that_is_not_the_login_sessions() {
    assert!(
        !the_real_session_holds(&label()),
        "this machine already holds {} before the test ran — boot it out by hand \
         (`launchctl bootout gui/$UID/{}`) before trusting anything below",
        label(),
        label()
    );

    let home = TempDir::new().unwrap();
    let out = nxs_from_a_pinned_home(&home, &["sync", "daemon", "install"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        !out.status.success(),
        "a pinned home must not be able to install a launchd agent: {stderr}"
    );
    assert!(
        stderr.contains("6j6v.kvda"),
        "the refusal names the incident it prevents, so the next reader does not have to guess \
         what the rule is for: {stderr}"
    );
    assert!(
        stderr.contains(&home.path().display().to_string()),
        "and it names the home this process actually has: {stderr}"
    );

    // **Nothing was written**, which is why the gate is checked before the plist and the alias
    // rather than after: a refused install must not leave a half-state of its own.
    assert!(
        !home
            .path()
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label()))
            .exists(),
        "a refused install writes no plist"
    );
    assert!(
        !home.path().join(".nexusflow-guardtest").exists(),
        "nor a service home"
    );

    // **And the real session is untouched.** This is the assertion the whole file exists for: if
    // the gate is ever removed, the line above will have registered a real job in a real login
    // session, and this is where that goes red.
    assert!(
        !the_real_session_holds(&label()),
        "a test just registered {} in this machine's real login session — that is the 6j6v.kvda \
         failure happening again. Boot it out with `launchctl bootout gui/$UID/{}`.",
        label(),
        label()
    );
}

/// `uninstall` is gated for the sharper half of the same reason. A `bootout` from a pinned home
/// does not boot out a test's job — there is none — it boots out the REAL one, and the production
/// clock stops without a word.
#[test]
fn the_real_uninstaller_is_refused_from_a_pinned_home_too() {
    let home = TempDir::new().unwrap();
    let out = nxs_from_a_pinned_home(&home, &["sync", "daemon", "uninstall"]);
    assert!(
        !out.status.success(),
        "a pinned home must not be able to boot a real agent out: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("6j6v.kvda"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `--json` refuses too, and says so in the machine-readable envelope rather than only in prose —
/// the same refusal, through the surface a script reads.
///
/// It also pins the sentence the memory `launchd-install-is-not-home-isolated` has carried since
/// 2026-08-30. That sentence is now said by the PRODUCT, to the process that is about to make the
/// mistake, instead of by a note to whoever might read it first.
#[test]
fn the_refusal_reaches_the_json_surface_with_its_reason() {
    let home = TempDir::new().unwrap();
    let out = nxs_from_a_pinned_home(&home, &["sync", "daemon", "install", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!out.status.success(), "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--json emits JSON: {e}: {stdout}"));
    assert_eq!(v["error"]["kind"], "forbidden", "{v}");
    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    assert!(
        msg.contains("A pinned $HOME isolates files and NOT launchd"),
        "{msg}"
    );
    assert!(msg.contains("6j6v.kvda"), "{msg}");
    assert!(!the_real_session_holds(&label()));
}

/// **The wiring, which the pure tests cannot reach.** `describe_registration`'s four sentences are
/// proved against `launchctl print`'s own bytes in `crates/service/src/launchd.rs` and
/// `crates/nxs/src/sync/daemon.rs`; what no pure test can prove is that `status` still PRINTS one.
/// This does — and it does it from the one state a test can reach honestly.
///
/// **What it deliberately does not do:** put a real poisoned registration in front of a real
/// `status`. That would mean bootstrapping a job into the machine's own login session, which is
/// the failure this file exists to prevent. The FOREIGN sentence reaching stdout was verified by
/// hand against a deliberately poisoned `nexus-flow-guardtest` registration on 2026-09-04, and the
/// reading is recorded in nxf 6j6v.kvda; here the covered claim is narrower and true: the line is
/// emitted, always, on the platform that has a launchd.
#[test]
fn status_always_says_something_about_the_registration_even_when_it_could_not_look() {
    let home = TempDir::new().unwrap();
    let out = nxs_from_a_pinned_home(&home, &["sync", "daemon", "status"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{stdout}");
    let line = stdout
        .lines()
        .find(|l| l.starts_with("registration: "))
        .unwrap_or_else(|| panic!("status prints a registration line: {stdout}"));
    assert!(
        line.contains("not checked"),
        "and from a pinned home it says it did not look, rather than saying nothing at all — \
         which would read exactly like `there is no registration`: {line}"
    );
    assert!(
        !line.contains(&home.path().display().to_string()),
        "the sentence is path-free, so it is the same on every machine that produces it: {line}"
    );
}

/// **The escape hatch must stay out of the test tree** (review of PR #428, Integrity #1).
///
/// `--allow-redirected-home` is the one way past the gate this file exists to prove. It is a FLAG
/// and not an environment variable for exactly this reason: a flag is typed once, on one command
/// line, and cannot be inherited by every subprocess of a test run the way one stray
/// `NXS_...=1` in a harness would be. What a flag still permits is somebody writing it into a test
/// to make a red one green — which would reopen the hole silently, in the file that is supposed to
/// hold it shut.
///
/// So: no test source in this repo may name it, this file's own explanation excepted. Reading
/// source text, in the shape `xtask/sweep-retired-vocabulary.sh` and
/// `crates/test-support/tests/every_pinned_home_pins_the_instance.rs` already use here — and with
/// the same honesty about its reach: it reads the test directories of this workspace's crates, so
/// a new crate's tests are covered from the day that crate has a `tests/` directory, and a test
/// that builds the string out of fragments is not read at all.
#[test]
fn no_test_names_the_redirected_home_escape() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/nxs -> crates -> repo root")
        .to_path_buf();
    let me = std::path::Path::new(file!())
        .file_name()
        .expect("this file has a name")
        .to_owned();

    let mut offenders = Vec::new();
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for crate_dir in std::fs::read_dir(repo.join("crates")).expect("crates/ is readable") {
        let dir = crate_dir.expect("a crate directory").path().join("tests");
        if dir.is_dir() {
            roots.push(dir);
        }
    }
    assert!(
        roots.len() >= 3,
        "the sweep found almost no test directories ({}) — it is reading the wrong tree, which \
         would make it pass by looking at nothing",
        roots.len()
    );

    while let Some(dir) = roots.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable test directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                roots.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            if path.file_name() == Some(me.as_os_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("allow-redirected-home") || text.contains("RedirectedIsAllowed") {
                offenders.push(path.display().to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these test files reach for the gate's escape hatch: {offenders:?}\n\
         `--allow-redirected-home` exists for an operator whose machine redirects $HOME on \
         purpose, not for a test that needs the guard out of the way. A test that needs to talk \
         to launchd is a test that must not run the real installer at all (nxf 6j6v.kvda)."
    );
}
