//! ACCEPTANCE of nxf 6j6v.fc5p — the Lamport clock belongs to the REPLICA, not to the open store
//! handle. TWO REAL OS PROCESSES, one workspace file.
//!
//! The bug's own description says why it went unnoticed for the substrate's whole life: **tests run
//! in ONE process**, where a single handle observes every op and the divergence cannot form. A test
//! that shows the repair inside one process shows nothing — it is precisely the blind spot that
//! carried the defect. So both proofs here put a second, genuinely separate OS process on the same
//! `.nxs/db.sqlite`, which is the form this platform ships in: several agents in one workspace, a
//! long-lived `Engine` handle beside short-lived `nxf`/`nxc` subprocesses, WAL mode chosen so they
//! can.
//!
//! The two tests are the two shapes the defect takes:
//!
//! 1. [`a_long_lived_handle_does_not_lose_a_write_to_a_sibling_process`] — the CONSEQUENCE, and the
//!    counter-proof. A handle whose clock stopped at the moment it opened mints a coordinate a
//!    sibling process already used; `fold_lww`'s keep-if-beats is a STRICT `>`, so the equal
//!    coordinate loses and the value is dropped without a word. Deterministic — no barrier, no race,
//!    the sequence alone produces it — so this one is RED on the pre-fix binary every time, and it
//!    is red on the symptom a user would report: the board reading `"from the subprocess"`, the
//!    handle's write gone, exit 0 and no error anywhere on the way.
//! 2. [`two_concurrent_processes_never_mint_the_same_coordinate`] — the CONDITION, in the form the
//!    item's own correction names: no embedding host required, two ordinary concurrent `nxf` calls
//!    are enough. This is what test 1 cannot show — that the read-modify-write which mints a
//!    coordinate is atomic across PROCESSES, not merely across handles in one. **It is not a
//!    reliable counter-proof and does not claim to be**: pre-fix it reddens only when both racers
//!    happen to seed their clocks before either commits, and since `Store::open` itself takes the
//!    write lock (`migrate`'s `BEGIN IMMEDIATE`), the barrier cannot force that ordering. Observed
//!    both ways on the pre-fix binary. Post-fix it holds for every interleaving, which is exactly
//!    what makes it worth keeping.
//!
//! Nothing here reads a variable the test set: the evidence is the op log the two processes actually
//! wrote and what `nxf show --json` renders off it afterwards.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use nexus_flow_core::store::Store;
use nxs_foundation::workspace;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// The `nxf` multicall symlink, as a raw [`std::process::Command`] path.
///
/// `nxs_test_support::cargo_bin` hands back an `assert_cmd::Command`, which always runs a child to
/// completion — it cannot express "start this and do NOT wait for it", which is the whole subject of
/// the barrier test below. The stale-binary gate that makes `cargo_bin` safe is therefore called
/// explicitly, so this suite is covered by it like every other black-box suite (nexus-flow-0yyw),
/// and the symlink's existence — which that gate does not watch, it watches `nxs` — is checked here
/// so a hand-cleaned `target/` names its own cause. Mirrors
/// `crates/chat/tests/working_tree_two_process_e2e.rs`, the file this one is modelled on.
fn nxf_binary() -> PathBuf {
    nxs_test_support::assert_multicall_binary_fresh();
    let exe = std::env::current_exe().expect("the test binary has a path");
    let profile = exe
        .ancestors()
        .nth(2)
        .expect("the test binary lives in target/<profile>/deps");
    let nxf = profile.join(format!("nxf{}", std::env::consts::EXE_SUFFIX));
    assert!(
        nxf.exists(),
        "the `nxf` multicall symlink is missing at {}.\n\n\
         `nxf`/`nxm`/`nxc` are argv[0] symlinks to the single `nxs` binary, created next to it by \
         `crates/nxs/build.rs` (nexus-flow-5jz.7). This suite resolves that path by hand.\n\n\
         Build the binary under test, then re-run:\n\n    cargo build -p nxs\n",
        nxf.display()
    );
    nxf
}

