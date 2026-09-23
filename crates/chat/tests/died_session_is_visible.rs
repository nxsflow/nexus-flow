//! A persona session that DIED must be machine-readable as such — nxf 6j6v.mqad.
//!
//! The finding, from the `nxc` proving ground (4jgn.g90w): the freshly spawned PM session died at an
//! infrastructure error before it ever answered. The sidecar's teardown settled the debt it owed, so
//! the thread did not go quiet — but it settled it with an ORDINARY reply, and the requester read:
//!
//!     "state":"answered", "awaiting_human":true, "stale":false, "warnings":[]
//!
//! A crash and a delivered answer were the same record. The ONLY thing separating them was a message
//! body that happened to begin with `sidecar:` — prose, in the place `nxc guide limits-and-safety`
//! tells an app to branch on a field.
//!
//! Two halves, and the tests here are the ENGINE half:
//!
//! * the sidecar posts its teardown reply with `--escalate` when (and only when) the session died —
//!   held by `agent-sidecar/test/main-teardown.test.mjs`, where the argv is observable;
//! * `nxc status` carries that fact as a FIELD, which is what a caller branches on. That is
//!   `StatusThread::escalated`, below.
//!
//! The second DoD point — such an abort is not folded into a `summarize` channel's summary — needs
//! nothing new: `--escalate` is what the pass-through rule already keys on, and
//! `channel_consolidator.rs`'s `a_fold_channel_whose_member_escalates_delivers_the_answers_instead_of_a_summary`
//! is that rule end to end. Choosing the carrier the engine already honours is the whole reason it
//! is the carrier.

mod common;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

const NOW: &str = "2026-08-22T10:00:00Z";
const ORIGIN: &str = "local";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", ORIGIN)
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

fn text_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn write_role(tmp: &TempDir, handle: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
    )
    .unwrap();
}

/// The human commissions `pm` — the shape the proving ground's first `nxc send` had, and the one
/// that declares the obligation the teardown reply below settles.
fn summon(tmp: &TempDir, handle: &str) -> String {
    write_role(tmp, handle);
    json_of(
        human(tmp)
            .arg("--json")
            .args(["send", "--to", handle, "<the product idea>"]),
    )["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string()
}

/// The teardown reply the sidecar posts when the session it was running died — verbatim the call
/// `postFailureReply` now makes, `--escalate` included.
fn teardown_after_a_death(tmp: &TempDir, handle: &str, session: &str, thread: &str) {
    persona(tmp, handle, session)
        .arg("--json")
        .args([
            "reply",
            "--thread",
            thread,
            "--if-unanswered",
            "--escalate",
            "sidecar: this session ended without answering — Error: Failed to authenticate",
        ])
        .assert()
        .success();
}

fn root_thread(report: &Value, thread: &str) -> Value {
    report["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|op| op["threads"].as_array().expect("threads").iter())
        .find(|t| t["thread_id"] == thread)
        .cloned()
        .unwrap_or_else(|| panic!("thread {thread} is in the report: {report}"))
}

#[test]
fn a_session_that_died_is_a_field_on_status_not_a_prefix_in_the_message_body() {
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_death(&tmp, "pm", "s-pm", &thread);

    let report = json_of(
        human(&tmp)
            .arg("--json")
            .args(["status", "--thread", &thread]),
    );
    let t = root_thread(&report, &thread);

    assert_eq!(
        t["escalated"], true,
        "the crash is a FIELD a dashboard branches on, which is the whole item: {report}"
    );
    // The two fields that made a crash indistinguishable from a delivered answer are unchanged —
    // deliberately. An escalating reply discharges the turn exactly like a finished one (6j6v.cg8g),
    // and the operation IS back with its human. What was missing is the third fact, not a different
    // answer to the first two.
    assert_eq!(t["state"], "answered", "{report}");
    assert_eq!(t["awaiting_human"], true, "{report}");
}

#[test]
fn a_round_that_was_really_answered_is_not_marked_escalated() {
    // The other side of the same field: it has to be quiet on the ordinary path, or an app that
    // branches on it treats every finished operation as a failure.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    persona(&tmp, "pm", "s-pm")
        .arg("--json")
        .args(["reply", "--thread", &thread, "here is the plan"])
        .assert()
        .success();

    let report = json_of(
        human(&tmp)
            .arg("--json")
            .args(["status", "--thread", &thread]),
    );
    let t = root_thread(&report, &thread);
    assert_eq!(t["escalated"], false, "{report}");
    assert_eq!(t["state"], "answered", "{report}");
}

#[test]
fn a_round_nobody_has_answered_yet_is_not_marked_escalated_either() {
    // "Nothing has come back" is not "something was handed back" — `state` is what says the round is
    // still open, and this field must not double as a second spelling of it.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");

    let report = json_of(
        human(&tmp)
            .arg("--json")
            .args(["status", "--thread", &thread]),
    );
    let t = root_thread(&report, &thread);
    assert_eq!(t["escalated"], false, "{report}");
    assert_eq!(t["state"], "open", "{report}");
}

#[test]
fn the_terminal_line_for_a_died_session_never_says_answered() {
    // The same fact where a terminal reader looks. Before this the human line read
    // `awaiting you (answered by local/pm)` over a session that had died at an authentication
    // error — the one word in that sentence that was false is the one it turned on.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_death(&tmp, "pm", "s-pm", &thread);

    let rendered = text_of(human(&tmp).args(["status", "--thread", &thread]));
    assert!(
        rendered.contains("HANDED BACK"),
        "the crash has to be visible without --json too:\n{rendered}"
    );
    assert!(
        !rendered.contains("answered by"),
        "and the word `answered` must not appear over a round nobody answered:\n{rendered}"
    );
}
