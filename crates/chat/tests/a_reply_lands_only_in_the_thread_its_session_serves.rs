//! **A session's reply lands only in a thread that session serves** (nxf 6j6v.f5rf).
//!
//! Measured three times in this workspace and once in another: a reviewer session started for one
//! review round posted its verdict — and its send-back mark — into a DIFFERENT round of the same
//! three reviewers. The transcript of the 2026-10-27 case (session m-01M5Y7DMJMYVYHFYB9FWCYFXAZ,
//! started for the 3eqa slot) shows how: its wake said "2 conversations are open for you", because
//! the addressee list was derived from the persona HANDLE across every open board in the workspace,
//! and the queued wxz0 design round owed an answer from the same handle. The agent answered both,
//! with an explicit `--thread` each time, and nothing checked that the second thread was not its
//! own.
//!
//! So the binding is the engine's, not the agent's choice of id:
//!
//! 1. the wake names only the threads THIS session serves, and an id-free `nxc reply` resolves
//!    among those alone — never to another session's slot of the same handle;
//! 2. `nxc reply --thread T` from a session is refused when the session does not serve T;
//! 3. a send-back mark rides on its reply, so it lands on the reply's own thread or nowhere.
//!
//! The real `nxc` binary against the dry worker, two rounds of the same three reviewers open at
//! once — the field shape — and nothing built by hand.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-10-29T10:00:00Z";

const REVIEW: &str = "- name: review\n  members: [code-quality, test-quality, integrity]\n";

const MEMBERS: [&str; 3] = ["code-quality", "test-quality", "integrity"];

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in MEMBERS.iter().chain(&["coder", "helper"]) {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(roles.join("channels.yaml"), REVIEW).unwrap();
    tmp
}

/// The human at the keyboard: no session, so nothing binds it to a thread.
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

/// A spawned reviewer session: the stamps `SidecarWorker` puts into a triggered role's environment.
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

fn stderr_of_failure(cmd: &mut Command) -> String {
    let out = cmd.assert().failure();
    String::from_utf8_lossy(&out.get_output().stderr).to_string()
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Every start the dry worker logged, as (role, session, wake text). The wake runs to the next
/// `trigger role=` line, because `msg=` is last on the line and carries real newlines.
fn triggers(tmp: &TempDir) -> Vec<(String, String, String)> {
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    log.split("trigger role=")
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| {
            let role = chunk.split_whitespace().next().unwrap_or_default();
            let session = chunk
                .split_whitespace()
                .find_map(|w| w.strip_prefix("session="))
                .unwrap_or_default();
            let msg = chunk
                .split_once(" msg=")
                .map(|(_, m)| m)
                .unwrap_or_default();
            (role.to_string(), session.to_string(), msg.to_string())
        })
        .collect()
}

/// One round of the review channel, opened by the human: its board, and per member the slot and
/// the session the engine started for it.
struct Round {
    board: String,
    slots: Vec<(String, String, String)>,
}

