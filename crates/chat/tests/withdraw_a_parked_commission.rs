//! `nxc withdraw` — taking back a commission that never started (nxf 6j6v.0h3p).
//!
//! In the proving ground (4jgn.g90w) the PM commissioned two coding rounds where the process
//! allowed one. The mistake was noticed at once, and there was no way back: `nxc --help` knew no
//! `cancel`, `abort`, `withdraw`, `stop` or `retract`. The second round was PARKED behind the
//! working-tree lease — nothing had happened, no session, no transcript, no model call — and it was
//! going to start the moment the first one let go, with nothing anybody could type to stop it short
//! of killing a process.
//!
//! That is the case this file answers, and the reason it is safe: **a parked commission has not
//! happened yet, so taking it back loses nothing.** A round that has STARTED is a different question
//! with different answers (work in flight, a session mid-turn) — answered since nxf 6j6v.b9nf, in
//! `withdraw_a_running_round.rs`: the session is stopped and its work parked, never rolled back.
//! Nothing here changed with that; the queued case behaves as it always did.
//!
//! Two properties are load-bearing and both are asserted below:
//!
//! * the withdrawn commission is really gone — it does not start when the lease is released, which
//!   is the only moment that could ever prove it;
//! * the withdrawal is visible IN THE THREAD, so it is a record and not merely an absence.

mod common;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

const NOW: &str = "2026-08-22T10:00:00Z";

/// Two personas that both need sole use of the working copy — the shape that makes the second
/// commission park instead of run.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "rival"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\nsystem_prompt: You are {handle}.\nworking_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    tmp
}

fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("valid json")
}

/// Whether the worker was ever handed a trigger for `handle` — a queued trigger is one the worker
/// never heard about, so this is the evidence that something did or did not START.
fn started(tmp: &TempDir, handle: &str) -> bool {
    std::fs::read_to_string(tmp.path().join("dry.log"))
        .unwrap_or_default()
        .lines()
        .any(|l| l.starts_with(&format!("trigger role={handle} ")))
}

/// The holder, then the commission that has to wait for it. Returns `(holder session, holder
/// thread, parked thread)`.
fn a_holder_and_a_parked_commission(tmp: &TempDir) -> (String, String, String) {
    let holder =
        json_of(human(tmp).args(["--json", "send", "--no-ref", "--to", "coder", "build it"]));
    assert_eq!(holder["spawned"], true, "the first one runs: {holder}");

    let parked = json_of(human(tmp).args([
        "--json",
        "send",
        "--no-ref",
        "--to",
        "rival",
        "and the next job",
    ]));
    assert_eq!(
        parked["queue_position"], 1,
        "the second one is parked behind the working copy: {parked}"
    );
    assert!(
        !started(tmp, "rival"),
        "a parked commission has no session, no transcript and no model call — that is why taking \
         it back costs nothing"
    );
    (
        holder["session"].as_str().unwrap().to_string(),
        holder["thread_id"].as_str().unwrap().to_string(),
        parked["thread_id"].as_str().unwrap().to_string(),
    )
}

#[test]
fn a_parked_commission_is_withdrawn_and_never_starts_when_the_lease_is_released() {
    let tmp = workspace();
    let (holder_session, holder_thread, parked_thread) = a_holder_and_a_parked_commission(&tmp);

    let receipt = json_of(human(&tmp).args(["--json", "withdraw", "--thread", &parked_thread]));
    let withdrawn = receipt["withdrawn"].as_array().expect("withdrawn");
    assert_eq!(withdrawn.len(), 1, "{receipt}");
    assert_eq!(withdrawn[0]["role"], "rival", "{receipt}");
    assert_eq!(withdrawn[0]["thread"], parked_thread.as_str(), "{receipt}");
    assert!(
        receipt["started_meanwhile"]
            .as_array()
            .is_some_and(|v| v.is_empty()),
        "nothing was running, so nothing was left alone: {receipt}"
    );

    // THE assertion. Everything above could be true of a verb that only edited a board; this is the
    // one moment that can prove the commission is really gone — the holder lets go, the queue is
    // drained, and the rival is not in it.
    persona(&tmp, "coder", &holder_session)
        .args(["--json", "reply", "--thread", &holder_thread, "done"])
        .assert()
        .success();
    assert!(
        !started(&tmp, "rival"),
        "the lease was released and the withdrawn commission still did not start — otherwise \
         `withdraw` is a lie told to a board"
    );
}

