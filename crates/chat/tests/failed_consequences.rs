//! **The defined, readable place a failed consequence surfaces** (nxf 6j6v.93zd; the DoD of nxf
//! 6j6v.hq71: "Es gibt EINEN definierten, lesbaren Ort, an dem eine gescheiterte Konsequenz
//! auftaucht — nicht eine Brotkrume auf stderr. Ein Test je Klasse.").
//!
//! The design §4.3 states the price of loose coupling: *"if a reply should advance a step and the
//! advance fails, the answering party acknowledged long ago. The answer arrived, the consequence did
//! not — and nobody learns of it."* §10.5 proves it at the source: a step whose triggering fails is
//! NOT retried, it is reported as skipped with a breadcrumb on stderr, and that is all.
//!
//! Two places answer that, and this suite drives the second one — **the synchronous report**. The
//! channel supervisor runs in the write path of `reply` (nxf 6j6v.pf6j), so a consequence that fails
//! has an addressee: the very session that is still running the call. It reads the failure off the
//! receipt of its own `nxc --json reply`, in the same call, as `warnings`.
//!
//! The other place — the DERIVED one, for the failures that leave a dead end nobody is watching — is
//! `nxc status`'s third thread state, and it is driven in `thread_tree.rs`.
//!
//! **One test per class, and each breaks a real consequence at the real path.** Nothing here
//! constructs a finding or asserts a struct field the test set itself:
//!
//! | class | how the consequence is broken here |
//! | --- | --- |
//! | a skipped step | the worker cannot start anything, so the supervisor's next turn reaches no member |
//! | a requester that was not woken | the same, at the moment the channel consolidates and wakes its requester |
//!
//! Two classes stood beside those and are gone with the run record (6j6v.dvyq §3) — see the note at
//! the foot of this file. One has been ADDED since and is driven elsewhere:
//! `ConsequenceClass::StepUnanswered`, a step that was started and let its window run out (nxf
//! 6j6v.rs9k), which needs a declared `timeout` and a clock rather than a broken worker and is
//! therefore driven where the flows are — `channel_flow.rs`'s pair on the lapsed middle step.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-16T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// A working `nxc`: the dry worker starts everything it is asked to start.
fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ORIGIN", "local")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

/// `nxc` acting as `handle`, optionally under the session the sidecar would have stamped.
///
/// The `_no_worker` twin is the injection this suite uses to break a consequence at the real path:
/// `LazyWorker` resolves `NXC_WORKER` on FIRST USE, so an unknown value is not a startup refusal —
/// the message is written, the obligation is registered, and the SPAWN is what fails, which is
/// exactly the shape design §10.5 describes.
fn as_role(tmp: &TempDir, handle: &str, session: Option<&str>) -> Command {
    let mut c = nxc(tmp);
    c.env("NXC_ACTOR", handle);
    if let Some(s) = session {
        c.env("NXC_SESSION", s);
    }
    c
}

/// [`as_role`] on the worker that starts nothing.
fn as_role_no_worker(tmp: &TempDir, handle: &str, session: Option<&str>) -> Command {
    let mut c = as_role(tmp, handle, session);
    c.env("NXC_WORKER", "not-a-real-worker");
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

/// The thread the supervisor opened for `handle` below `board` (nxf 6j6v.pf6j).
fn member_thread(tmp: &TempDir, board: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(board)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no member thread for {handle} under {board}"))
}

/// The `warnings` a receipt carries — always present, so an absent key is itself the failure.
fn warnings(receipt: &Value) -> Vec<Value> {
    receipt["warnings"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("`warnings` is the defined place and is never omitted: {receipt}")
        })
        .clone()
}

const PAIR: &str = "- name: pair\n  members: [reviewer-a, reviewer-b]\n";

// ---- class 1: a skipped step ------------------------------------------------------------------

