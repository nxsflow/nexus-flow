//! File-based change notification (E5 #9t7.3): an external write (== another process, to
//! SQLite) fires a `Change` on the live handle; no spurious events across completed poll cycles;
//! fan-out to multiple subscribers; and the watcher stops when the engine is dropped (no leaked
//! daemon). Tests drive a short poll interval via the `open_with_poll_interval` seam so they can
//! assert across several *completed* cycles quickly and deterministically.

use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::watch::Change;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// A short poll interval so several full poll cycles complete inside a sub-second assertion
/// window — used by every watch test for speed and determinism.
const FAST: Duration = Duration::from_millis(40);

/// Assert the change stream QUIESCES after a re-arm: drain any bounded trailing Changes, then
/// require a quiet gap of `quiet` (many poll cycles) with none. A same-path recreate legitimately
/// emits MORE than one Change — the identity re-baseline plus a few `data_version` deltas as the new
/// db's schema-init commits land (the watcher contract is "coalesced, not counted"; #9e9t evidence
/// showed 1–3 under load). Those settle within a bounded window, so draining-then-requiring-a-gap
/// tolerates them. A genuine STORM — the identity branch re-firing every tick because `current_id`
/// was never advanced — never produces the gap, so it exhausts `budget` and fails loudly. This is
/// the real property the old "exactly one Change in 400ms" check meant to protect, without racing
/// the settle deltas that made it flaky under full-suite load.
fn assert_quiesces(rx: &Receiver<Change>, quiet: Duration, budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        match rx.recv_timeout(quiet) {
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(_) => assert!(
                Instant::now() < deadline,
                "the recreate re-arm never quiesced within {budget:?} — Changes keep arriving every \
                 poll cycle, i.e. the identity branch is re-firing every tick (a storm), not the \
                 bounded re-baseline + schema-init settle it should be"
            ),
        }
    }
}

fn workspace_dir() -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    tmp
}

/// Commit one new item via a fresh connection — a foreign writer, indistinguishable from another
/// process to SQLite.
fn foreign_write(dir: &std::path::Path, id: &str) {
    let ws = workspace::discover(dir).unwrap();
    let mut store = ws.open_store().unwrap();
    store.create_item(id, "task", "x", "t");
}

/// Drain any already-queued change events without blocking — used after observing one event so a
/// later `recv` asserts the NEXT distinct trigger rather than a coalesced leftover.
fn drain(rx: &Receiver<Change>) {
    while rx.try_recv().is_ok() {}
}

/// Delete + recreate the workspace db at the SAME path: unlink the db file (and its WAL/SHM
/// sidecars) and materialize a fresh, schema-initialized store there. This mirrors a real
/// `nxf init` over an existing workspace, an E4 sync reset, or a plain `rm` + recreate — a new
/// inode at the old path. Any connection still open on the old file is now pinned to an unlinked
/// inode.
fn recreate_db(dir: &Path) {
    let ws = workspace::discover(dir).unwrap();
    let db = ws.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    // Materialize a fresh db.sqlite (+ schema) at the same path — a different inode.
    ws.open_store().unwrap();
}

#[test]
fn data_version_bumps_on_a_foreign_commit() {
    // Isolate the primitive directly (Test Quality #4): two connections on the same file; a
    // commit on B changes the value A reads — the basis the watcher polls on.
    let tmp = workspace_dir();
    let ws = workspace::discover(tmp.path()).unwrap();
    let reader = ws.open_store().unwrap();
    let mut writer = ws.open_store().unwrap();

    let before = reader.data_version().unwrap();
    writer.create_item("ab12.0001", "task", "A", "t");
    let after = reader.data_version().unwrap();
    assert_ne!(
        before, after,
        "a foreign commit bumps the reader's data_version"
    );
}

#[test]
fn external_write_fires_a_change_event() {
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx = engine.subscribe().unwrap();

    foreign_write(tmp.path(), "ab12.0001");

    let change = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a Change event after the external write");
    assert!(change.data_version > 0, "carries the new data_version");

    // And the same handle now reads the new item — reactivity + live read together.
    assert_eq!(engine.list(None, None).unwrap().len(), 1);
}

