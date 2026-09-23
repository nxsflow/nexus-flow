//! ACCEPTANCE of the working-tree lease epic (nxf 6j6v.dbwn, epic 6j6v.bqe0) — TWO REAL OS
//! PROCESSES, one workspace, one working copy.
//!
//! Every other proof this epic has is either a direct library call (`orchestration_trigger.rs`,
//! `orchestration_reply.rs`, `working_tree.rs`'s own unit tests) or a SEQUENCE of `nxc` invocations
//! that never overlap in time (`surface_cli.rs`). Both are worth having and neither is this: a lease
//! whose whole reason to exist is that "`nxc send` and `nxc reply` are separate PROCESSES against the
//! same file" (`working_tree.rs`'s module doc) is not accepted by a test that runs one process at a
//! time. The shape followed here is `crates/cli/tests/convergence_collision_e2e.rs` — this repo's
//! established way to make two genuinely concurrent processes collide on one workspace.
//!
//! **The barrier, and why it is a barrier rather than a hope.** Two `Command::spawn`s back to back
//! overlap only by luck: the first can be completely finished before the second has linked its
//! dynamic loader. So this test holds the workspace's own SQLite write lock (`BEGIN IMMEDIATE` from
//! the test process) across both spawns. Every racer's first act is a write, so neither CAN complete
//! while the lock is held — that is checked, not assumed ([`assert_still_running`]) — and both are
//! released by one event, the test's `COMMIT`. From that instant the two processes are inside the
//! same compare-and-swap window, and which of them wins is decided by
//! `ChatStore::acquire_working_tree`, never by this file.
//!
//! **Nothing here reads a variable the test set.** The evidence is what the runtime produced: the dry
//! worker's log (a queued trigger is one the worker never heard about), the `--json` receipts the two
//! racers printed, and the thread board `nxc threads list` renders afterwards. Which racer wins is
//! deliberately not pinned — a test that demanded a particular winner would be asserting its own
//! scheduling luck.
//!
//! `NXF_DETERMINISTIC_IDS` is deliberately NOT set, unlike every sibling suite: it fixes the replica
//! prefix and the minted-id counter, so two concurrent processes would mint the SAME thread id and
//! the two "different scopes" this epic is about would silently become one.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

/// The instant both racers are pinned to. Pinned rather than ambient so the lease's `expires` is a
/// known value and the reclaim test below can name an instant past it exactly.
const NOW: &str = "2026-08-12T09:00:00Z";
/// Inside `WORKING_TREE_LEASE_BOUND` ("2h") from [`NOW`] — a lease taken at `NOW` still stands here.
const WITHIN_THE_BOUND: &str = "2026-08-12T10:59:59Z";
/// One second past `NOW` + the bound. A lease taken at [`NOW`] is reclaimable from here on.
const PAST_THE_BOUND: &str = "2026-08-12T11:00:01Z";

// ---- harness ----------------------------------------------------------------------------------

/// A fresh chat workspace with the declarations all three tests share.
///
/// * `coder` / `solo` — personas that declare `working_tree: exclusive` (the `send --to <persona>`
///   entrance);
/// * `reviewer` — declares nothing about the working copy, exactly as every role written before this
///   epic does;
/// * channel `review` — one member, `working_tree: exclusive` (the `send --to <channel>` entrance:
///   the work is exclusive because of what it is PART of, not because of who does it);
/// * channel `coding` — TWO members, `working_tree: exclusive`: one claim area that owes two replies,
///   which is what the chain test needs.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("solo.yaml"),
        "handle: solo\njob_title: Second order\nsystem_prompt: You are solo.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("reviewer.yaml"),
        "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You are the reviewer.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: review\n  members: [reviewer]\n  working_tree: exclusive\n\
         - name: coding\n  members: [coder, reviewer]\n  working_tree: exclusive\n",
    )
    .unwrap();
    tmp
}

/// `target/<profile>/nxc` — the argv[0] symlink to the multicall `nxs` binary, resolved the same way
/// `nxs_test_support::profile_dir` does (from the running test binary, so it follows
/// `CARGO_TARGET_DIR`/`--release`/`--target` for free).
///
/// A raw [`std::process::Command`] rather than `nxs_test_support::cargo_bin`, which hands back an
/// `assert_cmd::Command`: that type runs a child to completion and hands back an `Output`, so it
/// cannot express "start this and do NOT wait for it" — which is the entire subject of this file. The
/// stale-binary gate that makes `cargo_bin` safe is called explicitly here so this suite is covered by
/// it exactly as every other black-box suite is (nexus-flow-0yyw).
///
/// **The existence check is not belt-and-braces.** `assert_multicall_binary_fresh` watches `nxs`
/// only — the `nxc` argv[0] symlink beside it is created by `crates/nxs/build.rs`, and nothing in
/// that gate looks at it. Resolving the path by hand therefore has one failure the gate cannot see:
/// the symlink is missing (a hand-cleaned `target/`, a profile that has never built `nxs`). Without
/// this check that surfaces four frames later as `Command::spawn` failing with a bare `No such file
/// or directory` under an `expect("start the persona racer")`, which names the racer and not the
/// cause. Here it names the cause and the fix.
fn nxc_binary() -> PathBuf {
    nxs_test_support::assert_multicall_binary_fresh();
    let exe = std::env::current_exe().expect("the test binary has a path");
    let profile = exe
        .ancestors()
        .nth(2)
        .expect("the test binary lives in target/<profile>/deps");
    let nxc = profile.join(format!("nxc{}", std::env::consts::EXE_SUFFIX));
    assert!(
        nxc.exists(),
        "the `nxc` multicall symlink is missing at {}.\n\n\
         `nxf`/`nxm`/`nxc` are argv[0] symlinks to the single `nxs` binary, created next to it by \
         `crates/nxs/build.rs` (nexus-flow-5jz.7). This suite resolves that path by hand, and the \
         stale-binary gate watches `nxs` itself — not the symlinks beside it — so a missing one \
         only shows up here.\n\n\
         Build the binary under test, then re-run:\n\n    cargo build -p nxs\n",
        nxc.display()
    );
    nxc
}

/// One `nxc` invocation by the human at the keyboard, at a stated instant.
///
/// **Both streams are piped, and the racers below are left unread for 250 ms while the write lock is
/// held — which is safe only because of how little they say.** A pipe holds 64 KiB on both Linux and
/// macOS before a writer blocks; an `nxc send --json` receipt is a few hundred bytes and its stderr
/// is empty or one `warning:` line. Nothing here can fill a pipe, so nothing can deadlock against
/// the barrier. That is a property of the OUTPUT, not of the code: a change that makes a racer
/// genuinely chatty (per-op tracing on stderr, a `--stream` follow) would wedge it silently, holding
/// the write lock until the test's own `assert_still_running` fires with a misleading message. If
/// that day comes, redirect the streams to files under `root` and read them after `wait`.
fn nxc(root: &Path, now: &str) -> Command {
    let mut c = Command::new(nxc_binary());
    c.current_dir(root)
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", root.join("dry.log"))
        // NEVER the host's real scheduler (nxf 6j6v.74c0). `nxs_test_support::cargo_bin` defaults
        // this for every other suite; this file deliberately does NOT go through it (see
        // [`nxc_binary`] — it needs a raw `Command` it can spawn without waiting), so the default
        // has to be repeated here. Without it these racers open channels with a declared
        // `timeout:` and bootstrap real one-shot launchd agents into the developer's own login
        // session, each due to fire against a `TempDir` that is gone by then.
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", root.join("timer.log"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Run an `nxc` invocation to completion and parse its `--json` receipt.
///
/// The exit code is deliberately NOT asserted: a `send --to` whose trigger produced warnings exits 1
/// while still printing the whole receipt on stdout (nxf 6j6v.hpv8), and this suite reads the receipt
/// either way. What IS asserted is that stdout carried one.
fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("nxc runs");
    receipt_of(&out)
}