#[test]
fn every_member_the_supervisor_could_not_start_is_named_not_just_the_first() {
    // The class §10.5 proves at the source, in the one place this engine still drops it: a second
    // turn hands every member of a channel its next task, and until this item ONE resume failure
    // reached the receipt (`wake_skipped`) while every further one was a breadcrumb on stderr and
    // nothing else. With two members and a worker that starts nothing, the difference between "the
    // first" and "all of them" is observable rather than argued.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "reviewer-a", "reviewer-b"], PAIR);

    // The pm commissions the board and both reviewers answer, so the turn is settled and the
    // requester may ask again.
    let pm_session = json_of(nxc(&tmp).args(["--json", "send", "--to", "pm", "plan it"]))
        ["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(as_role(&tmp, "pm", Some(&pm_session)).args([
        "--json",
        "send",
        "--to",
        "pair",
        "first look",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = member_thread(&tmp, &board, "reviewer-a");
    let b = member_thread(&tmp, &board, "reviewer-b");
    json_of(as_role(&tmp, "reviewer-a", None).args(["--json", "reply", "--thread", &a, "A1"]));
    json_of(as_role(&tmp, "reviewer-b", None).args(["--json", "reply", "--thread", &b, "B1"]));

    // The second turn, with nothing able to start. Both members are re-declared and handed a task
    // they will never be woken for.
    let again = json_of(as_role_no_worker(&tmp, "pm", Some(&pm_session)).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "second look",
    ]));
    assert_eq!(again["posted"], true, "the reply IS durable: {again}");

    let found = warnings(&again);
    assert_eq!(
        found.len(),
        2,
        "one per member the supervisor could not start — not just the first: {again}"
    );
    let threads: Vec<&str> = found
        .iter()
        .map(|w| w["thread"].as_str().unwrap())
        .collect();
    assert!(
        threads.contains(&a.as_str()) && threads.contains(&b.as_str()),
        "both member threads are named, so a caller can say WHICH step did not run: {again}"
    );
    for w in &found {
        assert_eq!(w["class"], "step_skipped", "{again}");
        assert_eq!(
            w["reason"], "trigger_failed",
            "machine-readable, at least as fine as the runtime itself distinguishes: {again}"
        );
        assert!(
            w["detail"].as_str().unwrap().contains("not-a-real-worker"),
            "…and the underlying failure for a human beside it: {again}"
        );
    }

    // The tasks really are in the members' threads and nobody is working on them — which is the
    // reason this must be reported rather than swallowed.
    let store = open_store(&tmp);
    for member in [&a, &b] {
        assert!(
            !store
                .thread_quorum(member, NOW)
                .unwrap()
                .unwrap()
                .outstanding
                .is_empty(),
            "the member owes an answer it was never asked for out loud"
        );
    }
}

// ---- class 2: a requester that was not woken ---------------------------------------------------

#[test]
fn a_requester_that_was_not_woken_is_named_in_the_receipt_of_the_reply_that_completed_the_board() {
    // hq71 §1's decisive argument for a SYNCHRONOUS supervisor, driven: the consolidation runs
    // inside this reply, so the party that could not be woken is reported to a session that is still
    // running. Asynchronously the answering session would be gone before the wake was even tried.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "reviewer-a", "reviewer-b"], PAIR);

    let pm_session = json_of(nxc(&tmp).args(["--json", "send", "--to", "pm", "plan it"]))
        ["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(as_role(&tmp, "pm", Some(&pm_session)).args([
        "--json",
        "send",
        "--to",
        "pair",
        "look at this",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = member_thread(&tmp, &board, "reviewer-a");
    let b = member_thread(&tmp, &board, "reviewer-b");
    json_of(as_role(&tmp, "reviewer-a", None).args(["--json", "reply", "--thread", &a, "lgtm"]));

    // reviewer-b's answer settles the SET, so the channel consolidates and wakes the pm — and the
    // wake is what fails.
    let settling = json_of(as_role_no_worker(&tmp, "reviewer-b", None).args([
        "--json",
        "reply",
        "--thread",
        &b,
        "fine by me",
    ]));
    assert!(
        settling["completed"].is_object(),
        "the channel DID consolidate — only the wake failed: {settling}"
    );
    assert!(settling["woke"].is_null(), "nobody was woken: {settling}");

    let found = warnings(&settling);
    assert_eq!(found.len(), 1, "{settling}");
    assert_eq!(found[0]["class"], "requester_not_woken", "{settling}");
    assert_eq!(
        found[0]["session"], pm_session,
        "the session left un-woken is named, so the caller can say WHO never heard: {settling}"
    );
    assert_eq!(found[0]["reason"], "trigger_failed", "{settling}");
}

// ---- class 3: a failed advance (and nxf 6j6v.zpv1's own scenario) -----------------------------

// A third test stood here: `an_advance_that_could_not_happen_is_named_in_the_receipt_of_the_reply
// _that_completed_the_step` — class 3, a run that stopped at a step whose board was finished
// because its declared workflow had gone away underneath it. REMOVED with the run record
// (6j6v.dvyq §3) together with the class itself. The two classes above are the two that survive
// HERE, and both are about a SESSION that was not put in motion — which is the whole of what this
// suite's injection (a worker that starts nothing) can break. The third class this engine has since
// grown is about a session that started and then said nothing (nxf 6j6v.rs9k); a broken worker
// cannot produce it, a declared window and a clock can, and `channel_flow.rs` is where it is driven.
