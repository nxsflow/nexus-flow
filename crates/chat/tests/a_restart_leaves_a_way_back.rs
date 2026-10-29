//! **A planned service restart leaves every running session a way back, in place** (nxf 6j6v.wxz0,
//! the field case 6j6v.8n9a).
//!
//! `nxs self-update` and `nxs sync daemon install` restart the background service with `bootout` +
//! `bootstrap`, and launchd's SIGTERM reaches every session the service started, because they share
//! its process group. Before this item that killed them mid-turn with nothing on record: the thread
//! said "nothing is coming", `nxc resume` found nothing on hold, and the uncommitted work sat in the
//! tree with no way back but a person.
//!
//! The restart now holds each session of the instance under the `service_restart` limit, stops it
//! with a control signal, waits a grace, and the new service resumes it IN PLACE at `until`: the
//! same session, the same checkout, the same transcript. These tests drive that across REAL
//! processes where the claim is about a process — every `nxc` is a real child against one
//! workspace, a session is a real child holding its claim the way the sidecar writes it, and
//! `$HOME` is pinned by `nxs_test_support::cargo_bin`. Nothing sleeps for a state: each step reads
//! a named signal (a table row, a log line, a file).
//!
//! **This repository's own board is never touched**: every `nxc` here strips the `NXC_*` variables
//! it would otherwise inherit from the agent running the suite.

#![cfg(unix)]

use std::path::Path;

use assert_cmd::Command;
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const T0: &str = "2026-10-27T10:00:00Z";

/// A bare workspace with a coder.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Coder\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();
    tmp
}

/// One `nxc` process at `now`, with nothing inherited from the agent running this.
fn nxc(root: &Path, now: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    for (var, _) in std::env::vars() {
        if var.starts_with("NXC_") {
            c.env_remove(var);
        }
    }
    c.current_dir(root)
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", root.join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", root.join("timer.log"));
    c
}

/// **A restart hold cannot be forged from the command line** (round-2 IR #4). A hold under
/// `service_restart` pins the working copy until `until` plus the margin, so one written by hand
/// with a far-future `until` would pin it for good. The restart's hook records through the Engine
/// seam; the runtime's callback refuses the name, and records nothing.
#[test]
fn the_runtime_callback_refuses_the_service_restart_limit() {
    let tmp = workspace();
    let root = tmp.path();
    let out = nxc(root, T0)
        .args(["send", "--to", "coder", "--no-ref", "--json", "build it"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let session: String = nexus_chat::workspace::Workspace::resolve(None, root)
        .and_then(|ws| {
            use nexus_chat::workspace::ChatWorkspaceExt;
            ws.open_chat_store()
        })
        .unwrap()
        .connection()
        .query_row(
            "SELECT internal_id FROM session_map ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let refused = nxc(root, T0)
        .env("NXC_SESSION", &session)
        .args([
            "session",
            "interrupted",
            &session,
            "--limit",
            "service_restart",
            "--until",
            "2099-01-01T00:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success(), "the forged hold was accepted");
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains("service_restart"),
        "the refusal names the limit: {said}"
    );
    let resume = nxc(root, T0)
        .args(["resume", "--session", &session, "--json"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&resume.stdout).contains("nothing_on_hold"),
        "nothing was recorded: {}",
        String::from_utf8_lossy(&resume.stdout)
    );
}