/// One `nxf` invocation in `root`. Streams piped so a racer held at the barrier cannot scribble over
/// the test's own output; every command here prints a few hundred bytes at most, far inside a pipe's
/// 64 KiB, so nothing can block on an unread pipe while the write lock is held.
fn nxf(root: &Path) -> Command {
    let mut c = Command::new(nxf_binary());
    c.current_dir(root)
        // The isolation `nxs_test_support::cargo_bin` would have carried, said by hand because the
        // paragraph above explains why this suite cannot use it (nxf 6j6v.y12q, review of PR #421).
        // Since y12q `nxf init` registers the workspace with this machine's background service, so
        // without this the `init` below writes a soon-deleted `TempDir` path into the developer's
        // own `~/.nexusflow/workspaces.toml`.
        .pin_home(root.join(".fake-home"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Run an `nxf` invocation to completion, asserting it succeeded, and return its stdout.
fn run(root: &Path, args: &[&str]) -> String {
    let out = nxf(root).args(args).output().expect("nxf runs");
    assert!(
        out.status.success(),
        "nxf {args:?} failed ({})\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

/// A fresh workspace with one item, and that item's id — the cell both writers below fight over.
fn workspace_with_an_item() -> (TempDir, String) {
    let tmp = TempDir::new().unwrap();
    run(tmp.path(), &["init", "--plugin", "issue-tracker"]);
    let created: serde_json::Value = serde_json::from_str(&run(
        tmp.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "the contested item",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ],
    ))
    .expect("create --json");
    let id = created["id"].as_str().expect("an id").to_string();
    (tmp, id)
}

/// A store handle over `root`'s workspace, opened the way flow's own write path opens it — the same
/// `Store::open(db, replica.site_id)` an embedding app's long-lived handle uses.
fn open_handle(root: &Path) -> Store {
    let ws = workspace::discover(root).expect("the workspace this test just created");
    Store::open(&ws.db_path_str().unwrap(), ws.replica.site_id).expect("open the substrate")
}

/// The item's `title` as the product surface renders it — read through a FRESH `nxf show`
/// subprocess, so this only ever sees what actually landed on disk.
fn title_on_the_board(root: &Path, id: &str) -> String {
    let shown: serde_json::Value =
        serde_json::from_str(&run(root, &["show", id, "--json"])).expect("show --json");
    shown["item"]["title"]
        .as_str()
        .expect("a title")
        .to_string()
}

/// How many `(lamport, site)` coordinates in the log are held by more than one op. The invariant is
/// 0: monotonicity per site is not a race outcome, so the same number twice from one stamp is proof
/// that some writer minted from a stale clock.
fn coordinate_collisions(root: &Path) -> i64 {
    open_handle(root)
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM ops GROUP BY lamport, site HAVING COUNT(*) > 1)",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// Every value the log carries for one item's `title` cell, in the order the ops were appended.
fn title_ops(root: &Path, id: &str) -> Vec<String> {
    let handle = open_handle(root);
    let mut st = handle
        .connection()
        .prepare(
            "SELECT value FROM ops
              WHERE target_kind='item' AND target_id=?1 AND field='title'
              ORDER BY lamport, site",
        )
        .unwrap();
    let rows = st
        .query_map([id], |r| r.get::<_, Option<String>>(0))
        .unwrap()
        .map(|v| v.unwrap().unwrap_or_default())
        .collect();
    rows
}

#[test]
fn a_long_lived_handle_does_not_lose_a_write_to_a_sibling_process() {
    let (tmp, id) = workspace_with_an_item();
    let root = tmp.path();

    // The app seam: a handle opened once and held. Its clock is seeded here, and before 6j6v.fc5p
    // this was the ONLY time it ever looked at the file.
    let mut long_lived = open_handle(root);
    let clock_at_open = long_lived.clock();

    // A short-lived sibling process writes the same cell — an agent at a second terminal, or the
    // `nxc reply` subprocess a triggered session answers from.
    run(root, &["update", &id, "--set", "title=from the subprocess"]);
    assert_eq!(
        title_on_the_board(root, &id),
        "from the subprocess",
        "the sibling's write landed (the premise, not the subject)"
    );

    // The long-lived handle now writes the SAME cell. Pre-fix it minted `clock_at_open + 1` — the
    // coordinate the subprocess had already used — and keep-if-beats' strict `>` discarded it in
    // silence. Nothing reported a failure then either; that is the whole complaint.
    long_lived.set_field(
        &id,
        "title",
        Some("from the long-lived handle".into()),
        "tester",
    );
    let clock_after_write = long_lived.clock();
    drop(long_lived);

    // The consequence first, because it is the complaint: the write is THERE. Pre-fix the board
    // still read "from the subprocess" at this point, with no error anywhere on the way.
    assert_eq!(
        title_on_the_board(root, &id),
        "from the long-lived handle",
        "the long-lived handle's write is on the board — pre-fix it was accepted, folded, \
         outranked by an EQUAL coordinate, and silently dropped"
    );
    // Then the mechanism, so a future regression names its own cause instead of only its symptom.
    assert!(
        clock_after_write > clock_at_open + 1,
        "the handle minted above the sibling's ops, not on top of them \
         (clock {clock_after_write} vs {clock_at_open} at open)"
    );
    assert_eq!(
        title_ops(root, &id),
        [
            "the contested item",
            "from the subprocess",
            "from the long-lived handle"
        ],
        "and the log orders all three writes by when they were made"
    );
    assert_eq!(coordinate_collisions(root), 0);
}

/// The SIBLING FAILURE named in 6j6v.fc5p, checked here rather than left to the argument that it has
/// the same root: a write that lands not on an equal coordinate but strictly UNDER one the register
/// already holds. `fold_lww` discards that just as silently — it is the same strict `>`, one step
/// further along. In chat it reads "a late declaration falls under a reply that already answered
/// it"; in flow it is a plain edit that never appears.
///
/// It differs from the case above only in how far the sibling advanced the clock: one op produces
/// the equal coordinate, several produce the under-the-register one. The repair covers both for the
/// same reason — a local mint is taken from the log's high-water mark, so it is above EVERY op in
/// the file, not merely above the last one this handle knew about.
#[test]
fn a_write_made_after_a_sibling_moved_the_register_does_not_fall_under_it() {
    let (tmp, id) = workspace_with_an_item();
    let root = tmp.path();

    let mut long_lived = open_handle(root);
    let clock_at_open = long_lived.clock();

    // Several ops, not one: the sibling carries the register well past where this handle stopped.
    for round in 1..=3 {
        run(
            root,
            &[
                "update",
                &id,
                "--set",
                &format!("title=sibling round {round}"),
            ],
        );
    }
    let sibling_high_water: i64 = long_lived
        .connection()
        .query_row("SELECT MAX(lamport) FROM ops", [], |r| r.get(0))
        .unwrap();
    assert!(
        sibling_high_water > clock_at_open + 1,
        "the sibling moved the register beyond the next coordinate this handle would have minted"
    );

    long_lived.set_field(&id, "title", Some("the late write".into()), "tester");
    drop(long_lived);

    assert_eq!(
        title_on_the_board(root, &id),
        "the late write",
        "the later write wins — pre-fix it was minted UNDER the register the sibling had moved \
         and keep-if-beats dropped it in silence"
    );
    assert_eq!(coordinate_collisions(root), 0);
}

#[test]
fn two_concurrent_processes_never_mint_the_same_coordinate() {
    let (tmp, id) = workspace_with_an_item();
    let root = tmp.path();

    // The barrier. `BEGIN IMMEDIATE` from the test process takes the workspace's write lock, and a
    // local op's first act is now to take that same lock (it has to: the clock read that mints the
    // coordinate must sit inside the same transaction as the INSERT). Two bare `spawn`s would
    // overlap only by luck — the first can finish before the second has linked its loader — so both
    // racers are started, held here, and released by one event, this COMMIT.
    let lock = open_handle(root);
    lock.connection()
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("take the workspace write lock");

    let mut racers: Vec<Child> = ["from racer A", "from racer B"]
        .iter()
        .map(|title| {
            nxf(root)
                .args(["update", &id, "--set", &format!("title={title}")])
                .spawn()
                .expect("start a racer")
        })
        .collect();

    // Neither CAN have finished — the write lock is held. A racer that exited died on its way to the
    // database, and reporting that as "the race happened" would be green-for-nothing.
    std::thread::sleep(Duration::from_millis(250));
    for (i, racer) in racers.iter_mut().enumerate() {
        if let Some(status) = racer.try_wait().expect("poll the racer") {
            panic!(
                "racer {i} exited ({status}) while the workspace write lock was still held — \
                 it never reached the op log, so nothing raced"
            );
        }
    }

    lock.connection()
        .execute_batch("COMMIT;")
        .expect("release the workspace write lock");
    drop(lock);

    for (i, racer) in racers.into_iter().enumerate() {
        let out = racer.wait_with_output().expect("a racer finishes");
        assert!(
            out.status.success(),
            "racer {i} failed ({})\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Which racer won is scheduling luck and is deliberately not pinned. What is pinned is that the
    // log came out of the window intact: two distinct coordinates for two writes, both durable, and
    // the board showing one of them rather than a value neither process wrote.
    assert_eq!(
        coordinate_collisions(root),
        0,
        "two concurrent processes on one device share a site_id — before 6j6v.fc5p they both \
         seeded their clock at the same MAX(lamport) and minted the SAME coordinate"
    );
    let mut titles = title_ops(root, &id);
    titles.sort();
    assert_eq!(
        titles,
        ["from racer A", "from racer B", "the contested item"],
        "both racers' writes are in the log, neither lost"
    );
    let winner = title_on_the_board(root, &id);
    assert!(
        winner == "from racer A" || winner == "from racer B",
        "the board shows the later of the two, not a value that lost to an equal coordinate: \
         {winner}"
    );
}
