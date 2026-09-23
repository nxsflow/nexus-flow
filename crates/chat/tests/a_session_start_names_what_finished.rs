//! ACCEPTANCE of nxf 6j6v.2hx9 — **a session start names the commissions that finished while it
//! was away**, derived from the OPERATION rather than from a read cursor.
//!
//! # What it replaces
//!
//! `prime --json`'s `threads_you_opened` was derived from the read cursor: a completed board dropped
//! off exactly when the opener's cursor passed the completing reply. Nothing ever called the ack
//! that moved it, so in practice the notice never cleared and every session start replayed every
//! commission the caller had ever finished — the same defect nxf 6j6v.1vxs removed from the
//! operation view, one surface further. (The cursor itself is gone since nxf 6j6v.4d2z, which this
//! replacement is what made possible.)
//!
//! # The promise it makes instead
//!
//! A WINDOW: what finished since this caller's previous session ended. The previous session's end is
//! a watermark the system already records ([`ChatStore::previous_session_end`]), and it needs no
//! acknowledgement because "finished" does not become unfinished. The cost is stated rather than
//! discovered, and this file pins it: **a caller sees the notice once, not until it acknowledges
//! it.**
//!
//! The run the item asks for is the whole of the first test: commission a round, end the session,
//! let the round finish, start again, see it once — and not twice.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// The PM is commissioned here and commissions two coders.
const T0: &str = "2026-09-07T09:00:00Z";
/// One coder answers WHILE the PM's first session is still running, so this board finishes BEFORE
/// the watermark — the half that proves the list is a window and not a backlog.
const EARLY: &str = "2026-09-07T09:01:00Z";
/// The PM's first session is over here. This is the watermark.
const ENDED: &str = "2026-09-07T09:02:00Z";
/// …and the slow coder answers only now, while nobody was there to be woken.
const LATE: &str = "2026-09-07T09:30:00Z";
/// The PM's SECOND session ends here, past the late answer — so the window moves on.
const ENDED_AGAIN: &str = "2026-09-07T10:00:00Z";
/// When the session start being tested happens.
const NEXT_START: &str = "2026-09-07T10:30:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "quick", "slow"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    tmp
}

