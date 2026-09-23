//! ACCEPTANCE of nxf 6j6v.7qfm — **the receipt says how the requester learns the answer**, and the
//! predicate fields it hands out are a PROMISE rather than a hint.
//!
//! The finding, measured with the owner on 2026-09-07: `nxc send --to` answered with
//! `opened thread <id> in dm:<24 hex> — reply with `nxc reply --thread <id> "…"`` and that was the
//! caller's whole guidance. It answers the wrong question — the reader is the REQUESTER, at the
//! moment nobody has answered yet — and the hash in it is derived from the two handles, so it tells
//! a reader who already knows who they wrote to precisely nothing. Three live rounds put a persona's
//! answer at eight to nine seconds: the return path was never slow, it was SILENT.
//!
//! Everything below is driven through the real binary, because the human line and the `--json`
//! record are the deliverable and a library call would test neither.
//!
//! # The four things this file holds
//!
//! 1. the human line names the two reads a requester actually takes, and carries no `dm:` hash;
//! 2. `--json` carries the same content as FIELDS: where to poll, done, stopped, where the answer
//!    sits, and the declared window;
//! 3. **the predicates are true of the payload they name** — the poll command is run, and the fields
//!    `done_when`/`stopped_when` name are read off its output, in both terminal states. This is the
//!    test the item asks for: it goes red if either marker stops being a field of that payload, and
//!    it drives BOTH sides of `stopped_when`'s OR (a hand-back with no window, and a window that ran
//!    out with nobody having answered) so a regression narrowing it to an AND is caught.
//!
//!    **What it does NOT hold, said rather than implied** (review of this branch, Test Quality):
//!    `done_when`'s AND — that `outstanding: []` alone is not enough, because a thread that expects
//!    NOBODY has an empty outstanding too — is pinned next door, at the derivation itself
//!    (`store.rs::thread_quorums_bulk_returns_a_deterministic_channel_then_id_order`'s `t-3` case:
//!    "empty E is never complete"). Duplicating it here would be a second spelling of one fact;
//!    what belongs here is that the FIELD says what that derivation means.
//! 4. a registered persona gets the OPPOSITE advice in the same field — it is resumed, and must not
//!    poll — and `reply` carries the whole block too.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-09-07T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("reviewer.yaml"),
        "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You review.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: review\n  members: [coder, reviewer]\n  timeout: 30m\n",
    )
    .unwrap();
    tmp
}

/// Commission `to` and hand back `(thread, session)` — the session is the REAL one the trigger
/// minted, which is what makes the caller a registered persona to every later call.
fn commission(tmp: &TempDir, from: Option<(&str, &str)>, to: &str) -> (String, String) {
    let mut cmd = match from {
        Some((handle, session)) => persona(tmp, handle, session),
        None => human(tmp),
    };
    let receipt = json_of(cmd.args(["--json", "send", "--to", to, "--no-ref", "do the thing"]));
    (
        receipt["thread_id"].as_str().unwrap().to_string(),
        receipt["session"].as_str().unwrap().to_string(),
    )
}

/// A human at the keyboard: no spawned-context stamps at all.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

/// A spawned persona session — the stamps a triggered role runs under.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
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

#[test]
fn the_human_line_says_how_to_learn_the_answer_and_carries_no_direct_channel_hash() {
    let tmp = workspace();
    let out = stdout_of(human(&tmp).args(["send", "--to", "coder", "--no-ref", "do the thing"]));

    // THE DEFECT, in one assertion: the line that stood here told the requester how to keep TALKING.
    assert!(
        !out.contains("reply with"),
        "the receipt must not answer the addressee's question: {out}"
    );
    // …and it named a hash derived from the two handles, which the reader can do nothing with.
    assert!(
        !out.contains("dm:"),
        "the direct-channel hash belongs in the record, not in the guidance: {out}"
    );

    // What stands there instead: who was asked, on which thread, and the two reads that answer
    // "did it arrive yet".
    assert!(out.starts_with("-> coder · thread m-"), "{out}");
    assert!(out.contains("The answer lands in this thread."), "{out}");
    assert!(out.contains("nxc threads show"), "{out}");
    assert!(out.contains("nxc status --thread"), "{out}");
    assert!(
        out.contains("add --stream next time"),
        "a human at a terminal can watch instead of coming back: {out}"
    );

    // The thread id is printed IN FULL on every line, because those lines are meant to be run.
    let thread = out
        .lines()
        .next()
        .unwrap()
        .rsplit(' ')
        .next()
        .unwrap()
        .to_string();
    assert!(thread.starts_with("m-"), "{out}");
    assert_eq!(
        out.matches(thread.as_str()).count(),
        3,
        "the header and both commands name the whole id: {out}"
    );
}