/// The `--json` receipt on a finished process's stdout, with the whole invocation's output in the
/// panic message when there is none — a racer that died has to say why.
fn receipt_of(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "expected a --json receipt on stdout ({e})\nstatus: {}\nstdout: {stdout}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// Every trigger the dry worker was handed, as its own log lines.
///
/// Filtered on the `trigger ` prefix rather than counting lines: the record ends in `msg=<message>`
/// and a message may contain newlines, so a raw line count would over-count a multi-line prompt
/// (`channel_fanout.rs` filters for the same reason).
fn triggers(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger "))
        .map(str::to_string)
        .collect()
}

/// `nxc threads list --json` — the board a human reads, keyed by thread id.
fn board(root: &Path, now: &str) -> Vec<Value> {
    json_of(nxc(root, now).args(["--json", "threads", "list"]))
        .as_array()
        .expect("threads list --json is an array")
        .clone()
}

/// One thread's row on that board.
/// The thread the channel supervisor opened for `handle` below `channel_thread` (nxf 6j6v.pf6j) —
/// where a channel member is triggered and where it answers. Read through the real `nxc status`, so
/// the test learns it exactly the way an app would.
fn member_thread(root: &Path, channel_thread: &str, handle: &str) -> String {
    let report = json_of(nxc(root, NOW).args(["--json", "status", "--thread", channel_thread]));
    let want = serde_json::json!([format!("local/{handle}")]);
    report["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|op| op["threads"].as_array().cloned().unwrap_or_default())
        .find(|t| t["parent"].as_str() == Some(channel_thread) && t["expects"] == want)
        .and_then(|t| t["thread_id"].as_str().map(str::to_string))
        .unwrap_or_else(|| panic!("no member thread for {handle} under {channel_thread}: {report}"))
}

fn row(board: &[Value], thread_id: &str) -> Value {
    board
        .iter()
        .find(|b| b["thread_id"].as_str() == Some(thread_id))
        .unwrap_or_else(|| panic!("no board row for thread {thread_id} in {board:?}"))
        .clone()
}

/// Hold the workspace's SQLite write lock, so no `nxc` process can get past its first write.
///
/// This is the two-process barrier. `BEGIN IMMEDIATE` takes the writer lock up front (the same
/// behaviour, and the same reason, as `release_working_tree_and_take_next`'s own transaction), and
/// every racer's first act — persisting its message — needs that lock. A racer that arrives early
/// waits on it inside SQLite's `busy_timeout` (5s, set by `Substrate::open`) rather than failing, so
/// process-start jitter is absorbed instead of deciding the race.
///
/// The returned store must be kept alive until [`release_write_lock`]; dropping it rolls back.
fn take_write_lock(root: &Path) -> ChatStore {
    let store = Workspace::resolve(None, root)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    store
        .connection()
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("take the workspace write lock");
    store
}

/// Release the barrier — both racers become runnable on this one statement.
fn release_write_lock(store: &ChatStore) {
    store
        .connection()
        .execute_batch("COMMIT;")
        .expect("release the workspace write lock");
}

/// The barrier's own assertion: this racer has not finished, so it is still ahead of the collision.
///
/// It cannot have finished — the write lock is held — so a `Some` here is a racer that died on its
/// way to the database, and reporting that as "the race happened" would be the exact green-for-nothing
/// this ticket exists to refuse.
fn assert_still_running(child: &mut Child, who: &str) {
    if let Some(status) = child.try_wait().expect("poll the racer") {
        panic!(
            "{who} exited ({status}) while the workspace write lock was still held — it never \
             reached the working-tree lease, so nothing raced"
        );
    }
}

// ---- 1: two real processes, both entrances, one working copy ----------------------------------

#[test]
fn two_real_processes_contend_for_one_working_tree_and_exactly_one_runs() {
    // The epic's own acceptance, first bullet: two processes at once against one workspace, each
    // triggering an `exclusive` role in a DIFFERENT claim area, and demonstrably exactly one runs
    // while the other stands in the queue. Third bullet too, in the same collision: the two racers
    // are the two entrances — `send --to <persona>` and `send --to <channel>` — competing for one
    // lease, not two invocations of the same one.
    let tmp = workspace();
    let root = tmp.path();

    let lock = take_write_lock(root);

    let mut persona = nxc(root, NOW)
        .args(["--json", "send", "--to", "coder", "build the website"])
        .spawn()
        .expect("start the persona racer");
    let mut channel = nxc(root, NOW)
        .args(["--json", "send", "--to", "review", "review the website"])
        .spawn()
        .expect("start the channel racer");

    // Both are alive and neither can be past its first write. This is the collision window: from the
    // COMMIT below, the two processes are inside one compare-and-swap.
    std::thread::sleep(Duration::from_millis(250));
    assert_still_running(&mut persona, "the persona racer");
    assert_still_running(&mut channel, "the channel racer");

    release_write_lock(&lock);

    let persona = persona
        .wait_with_output()
        .expect("the persona racer finishes");
    let channel = channel
        .wait_with_output()
        .expect("the channel racer finishes");
    let persona = receipt_of(&persona);
    let channel = receipt_of(&channel);

    let persona_thread = persona["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let channel_thread = channel["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_ne!(
        persona_thread, channel_thread,
        "the two racers opened genuinely different claim areas: {persona} / {channel}"
    );

    // (a) The runtime's own record of what was STARTED: exactly one trigger reached the worker. This
    //     is the load-bearing half — a queued trigger is one the worker never heard about.
    let started = triggers(root);
    assert_eq!(
        started.len(),
        1,
        "one working copy, one running session — got:\n{}",
        started.join("\n")
    );

    // (b) The board afterwards agrees, and says which is which: one thread holding, one waiting at
    //     position 1. Read back off `nxc threads list`, not off anything this test remembered.
    let board = board(root, NOW);
    let persona_row = row(&board, &persona_thread);
    let channel_row = row(&board, &channel_thread);
    let states: Vec<Option<&str>> = vec![
        persona_row["working_tree"].as_str(),
        channel_row["working_tree"].as_str(),
    ];
    assert!(
        states.contains(&Some("holding")) && states.contains(&Some("waiting")),
        "exactly one of the two chains holds the working copy and the other waits for it — got \
         persona: {persona_row}, channel: {channel_row}"
    );

    // (c) And the winner named by the board is the one the worker actually started.
    let (winner_role, winner_thread, loser_row) = if persona_row["working_tree"] == "holding" {
        ("role=coder", &persona_thread, &channel_row)
    } else {
        ("role=reviewer", &channel_thread, &persona_row)
    };
    assert!(
        started[0].contains(winner_role),
        "the session the worker started is the chain the board calls the holder ({winner_thread}), \
         got: {}",
        started[0]
    );
    assert_eq!(
        loser_row["working_tree_queue_position"].as_i64(),
        Some(1),
        "and the other one is first in line, not lost: {loser_row}"
    );

    // (d) The persona entrance's own receipt is the third witness, and it is a typed one: when the
    //     persona lost, it says so in the two fields nxf 6j6v.303b added and names the holder by its
    //     scope key; when it won, it is byte-identical to a send that never met a lease at all.
    if persona_row["working_tree"] == "waiting" {
        assert_eq!(
            persona["queue_position"].as_i64(),
            Some(1),
            "the losing persona send reports its place in line: {persona}"
        );
        assert_eq!(
            persona["queued_behind"].as_str(),
            Some(format!("thread:{channel_thread}").as_str()),
            "and names the chain it is behind: {persona}"
        );
        assert_eq!(
            persona["spawned"].as_bool(),
            Some(false),
            "nothing is running for it: {persona}"
        );
    } else {
        assert!(persona.get("queue_position").is_none(), "{persona}");
        assert!(persona.get("queued_behind").is_none(), "{persona}");
        assert_eq!(persona["spawned"].as_bool(), Some(true), "{persona}");
    }
}

// ---- 2: the hard-death fallback ---------------------------------------------------------------

#[test]
fn a_hard_killed_holder_loses_the_lease_past_the_bound_but_the_queue_does_not_self_drain() {
    // The epic's fourth bullet: a session killed outright — no teardown block, so nothing ever
    // discharges its expectation and nothing ever releases — must not wedge this device's working
    // copy forever. `WORKING_TREE_LEASE_BOUND` is the backstop, and the way a test reaches it is the
    // way this codebase threads time everywhere: `NXC_NOW`, the CLI's injected clock. No sleep.
    //
    // The dry worker IS the hard death: it records a trigger and starts nothing, so the coder's
    // session never exists, never replies, and never releases — exactly the state a `kill -9` leaves.
    //
    // AND the half that is easy to assume and is NOT true OF THIS SEQUENCE: nothing drains the queue
    // unless SOMEBODY COMES. Merely believing the time is later does nothing on its own, so the
    // rival parked behind a lease that expires does not start by MOVING THE CLOCK. That is asserted
    // here rather than glossed over, because a reader who assumes otherwise would file the real
    // behaviour as a bug.
    //
    // **It is no longer the whole truth about the SYSTEM**, and this case is deliberately the one
    // that leaves every event out. Three of them now hand the turn to a waiter, each with its own
    // test beside this one: a `nxc tick` past the bound sweeps (6j6v.fabb —
    // `a_tick_past_the_bound_drains_the_queue_the_expiry_alone_did_not`, and a parked commission
    // arms exactly that tick when it parks), an ordinary release does (6j6v.fe0f), and a NEWCOMER's
    // own acquire does rather than overtaking (6j6v.yd4w —
    // `at_expiry_the_chain_already_waiting_goes_before_the_newcomer_that_reclaims`). What stayed
    // true is what this case runs: nothing here ticks, releases, or asks.
    let tmp = workspace();
    let root = tmp.path();

    let holder = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let holder_thread = holder["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(holder["spawned"].as_bool(), Some(true), "{holder}");

    // A rival arrives while the lease still stands, and is parked.
    let parked = json_of(nxc(root, WITHIN_THE_BOUND).args([
        "--json",
        "send",
        "--to",
        "review",
        "review it",
    ]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(
        triggers(root).len(),
        1,
        "the rival is parked, not started, while the lease stands"
    );
    let within = board(root, WITHIN_THE_BOUND);
    assert_eq!(row(&within, &holder_thread)["working_tree"], "holding");
    assert_eq!(row(&within, &parked_thread)["working_tree"], "waiting");

    // Past the bound. Nothing has run in between — no sweeper, no daemon, no timer: the ONLY thing
    // that changed is what a later process believes the time to be.
    let after = board(root, PAST_THE_BOUND);
    assert_eq!(
        row(&after, &holder_thread)["working_tree"],
        Value::Null,
        "the dead chain's lease no longer holds anything once its bound has elapsed"
    );

    // And the honest half. The rival that was already parked is STILL parked: the expiry freed the
    // lease for whoever asks next, it did not hand the queue's turn out. Nothing has run, so its
    // position is unchanged.
    assert_eq!(
        triggers(root).len(),
        1,
        "moving the clock started nobody — got:\n{}",
        triggers(root).join("\n")
    );
    assert_eq!(
        row(&after, &parked_thread)["working_tree"],
        "waiting",
        "the parked rival did NOT start itself when the bound elapsed — a clock that merely moves \
         hands the queue's turn to nobody"
    );
    assert_eq!(
        row(&after, &parked_thread)["working_tree_queue_position"].as_i64(),
        Some(1),
        "and it kept its place in line"
    );
}

// ---- 2a: fairness at the bound (nxf 6j6v.yd4w) -------------------------------------------------

/// **A newcomer arriving at an expired lease does NOT overtake the chain that is already waiting.**
///
/// The scene the test above ends on, with the one event it leaves out: somebody comes. Until nxf
/// 6j6v.yd4w that newcomer took the working copy — [`ChatStore::acquire_working_tree`] reclaims an
/// expired lease in a single compare-and-swap that never looks at the queue — and the rival at
/// position 1, who had been waiting for exactly this moment since before the newcomer existed,
/// stayed at position 1. This test used to assert that behaviour, in as many words ("the reclaimer
/// holds the working copy now"), because it was the behaviour; the ticket calls it out as the one
/// place where waiting is punished by waiting.
///
/// Now the newcomer's own trigger hands the queue's turn out first and then competes for what is
/// left — so the WAITER starts and the newcomer is the one that queues.
#[test]
fn at_expiry_the_chain_already_waiting_goes_before_the_newcomer_that_reclaims() {
    let tmp = workspace();
    let root = tmp.path();

    let holder = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let holder_thread = holder["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let parked = json_of(nxc(root, WITHIN_THE_BOUND).args([
        "--json",
        "send",
        "--to",
        "review",
        "review it",
    ]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(
        triggers(root).len(),
        1,
        "the rival is parked, not started, while the lease stands"
    );

    // Past the bound, a THIRD chain arrives — the newcomer. No tick, no sweep, no daemon: its own
    // `send` is the only thing that runs.
    let out = nxc(root, PAST_THE_BOUND)
        .args(["--json", "send", "--to", "coder", "build it again"])
        .output()
        .expect("nxc runs");
    let newcomer = receipt_of(&out);
    // **This hand-off parks the dead chain's work first, and it still exits 0** (fix round 1 of nxf
    // 6j6v.8bv9). This workspace is not a git repository and the dry worker names no working copy,
    // so the park is refused PERMANENTLY and the copy goes on unparked, exactly as it always did —
    // with a named `work_handed_on_unparked` finding that did not exist before. `send --to` is the
    // one verb whose exit code follows its warnings (`changes_the_exit_code`), so without an
    // exception for the park classes every routine reclaim on a non-git workspace would exit 1,
    // about uncommitted work in a tree that does not exist.
    assert!(
        newcomer["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|w| w["class"] == "work_handed_on_unparked"),
        "the hand-off says the dead chain's work was not parked: {newcomer}"
    );
    assert!(
        out.status.success(),
        "the send did everything it was asked — status {}, stderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let newcomer_thread = newcomer["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();

    // (a) The runtime's own record: exactly one more session started, and it is the one that was
    //     WAITING — not the one that just asked.
    let started = triggers(root);
    assert_eq!(
        started.len(),
        2,
        "one chain started on the newcomer's arrival — got:\n{}",
        started.join("\n")
    );
    assert!(
        started[1].contains("role=reviewer") && started[1].contains("msg=review it"),
        "and it is the rival that had been waiting, re-composed from its own parked entry and \
         carrying the order as it was given — not the newcomer, whose role is `coder`:\n{}",
        started[1]
    );

    // (b) The newcomer's own receipt says it is the one standing in line now, behind the chain it
    //     just started.
    assert_eq!(
        newcomer["spawned"].as_bool(),
        Some(false),
        "the newcomer did not overtake: {newcomer}"
    );
    assert_eq!(
        newcomer["queued_behind"].as_str(),
        Some(format!("thread:{parked_thread}").as_str()),
        "…and it names the chain it is behind — the one that was ahead of it all along: {newcomer}"
    );
    assert_eq!(newcomer["queue_position"].as_i64(), Some(1), "{newcomer}");

    // (c) The board agrees, on all three chains at once.
    let after = board(root, PAST_THE_BOUND);
    assert_eq!(
        row(&after, &parked_thread)["working_tree"],
        "holding",
        "the chain that waited holds the working copy"
    );
    assert_eq!(
        row(&after, &newcomer_thread)["working_tree"],
        "waiting",
        "the newcomer waits its turn"
    );
    assert_eq!(
        row(&after, &holder_thread)["working_tree"],
        Value::Null,
        "and the dead chain holds nothing"
    );
}

// ---- 2b: the way out, and the drain (nxf 6j6v.fabb) --------------------------------------------

/// The scene two tests above ends on, plus the one call it leaves out — the OTHER one, since
/// `at_expiry_the_chain_already_waiting_goes_before_the_newcomer_that_reclaims` covers the arrival
/// of a rival and this covers the arrival of nobody at all.
///
/// The bound freed the lease and left the rival parked, "until some later chain releases". Nothing
/// was going to: the queue was drained only by a release, nothing swept the lease table, and
/// nothing polled the queue. `nxc tick` sweeps now — and a commission that parks arms one for its
/// holder's own expiry, so the poll exists even when nobody else comes.
#[test]
fn a_tick_past_the_bound_drains_the_queue_the_expiry_alone_did_not() {
    let tmp = workspace();
    let root = tmp.path();

    let holder = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let holder_thread = holder["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let parked = json_of(nxc(root, WITHIN_THE_BOUND).args([
        "--json",
        "send",
        "--to",
        "review",
        "review it",
    ]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(triggers(root).len(), 1, "the rival is parked, not started");

    // Arming is not a hope, and BOTH of its values are the claim (review of this branch, Test
    // Quality #2). The instant must be the HOLDER's expiry off the lease row — `NOW` + the two-hour
    // bound — and not what this trigger's own lease would have run to; the key must be the parked
    // commission's own thread, which is what makes a re-park replace rather than stack. A
    // substring check on `schedule thread=` would have let either swap through.
    let armed = std::fs::read_to_string(root.join("timer.log")).unwrap_or_default();
    // Keyed on the commission's OWN thread — the reviewer's slot below the board, not the board the
    // caller was handed. That is what `entry.thread` is, and what makes a re-park replace the
    // pending job instead of stacking a second one beside it.
    let parked_slot = member_thread(root, &parked_thread, "reviewer");
    assert!(
        armed.contains(&format!(
            "schedule thread={parked_slot} deadline=2026-08-12T11:00:00Z"
        )),
        "the drain is armed for the HOLDER's bound, keyed on the parked commission — timer log:\n\
         {armed}"
    );
    assert!(
        armed.contains(&format!("command=nxc tick --thread {parked_slot}")),
        "…and what it runs is the tick that sweeps — timer log:\n{armed}"
    );

    // The tick. Keyed on the PARKED commission's own thread, which is what the armed job runs — and
    // the sweep is deliberately not about that thread at all: it is about the device.
    let out = nxc(root, PAST_THE_BOUND)
        .args(["--json", "tick", "--thread", &parked_thread])
        .output()
        .expect("nxc runs");
    let ticked = receipt_of(&out);
    // **The drain reports the park it could not do, and still exits 0** (fix round 1 of nxf
    // 6j6v.8bv9). This workspace is not a git repository and its worker names no working copy, so
    // since 6j6v.8bv9 every hand-off here reports `work_handed_on_unparked` — a PERMANENT refusal,
    // so the copy goes on exactly as it always did. `tick` exits 0 by construction (see
    // `cli::print_tick_result`'s own doc); what is pinned here is that the finding is on the
    // receipt and that the verb did not grow an exit-code branch for it. The rule itself is pinned
    // on `send --to`, which is the one verb whose exit code follows its warnings — see
    // `at_expiry_the_chain_already_waiting_goes_before_the_newcomer_that_reclaims`.
    assert!(
        ticked["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|w| w["class"] == "work_handed_on_unparked"),
        "the finding is on the receipt: {ticked}"
    );
    assert!(
        out.status.success(),
        "the drain did everything it was asked — status {}, stderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let swept = &ticked["working_tree"];
    assert_eq!(
        swept["scope"].as_str(),
        Some(format!("thread:{holder_thread}").as_str()),
        "the tick names the expired lease it reclaimed: {ticked}"
    );
    assert_eq!(
        swept["started"].as_array().map(Vec::len),
        Some(1),
        "and the commission that had been waiting is started: {ticked}"
    );

    // The board agrees, and the rival is not merely un-queued — it is HOLDING and RUNNING.
    let after = board(root, PAST_THE_BOUND);
    assert_eq!(row(&after, &parked_thread)["working_tree"], "holding");
    assert_eq!(
        row(&after, &holder_thread)["working_tree"],
        Value::Null,
        "the dead chain holds nothing"
    );
    assert_eq!(
        triggers(root).len(),
        2,
        "the rival really started — got:\n{}",
        triggers(root).join("\n")
    );
}

/// **A tick INSIDE the bound sweeps nothing** — the direction that would be a disaster to get
/// wrong, since it is the collision the whole epic exists to prevent.
#[test]
fn a_tick_within_the_bound_leaves_a_live_lease_alone() {
    let tmp = workspace();
    let root = tmp.path();

    let holder = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let holder_thread = holder["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let parked = json_of(nxc(root, NOW).args(["--json", "send", "--to", "review", "review it"]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();

    let ticked =
        json_of(nxc(root, WITHIN_THE_BOUND).args(["--json", "tick", "--thread", &parked_thread]));
    assert!(
        ticked.get("working_tree").is_none(),
        "a lease still inside its bound is not swept, and the field is absent rather than empty: \
         {ticked}"
    );
    let after = board(root, WITHIN_THE_BOUND);
    assert_eq!(row(&after, &holder_thread)["working_tree"], "holding");
    assert_eq!(row(&after, &parked_thread)["working_tree"], "waiting");
    assert_eq!(triggers(root).len(), 1, "nothing new started");
}

/// **A decline RE-ARMS**, or the alarm is one-shot and the queue is stranded the moment anything
/// consumes it early.
///
/// The tick a park schedules fires once. A holder rides its bound forward every time it triggers
/// (`acquire_working_tree` renews `expires` on an inherit), so the armed job can fire while the
/// lease is still standing — and a decline that scheduled nothing would put the parked commission
/// back to "waits until some later chain releases", which is the bug 6j6v.fabb exists to close.
/// Found by the review of this branch (Code Quality #2).
///
/// **And each look is a MINUTE out, not the lease's two hours** (nxf 6j6v.xb24). This used to count
/// two schedules at one instant — the holder's bound, which both the park and the decline named,
/// because nothing reachable from the command line renews a lease. Since a holder can DIE inside
/// its bound, a wait sent away until the bound would find out two hours late, so every look behind a
/// claim now comes back on the liveness cadence. That makes the two schedules DISTINCT, which is a
/// stronger assertion than a count and is what this now makes: the park's look, and the decline's
/// look one minute after the tick that declined.
#[test]
fn a_tick_that_finds_the_lease_still_standing_arms_the_drain_again() {
    let tmp = workspace();
    let root = tmp.path();

    json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let parked = json_of(nxc(root, NOW).args(["--json", "send", "--to", "review", "review it"]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let parked_slot = member_thread(root, &parked_thread, "reviewer");

    let armed = |expect: &[&str]| {
        let log = std::fs::read_to_string(root.join("timer.log")).unwrap_or_default();
        let got: Vec<String> = log
            .lines()
            .filter_map(|l| {
                l.strip_prefix(&format!("schedule thread={parked_slot} deadline="))
                    .and_then(|rest| rest.split(' ').next())
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(got, expect, "timer log:\n{log}");
    };
    // The park's own: one liveness re-check after the commission was parked.
    armed(&["2026-08-12T09:01:00Z"]);

    let ticked =
        json_of(nxc(root, WITHIN_THE_BOUND).args(["--json", "tick", "--thread", &parked_slot]));
    assert!(
        ticked.get("working_tree").is_none(),
        "a lease still inside its bound is not swept: {ticked}"
    );
    // …and the decline's, which is the EARLIEST of the ways this wait can end. The tick fires one
    // second before the bound, so here the bound itself is sooner than a cadence a minute out and
    // wins — a look is never pushed LATER by the new rule, only ever earlier. The park's own
    // schedule above, two hours before the same bound, is the assertion that it is earlier.
    armed(&["2026-08-12T09:01:00Z", "2026-08-12T11:00:00Z"]);
}

/// **A holder that is past its bound but STILL RUNNING keeps the working copy** — the guard the
/// review of this branch found missing (Code Quality #1, Integrity #2), and the one direction that
/// would be a disaster to get wrong.
///
/// `WORKING_TREE_LEASE_BOUND`'s own doc names the case: a single step that WORKS for more than two
/// hours without commissioning anything loses the lease under itself. Before the drain existed that
/// stayed theoretical for a parked rival, because nothing polled; arming a tick for exactly that
/// instant would otherwise have turned an unlikely collision into a scheduled one.
///
/// Driven live, because the guard asks a real `SidecarWorker` a real question: a pid file under
/// `.nxs/agent-logs/` holding the pid of a process that is genuinely alive.
#[test]
fn a_tick_past_the_bound_does_not_take_the_copy_from_a_chain_that_is_still_running() {
    let tmp = workspace();
    let root = tmp.path();

    let holder = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "build it"]));
    let holder_thread = holder["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let holder_session = holder["session"].as_str().expect("a session").to_string();
    let parked = json_of(nxc(root, WITHIN_THE_BOUND).args([
        "--json",
        "send",
        "--to",
        "review",
        "review it",
    ]));
    let parked_thread = parked["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();

    // The dry worker records and starts nothing, so it leaves no pid file — this is what a LIVE
    // holder looks like to `SidecarWorker::session_is_running`: its own claim file, holding the pid
    // of a process that answers `kill(pid, 0)`. `sleep` is that process, and it is reaped below.
    let logs = root.join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    let mut alive = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a real process to be alive");
    std::fs::write(
        logs.join(format!("{holder_session}.pid")),
        alive.id().to_string(),
    )
    .unwrap();

    let ticked = json_of(
        nxc(root, PAST_THE_BOUND)
            .env("NXC_WORKER", "sidecar")
            .args(["--json", "tick", "--thread", &parked_thread]),
    );
    let _ = alive.kill();
    let _ = alive.wait();

    assert!(
        ticked.get("working_tree").is_none(),
        "a chain that is still running keeps the working copy, bound or no bound: {ticked}"
    );
    let after = board(root, PAST_THE_BOUND);
    assert_eq!(
        row(&after, &parked_thread)["working_tree"],
        "waiting",
        "and the rival is still waiting rather than started into a live checkout"
    );
    assert_eq!(
        triggers(root).len(),
        1,
        "nothing new was started — got:\n{}",
        triggers(root).join("\n")
    );
    // The holder's own row reads free (its bound HAS elapsed); what did not happen is the handover.
    assert_eq!(row(&after, &holder_thread)["working_tree"], Value::Null);
}

// `release_hands_the_working_copy_on_when_the_chain_holding_it_is_over` and
// `release_says_in_words_what_it_released_and_what_it_started` were here — two real-process
// exercises of `nxc release --thread <id>`: naming a thread outside the holding chain refused by
// name, the real release starting whoever was waiting, idempotency on a second call, and the
// narrated STDOUT text for both the within-bound and past-the-bound halves. REMOVED with the verb
// (nxf 6j6v.b9nf): the subject of both tests IS `nxc release`, which no longer exists on the CLI —
// `nxc release --thread <id>` now fails at clap's own unknown-subcommand parsing before ever
// reaching a binary this file could assert JSON or narrative text against. The real STOPPING of a
// live process, and `nxc withdraw`'s own narrated STDOUT text, are
// `crates/chat/tests/withdraw_a_running_round.rs`'s
// `nxc_withdraw_on_a_running_round_stops_the_session_and_says_the_work_will_be_parked` and
// `nxc_withdraw_says_in_words_what_it_stopped_and_what_becomes_of_the_work` to hold — a REAL
// `sleep` dies of a REAL `SIGTERM`, and the receipt and stdout are read off the real binary. What
// NEITHER ONE holds is the TWO-PROCESS hand-off this file is named for (fix round 3 of nxf
// 6j6v.b9nf's review, Test Quality #9: the earlier wording here overstated this). Both stop at the
// receipt of the ONE `nxc withdraw` call — no rival process, no `nxc tick` as a second invocation,
// no second reader of the marker or the parked branch. A dead chain's working copy going on
// WITHOUT a person naming a thread at all is the sweep's, covered by
// `crates/chat/tests/a_way_out_of_a_held_lease.rs` and `a_holder_past_its_bound_is_parked.rs`. Not
// this file's to re-invent for a verb that is gone — and the genuine two-process `nxc withdraw`
// hand-off (a rival process, a second `nxc tick`, a second reader of the marker) remains unwritten.

/// **`nxc release` is gone from the CLI** (nxf 6j6v.b9nf) — the real binary, the real `clap` parser,
/// no stand-in. `--help` rather than `--thread <id>` on purpose: this is not about a refusal this
/// crate writes, it is about a subcommand `clap` itself no longer knows, so the failure has to come
/// from the parser before this binary's own code runs at all — the same shape
/// `consolidated_entrances.rs`'s `refused` helper pins for every entrance a different item removed.
#[test]
fn the_release_verb_is_gone() {
    let tmp = workspace();
    let out = nxc(tmp.path(), NOW)
        .args(["release", "--help"])
        .output()
        .expect("nxc runs");
    assert!(
        !out.status.success(),
        "`nxc release --help` must fail: the verb left the surface"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("unrecognized subcommand"),
        "clap's own unknown-subcommand error, not a refusal this crate wrote: {stderr}"
    );
    assert!(
        stderr.contains("release"),
        "names the subcommand it did not recognize: {stderr}"
    );
}

// ---- 3: the chain, at process level -----------------------------------------------------------

#[test]
fn a_second_order_stays_queued_until_chain_one_discharges_its_last_expectation() {
    // The epic's second bullet at PROCESS level: a second order arrives while chain one is mid-flight
    // and does not start; it starts when — and only when — chain one has discharged its LAST
    // outstanding expectation, not its first.
    //
    // Chain one is a `coding` board that owes two replies (coder and reviewer). That is the shape the
    // gap between "the coder is done" and "the review is commissioned" has in this runtime: one claim
    // area, more than one obligation outstanding in it. The first reply discharges one of the two and
    // MUST NOT release — a session-scoped lock would have released right there, which is the mistake
    // the epic's second correction exists to prevent.
    //
    // Every step here is its own `nxc` process, exactly as the sidecar drives them.
    let tmp = workspace();
    let root = tmp.path();

    // Chain one. One board, two members, one claim area: the first member acquires the lease and the
    // second INHERITS it, so both start.
    let chain = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coding", "ship the epic"]));
    let chain_thread = chain["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(chain["target"].as_str(), Some("channel"), "{chain}");
    assert_eq!(
        triggers(root).len(),
        2,
        "both members of ONE claim area run — the second inherits, it does not queue"
    );

    // The second order, while chain one is very much alive.
    let second = json_of(nxc(root, NOW).args(["--json", "send", "--to", "solo", "the next job"]));
    let second_thread = second["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let second_session = second["session"]
        .as_str()
        .expect("a minted session")
        .to_string();
    assert_eq!(
        second["queued_behind"].as_str(),
        Some(format!("thread:{chain_thread}").as_str()),
        "the second order waits on chain one: {second}"
    );
    assert_eq!(second["queue_position"].as_i64(), Some(1), "{second}");
    assert_eq!(
        second["spawned"].as_bool(),
        Some(false),
        "and nothing is running for it: {second}"
    );
    assert_eq!(triggers(root).len(), 2, "the second order did not start");

    // The coder finishes. Its own `nxc reply --thread` — the exact command its prime block tells it to
    // answer with — discharges the FIRST of chain one's two expectations.
    let coder_thread = member_thread(root, &chain_thread, "coder");
    let coder_done = json_of(nxc(root, NOW).env("NXC_ACTOR", "coder").args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "implemented",
    ]));
    assert_eq!(coder_done["posted"].as_bool(), Some(true), "{coder_done}");

    // THE POINT OF THIS TEST. Chain one still owes the review, so it still holds the working copy,
    // and the second order has still not started.
    assert_eq!(
        triggers(root).len(),
        2,
        "a reply that discharges only the FIRST of two expectations must not release the working copy"
    );
    let mid = board(root, NOW);
    assert_eq!(
        row(&mid, &chain_thread)["working_tree"],
        "holding",
        "chain one still holds it between 'the coder is done' and 'the review is in'"
    );
    assert_eq!(
        row(&mid, &second_thread)["working_tree"],
        "waiting",
        "so the second order is still standing in line: {mid:?}"
    );

    // The reviewer answers: chain one's LAST outstanding expectation is discharged.
    let reviewer_thread = member_thread(root, &chain_thread, "reviewer");
    let review_done = json_of(nxc(root, NOW).env("NXC_ACTOR", "reviewer").args([
        "--json",
        "reply",
        "--thread",
        &reviewer_thread,
        "approved",
    ]));
    assert_eq!(review_done["posted"].as_bool(), Some(true), "{review_done}");

    // Only now does the second order run — on the reply itself, with nothing polling and no daemon.
    let started = triggers(root);
    assert_eq!(
        started.len(),
        3,
        "the reply that discharged the LAST expectation released the working copy and started the \
         chain waiting for it — got:\n{}",
        started.join("\n")
    );
    assert!(
        started[2].contains(&format!("role=solo session={second_session}")),
        "and it is the SAME session the queued receipt already named, re-composed from the entry's \
         own raw inputs — not a second one:\n{}",
        started[2]
    );
    assert!(
        started[2].contains("msg=the next job"),
        "carrying the order as it was given:\n{}",
        started[2]
    );

    let after = board(root, NOW);
    assert_eq!(
        row(&after, &second_thread)["working_tree"],
        "holding",
        "the second order has the working copy now"
    );
    assert_eq!(
        row(&after, &second_thread)["working_tree_queue_position"],
        Value::Null,
        "and is nobody's queue entry any more"
    );
}

// ---- 3b: two replies at once into ONE claim area (nxf 6j6v.jzaj) -------------------------------

/// **Two concurrent replies that both complete the same claim area promote the waiting chain
/// EXACTLY ONCE** — the probe nxf 6j6v.jzaj asked for, and its answer.
///
/// The finding that item carries (independent review of PR #339, finding 2, Medium) was explicitly
/// NOT a demonstrated bug: "erst zeigen, ob es etwas zu reparieren gibt". Its worry is real as
/// stated — the order *release, take, fire* is one transaction WITHIN a process, and two `nxc reply`
/// processes can both walk into `release_working_tree_if_scope_is_done` for the same area, where
/// what the second sees depends on how far the first has got. Neither the review nor this session
/// could construct a mis-interleaving, and this test is why: whichever order the two land in, the
/// outcome is the same one.
///
/// The two shapes the barrier can produce, and both are exercised by running it — which one this
/// run got is the scheduler's business, not this test's:
///
/// * **serialised** — the first reply's release check runs before the second's message is
///   persisted, sees the second member still owing, and does not release; the second then discharges
///   the last expectation and releases. One promotion, by the ordinary path.
/// * **overlapped** — both messages are persisted before either check runs, so BOTH replies see
///   "nothing outstanding" and both call the release. One promotion still, and this is the case the
///   finding is about: the release is a DELETE guarded on the holder's own `scope_key`, inside the
///   transaction that also takes the queue. The first deletes the row and hands the copy to the
///   promoted area in the same breath; the second's delete matches nothing, so it takes nothing —
///   `working_tree::tests::a_second_release_of_the_same_scope_takes_nothing` pins that mechanism on
///   its own.
///
/// So the assurance is: a queued claim area cannot be started twice, and cannot be swallowed,
/// however two replies into one area interleave. What that does NOT cover is named on
/// `release_working_tree_if_scope_is_done` rather than left here.
#[test]
fn two_concurrent_replies_completing_one_claim_area_promote_the_queue_exactly_once() {
    let tmp = workspace();
    let root = tmp.path();

    // Chain one: one board, two members, two obligations in ONE claim area.
    let chain = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coding", "ship the epic"]));
    let chain_thread = chain["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(
        triggers(root).len(),
        2,
        "both members of one claim area run"
    );

    // The second order, parked behind it — the entry the two replies below race to promote.
    let second = json_of(nxc(root, NOW).args(["--json", "send", "--to", "solo", "the next job"]));
    let second_thread = second["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let second_session = second["session"]
        .as_str()
        .expect("a minted session")
        .to_string();
    assert_eq!(second["queue_position"].as_i64(), Some(1), "{second}");

    // A THIRD order behind it, and it is not decoration: with only one entry on the queue a release
    // that took the head WITHOUT having released anything would take an empty queue and look
    // identical to a correct one. With two areas parked, a second release that is not a no-op
    // promotes the one that was not its turn — and that is visible.
    let third = json_of(nxc(root, NOW).args(["--json", "send", "--to", "coder", "and then this"]));
    let third_thread = third["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(third["queue_position"].as_i64(), Some(2), "{third}");
    assert_eq!(triggers(root).len(), 2, "neither order started");

    let coder_thread = member_thread(root, &chain_thread, "coder");
    let reviewer_thread = member_thread(root, &chain_thread, "reviewer");

    // The barrier: both replies are held at their first write and become runnable on one COMMIT.
    let lock = take_write_lock(root);
    let mut coder = nxc(root, NOW)
        .env("NXC_ACTOR", "coder")
        .args(["--json", "reply", "--thread", &coder_thread, "implemented"])
        .spawn()
        .expect("start the coder's reply");
    let mut reviewer = nxc(root, NOW)
        .env("NXC_ACTOR", "reviewer")
        .args(["--json", "reply", "--thread", &reviewer_thread, "approved"])
        .spawn()
        .expect("start the reviewer's reply");

    std::thread::sleep(Duration::from_millis(250));
    assert_still_running(&mut coder, "the coder's reply");
    assert_still_running(&mut reviewer, "the reviewer's reply");

    release_write_lock(&lock);

    let coder = coder
        .wait_with_output()
        .expect("the coder's reply finishes");
    let reviewer = reviewer
        .wait_with_output()
        .expect("the reviewer's reply finishes");
    let coder = receipt_of(&coder);
    let reviewer = receipt_of(&reviewer);
    assert_eq!(
        coder["posted"].as_bool(),
        Some(true),
        "both replies really landed: {coder}"
    );
    assert_eq!(reviewer["posted"].as_bool(), Some(true), "{reviewer}");

    // THE CLAIM. Exactly one more session started: not two (the parked area promoted twice) and not
    // none (its entry taken by one release and dropped by the other).
    let started = triggers(root);
    assert_eq!(
        started.len(),
        3,
        "the queue's next claim area was started exactly once, however the two replies interleaved \
         — got:\n{}",
        started.join("\n")
    );
    assert!(
        started[2].contains(&format!("role=solo session={second_session}")),
        "and it is the session the queued receipt already named, not a second one:\n{}",
        started[2]
    );

    // The board afterwards: the copy moved, once, to the chain that was waiting.
    let after = board(root, NOW);
    assert_eq!(row(&after, &second_thread)["working_tree"], "holding");
    assert_eq!(
        row(&after, &second_thread)["working_tree_queue_position"],
        Value::Null,
        "and it is nobody's queue entry any more: {after:?}"
    );
    assert_eq!(
        row(&after, &chain_thread)["working_tree"],
        Value::Null,
        "the chain that finished holds nothing"
    );
    assert_eq!(
        row(&after, &third_thread)["working_tree"],
        "waiting",
        "and the order behind it kept its turn — the second release took nothing: {after:?}"
    );
    assert_eq!(
        row(&after, &third_thread)["working_tree_queue_position"].as_i64(),
        Some(1),
        "moving up one place, because the area ahead of it started: {after:?}"
    );
}

// ---- 4: the live run, against the REAL sidecar -------------------------------------------------
//
// The epic's fifth and last bullet: verified in a real run, not only in a test. Everything above
// runs `NXC_WORKER=dry` — zero API calls, no wall-clock — which is exactly right for a gate that has
// to run on every commit, and exactly not enough for an acceptance that says "two role sessions
// against the real sidecar, with the proof that the second waited".
//
// `#[ignore]`d, following `smoke_v3.rs`, this repo's established live-smoke pattern (that file's own
// module doc argues each environment choice in full): real Claude Agent SDK sessions, real
// subscription usage, tens of seconds of wall clock. Run it on its own:
//
//   cargo build -p nxs && \
//   cargo test -p nexus-chat --test working_tree_two_process_e2e -- \
//       --ignored --nocapture --test-threads=1 live_
//
// **The workspace is a fresh `TempDir`, never this repo's own working copy** — the same rule
// `smoke_v3.rs` states and for the same reason: these are real agent sessions with real tools, and
// pointing them at the checkout under test would let the proof damage the thing it is proving. It
// costs nothing in evidence: the lease is one row keyed on the workspace's own database, and the
// mechanism cannot tell a git checkout from any other directory.

/// The repository root, from this crate's manifest dir (`<root>/crates/chat`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/chat")
        .to_path_buf()
}

/// Refuse the live run up front when the environment cannot possibly support it.
///
/// **Every one of these failures is otherwise a TIMEOUT, not an error.** The live test's evidence is
/// a session that eventually answers, so its only way to notice "nothing was ever going to answer"
/// is to run out its 600-second bound — and it does that for a missing `claude`, an unreadable
/// sidecar and a stale binary alike, then reports the same "never reached working_tree=holding"
/// either way. Ten wasted minutes and a diagnosis that names the wrong thing. Each check below costs
/// microseconds and names its own cause.
///
/// `ANTHROPIC_API_KEY` is a hard refusal rather than a warning, following `smoke_v3.rs`: this
/// machine authenticates through an existing `claude` CLI session, and the SDK PREFERS an ambient
/// key over that. A placeholder or expired one left in the environment does not degrade the run, it
/// breaks every session in it — while looking exactly like a lease bug.
fn live_preflight() {
    let sidecar = repo_root().join("agent-sidecar/src/main.mjs");
    assert!(
        sidecar.is_file(),
        "live run: the sidecar entry point is missing at {} — this test cannot spawn a real session",
        sidecar.display()
    );
    // The same resolution the sidecar itself will do, done now: `SidecarWorker` records
    // `claudePath` into the spec, and a missing `claude` there is a session that never starts.
    let found = Command::new("sh")
        .args(["-c", "command -v claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        found,
        "live run: `claude` is not resolvable on PATH. The Claude Agent SDK needs the CLI, and it \
         must be authenticated (an existing `claude` login under ~/.claude/) — run `claude` once \
         interactively first, then re-run this test."
    );
    assert!(
        std::env::var_os("ANTHROPIC_API_KEY").is_none(),
        "live run: ANTHROPIC_API_KEY is set. The SDK prefers an ambient key over this machine's \
         `claude` CLI session, so a placeholder or expired one breaks every session in this run \
         while looking like a working-tree-lease failure. Unset it and re-run."
    );
    // Not a live-only concern, but the one place where a stale binary costs ten minutes instead of
    // one assertion: `nxc_binary` runs the freshness gate and the symlink check.
    let _ = nxc_binary();
}

/// One `nxc` invocation wired to the REAL sidecar, mirroring `smoke_v3.rs::cmd`: `target/debug` is
/// prepended to `PATH` so both this call and every NESTED `nxc` a spawned role makes resolve the
/// freshly built binary (`worker.rs::forwarded_real_env` forwards `PATH` verbatim into the sidecar),
/// and no `ANTHROPIC_API_KEY` is set — this machine authenticates through the ambient `claude` CLI
/// session, and a placeholder key would make the SDK prefer it over that.
///
/// No `NXC_NOW`: the lease's own bound is wall-clock, and a live run is the one place it must be.
fn nxc_live(root: &Path) -> Command {
    let path = format!(
        "{}:{}",
        repo_root().join("target/debug").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut c = Command::new(nxc_binary());
    c.current_dir(root)
        .env_remove("NXC_SESSION")
        .env_remove("NXC_NOW")
        .env("PATH", path)
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_WORKER", "sidecar")
        // LIVE in the dimension under test, and only that one. What these cases exercise for real
        // is the sidecar and the lease across two processes; the clock is not part of it, and the
        // real `launchd` backend (nxf 6j6v.74c0) would leave one one-shot agent per channel opened
        // in the runner's own login session, due to fire against a `TempDir` that is gone the
        // moment the case ends. The clock has its own live proof, in 6j6v.74c0.
        .env("NXC_TIMER", "dry")
        .env(
            "NXC_SIDECAR",
            repo_root().join("agent-sidecar/src/main.mjs"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Poll the live board until `thread` reports `working_tree == want`, or give up after `bound`.
/// Returns how long it took, so the test can report the real timings it observed.
fn await_working_tree(root: &Path, thread: &str, want: &str, bound: Duration) -> Duration {
    let start = std::time::Instant::now();
    loop {
        let out = nxc_live(root)
            .args(["--json", "threads", "list"])
            .output()
            .expect("nxc threads list runs");
        let boards = receipt_of(&out);
        let state = boards
            .as_array()
            .expect("an array")
            .iter()
            .find(|b| b["thread_id"].as_str() == Some(thread))
            .map(|b| b["working_tree"].clone())
            .unwrap_or(Value::Null);
        if state.as_str() == Some(want) {
            return start.elapsed();
        }
        if start.elapsed() > bound {
            panic!(
                "thread {thread} never reached working_tree={want} within {bound:?} (last: \
                 {state}); the live board was:\n{boards:#}"
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// The body of `handle`'s own reply in `thread`, waiting up to `bound` for it to land — and refusing
/// the sidecar's fallback text.
///
/// **This is what keeps the live run from proving nothing.** `agent-sidecar`'s teardown discharges
/// the thread's expectation UNCONDITIONALLY (`nxc reply --thread <id> --if-unanswered`, nxf
/// 6j6v.7e9d), including for a session that crashed before it ever reached the model — in which case
/// it posts `sidecar: this session ended without …` on the session's behalf. That discharge releases
/// the lease exactly as a real answer would, so a live test that only watched the lease move would go
/// green against a sidecar that never spoke to Claude at all. Reading the reply back and rejecting
/// that text is the difference between "the handover happened" and "the handover happened because
/// two real sessions ran".
fn await_real_reply(root: &Path, thread: &str, handle: &str, bound: Duration) -> String {
    let sender = format!("local/{handle}");
    let start = std::time::Instant::now();
    loop {
        let out = nxc_live(root)
            .args(["--json", "threads", "show", thread])
            .output()
            .expect("nxc threads show runs");
        let board = receipt_of(&out);
        let reply = board["messages"]
            .as_array()
            .expect("a board carries messages")
            .iter()
            .find(|m| m["sender"].as_str() == Some(sender.as_str()))
            .and_then(|m| m["body"].as_str())
            .map(str::to_string);
        if let Some(body) = reply {
            assert!(
                !body.starts_with("sidecar:"),
                "{handle}'s reply is the sidecar's own fallback for a session that never answered, \
                 not a live answer — the model was never reached: {body}"
            );
            return body;
        }
        if start.elapsed() > bound {
            panic!(
                "{sender} never answered in thread {thread} within {bound:?}; the live board was:\n\
                 {board:#}"
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[test]
#[ignore = "live: spawns two real Claude Agent SDK sessions through the real sidecar"]
fn live_a_second_role_session_waits_for_the_working_copy_and_starts_when_it_is_freed() {
    live_preflight();
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup(root, &chat_config()).expect("seed chat workspace");
    let roles = root.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    // Two personas that each declare they need the working copy to themselves, and whose whole job
    // is to answer. Deliberately trivial: what is under test is the LEASE, not the model's work, and
    // a session that does real work would only make the run longer and its evidence noisier.
    //
    // `tools: [Bash]` is declared here to keep the run NARROW, not to make it work: `Read`/`Write`
    // are deliberately absent, so this live run cannot touch a file even if the model decided to.
    //
    // It used to be load-bearing, and finding that out is what the first version of this test got
    // wrong. The sidecar mapped a role's `tools:` onto the SDK's `allowedTools` — its AUTO-APPROVE
    // list — so a role that declared none reached the model with an empty one, its `nxc reply` came
    // back `"This command requires approval"`, the session ended having answered nothing, and the
    // teardown's unconditional `--if-unanswered` posted on its behalf. The lease then changed hands
    // exactly as it should, which is why this looked green before [`await_real_reply`] existed.
    // nxf 6j6v.kffm closed that at the trigger: a session that is ORDERED to reply is granted what
    // running the reply takes, so the same declaration written without `tools:` would answer too.
    for handle in ["builder", "checker"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\njob_title: Live lease smoke\n\
                 system_prompt: |\n  \
                   You are a test persona in a live acceptance run. Read, write or change NO files.\n  \
                   Run exactly one command: the `nxc reply --thread <id>` you were asked for, with\n  \
                   the single word `ready` as its body. Then stop.\n\
                 tools: [Bash]\n\
                 working_tree: exclusive\n"
            ),
        )
        .unwrap();
    }

    // Chain one takes the working copy and a real session starts on it.
    let first = json_of(nxc_live(root).args(["--json", "send", "--to", "builder", "say ready"]));
    let first_thread = first["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    assert_eq!(
        first["spawned"].as_bool(),
        Some(true),
        "the first live session started: {first}"
    );
    eprintln!(
        "LIVE builder thread={first_thread} session={}",
        first["session"]
    );

    // Chain two arrives while it is running — and is told, in its own receipt, that it did not.
    let second = json_of(nxc_live(root).args(["--json", "send", "--to", "checker", "say ready"]));
    let second_thread = second["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let second_session = second["session"].as_str().expect("a session").to_string();
    assert_eq!(
        second["queued_behind"].as_str(),
        Some(format!("thread:{first_thread}").as_str()),
        "the second live session waits on the first: {second}"
    );
    assert_eq!(second["queue_position"].as_i64(), Some(1), "{second}");
    assert_eq!(
        second["spawned"].as_bool(),
        Some(false),
        "and nothing is running for it yet: {second}"
    );
    eprintln!("LIVE checker thread={second_thread} session={second_session} QUEUED at 1");

    // The evidence the receipt cannot give: the sidecar wrote a session log for the holder and none
    // for the one that waited. `SidecarWorker` creates `<ws>/.nxs/agent-logs/<session>.spec.json`
    // as it spawns, so its ABSENCE is proof no process was ever started for that session.
    let logs = root.join(".nxs/agent-logs");
    let queued_spec = logs.join(format!("{second_session}.spec.json"));
    assert!(
        !queued_spec.exists(),
        "the sidecar never spawned anything for the queued session — {} must not exist",
        queued_spec.display()
    );

    // Now wait for the real session to finish. Its own teardown discharges the expectation
    // unconditionally (`nxc reply --thread <id> --if-unanswered`, nxf 6j6v.7e9d), which is what
    // releases the lease and fires the trigger that has been parked all along.
    let waited = await_working_tree(root, &second_thread, "holding", Duration::from_secs(600));
    eprintln!("LIVE the queued chain got the working copy after {waited:?}");

    assert!(
        queued_spec.exists(),
        "and only THEN was a real session spawned for it — {} must exist now",
        queued_spec.display()
    );
    let after = board_live(root);
    assert_eq!(
        row(&after, &first_thread)["working_tree"],
        Value::Null,
        "the first chain has given the working copy up"
    );
    assert_eq!(
        row(&after, &second_thread)["working_tree_queue_position"],
        Value::Null,
        "and the second is nobody's queue entry any more"
    );

    // And BOTH sessions were real ones that reached the model — see [`await_real_reply`] for why
    // this is the assertion that makes the whole live run mean something.
    let built = await_real_reply(root, &first_thread, "builder", Duration::from_secs(60));
    eprintln!("LIVE builder answered: {built}");
    let checked = await_real_reply(root, &second_thread, "checker", Duration::from_secs(600));
    eprintln!("LIVE checker answered: {checked}");

    // The second session really did run only AFTER the first had finished — that is the whole claim,
    // and here it is one last time from the lease's own point of view: with both chains done, nobody
    // holds the working copy and nobody is queued for it.
    let done = board_live(root);
    for thread in [&first_thread, &second_thread] {
        assert_eq!(
            row(&done, thread)["working_tree"],
            Value::Null,
            "both live chains have finished and released: {done:?}"
        );
    }
}

/// `nxc threads list --json` against a live workspace (no pinned clock).
fn board_live(root: &Path) -> Vec<Value> {
    let out = nxc_live(root)
        .args(["--json", "threads", "list"])
        .output()
        .expect("nxc threads list runs");
    receipt_of(&out)
        .as_array()
        .expect("threads list --json is an array")
        .clone()
}