fn nxc(tmp: &TempDir, now: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn as_persona(tmp: &TempDir, now: &str, handle: &str, session: &str) -> Command {
    let mut c = nxc(tmp, now);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_slice(&out.get_output().stdout).expect("valid json")
}

/// The commissions a session start names, by thread id.
fn notice(tmp: &TempDir, now: &str) -> Vec<String> {
    let prime = json_of(nxc(tmp, now).args([
        "--json",
        "prime",
        "--persona",
        "pm",
        "--consumer",
        "local/pm",
    ]));
    prime["threads_you_opened"]["complete"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| r["thread_id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_session_start_names_what_finished_while_it_was_away_and_names_it_once() {
    let tmp = workspace();

    // The PM is started, and commissions two coders from inside its own session.
    let pm = json_of(nxc(&tmp, T0).args(["--json", "send", "--to", "pm", "--no-ref", "plan it"]))
        ["session"]
        .as_str()
        .unwrap()
        .to_string();
    let early = json_of(
        as_persona(&tmp, T0, "pm", &pm)
            .args(["--json", "send", "--to", "quick", "--no-ref", "do this"]),
    )["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let late = json_of(
        as_persona(&tmp, T0, "pm", &pm)
            .args(["--json", "send", "--to", "slow", "--no-ref", "and this"]),
    )["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // The quick one answers while the PM is still there — this board finishes BEFORE the watermark.
    as_persona(&tmp, EARLY, "quick", "s-quick")
        .args(["reply", "--thread", &early, "done already"])
        .assert()
        .success();

    // The PM's session ends. THIS instant is the watermark.
    nxc(&tmp, ENDED)
        .args(["session", "ended", &pm])
        .assert()
        .success();

    // …and only now, with nobody there to be woken, the slow one answers.
    as_persona(&tmp, LATE, "slow", "s-slow")
        .args(["reply", "--thread", &late, "took a while"])
        .assert()
        .success();

    // THE RUN: the next session start names exactly the commission that finished while it was away.
    assert_eq!(
        notice(&tmp, NEXT_START),
        vec![late.clone()],
        "the board that finished after the previous session ended, and only that one"
    );
    assert!(
        !notice(&tmp, NEXT_START).contains(&early),
        "a board that finished BEFORE the watermark is behind the window — this is what stops the \
         notice growing with the age of the workspace"
    );

    // …and it is a WINDOW, not a debt: nothing has to acknowledge it, and once the caller's own
    // session has ended past that answer, the next start does not repeat it.
    let second = json_of(
        nxc(&tmp, NEXT_START).args(["--json", "send", "--to", "pm", "--no-ref", "carry on"]),
    )["session"]
        .as_str()
        .unwrap()
        .to_string();
    nxc(&tmp, ENDED_AGAIN)
        .args(["session", "ended", &second])
        .assert()
        .success();
    assert!(
        notice(&tmp, "2026-09-07T11:00:00Z").is_empty(),
        "seen once, not until acknowledged — the promise this item makes, stated as a cost"
    );
}

#[test]
fn a_persona_that_has_never_finished_a_session_here_has_no_window_either() {
    // The other half of "no watermark, no window" (review of this branch, Test Quality). The human
    // case below is the obvious one; this is the one that could surprise — a DECLARED persona whose
    // role has simply never had a session END on this device. Its very first start has nothing to be
    // a window over, and the honest answer is the same absent block rather than "everything, ever".
    let tmp = workspace();
    let pm = json_of(nxc(&tmp, T0).args(["--json", "send", "--to", "pm", "--no-ref", "plan it"]))
        ["session"]
        .as_str()
        .unwrap()
        .to_string();
    let thread = json_of(
        as_persona(&tmp, T0, "pm", &pm)
            .args(["--json", "send", "--to", "quick", "--no-ref", "do this"]),
    )["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    as_persona(&tmp, EARLY, "quick", "s-quick")
        .args(["reply", "--thread", &thread, "done already"])
        .assert()
        .success();

    // The pm's session is still running — it has never announced an end — so there is no watermark.
    assert!(
        notice(&tmp, NEXT_START).is_empty(),
        "a first start has no previous session to derive a window from"
    );
    let prime = json_of(nxc(&tmp, NEXT_START).args([
        "--json",
        "prime",
        "--persona",
        "pm",
        "--consumer",
        "local/pm",
    ]));
    assert!(
        prime.get("threads_you_opened").is_none(),
        "and the block is omitted rather than carried empty: {prime}"
    );

    // …and the moment that session ends, the window exists and the notice is not lost: the board
    // finished BEFORE this end, so it is behind the window — which is the promise, stated as its
    // cost on the item.
    nxc(&tmp, ENDED)
        .args(["session", "ended", &pm])
        .assert()
        .success();
    assert!(
        notice(&tmp, NEXT_START).is_empty(),
        "seen never rather than seen twice: this board finished before the watermark"
    );
}

#[test]
fn a_human_caller_is_not_served_by_this_at_all_and_is_not_pretended_to_be() {
    // The item says so rather than leaving it implied: a human at a terminal has no session whose
    // end could be a watermark, so there is no window — and `nxc status` answers the same question
    // on demand. The block is ABSENT rather than empty, which is what the key has always meant.
    let tmp = workspace();
    let thread = json_of(nxc(&tmp, T0).args(["--json", "send", "--to", "quick", "--no-ref", "go"]))
        ["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    as_persona(&tmp, EARLY, "quick", "s-quick")
        .args(["reply", "--thread", &thread, "done"])
        .assert()
        .success();

    let prime = json_of(nxc(&tmp, NEXT_START).args(["--json", "prime"]));
    assert!(
        prime.get("threads_you_opened").is_none(),
        "no watermark, no window — and the block is omitted, not carried empty: {prime}"
    );
    // …and the same question IS answerable, on demand, by the surface built for it.
    let status = json_of(nxc(&tmp, NEXT_START).args(["--json", "status", "--all"]));
    assert!(
        status["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["root"] == thread.as_str()),
        "`nxc status --all` still has the finished operation: {status}"
    );
}
