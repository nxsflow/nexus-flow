//! **A stranded session is noticed across REAL processes** (nxf 6j6v.ccvy, AC 2).
//!
//! `a_stranded_session_is_recorded_and_escalated.rs` drives the rule through the compute layer with
//! a worker that ANSWERS which sessions run. This file asks the operating system instead: every
//! step is a real `nxc` process against one workspace, the session's process is a real child the
//! test starts (`sleep`), its claim is the file the sidecar writes (`.nxs/agent-logs/<id>.pid`,
//! through [`nexus_chat::worker::session_claim_for`] — a bare pid reads as unverifiable and would
//! never strand), and the callbacks a sidecar makes — `session bind`, a transcript flush,
//! `session ended`, `session interrupted` — are made by real `nxc` processes with `NXC_SESSION` set.
//!
//! **Two workers, on purpose.** The commission runs under `NXC_WORKER=dry`, so nothing is spawned
//! and the test owns the session's process. The ticks run under `NXC_WORKER=sidecar`, so the sweep's
//! liveness question goes to the real pid probe. `NXC_NOW` is pinned on every process; nothing
//! sleeps — each step reads a named signal (a table row, the dry log, the timer log).
//!
//! **Since the ladder (nxf 6j6v.558h) these slots are COORDINATED** — the sidecar worker looks, stops
//! and asks — so what a look does about a session that will not answer is its coordinator's rung,
//! not cut 1a's record: the ended session is continued in the same session, the one that never
//! bound is started afresh. The sidecar the ticks start is a stub (`NXC_SIDECAR`) that writes the
//! spec it was handed to `stub.log` and exits, so what was started is read off that file. (Cut 1a's
//! record and escalation, which a slot nobody coordinates still gets, is pinned in
//! `a_stranded_session_is_recorded_and_escalated.rs`.)
//!
//! **This repository's own board is never touched**: every `nxc` here strips the `NXC_*` variables it
//! would otherwise inherit from the agent running the suite (an `NXC_DB` pointing at the live
//! workspace among them), and `$HOME` is pinned by `nxs_test_support::cargo_bin`.

#![cfg(unix)]

use std::path::Path;
use std::process::{Child, Command as StdCommand, Stdio};

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const T0: &str = "2026-10-18T10:00:00Z";

fn at(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

/// A workspace with one verifier, a one-step review channel with a two-hour window, and the same
/// channel claiming the working copy alone.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("verifier.yaml"),
        "handle: verifier\njob_title: Verifier\nsystem_prompt: You are the verifier.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Coder\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: review\n  members: [verifier]\n  flow: sequential\n  timeout: 2h\n\
         - name: guarded\n  members: [verifier]\n  flow: sequential\n  timeout: 2h\n  \
         working_tree: exclusive\n\
         - name: rival\n  members: [coder]\n  working_tree: exclusive\n",
    )
    .unwrap();
    // The sidecar a tick starts: it writes what it was handed and exits, as a session that dies at
    // once would — which is also what keeps a test from ever starting a real model session.
    std::fs::write(
        tmp.path().join("stub-sidecar.mjs"),
        format!(
            "import {{ appendFileSync, readFileSync }} from \"node:fs\";\n\
             const spec = JSON.parse(readFileSync(process.argv[process.argv.indexOf(\"--spec\") + 1], \"utf8\"));\n\
             appendFileSync({log:?}, JSON.stringify(spec) + \"\\n\");\n",
            log = tmp.path().join("stub.log").display().to_string()
        ),
    )
    .unwrap();
    tmp
}

/// Every spec the stub sidecar was handed, oldest first — once `n` of them are there (the stub is
/// spawned detached, so its line lands after the tick returns), bounded at ten seconds.
fn started_n(root: &Path, n: usize) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let got = started(root);
        if got.len() >= n || std::time::Instant::now() > deadline {
            return got;
        }
        std::thread::yield_now();
    }
}