#[test]
fn the_json_receipt_carries_the_same_content_as_fields() {
    let tmp = workspace();
    let receipt = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "do the thing",
    ]));
    let thread = receipt["thread_id"].as_str().unwrap().to_string();
    let guidance = &receipt["await"];

    assert_eq!(guidance["how"], "poll", "nobody will wake a human");
    assert_eq!(
        guidance["poll"],
        serde_json::json!(["nxc", "threads", "show", thread, "--json"]),
        "argv, not a command line: a caller with only the receipt must not have to quote a shell"
    );
    assert_eq!(
        guidance["done_when"],
        serde_json::json!({ "complete": true, "outstanding": [] })
    );
    assert_eq!(
        guidance["stopped_when"],
        serde_json::json!({ "stale": true, "escalated": true })
    );
    assert_eq!(guidance["answer_at"], "messages[-1]");
    assert_eq!(
        guidance["deadline"],
        Value::Null,
        "no persona declares a window, so there is none — and the field says so rather than being \
         absent"
    );

    // The hash left the human line and stayed in the record, which is the other half of that
    // decision.
    assert!(
        receipt["channel"].as_str().unwrap().starts_with("dm:"),
        "{receipt}"
    );
}

#[test]
fn a_declared_window_reaches_the_caller_as_the_deadline_it_is() {
    // Decision 3 of the item: `deadline` says how patient to be, and the engine has always known
    // this number. A channel with a declared `timeout:` resolves it to an absolute instant when the
    // board opens, and until this it never reached the `await` block at all.
    let tmp = workspace();
    let receipt = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "review",
        "--no-ref",
        "have a look",
    ]));
    assert_eq!(
        receipt["await"]["deadline"], receipt["deadline"],
        "one number, reported once — never re-derived beside itself: {receipt}"
    );
    assert_eq!(
        receipt["await"]["deadline"], "2026-09-07T10:30:00Z",
        "the channel's declared 30m window, resolved: {receipt}"
    );
}

#[test]
fn a_registered_persona_is_told_the_opposite_in_the_same_field() {
    // Decision 1: `nxc` already separates a registered persona from everybody else — the `--stream`
    // refusal gates on exactly that — and for the two the correct advice is opposite. A persona is
    // RESUMED by the coordinator; telling it to poll would be a session burning turns on a question
    // it will be handed.
    let tmp = workspace();
    // The session a real trigger minted for the coder — not a made-up id, because "is this a
    // registered persona" is answered from `session_map` and a stranger's id reads as a human.
    let (_, coder) = commission(&tmp, None, "coder");

    let receipt = json_of(
        persona(&tmp, "coder", &coder)
            .args(["--json", "send", "--to", "reviewer", "--no-ref", "look?"]),
    );
    assert_eq!(receipt["await"]["how"], "resume");
    assert_eq!(
        receipt["await"]["poll"],
        Value::Null,
        "a persona that polls is a session burning turns on a question it will be handed"
    );
    // The PROMISE is the same for both readers; only the way it arrives differs.
    assert_eq!(
        receipt["await"]["done_when"],
        serde_json::json!({ "complete": true, "outstanding": [] })
    );
    assert_eq!(
        receipt["await"]["stopped_when"],
        serde_json::json!({ "stale": true, "escalated": true })
    );

    let out = stdout_of(
        persona(&tmp, "coder", &coder).args(["send", "--to", "reviewer", "--no-ref", "again?"]),
    );
    assert!(out.contains("do not poll for it"), "{out}");
    assert!(
        !out.contains("--stream"),
        "--stream is refused to a persona, so it must not be offered to one: {out}"
    );
}

#[test]
fn the_done_predicate_holds_against_the_payload_it_names() {
    // **The test the item asks for**: not that the fields are spelled correctly, but that they mean
    // what they say about the thing `poll` produces. It goes red if `threads show --json` ever stops
    // carrying one of them, or if one of them ever stops meaning "finished" / "stopped".
    let tmp = workspace();

    // (a) DONE. The commissioned persona answers, and the poll command the receipt handed out says
    //     so in exactly the fields `done_when` names.
    let receipt = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "--no-ref", "go"]));
    let thread = receipt["thread_id"].as_str().unwrap().to_string();
    let poll: Vec<String> = receipt["await"]["poll"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(poll[0], "nxc", "the caller runs the tool it already has");

    let coder = receipt["session"].as_str().unwrap().to_string();
    persona(&tmp, "coder", &coder)
        .args(["reply", "--thread", &thread, "here it is"])
        .assert()
        .success();

    let board = json_of(human(&tmp).args(poll[1..].iter().map(String::as_str)));
    assert_eq!(board["complete"], true, "done_when.complete: {board}");
    assert_eq!(
        board["outstanding"],
        serde_json::json!([]),
        "done_when.outstanding: {board}"
    );
    // `answer_at` — the newest message is the answer, which is what the field says.
    let messages = board["messages"].as_array().unwrap();
    assert_eq!(
        messages.last().unwrap()["body"],
        "here it is",
        "answer_at `messages[-1]`: {board}"
    );
}