#[test]
fn the_withdrawal_is_a_record_in_the_thread_and_the_board_stops_waiting() {
    let tmp = workspace();
    let (_holder_session, _holder_thread, parked_thread) = a_holder_and_a_parked_commission(&tmp);

    human(&tmp)
        .args(["withdraw", "--thread", &parked_thread])
        .assert()
        .success();

    let board = json_of(human(&tmp).args([
        "--json",
        "threads",
        "show",
        &parked_thread,
        "--consumer",
        "local/carsten",
    ]));
    assert_eq!(
        board["complete"], true,
        "the thread ENDS — it does not sit open forever waiting for somebody who will never come: \
         {board}"
    );
    let bodies: Vec<&str> = board["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["body"].as_str())
        .collect();
    assert!(
        bodies
            .iter()
            .any(|b| b.contains("[withdrawn by local/carsten]")),
        "and it says who took it back and that nothing ran: {bodies:?}"
    );
}

// ---- a round taken back WHOLE stops counting as one that is running (nxf 6j6v.7me0) -----------

/// The channel shape: a persona that holds the working copy, plus a declared `coding` round that
/// therefore parks behind it. `send --to <channel>` hands back the CHANNEL thread, and the
/// commission that actually parks hangs on a member thread below it — which is exactly the split
/// this item is about.
fn channel_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["holder", "coder"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\nsystem_prompt: You are {handle}.\nworking_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coder]\n  working_tree: exclusive\n",
    )
    .unwrap();
    tmp
}

#[test]
fn a_round_whose_every_commission_was_taken_back_no_longer_counts_as_an_open_operation() {
    // The measured shape (nxf 6j6v.7me0): the child was correctly on `__withdrawn__` and the parent
    // stayed `state=open expects=['<origin>/__channel__']`, so `nxc status` went on reporting an
    // open operation for a round nobody would ever run. Whoever reads open threads to decide
    // whether anything is still going gets the wrong answer — and that is the one thing that view
    // is for.
    let tmp = channel_workspace();
    let holder =
        json_of(human(&tmp).args(["--json", "send", "--no-ref", "--to", "holder", "build it"]));
    assert_eq!(holder["spawned"], true, "the first one runs: {holder}");
    let round = json_of(human(&tmp).args([
        "--json",
        "send",
        "--no-ref",
        "--to",
        "coding",
        "do the round",
    ]));
    let channel_thread = round["thread_id"].as_str().unwrap().to_string();

    let before = json_of(human(&tmp).args(["--json", "status", "--thread", &channel_thread]));
    assert_eq!(
        before["operations"][0]["open"], 2,
        "the premise: while the round is parked, BOTH its threads are open — the channel thread \
         waiting on its supervisor and the member slot waiting on the coder. Otherwise the \
         assertion below proves nothing: {before}"
    );

    let receipt = json_of(human(&tmp).args(["--json", "withdraw", "--thread", &channel_thread]));
    assert_eq!(
        receipt["withdrawn"].as_array().map(Vec::len),
        Some(1),
        "the parked member commission is taken back: {receipt}"
    );

    let after = json_of(human(&tmp).args(["--json", "status", "--thread", &channel_thread]));
    assert_eq!(
        after["operations"][0]["open"], 0,
        "and the round it belonged to stops counting as running: {after}"
    );
    let bodies: Vec<String> = after["operations"][0]["threads"]
        .as_array()
        .expect("threads")
        .iter()
        .map(|t| t["state"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        bodies.iter().all(|st| st == "answered"),
        "every thread of the round is discharged, not merely the member's: {after}"
    );
}

#[test]
fn the_withdrawn_round_says_in_the_channel_thread_that_there_is_nothing_to_consolidate() {
    // A record, not an absence — the same discipline the member thread's own withdrawal message
    // keeps. And it is deliberately NOT a consolidation: `withdraw` refuses to deliver an empty
    // round's answers, because what became of a round is a decision about what a channel means.
    // Saying "it was withdrawn" is the opposite statement, and it belongs in the register.
    let tmp = channel_workspace();
    json_of(human(&tmp).args(["--json", "send", "--no-ref", "--to", "holder", "build it"]));
    let round = json_of(human(&tmp).args([
        "--json",
        "send",
        "--no-ref",
        "--to",
        "coding",
        "do the round",
    ]));
    let channel_thread = round["thread_id"].as_str().unwrap().to_string();

    human(&tmp)
        .args(["withdraw", "--thread", &channel_thread])
        .assert()
        .success();

    let board = json_of(human(&tmp).args([
        "--json",
        "threads",
        "show",
        &channel_thread,
        "--consumer",
        "local/carsten",
    ]));
    assert_eq!(board["complete"], true, "{board}");
    let bodies: Vec<&str> = board["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["body"].as_str())
        .collect();
    assert!(
        bodies
            .iter()
            .any(|b| b.contains("[withdrawn by local/carsten]") && b.contains("consolidate")),
        "the channel thread says what happened to the round: {bodies:?}"
    );
}

#[test]
fn a_thread_whose_only_session_the_dry_worker_cannot_answer_liveness_for_is_refused_by_name() {
    // A round that has STARTED is withdrawable since nxf 6j6v.b9nf — but only where the worker
    // says a session of it is RUNNING. This file's CLI runs the dry worker, and `DryWorker` does
    // NOT override `Worker::answers_liveness` — it rests on the trait DEFAULT (`false`), which
    // means it does not say the coder's session is not running, it says it CANNOT ANSWER whether
    // it is (the distinction `every_worker_answers_for_itself.rs`'s module doc argues for its
    // third defaulted method). The holder's area has nothing queued behind it and the coder's
    // session has never reported an end, so `withdraw` refuses BY NAME rather than claim the round
    // is over — a `validation`, not the `not_found` this test used to assert before nxf 6j6v.b9nf
    // changed what "nothing is running" is allowed to mean on a worker that never looked. The
    // holder is untouched by the refusal either way. The running case itself is
    // `withdraw_a_running_round.rs`; the true `not_found` this refusal replaced — a worker that
    // DOES answer liveness and finds nothing, or an area with no unended session at all — is
    // pinned there, in `a_host_that_answers_liveness_and_finds_nothing_running_is_a_not_found`.
    let tmp = workspace();
    let (session, holder_thread, _parked) = a_holder_and_a_parked_commission(&tmp);

    let out = human(&tmp)
        .args(["withdraw", "--thread", &holder_thread])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(&session) && stderr.contains("does not answer the liveness question"),
        "the refusal names the session it cannot answer for, and the capability that is missing, \
         not a claim that nothing is waiting or running: {stderr}"
    );
    // …and the holder's commission is untouched by the attempt.
    assert!(started(&tmp, "coder"));
}

#[test]
fn a_caller_who_cannot_even_read_the_board_cannot_withdraw_what_is_parked_on_it() {
    // Raised by the independent review of this branch (Integrity/Medium): until this, a thread id
    // was the WHOLE credential — any local caller could take back somebody else's commissioned work
    // by naming it. The bar is now the READ gate, which is the same question `nxc threads show`
    // asks: what you can see, you can take back. It is not access control (this surface has none;
    // that is nxf 6j6v.6aza) — it stops a caller reaching into a tree it cannot even read.
    let tmp = workspace();
    let (_holder_session, _holder_thread, parked_thread) = a_holder_and_a_parked_commission(&tmp);

    let out = human(&tmp)
        .env("NXC_ACTOR", "outsider")
        .args(["withdraw", "--thread", &parked_thread])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("not a member") || stderr.contains("did not open the operation"),
        "the refusal is the read gate's own, not a second vocabulary: {stderr}"
    );
    // …and it comes BEFORE the opener check (nxf 6j6v.ezbr): a caller who cannot even read the
    // thread is not told who opened the operation it belongs to.
    assert!(
        !stderr.contains("carsten"),
        "the read gate answers first, naming nobody: {stderr}"
    );

    // …and it really is still there: the commission is untouched, so the owner can still take it
    // back. A guard that refused the stranger AND ate the entry would be the worse outcome.
    let receipt = json_of(human(&tmp).args(["--json", "withdraw", "--thread", &parked_thread]));
    assert_eq!(
        receipt["withdrawn"].as_array().map(Vec::len),
        Some(1),
        "the rightful caller still withdraws it: {receipt}"
    );
}