/// Every spec the stub sidecar has finished writing, oldest first. Only newline-terminated lines
/// count: the newline is the last byte of each append, so a line without it is still being written
/// (nxf 6j6v.b53g).
fn started(root: &Path) -> Vec<Value> {
    let log = std::fs::read_to_string(root.join("stub.log")).unwrap_or_default();
    let finished = log.rfind('\n').map_or("", |end| &log[..end]);
    finished
        .lines()
        .map(|l| serde_json::from_str(l).expect("a spec"))
        .collect()
}

/// The ladder's record of the one slot here: (action, outcome).
fn rungs(root: &Path) -> Vec<(String, Option<String>)> {
    let s = store(root);
    let mut st = s
        .connection()
        .prepare("SELECT action, outcome FROM recovery_log ORDER BY id")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// One `nxc` process at `now`, under `worker`, with nothing inherited from the agent running this.
fn nxc(root: &Path, now: &str, worker: &str) -> Command {
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
        .env("NXC_WORKER", worker)
        .env("NXC_DRY_LOG", root.join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", root.join("timer.log"))
        .env("NXC_TIMER_LOOK_LOG", root.join("looks.log"))
        .env("NXC_SIDECAR", root.join("stub-sidecar.mjs"));
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).unwrap_or_else(|e| {
        panic!(
            "not a --json receipt ({e}): {}",
            String::from_utf8_lossy(out)
        )
    })
}

fn store(root: &Path) -> ChatStore {
    Workspace::resolve(None, root)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Commission `channel` and return (root thread, slot, session) of its one member.
fn commission(root: &Path, channel: &str) -> (String, String, String) {
    let out = nxc(root, T0, "dry")
        .args([
            "send",
            "--to",
            channel,
            "--no-ref",
            "--json",
            "verify the build",
        ])
        .output()
        .unwrap();
    let receipt = json_of(&out.stdout);
    let thread = receipt["thread_id"].as_str().unwrap().to_string();
    let (slot, session): (String, String) = store(root)
        .connection()
        .query_row(
            "SELECT thread, session FROM owed_starts WHERE role = 'local/verifier'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the obligation is recorded");
    (thread, slot, session)
}

/// A real process standing in for the session: started, and its claim written in the shape the
/// sidecar writes.
fn stand_in(root: &Path, session: &str) -> Child {
    let child = StdCommand::new("sleep")
        .arg("600")
        .stdin(Stdio::null())
        .spawn()
        .expect("start the stand-in process");
    let logs = nexus_chat::worker::agent_logs_dir(root);
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join(format!("{session}.pid")),
        nexus_chat::worker::session_claim_for(child.id()),
    )
    .unwrap();
    child
}

/// `kill -9` and reap it, leaving the claim file behind as a SIGKILLed sidecar does.
fn kill_hard(mut child: Child) {
    child.kill().expect("SIGKILL the stand-in");
    child.wait().expect("reap the stand-in");
}

/// The session's own callbacks, as a sidecar makes them.
fn bind(root: &Path, now: &str, session: &str) {
    nxc(root, now, "dry")
        .env("NXC_SESSION", session)
        .args(["session", "bind", session, &format!("real-{session}")])
        .assert()
        .success();
}

fn write_transcript(root: &Path, now: &str, session: &str) {
    nxc(root, now, "dry")
        .env("NXC_SESSION", session)
        .args(["transcript", "append", "--session", session])
        .write_stdin(r#"{"kind":"assistant","data":{"text":"looking at the build"}}"#)
        .assert()
        .success();
}

fn ended(root: &Path, now: &str, session: &str) {
    nxc(root, now, "dry")
        .env("NXC_SESSION", session)
        .args(["session", "ended", session])
        .assert()
        .success();
}

/// The look the service runs: `nxc tick` under the sidecar worker, which probes the real pid.
fn tick(root: &Path, now: &str, thread: &str) -> Value {
    let out = nxc(root, now, "sidecar")
        .args(["tick", "--thread", thread, "--json"])
        .output()
        .unwrap();
    json_of(&out.stdout)
}

/// `(observed, escalation)` of every stranded record.
fn records(root: &Path) -> Vec<(i64, Option<String>)> {
    let s = store(root);
    let mut st = s
        .connection()
        .prepare("SELECT observed, escalation FROM stranded_sessions ORDER BY recorded_at")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn suspected(root: &Path) -> bool {
    store(root)
        .connection()
        .query_row("SELECT COUNT(*) FROM stranded_suspects", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
        > 0
}

fn triggers(root: &Path) -> usize {
    std::fs::read_to_string(root.join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger "))
        .count()
}

fn transcript_entries(root: &Path, session: &str) -> i64 {
    store(root)
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agent_transcript WHERE internal_session = ?1",
            [session],
            |r| r.get(0),
        )
        .unwrap()
}

fn escalations_in(root: &Path, thread: &str) -> usize {
    store(root)
        .acting_messages_in_thread(thread)
        .unwrap()
        .iter()
        .filter(|m| m.kind == "escalation")
        .count()
}

/// **AC 2 (a), field case 1 in real processes** — the session's process ends owing an answer and NO
/// substitute is posted (a coordinated slot's teardown posts none). A look at `ended + 120 s` does
/// not record and escalate: its coordinator CONTINUES the same session — the real sidecar worker
/// starts the sidecar on the same internal session, resuming the runtime session it bound, told
/// which thread waits and what the task was. The transcript is intact.
/// The stub appends each spec as one line, but a concurrent reader can see that append half done —
/// a spec past 4 KiB was read cut at column 4096 on CI (nxf 6j6v.b53g). A line without its newline is
/// still being written, so `started` must leave it for the next poll instead of parsing it.
#[test]
fn a_spec_line_the_stub_is_still_appending_is_not_read_yet() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("stub.log"),
        "{\"session\":\"one\"}\n{\"session\":\"tw",
    )
    .unwrap();

    let got = started(tmp.path());

    assert_eq!(got.len(), 1, "only the finished line is a spec: {got:?}");
    assert_eq!(got[0]["session"], "one");
}

#[test]
fn a_session_that_ended_owing_an_answer_is_continued_in_the_same_session() {
    let tmp = workspace();
    let root = tmp.path();
    let (thread, slot, s) = commission(root, "review");
    let child = stand_in(root, &s);
    bind(root, &at(5), &s);
    write_transcript(root, &at(6), &s);
    kill_hard(child);
    let end = at(30);
    ended(root, &end, &s);
    // The end armed its look, on the channel thread.
    let looks = std::fs::read_to_string(root.join("looks.log")).unwrap_or_default();
    assert!(
        // Filed under the SLOT since nxf 6j6v.kgfn: `<thread>-<bare role>`.
        looks.contains(&format!(
            "slot={slot}-verifier thread={thread} deadline={}",
            at(150)
        )),
        "a look at ended + 120 s: {looks}"
    );

    tick(root, &at(149), &thread);
    assert!(rungs(root).is_empty(), "not before the grace");
    tick(root, &at(150), &thread);
    let specs = started_n(root, 1);
    assert_eq!(specs.len(), 1, "one continuation: {specs:?}");
    let cont = &specs[0];
    assert_eq!(cont["session"], s.as_str(), "the SAME session");
    assert_eq!(
        cont["resume"],
        format!("real-{s}").as_str(),
        "resuming what it bound"
    );
    assert_eq!(cont["replyThread"], slot.as_str());
    assert_eq!(cont["coordinated"], true);
    let message = cont["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "Your turn for thread {slot} ended without a reply"
        )) && message.contains("the task was: verify the build"),
        "{message}"
    );
    assert!(records(root).is_empty(), "nothing recorded");
    assert_eq!(escalations_in(root, &slot), 0, "nothing escalated");
    assert_eq!(
        rungs(root),
        vec![("continued".to_string(), Some("triggered".to_string()))]
    );
    assert_eq!(transcript_entries(root, &s), 1, "the transcript is intact");
    assert_eq!(
        triggers(root),
        1,
        "the dry worker started only the round's own first start"
    );
}