#[test]
fn the_stopped_predicate_is_a_field_of_the_same_payload() {
    // The other terminal state, and the reason `stopped_when` exists at all: a loop that waits for
    // `done_when` alone hangs forever on a round that ended without an answer. A hand-back is one,
    // and until this item `threads show --json` did not carry the fact at all — the caller could
    // read `complete: false` forever over a round that had already stopped.
    //
    // **Its own workspace**, and that is not tidiness: under `NXF_DETERMINISTIC_IDS` a thread id and
    // a message id are both `m-` plus a per-table counter, so the second thread of a workspace
    // collides with its second message — and `reply`'s target resolution reads a message id first.
    // A fixture that opens one thread cannot construct that collision.
    let tmp = workspace();
    let receipt =
        json_of(human(&tmp).args(["--json", "send", "--to", "coder", "--no-ref", "and this"]));
    let stuck = receipt["thread_id"].as_str().unwrap().to_string();
    let coder = receipt["session"].as_str().unwrap().to_string();
    persona(&tmp, "coder", &coder)
        .args([
            "reply",
            "--thread",
            &stuck,
            "--escalate",
            "I cannot reach the repository",
        ])
        .assert()
        .success();
    let board = json_of(human(&tmp).args(["--json", "threads", "show", &stuck]));
    assert_eq!(
        board["escalated"], true,
        "stopped_when.escalated is a field of the payload `poll` produces, not a word in prose: \
         {board}"
    );
    assert_eq!(
        board["stale"], false,
        "…and it is stopped by the hand-back ALONE — `stopped_when` is an OR, so a regression that \
         narrowed it to an AND would need this board to be stale as well: {board}"
    );
}

#[test]
fn the_other_side_of_stopped_when_is_a_window_that_ran_out() {
    // The second arm of the OR, driven from the side the escalation test cannot reach: nobody
    // answered at all, and the declared window passed. A caller looping on `done_when` alone waits
    // forever here — which is the whole reason `stopped_when` exists — and `stale` is the field
    // that says so, on the same payload `poll` produces.
    let tmp = workspace();
    let receipt = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "review",
        "--no-ref",
        "have a look",
    ]));
    let thread = receipt["thread_id"].as_str().unwrap().to_string();
    let due = receipt["await"]["deadline"].as_str().unwrap().to_string();
    assert_eq!(
        due, "2026-09-07T10:30:00Z",
        "the declared 30m window: {receipt}"
    );

    // Before the window: neither done nor stopped — the state a caller waits in.
    let board = json_of(human(&tmp).args(["--json", "threads", "show", &thread]));
    assert_eq!(board["complete"], false, "{board}");
    assert_eq!(board["stale"], false, "{board}");
    assert_eq!(board["escalated"], false, "{board}");

    // Past it, with nobody having answered: STOPPED, by the marker `deadline` told the caller to
    // expect.
    let mut past = human(&tmp);
    past.env("NXC_NOW", "2026-09-07T11:00:00Z");
    let board = json_of(past.args(["--json", "threads", "show", &thread]));
    assert_eq!(
        board["stale"], true,
        "stopped_when.stale, read off the same payload: {board}"
    );
    assert_eq!(
        board["complete"], false,
        "…and `done_when` never becomes true on its own, which is the hang this prevents: {board}"
    );
}

#[test]
fn reply_gets_the_same_treatment_because_whoever_hands_a_turn_back_is_in_the_same_position() {
    let tmp = workspace();
    let receipt = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "--no-ref", "go"]));
    let thread = receipt["thread_id"].as_str().unwrap().to_string();
    let coder = receipt["session"].as_str().unwrap().to_string();

    let answered = json_of(
        persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &thread, "done"]),
    );
    let guidance = &answered["await"];
    assert_eq!(
        guidance["how"], "resume",
        "the replier here IS a registered persona: {answered}"
    );
    assert_eq!(guidance["answer_at"], "messages[-1]");
    assert_eq!(
        guidance["done_when"],
        serde_json::json!({ "complete": true, "outstanding": [] }),
        "one promise, both verbs: {answered}"
    );

    // And the human form carries the block under the line it always printed.
    let out = stdout_of(human(&tmp).args(["reply", "--thread", &thread, "thanks"]));
    assert!(out.starts_with("replied m-"), "{out}");
    assert!(out.contains("The answer lands in this thread."), "{out}");
}
