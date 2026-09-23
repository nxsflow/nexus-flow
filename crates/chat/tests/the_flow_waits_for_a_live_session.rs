//! **The same gate as `a_step_ends_when_its_session_ends.rs`, through the REAL `nxc`** (nxf
//! 6j6v.10yb) — with a real live process behind the session instead of a worker that answers from a
//! set.
//!
//! It exists because the seam suite was green while the shipped path was inert, and the live run is
//! what found it: `cli.rs`'s `LazyWorker` wraps whichever worker the environment selects and
//! forwarded only `trigger`, so `Worker::session_is_running`'s DEFAULT (`false`) answered for every
//! command-line call there is. `Worker`'s default is what makes the method safe to add to a trait a
//! host implements — and it is exactly what lets a delegator swallow it in silence, because nothing
//! fails to compile. Measured before the fix, with two real Claude sessions: the coder replied at
//! 11:52:48.336 and the finisher's session was minted at 11:52:48.344, eight milliseconds later,
//! into the checkout the channel had declared it needs alone.
//!
//! So the process here is REAL — a `node` that stays alive, spawned by the real `SidecarWorker`
//! through the real CLI — and the pid it writes is what the gate reads. Nothing is stubbed between
//! `nxc` and that file.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

/// A workspace declaring the measured channel, plus a `node` script that simply stays alive — the
/// stand-in for a session that is still writing when its own answer arrives.
///
/// Deliberately NOT named like the shipped bundle: `SidecarWorker` demands a resolvable `claude`
/// only for the bundled sidecar, and this is about the liveness read, not about the SDK.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "finisher"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n  \
         working_tree: exclusive\n",
    )
    .unwrap();
    // Long enough that no assertion below can race it, short enough that a leaked one is not left
    // for a human to find.
    std::fs::write(
        tmp.path().join("stub-sidecar.mjs"),
        "setTimeout(() => {}, 30_000);\n",
    )
    .unwrap();
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_WORKER", "sidecar")
        .env("NXC_SIDECAR", tmp.path().join("stub-sidecar.mjs"))
        // A declared `timeout:` would otherwise leave one-shot `at` jobs on the machine this ran on.
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

/// The same command stamped as a spawned role session — what the sidecar puts into a triggered
/// role's environment, and therefore how that role's own `nxc reply` is attributed.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = nxc(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json_of(cmd: &mut Command) -> Value {
    serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
}

/// Every spec file the worker has written, by internal session id — one per STARTED session, so
/// their count is how many steps have actually run.
fn started_sessions(tmp: &TempDir) -> Vec<String> {
    let logs = tmp.path().join(".nxs/agent-logs");
    let mut out: Vec<String> = std::fs::read_dir(&logs)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_suffix(".spec.json"))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The role and the thread a started session was handed, read back off its own spec file — the same
/// contract the sidecar reads, so nothing here reconstructs what the worker decided.
fn spec_of(tmp: &TempDir, session: &str) -> Value {
    let path = tmp
        .path()
        .join(".nxs/agent-logs")
        .join(format!("{session}.spec.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("the spec was written"))
        .expect("valid json")
}

/// Kill and reap every `node` this test started. A leaked 30-second sleeper per run is exactly the
/// kind of debris the item next door is about.
fn reap(tmp: &TempDir) {
    if let Ok(out) = std::process::Command::new("pgrep")
        .args([
            "-f",
            &format!("{}", tmp.path().join("stub-sidecar.mjs").display()),
        ])
        .output()
    {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid])
                .status();
        }
    }
}