/// **AC 2 (b), rung B in real processes** — a `five_hour` hold that names NO instant, then the
/// session's end. Nothing is done while the hold is younger than the grace; no resume will ever
/// take a hold up that names no instant, so one grace after it was taken its coordinator does —
/// continuing the same session — and the hold no longer stands. Nothing is recorded.
#[test]
fn a_hold_naming_no_instant_is_continued_by_its_coordinator_one_grace_after_it() {
    let tmp = workspace();
    let root = tmp.path();
    let (thread, slot, s) = commission(root, "review");
    let child = stand_in(root, &s);
    bind(root, &at(5), &s);
    write_transcript(root, &at(6), &s);
    kill_hard(child);
    nxc(root, &at(30), "dry")
        .env("NXC_SESSION", &s)
        .args(["session", "interrupted", &s, "--limit", "five_hour"])
        .assert()
        .success();
    ended(root, &at(31), &s);
    tick(root, &at(149), &thread);
    assert!(rungs(root).is_empty(), "inside the grace of the hold");
    tick(root, &at(150), &thread);
    let specs = started_n(root, 1);
    assert_eq!(specs.len(), 1, "{specs:?}");
    assert_eq!(specs[0]["session"], s.as_str(), "the same session");
    assert_eq!(specs[0]["replyThread"], slot.as_str());
    let out = nxc(root, &at(160), "dry")
        .args(["resume", "--session", &s, "--json"])
        .output()
        .unwrap();
    assert_eq!(
        json_of(&out.stdout)["outcome"],
        "nothing_on_hold",
        "the hold was taken up"
    );
    assert!(records(root).is_empty());
    assert_eq!(escalations_in(root, &slot), 0);
}