#[test]
fn a_session_this_workspace_started_may_not_withdraw_even_under_the_openers_name() {
    // nxf 6j6v.ezbr — the owner's decision of 2026-09-20: `withdraw` is a person's verb. The call
    // comes from INSIDE the holder's session (its `NXC_SESSION`, exactly as the sidecar stamps it)
    // while presenting the OPENER's name, which is what isolates the gate under test: the opener
    // check alone would let this caller through. What refuses it is the session map, which knows
    // the session was started here.
    let tmp = workspace();
    let (holder_session, holder_thread, parked_thread) = a_holder_and_a_parked_commission(&tmp);

    let out = persona(&tmp, "carsten", &holder_session)
        .args(["withdraw", "--thread", &parked_thread])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(&holder_session) && stderr.contains("started for coder"),
        "the refusal names the session and the role it was started for: {stderr}"
    );
    assert!(
        stderr.contains(&format!("nxc reply --thread {holder_thread} --escalate")),
        "and the way it has instead — escalation, on the thread it stands in: {stderr}"
    );

    // Nothing changed: the commission is still queued, and the person who sent it takes it back.
    assert!(!started(&tmp, "rival"));
    let receipt = json_of(human(&tmp).args(["--json", "withdraw", "--thread", &parked_thread]));
    assert_eq!(
        receipt["withdrawn"].as_array().map(Vec::len),
        Some(1),
        "the rightful caller still withdraws it: {receipt}"
    );
}