impl Round {
    fn open(tmp: &TempDir, what: &str) -> Round {
        let board = json_of(human(tmp).args(["--json", "send", "--to", "review", what]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
        let store = open_store(tmp);
        let slots = MEMBERS
            .iter()
            .map(|member| {
                let want = format!("local/{member}");
                let slot = store
                    .supervised_children(&board)
                    .unwrap()
                    .into_iter()
                    .find(|t| {
                        store
                            .thread_quorum(t, NOW)
                            .unwrap()
                            .is_some_and(|q| q.expects == [want.clone()])
                    })
                    .unwrap_or_else(|| panic!("no slot for {member} under {board}"));
                let session = triggers(tmp)
                    .into_iter()
                    .filter(|(role, _, _)| role == member)
                    .map(|(_, s, _)| s)
                    .find(|s| store.session_thread(s).unwrap().as_deref() == Some(slot.as_str()))
                    .unwrap_or_else(|| panic!("no session started for {member}'s slot {slot}"));
                (member.to_string(), slot, session)
            })
            .collect();
        Round { board, slots }
    }

    fn slot(&self, member: &str) -> &str {
        &self.slots.iter().find(|(m, _, _)| m == member).unwrap().1
    }

    fn session(&self, member: &str) -> &str {
        &self.slots.iter().find(|(m, _, _)| m == member).unwrap().2
    }
}

fn messages_in(tmp: &TempDir, thread: &str) -> usize {
    open_store(tmp).messages_in_thread(thread).unwrap().len()
}

/// Two rounds of the same three reviewers, open at once — the field shape.
fn two_rounds(tmp: &TempDir) -> (Round, Round) {
    let a = Round::open(tmp, "review change A");
    let b = Round::open(tmp, "review change B");
    (a, b)
}

/// Round A's three reviewers answer, each into its own slot, so A is consolidated and over.
fn finish(tmp: &TempDir, round: &Round) {
    for member in MEMBERS {
        json_of(persona(tmp, member, round.session(member)).args([
            "--json",
            "reply",
            "--thread",
            round.slot(member),
            &format!("{member}: grade B"),
        ]));
    }
}

#[test]
fn a_reviewer_is_woken_naming_only_its_own_slot_while_another_round_of_its_handle_is_open() {
    // The cause, as the transcript shows it: the session of the later round was told two
    // conversations were open, and the second was the earlier round's slot of the same handle.
    let tmp = workspace();
    let (a, b) = two_rounds(&tmp);
    let wake = triggers(&tmp)
        .into_iter()
        .find(|(_, s, _)| s == b.session("test-quality"))
        .map(|(_, _, msg)| msg)
        .expect("B's test-quality session was started");
    assert!(
        wake.contains(b.slot("test-quality")),
        "the wake names the slot the session was started for: {wake}"
    );
    assert!(
        !wake.contains(a.slot("test-quality")),
        "…and never round A's slot of the same handle, which another session serves: {wake}"
    );
    assert!(
        wake.contains("needs no thread id"),
        "with one conversation of its own open, the id may be left out: {wake}"
    );
}

#[test]
fn a_late_id_free_reply_from_a_finished_round_is_refused_and_writes_nothing_into_the_other() {
    let tmp = workspace();
    let (a, b) = two_rounds(&tmp);
    finish(&tmp, &a);
    let before = messages_in(&tmp, b.slot("test-quality"));

    let err = stderr_of_failure(
        persona(&tmp, "test-quality", a.session("test-quality"))
            .args(["reply", "-"])
            .write_stdin("a late verdict about change A"),
    );

    assert!(
        err.contains(a.slot("test-quality")),
        "the refusal names the thread this session serves: {err}"
    );
    assert!(
        !err.contains(b.slot("test-quality")),
        "…and never offers round B's slot of the same handle as somewhere to answer: {err}"
    );
    assert_eq!(
        messages_in(&tmp, b.slot("test-quality")),
        before,
        "nothing is written into round B"
    );
}

#[test]
fn a_reply_naming_a_thread_the_session_does_not_serve_is_refused() {
    let tmp = workspace();
    let (a, b) = two_rounds(&tmp);
    finish(&tmp, &a);
    let before = messages_in(&tmp, b.slot("test-quality"));

    let err = stderr_of_failure(
        persona(&tmp, "test-quality", a.session("test-quality"))
            .args([
                "reply",
                "--thread",
                b.slot("test-quality"),
                "--needs-rework",
                "-",
            ])
            .write_stdin("Send back: yes"),
    );

    assert!(
        err.contains(b.slot("test-quality")) && err.contains(a.slot("test-quality")),
        "the refusal names the thread asked for and the one this session serves: {err}"
    );
    assert_eq!(
        messages_in(&tmp, b.slot("test-quality")),
        before,
        "neither the verdict nor its send-back mark reaches round B"
    );
}

#[test]
fn a_reply_naming_another_open_slot_is_refused_while_the_sessions_own_round_still_runs() {
    // The 2026-10-27 shape exactly: both rounds still open, the agent naming the other slot itself.
    let tmp = workspace();
    let (a, b) = two_rounds(&tmp);
    let before = messages_in(&tmp, a.slot("test-quality"));

    stderr_of_failure(
        persona(&tmp, "test-quality", b.session("test-quality")).args([
            "reply",
            "--thread",
            a.slot("test-quality"),
            "--needs-rework",
            "verdict about change B",
        ]),
    );

    assert_eq!(messages_in(&tmp, a.slot("test-quality")), before);
    let store = open_store(&tmp);
    let q = store
        .thread_quorum(a.slot("test-quality"), NOW)
        .unwrap()
        .unwrap();
    assert_eq!(
        q.outstanding,
        vec!["local/test-quality".to_string()],
        "round A's slot still owes its own reviewer's answer — a stranger's did not discharge it"
    );
}

#[test]
fn the_sessions_own_round_takes_its_answer_and_its_mark_as_before() {
    let tmp = workspace();
    let (a, b) = two_rounds(&tmp);
    finish(&tmp, &a);

    // Id-free: the one conversation this session serves.
    let receipt = json_of(
        persona(&tmp, "test-quality", b.session("test-quality"))
            .args(["--json", "reply", "--needs-rework", "-"])
            .write_stdin("B needs another pass"),
    );
    assert!(receipt["message_id"].is_string(), "posted: {receipt}");

    let store = open_store(&tmp);
    let in_b = store.messages_in_thread(b.slot("test-quality")).unwrap();
    let last = in_b.last().expect("B's slot has the answer");
    assert_eq!(last.body, "B needs another pass");
    assert_eq!(
        last.kind,
        nexus_chat::model::KIND_NEEDS_REWORK,
        "the send-back mark travels with the reply that carries it"
    );
    for member in MEMBERS {
        assert!(
            store
                .messages_in_thread(a.slot(member))
                .unwrap()
                .iter()
                .all(|m| m.body != "B needs another pass"),
            "nothing of it lands in round A's {member} slot"
        );
    }
    assert!(
        !b.board.is_empty() && !a.board.is_empty(),
        "both rounds were really opened"
    );
}

#[test]
fn a_human_at_the_terminal_may_still_answer_any_thread() {
    // Outside any session nothing binds the caller to a thread, and nothing changes for it.
    let tmp = workspace();
    let (_a, b) = two_rounds(&tmp);
    let before = messages_in(&tmp, b.slot("integrity"));
    json_of(human(&tmp).args([
        "--json",
        "reply",
        "--thread",
        b.slot("integrity"),
        "a note from the owner",
    ]));
    assert_eq!(messages_in(&tmp, b.slot("integrity")), before + 1);
}

#[test]
fn a_session_resumed_into_its_own_consultation_still_answers_its_commission() {
    // The binding must not cost the shape the id-free form was built around (nxf 6j6v.dq59): the
    // session's POSITION is the consultation once the answer resumes it there, while the thread it
    // owes is the one it was commissioned on. It serves both.
    let tmp = workspace();
    let commission = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build it"]));
    let own = commission["thread_id"].as_str().unwrap().to_string();
    let coder = commission["session"].as_str().unwrap().to_string();
    let consult = json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "send",
        "--to",
        "helper",
        "which hook owns the retry?",
    ]));
    let asked = consult["thread_id"].as_str().unwrap().to_string();
    let helper = consult["session"].as_str().unwrap().to_string();
    json_of(persona(&tmp, "helper", &helper).args([
        "--json",
        "reply",
        "--thread",
        &asked,
        "the ladder's",
    ]));
    assert_eq!(
        open_store(&tmp).session_thread(&coder).unwrap().as_deref(),
        Some(asked.as_str()),
        "the answer resumed the coder into its consultation — the shape under test"
    );

    let before = messages_in(&tmp, &own);
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &own,
        "built, the ladder owns the retry",
    ]));
    assert_eq!(
        messages_in(&tmp, &own),
        before + 1,
        "the commission it was started for takes its answer"
    );

    // …and the helper, which serves only the consultation, may not answer the coder's commission.
    stderr_of_failure(persona(&tmp, "helper", &helper).args([
        "reply",
        "--thread",
        &own,
        "done on the coder's behalf",
    ]));
    assert_eq!(messages_in(&tmp, &own), before + 1);
}