/// **AC 2 (c)** — bound, writing, healthy at the first look; killed from outside AFTER it (the pid
/// file left behind): suspected at the next look, and one cadence later — no live process, bound,
/// with work of its own — CONTINUED in the same session by its coordinator.
#[test]
fn a_session_killed_after_a_healthy_look_is_continued_by_its_coordinator() {
    let tmp = workspace();
    let root = tmp.path();
    let (thread, _slot, s) = commission(root, "review");
    let child = stand_in(root, &s);
    bind(root, &at(5), &s);
    write_transcript(root, &at(6), &s);
    tick(root, &at(120), &thread);
    assert!(!suspected(root), "a live claim is never suspected");
    kill_hard(child);
    tick(root, &at(180), &thread);
    assert!(suspected(root));
    assert!(rungs(root).is_empty());
    tick(root, &at(240), &thread);
    let specs = started_n(root, 1);
    assert_eq!(specs.len(), 1, "{specs:?}");
    assert_eq!(
        specs[0]["session"],
        s.as_str(),
        "continued in the same session"
    );
    assert_eq!(specs[0]["resume"], format!("real-{s}").as_str());
    assert!(records(root).is_empty());
}

/// **AC 2 (c′), rung D in real processes** — the same process killed BEFORE it bound: there is no
/// runtime session to continue, so its coordinator RE-COMMISSIONS the slot — a fresh session, the
/// original body, into the same slot.
#[test]
fn a_session_killed_before_it_bound_is_recommissioned_with_its_original_body() {
    let tmp = workspace();
    let root = tmp.path();
    let (thread, slot, s) = commission(root, "review");
    kill_hard(stand_in(root, &s));
    tick(root, &at(120), &thread);
    tick(root, &at(180), &thread);
    let specs = started_n(root, 1);
    assert_eq!(specs.len(), 1, "{specs:?}");
    assert_ne!(specs[0]["session"], s.as_str(), "a fresh session");
    assert!(specs[0]["resume"].is_null(), "nothing resumed");
    assert_eq!(specs[0]["replyThread"], slot.as_str(), "into the same slot");
    assert!(specs[0]["message"]
        .as_str()
        .unwrap()
        .contains("verify the build"));
    assert_eq!(
        rungs(root),
        vec![("recommissioned".to_string(), Some("triggered".to_string()))]
    );
    assert!(records(root).is_empty());
}

