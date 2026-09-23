//! **A reply into a round the channel has already closed is REFUSED, and no receipt promises an
//! answer nobody owes** (nxf 6j6v.0vd9).
//!
//! Measured on 2026-09-08 in a live `watch-bundestag` operation, not at a desk. A `decl:coding`
//! operation was finished — every step answered, `__delivered__` set, `nxc status` reading
//! `8 thread(s), 0 open · finished`. A reply into the coder's own step thread, exactly as that
//! persona's declaration invites ("answer on your thread"), came back:
//!
//! ```text
//! replied m-01M211P1APCJSPHZJ04KEV0G1Y in thread m-01M20XBQX1FX4GBRA0NC6H2HM2
//!   The answer lands in this thread.
//!   The answer is due by 2026-09-08T22:41:34.245261Z.
//! ```
//!
//! Nothing was started, nothing could be: the supervisor had consolidated that set long before, so
//! `supervisor_consider_set` sees a channel thread that no longer expects it and does nothing —
//! and every other route out of `orchestration::reply` is closed on that thread by construction
//! (the direct resume skips a quorum board, the completion wake is gated on `!supervised`, and the
//! surface's own fallback hands the turn back only to the thread's OPENER, which here is the
//! supervisor). The owner believed a rework was commissioned for twenty minutes. An agent reading
//! that deadline would have believed it too — it is precisely the field a waiting loop keys on.
//!
//! Two claims are held here, one per half of the item:
//!
//! 1. such a reply is refused, with a reason that names the thread that DOES take another turn,
//!    and nothing is written;
//! 2. a receipt never carries a deadline while nobody owes an answer on the thread it describes.
//!
//! The harness is the one `a_declared_cycle_sends_the_work_back.rs` uses — the real `nxc` binary
//! against the dry worker, no thread, parent edge or expectation built by hand — because the whole
//! finding is about what a caller at a terminal reads after a run that really ran.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-09-08T10:00:00Z";

/// The measured declaration, reduced to what the finding needs: an ordered run that ENDS, and a
/// `timeout:` — without one no slot thread carries a deadline at all, and the deadline is half of
/// what went wrong.
const CODING: &str = "- name: coding\n  members: [coder, review, finisher]\n  timeout: 20m\n  \
                      steps:\n    \
                      - id: build\n      target: coder\n      next: check\n    \
                      - id: check\n      target: review\n      next: ship\n    \
                      - id: ship\n      target: finisher\n";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

/// A spawned role session: the stamps `SidecarWorker` puts into a triggered role's environment.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn write_roles(tmp: &TempDir, handles: &[&str], channels: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in handles {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(roles.join("channels.yaml"), channels).unwrap();
}

fn trigger_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

fn field(line: &str, key: &str) -> String {
    let marker = format!("{key}=");
    let start = line
        .find(&marker)
        .unwrap_or_else(|| panic!("no {marker} in {line}"))
        + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The newest session the engine started for `role`.
fn session_of(tmp: &TempDir, role: &str) -> String {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role} in {:?}", trigger_lines(tmp)))
}

