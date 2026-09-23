//! The two doors onto the collecting coordinator's delivery (nxf 6j6v.gn8b): `nxc session deliver`
//! and [`Engine::deliver_held`].
//!
//! `the_coordinator_collects.rs` drives the mechanism — what is held, what one delivery says, when
//! it goes out. This file is about the SEAM PARITY the project's own rule asks for: a verb on the
//! command line has a counterpart on the library handle, and both are exercised at the surface a
//! consumer actually reaches.
//!
//! The queue is the INPUT here, written directly, because what is under test is the hand-over and
//! not the refusal that fills it.

use nexus_chat::collecting::Held;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-09-07T10:00:00Z";
const SESSION: &str = "s-pm";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: You are the pm.\n",
    )
    .unwrap();
    tmp
}

/// A session of the `pm` persona with one answer waiting for it.
fn a_caller_with_something_held(dir: &Path, body: &str) {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve")
        .open_chat_store()
        .expect("open store");
    store
        .create_pending_session(SESSION, "pm")
        .expect("mint the session");
    store
        .hold_wake(
            SESSION,
            &Held {
                thread_id: "m-t1".to_string(),
                message_id: "m-1".to_string(),
                sender: "local/coder".to_string(),
                body: body.to_string(),
                escalated: false,
            },
            NOW,
        )
        .expect("hold");
}

#[test]
fn the_seam_hands_a_settled_caller_what_was_held_for_it() {
    let tmp = workspace();
    a_caller_with_something_held(tmp.path(), "the seam's answer");
    let log = tmp.path().join("dry.log");

    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry {
                log: Some(log.clone()),
            },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let receipt = eng
        .deliver_held(
            Caller {
                session: None,
                actor: Some("host"),
                now: Some(NOW),
            },
            SESSION,
        )
        .expect("the delivery runs");
    assert_eq!(receipt.delivered, 1);
    assert_eq!(receipt.woke.as_deref(), Some(SESSION));
    assert!(receipt.outstanding.is_empty());

    let handed = std::fs::read_to_string(&log).expect("the worker was handed something");
    assert!(
        handed.contains("While you were working, 1 message(s) arrived"),
        "{handed}"
    );
    assert!(handed.contains("the seam's answer"), "{handed}");

    // Idempotent from the seam too: a host that calls this speculatively — which is exactly what a
    // host that knows when its own sessions settle will do — gets a clean no-op the second time.
    let again = eng
        .deliver_held(
            Caller {
                session: None,
                actor: Some("host"),
                now: Some(NOW),
            },
            SESSION,
        )
        .expect("the second call runs");
    assert_eq!(again.delivered, 0);
    assert!(again.woke.is_none());
}

#[test]
fn the_command_line_hands_over_the_same_thing_and_says_so() {
    let tmp = workspace();
    a_caller_with_something_held(tmp.path(), "the command line's answer");

    let mut cmd = nxs_test_support::cargo_bin("nxc");
    cmd.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "service")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    let out = cmd
        .args(["--json", "session", "deliver", SESSION])
        .assert()
        .success();
    let receipt: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("valid json");
    assert_eq!(receipt["session"], SESSION);
    assert_eq!(receipt["delivered"], 1);
    assert_eq!(receipt["escalations"], 0);
    assert_eq!(receipt["woke"], SESSION);

    let handed =
        std::fs::read_to_string(tmp.path().join("dry.log")).expect("the worker was handed");
    assert!(handed.contains("the command line's answer"), "{handed}");

    // …and an empty queue is a clean no-op with a sentence, not an error: the scheduled job and the
    // tick's sweep may both reach a session whose queue the other has drained.
    let mut again = nxs_test_support::cargo_bin("nxc");
    again
        .current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "service")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry");
    let out = again
        .args(["session", "deliver", SESSION])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(text.contains("nothing was held for session"), "{text}");
}