/// **AC 9 (b)** — a session bound BY HAND (no process claim at all) reads as not running under the
/// sidecar worker, and at +180 s its coordinator takes the slot up — the stated residual, re-read
/// for the ladder: a session driven by hand must answer within the grace or the slot is started
/// again. Bound but with nothing written, there is no turn of its own to continue: a fresh session.
#[test]
fn a_session_bound_by_hand_with_no_claim_is_taken_up_by_its_coordinator() {
    let tmp = workspace();
    let root = tmp.path();
    let (thread, _slot, s) = commission(root, "review");
    bind(root, &at(5), &s);
    tick(root, &at(120), &thread);
    tick(root, &at(180), &thread);
    let specs = started_n(root, 1);
    assert_eq!(specs.len(), 1, "{specs:?}");
    assert_ne!(
        specs[0]["session"],
        s.as_str(),
        "nothing of its own to continue"
    );
}

/// **AC 2 (d)** — the working copy, before `PARK_AFTER`: as (a), on a channel that holds the copy
/// alone, with an uncommitted file in it. Its coordinator takes the slot up (rung D: it never bound)
/// and the file is untouched, nothing was committed or checked out, the operation still holds the
/// copy, and a rival queued behind it stays first in line.
#[test]
fn a_holder_being_recovered_keeps_the_working_copy_and_its_files_untouched() {
    let tmp = workspace();
    let root = tmp.path();
    let git = |args: &[&str]| {
        let out = StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), ".nxs/\n*.log\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let head = git(&["rev-parse", "HEAD"]);

    let (thread, _slot, s) = commission(root, "guarded");
    std::fs::write(root.join("half-done.rs"), "fn half() {}\n").unwrap();
    nxc(root, &at(10), "dry")
        .args(["send", "--to", "rival", "--no-ref", "--json", "rival work"])
        .assert()
        .success();
    let child = stand_in(root, &s);
    kill_hard(child);
    ended(root, &at(30), &s);
    tick(root, &at(150), &thread);
    assert_eq!(started_n(root, 1).len(), 1, "taken up: {:?}", started(root));
    assert!(records(root).is_empty());

    assert_eq!(
        std::fs::read_to_string(root.join("half-done.rs")).unwrap(),
        "fn half() {}\n",
        "the uncommitted file is untouched"
    );
    assert_eq!(git(&["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    let status = json_of(
        &nxc(root, &at(150), "sidecar")
            .args(["status", "--all", "--json"])
            .output()
            .unwrap()
            .stdout,
    );
    let ops = status["operations"].as_array().unwrap();
    let held = ops
        .iter()
        .find(|op| op["root"] == thread.as_str())
        .expect("the stranded operation");
    assert_eq!(held["holds_working_tree"], true, "{held}");
    assert_eq!(
        held["needs_decision"], false,
        "nothing escalated while it is recovered: {held}"
    );
    assert_eq!(held["hangs_at"]["recovery"], "recovering", "{held}");
    let queued = ops
        .iter()
        .flat_map(|op| op["threads"].as_array().cloned().unwrap_or_default())
        .find(|t| t["working_tree_queue_position"].is_number())
        .expect("the rival is still queued");
    assert_eq!(queued["working_tree_queue_position"], 1);
}

/// **AC 2 (e)** — the grace's falsifier: the trigger funnel's own work between the obligation row
/// and the spawn (the lease gate, the prime block) is far inside `STRANDED_GRACE_SECS`. Measured as
/// the wall time of the whole `nxc send` process, an UPPER bound on that span, and printed as data.
/// It is the fast path: a slow `nxf prime` is bounded by its own 30 s timeout, which is the floor
/// this number cannot see and the reason the grace is 120 s.
#[test]
fn the_owed_row_to_spawn_latency_is_far_inside_the_grace() {
    let tmp = workspace();
    let root = tmp.path();
    let started = std::time::Instant::now();
    let (thread, _, _) = commission(root, "review");
    let took = started.elapsed();
    eprintln!("owed row → spawn upper bound (whole `nxc send`): {took:?}");
    // …and what one look costs as a process (build item 17): a cold `nxc tick` of the channel.
    let look = std::time::Instant::now();
    tick(root, &at(30), &thread);
    eprintln!(
        "one cold `nxc tick` of a channel thread: {:?}",
        look.elapsed()
    );
    assert!(
        took.as_secs() < nexus_chat::orchestration::STRANDED_GRACE_SECS as u64,
        "{took:?}"
    );
}