/// Open the round and hand back (the coder's internal session, its slot thread, the channel thread).
fn a_started_round(tmp: &TempDir) -> (String, String, String) {
    let opened = json_of(nxc(tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
    let channel_thread = opened["thread_id"].as_str().unwrap().to_string();
    let coder = started_sessions(tmp)
        .into_iter()
        .next()
        .expect("step one started");
    let slot = spec_of(tmp, &coder)["replyThread"]
        .as_str()
        .expect("a commissioned step is told its thread")
        .to_string();
    (coder, slot, channel_thread)
}

#[test]
fn a_tick_says_what_the_board_is_waiting_for_instead_of_nothing_to_do() {
    // Review of PR #361, Code Quality #1 + Test Quality #4. The PULL path has its own call site of
    // the liveness gate and had no test at all — which matters here more than symmetry, because
    // `nxc tick` is what runs when a session's own end announcement is lost — by hand here, and on
    // a clock since nxf 6j6v.858n (`a_clock_watches_the_liveness_gate.rs` holds that half). A tick that
    // answered `not_due` over a settled board would send an operator looking for a round that is
    // not yet ripe, when the truth is the opposite: it is ripe and waiting on a process.
    let tmp = workspace();
    let (coder, slot, channel_thread) = a_started_round(&tmp);
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &slot, "done"]));
    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "premise: the PUSH path declined, so there is something for the tick to find"
    );

    let ticked = json_of(nxc(&tmp).args(["--json", "tick", "--thread", &channel_thread]));
    assert_eq!(ticked["acted"], false, "{ticked}");
    assert_eq!(
        ticked["reason"], "waiting_for_a_session",
        "not `not_due`: the set IS settled — what is missing is a process exit, and no clock \
         watches that one: {ticked}"
    );
    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "and it started nothing, which is the whole point of declining"
    );
    // The same fact in the terminal, for the reader who typed it rather than parsed it.
    let human = stdout_of(nxc(&tmp).args(["tick", "--thread", &channel_thread]));
    assert!(
        human.contains("still running"),
        "the human line names what it is waiting for, got: {human}"
    );

    // …and the same verb is what releases it once the fact changes.
    json_of(nxc(&tmp).args(["--json", "session", "ended", &coder]));
    let after = json_of(nxc(&tmp).args(["--json", "tick", "--thread", &channel_thread]));
    assert_ne!(
        after["reason"], "waiting_for_a_session",
        "once the session is over the board is no longer waiting on it: {after}"
    );

    reap(&tmp);
}

#[test]
fn an_escalation_under_a_live_session_consolidates_when_the_process_goes_rather_than_advancing() {
    // **Where nxf 6j6v.ma7v and nxf 6j6v.10yb meet, and the reason the `advanced` flag exists**
    // (review of PR #376, Code Quality #3 — which found this combination untested).
    //
    // Each half alone is covered next door: an escalation ends an ordered chain (`channel_flow.rs`),
    // and a settled step whose session is still alive does not advance (this file). Together they
    // reach the one path where `tick_move` and the supervisor DISAGREE — the tick reads the board's
    // rows, decides `Advance`, and the supervisor then consolidates instead, because only it can see
    // that the settled set escalated. That disagreement is what `SupervisorRun::advanced` is for, and
    // the tick's own receipt depends on getting it right.
    let tmp = workspace();
    let (coder, slot, channel_thread) = a_started_round(&tmp);

    // Step one says "I cannot" while its own process is still writing.
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &slot,
        "--escalate",
        "I cannot: the credentials are missing",
    ]));
    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "premise: the PUSH path declined on the live session, so the tick has something to find"
    );

    // The session ends. Now the board is ripe — and what it is ripe FOR is the question.
    json_of(nxc(&tmp).args(["--json", "session", "ended", &coder]));

    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "the finisher is NOT started: a step that could not carry out its task does not commission \
         the work that was to follow it — sessions: {:?}",
        started_sessions(&tmp)
    );
    // What happened INSTEAD is a consolidation, and it carried the escalation up rather than folding
    // it away — the channel thread is the flow's answer of record (nxf 6j6v.wt37 / 6j6v.1xw1).
    let status = json_of(nxc(&tmp).args(["--json", "status", "--thread", &channel_thread]));
    let board = status["operations"][0]["threads"]
        .as_array()
        .expect("the operation's threads")
        .iter()
        .find(|t| t["thread_id"] == channel_thread.as_str())
        .expect("the channel thread is in its own operation")
        .clone();
    assert_eq!(
        board["escalated"], true,
        "the round was consolidated WITH the escalation on it: {status}"
    );

    // And a tick over the same board finds nothing left to do — not an advance it could still make.
    let ticked = json_of(nxc(&tmp).args(["--json", "tick", "--thread", &channel_thread]));
    assert_ne!(
        ticked["reason"], "advanced",
        "the receipt never claims an advance that did not happen: {ticked}"
    );
    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "and the tick starts nothing either — the chain is over"
    );

    reap(&tmp);
}