/// **The NEWEST slot under `parent` that expects `handle`** — newest because a run may open a
/// second slot for the same role; ids are ULIDs and each slot is minted by its own `nxc` process.
fn newest_slot_for(tmp: &TempDir, parent: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(parent)
        .unwrap()
        .into_iter()
        .rfind(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no slot for {handle} under {parent}"))
}

fn messages_in(tmp: &TempDir, thread: &str) -> usize {
    open_store(tmp).messages_in_thread(thread).unwrap().len()
}

/// Run `coding` from the work order to the delivered answer, and hand back the board thread and
/// the coder's step thread — the two ends the finding is about.
fn a_finished_operation(tmp: &TempDir) -> (String, String) {
    write_roles(tmp, &["pm", "coder", "review", "finisher"], CODING);
    let pm = json_of(human(tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let build = newest_slot_for(tmp, &board, "coder");
    json_of(persona(tmp, "coder", &session_of(tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));
    let check = newest_slot_for(tmp, &board, "review");
    json_of(persona(tmp, "review", &session_of(tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "looks good",
    ]));
    let ship = newest_slot_for(tmp, &board, "finisher");
    let last = json_of(
        persona(tmp, "finisher", &session_of(tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    assert!(
        last["completed"].is_object(),
        "the last step consolidates and the operation is finished: {last}"
    );
    (board, build)
}

#[test]
fn a_reply_into_a_step_of_a_consolidated_round_is_refused_and_writes_nothing() {
    // The measured case, verbatim: the operation is finished and somebody answers on the coder's
    // own thread to pick the work back up.
    let tmp = workspace();
    let (board, build) = a_finished_operation(&tmp);
    let before = messages_in(&tmp, &build);

    let out = human(&tmp)
        .args(["reply", "--thread", &build, "one more thing, please fix X"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();

    assert!(
        err.contains("this round is over"),
        "the refusal says what state the world is in: {err}"
    );
    assert!(
        err.contains(&board),
        "…names the round this thread served, so the caller can go and read how it ended: {err}"
    );
    assert!(
        err.contains("nxc send --to coding"),
        "…and names the door that DOES start work, with the channel filled in: {err}"
    );
    assert!(
        !err.contains("due by"),
        "a refusal must not carry the deadline that caused the finding: {err}"
    );
    assert_eq!(
        messages_in(&tmp, &build),
        before,
        "a refused reply writes nothing — a message in a thread nobody reads is the litter this \
         verb used to leave behind"
    );
}

#[test]
fn the_refusal_reaches_an_app_through_the_same_door_it_reaches_the_terminal() {
    // `--json` is the agent's door, and an agent is exactly the reader that believed the deadline.
    // The refusal must be the CALL's answer there too, not a receipt with falsy fields.
    let tmp = workspace();
    let (_board, build) = a_finished_operation(&tmp);
    human(&tmp)
        .args(["--json", "reply", "--thread", &build, "pick this back up"])
        .assert()
        .failure();
}

#[test]
fn a_step_that_is_still_the_supervisors_business_still_takes_a_reply() {
    // The other side of the gate, and the reason it is drawn at the SET rather than at "this thread
    // has been answered": while the run is going the supervisor is still the party the channel
    // thread expects, so a second word on a step that already answered is ordinary traffic and must
    // not be refused. Refusing it would take `--if-unanswered`'s teardown and every mid-run
    // follow-up with it.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "review", "finisher"], CODING);
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));

    // `check` is running now; the coder's slot is answered but its SET is not consolidated.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "one correction to what I said",
    ]));
}

#[test]
fn a_receipt_names_no_deadline_once_nobody_owes_an_answer_on_the_thread() {
    // The second half of the item, and the half that did the damage. The window a channel declares
    // is what a caller waits out; printed on a thread whose register nobody stands behind, it is a
    // waiting instruction with no answer at the end of it.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "review", "finisher"], CODING);
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(&tmp, &board, "coder");

    // While the coder still owes its answer the window is real, and the thread says so.
    assert!(
        open_store(&tmp)
            .thread_quorum(&build, NOW)
            .unwrap()
            .unwrap()
            .deadline
            .is_some(),
        "the declared `timeout:` puts a window on the step thread — without one this test proves \
         nothing"
    );

    // The coder answers. Its own obligation is discharged, so nothing on this thread is owed —
    // and the receipt must stop naming a due date.
    let receipt = json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));
    assert_eq!(
        receipt["await"]["deadline"],
        Value::Null,
        "nobody owes an answer in this thread any more, so there is no instant to wait for: \
         {receipt}"
    );

    // …and the human rendering, which is where the finding was actually read.
    let out = persona(&tmp, "review", &session_of(&tmp, "review"))
        .args([
            "reply",
            "--thread",
            &newest_slot_for(&tmp, &board, "review"),
            "looks good",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        !text.contains("due by"),
        "the line a terminal reader takes for an accepted delivery must not name a window nobody \
         stands behind: {text}"
    );
}