#[test]
fn no_change_event_across_several_completed_poll_cycles() {
    // The real anti-spurious property: polls actually RUN (the window is ~10× the interval) and,
    // seeing data_version unchanged, send nothing. Not merely "quiet before the first poll"
    // (#9t7.3 review, Test Quality #3).
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx = engine.subscribe().unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(400)),
        Err(RecvTimeoutError::Timeout),
        "no event when nobody writes, across ~10 completed poll cycles"
    );
}

#[test]
fn fans_out_to_multiple_subscribers() {
    // The broadcast `Vec<Sender>` must reach every subscriber (Code Quality #4).
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx1 = engine.subscribe().unwrap();
    let rx2 = engine.subscribe().unwrap();

    foreign_write(tmp.path(), "ab12.0001");

    assert!(
        rx1.recv_timeout(Duration::from_secs(5)).is_ok(),
        "subscriber 1 notified"
    );
    assert!(
        rx2.recv_timeout(Duration::from_secs(5)).is_ok(),
        "subscriber 2 notified"
    );
}

#[test]
fn watcher_stops_when_engine_dropped() {
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx = engine.subscribe().unwrap();

    // Dropping the only engine tears the watcher down (its sender is released), so the channel
    // disconnects instead of hanging — proof the background thread is not a leaked daemon.
    drop(engine);
    match rx.recv_timeout(Duration::from_secs(2)) {
        Err(RecvTimeoutError::Disconnected) => {}
        other => panic!("expected disconnect after engine drop, got {other:?}"),
    }
}

#[test]
fn detects_same_path_db_delete_and_recreate() {
    // #i8o: a same-path delete+recreate of the workspace db (nxf init over an existing workspace,
    // an E4 sync reset, rm + recreate) must not silently deafen the watcher. The old poll
    // connection stays pinned to the now-unlinked inode and can keep answering data_version()
    // WITHOUT erroring — so the error-path reopen never fires. The watcher must instead detect the
    // file-identity change, re-arm on the new file, and keep delivering Changes.
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx = engine.subscribe().unwrap();

    // Baseline: the watcher is live and delivering on the ORIGINAL db.
    foreign_write(tmp.path(), "ab12.0001");
    rx.recv_timeout(Duration::from_secs(5))
        .expect("baseline change on the original db");
    drain(&rx);

    // Delete + recreate the db at the same path. The watcher's poll connection is now pinned to an
    // unlinked inode; without identity detection it never sees the new file again.
    recreate_db(tmp.path());

    // (1) The wholesale db swap is itself a "go re-read" signal — the watcher noticed the new file.
    rx.recv_timeout(Duration::from_secs(5))
        .expect("a Change after the db was recreated at the same path");

    // (2) The re-arm is BOUNDED, not a per-tick storm: once it reopens on the new file it records the
    // new identity, so the identity check must not keep firing. A recreate legitimately emits a few
    // Changes (the identity re-baseline + the new db's schema-init `data_version` settle deltas; the
    // contract is "coalesced, not counted", #9e9t) — so we drain those and require the stream to
    // QUIESCE. If the identity branch kept re-firing every tick the stream would never go quiet and
    // this fails loudly. The `quiet` gap spans many poll cycles, so a settle delta delayed under load
    // is NOT mistaken for the storm — which is what made the old bare 400ms window flaky.
    assert_quiesces(&rx, Duration::from_millis(400), Duration::from_secs(5));

    // (3) And post-recreate writes to the NEW db keep flowing — the watcher is genuinely re-armed,
    // not merely signalling the reset once.
    foreign_write(tmp.path(), "ab12.0002");
    rx.recv_timeout(Duration::from_secs(5))
        .expect("a Change for a post-recreate external write");
}

#[test]
fn change_event_survives_while_a_clone_remains() {
    // The watcher lives as long as ANY clone of the handle. Dropping one clone must not stop it.
    let tmp = workspace_dir();
    let engine = Engine::open_with_poll_interval(None, tmp.path(), FAST).unwrap();
    let rx = engine.subscribe().unwrap();
    let clone = engine.clone();
    drop(engine); // one clone remains → watcher stays alive

    foreign_write(tmp.path(), "ab12.0002");

    assert!(
        rx.recv_timeout(Duration::from_secs(5)).is_ok(),
        "watcher still delivers while a clone is alive"
    );
    drop(clone);
}
