//! E5 #9t7.4 — Read-Acceptance: the embedding seam (Tauri-shaped), automated for CI.
//!
//! A long-lived in-process [`Engine`] handle — what a Tauri backend holds in managed state — lists
//! `next`/`blocked`, while a GENUINELY EXTERNAL `nxf` process (a separate OS process: the agent
//! seam) creates, links, and closes items in the same workspace. Each external write is delivered
//! to the handle as a [`Change`] (the app's cue to re-render), and the SAME handle — never
//! reopened — reads the new derivation live. This is the vertical proof that read + write +
//! reactivity compose over one store with no second semantics layer: the derivation the app sees
//! is exactly the CLI's, driven by real `nxf` writes.
//!
//! The companion manual proof against a real GUI build lives in `examples/tauri-board/`; this
//! file is the deterministic CI gate for the same acceptance.

use assert_cmd::Command;
use nexus_flow_core::model::ItemRow;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::watch::Change;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tempfile::TempDir;

/// Short poll interval so the reactive loop closes within a sub-second assertion window. Production
/// uses the default cadence; this is the documented test seam (`open_with_poll_interval`).
const FAST: Duration = Duration::from_millis(40);

/// A fixed reference time for `ready` derivation. The items here carry no defer date, so the exact
/// value is irrelevant — it only has to be a well-formed instant the engine accepts.
const NOW: &str = "2026-06-17T00:00:00Z";

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// Run an external `nxf … --json` command to success and return its parsed stdout. A real
/// subprocess — to SQLite, indistinguishable from the agent CLI mutating the store behind the
/// app's back.
fn nxf_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("nxf json output")
}

/// Block until the watcher delivers the next change (the app's re-render cue), then coalesce any
/// further queued events so a later wait observes the NEXT distinct external write rather than a
/// leftover. Times out loudly rather than hanging if reactivity is broken.
fn await_change(rx: &Receiver<Change>) {
    rx.recv_timeout(Duration::from_secs(5))
        .expect("a Change after the external nxf write");
    while rx.try_recv().is_ok() {}
}

fn ids(rows: &[ItemRow]) -> Vec<String> {
    rows.iter().map(|r| r.id.clone()).collect()
}

#[test]
fn external_nxf_writes_appear_live_through_the_engine_handle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // The agent seam initializes the workspace (real external process).
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();

    // The app opens ONE long-lived handle and subscribes — held for the whole test, never reopened.
    let engine = Engine::open_with_poll_interval(None, dir, FAST).unwrap();
    let rx = engine.subscribe().unwrap();

    // Initial in-process read: nothing yet.
    assert!(
        engine.next(NOW).unwrap().is_empty(),
        "no ready work at start"
    );
    assert!(
        engine.blocked().unwrap().is_empty(),
        "nothing blocked at start"
    );

    // 1) External `nxf create` of a task → live on the same handle, no reopen.
    let task = nxf_json(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Ship E5",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let tid = task["id"].as_str().unwrap().to_string();
    await_change(&rx);
    assert!(
        ids(&engine.next(NOW).unwrap()).contains(&tid),
        "the externally-created task is ready, live"
    );

    // 2) An external blocker + dependency → the task moves to blocked, observed live.
    let blocker = nxf_json(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Spec sign-off",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let bid = blocker["id"].as_str().unwrap().to_string();
    await_change(&rx);
    nxf_json(dir, &["dep", "add", &tid, &bid]); // tid depends on bid
    await_change(&rx);

    let blocked = engine.blocked().unwrap();
    assert!(
        blocked.iter().any(|b| b.item.id == tid),
        "the task is blocked by its external dependency, live"
    );
    let ready_now = ids(&engine.next(NOW).unwrap());
    assert!(
        !ready_now.contains(&tid),
        "the blocked task has left the ready set"
    );
    assert!(
        ready_now.contains(&bid),
        "while the blocker itself is ready"
    );

    // 3) External `nxf close` of the blocker → the task becomes ready again, live, no restart.
    nxf_json(dir, &["close", &bid, "--reason", "approved"]);
    await_change(&rx);
    assert!(
        ids(&engine.next(NOW).unwrap()).contains(&tid),
        "closing the blocker externally unblocks the task, live on the same handle"
    );
    assert!(
        engine.blocked().unwrap().is_empty(),
        "nothing blocked once the blocker is closed"
    );

    // 4) An external `nxf claim` → the task is IN PROGRESS and STAYS in `next`.
    //
    // This pins the MEANING of the seam, not just its shape (review of PR #422, Test Quality #1).
    // `next` is `ready ∪ in_progress` — claimed work is always included — and that has not always
    // been so: the seam once took a second argument that kept it out. When the argument went and
    // inclusion became unconditional, the signature change was caught by the compiler, and the
    // MEANING change was caught by nobody. `examples/tauri-board` went on labelling this list
    // "Ready" for seven weeks, which by then meant the opposite of what it showed.
    //
    // The production change that would fail this: `read::next` dropping in-progress items again.
    // The status assertion below is what stops it passing vacuously — without it, a `claim` that
    // silently did nothing would leave the item in `next` for the wrong reason.
    nxf_json(dir, &["claim", &tid]);
    await_change(&rx);
    let claimed = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .find(|r| r.id == tid)
        .expect("claimed work stays in `next` — the set is `ready ∪ in_progress`");
    assert_eq!(
        claimed.status.as_deref(),
        Some("in_progress"),
        "and it really is claimed, so the assertion above is about inclusion and not about a \
         claim that quietly did nothing"
    );
}