#[test]
fn an_operation_holding_the_working_copy_says_so_at_its_root() {
    // Review of PR #361, Test Quality #3: `holds_working_tree` had no test for `true` anywhere, and
    // the CLI's three non-empty rendering arms had none at all. A bug pinning the field permanently
    // false would have passed the whole suite — which is the same shape of hole the `LazyWorker`
    // incident was.
    let tmp = workspace();
    let (coder, slot, channel_thread) = a_started_round(&tmp);

    let held = json_of(nxc(&tmp).args(["--json", "status", "--thread", &channel_thread]));
    assert_eq!(
        held["operations"][0]["holds_working_tree"], true,
        "an exclusive round in flight holds the checkout, and the ROOT is where a reader learns it \
         without asking every thread in the tree: {held}"
    );
    assert_eq!(held["operations"][0]["needs_decision"], false, "{held}");
    let line = stdout_of(nxc(&tmp).args(["status", "--thread", &channel_thread]));
    assert!(
        line.contains("· holds working tree") && !line.contains("NEEDS DECISION"),
        "the second of the CLI's three arms, and only it, got:\n{line}"
    );

    // A hand-back on the same round lights the OTHER flag — and the third arm renders both.
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &slot,
        "--escalate",
        "I cannot: the credentials are missing",
    ]));
    let handed_back = json_of(nxc(&tmp).args(["--json", "status", "--thread", &channel_thread]));
    assert_eq!(
        handed_back["operations"][0]["needs_decision"], true,
        "{handed_back}"
    );
    assert_eq!(
        handed_back["operations"][0]["holds_working_tree"], true,
        "{handed_back}"
    );
    let both = stdout_of(nxc(&tmp).args(["status", "--thread", &channel_thread]));
    assert!(
        both.contains("· NEEDS DECISION · holds working tree"),
        "the third arm, which is the one an operator meets when a chain has actually stopped, \
         got:\n{both}"
    );

    reap(&tmp);
}

#[test]
fn through_the_real_cli_the_next_step_waits_for_the_previous_session_to_be_over() {
    let tmp = workspace();

    let opened = json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
    assert_eq!(opened["spawned"], true, "the round starts: {opened}");
    let sessions = started_sessions(&tmp);
    assert_eq!(
        sessions.len(),
        1,
        "step one, and only step one: {sessions:?}"
    );
    let coder = sessions[0].clone();
    let spec = spec_of(&tmp, &coder);
    assert_eq!(spec["role"], "coder", "{spec}");
    let slot = spec["replyThread"]
        .as_str()
        .expect("a commissioned step is told its thread")
        .to_string();

    // The premise: a REAL process holds this session, and the pid file its own lock wrote is what
    // says so. Without this the assertion below would pass for a session that had simply never
    // started, which is the wrong reason.
    assert!(
        tmp.path()
            .join(".nxs/agent-logs")
            .join(format!("{coder}.pid"))
            .exists(),
        "the worker took a claim for the session it spawned"
    );

    // 00:16:33 of the measured timeline: the coder answers while it is still writing.
    let receipt = json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &slot,
        "INTERIM - not the finish",
    ]));
    assert_eq!(receipt["posted"], true, "{receipt}");

    assert_eq!(
        started_sessions(&tmp),
        vec![coder.clone()],
        "the finisher must NOT have started: the answer settled a message board, and the process \
         behind it is still writing into the working copy this channel declared it needs alone"
    );

    // 00:34:27: the process is over — announced by the session itself, which is the only party that
    // can know it at the moment it matters. The stub is still alive on purpose: the RECORDED end
    // outranks a liveness read taken while the ending session speaks, and that is the whole reason
    // `session_map.ended` exists beside the pid file.
    let ended = json_of(nxc(&tmp).args(["--json", "session", "ended", &coder]));
    assert_eq!(ended["reason"], "considered", "{ended}");

    let after = started_sessions(&tmp);
    assert_eq!(
        after.len(),
        2,
        "the flow advances now — and only now: {after:?}"
    );
    let finisher = after
        .iter()
        .find(|s| **s != coder)
        .expect("a second session");
    assert_eq!(spec_of(&tmp, finisher)["role"], "finisher");

    reap(&tmp);
}
