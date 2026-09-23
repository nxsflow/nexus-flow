//! **A step whose target is a whole CHANNEL is not answered until that channel has DELIVERED**
//! (nxf 6j6v.pn0s) — so `next:` fires after the consolidation, never beside it.
//!
//! # What was observed, and why this suite starts by measuring rather than by fixing
//!
//! `watch-bundestag`, 2026-09-08, operation `m-01M20XBQW3MBEY7325ZZ2JFEDY`. Its declaration runs
//! `judge` (target: the `review` channel, three members, `expects: all`, `on_complete: summarize`)
//! with `next: finish`. A minute-resolution monitor transcript of that run reads:
//!
//! ```text
//! 18.6 min  coder:ended · verifier:ended · code-quality/test-quality/integrity: open/running
//! 19.6 min  ... the three still open/running ... · finisher: open/running
//! 20.6 min  the three: answered/ended · __synth__: open/running
//! 22.1 min  __synth__: answered/ended · finisher: answered/ended
//! ```
//!
//! The finisher and the review fan appear to run side by side, and the finisher itself reported
//! reading the review branch as `answered` while two of its children were still running and the
//! third read `ORPHANED`. The ticket was filed as an OBSERVATION and not as a diagnosis, on purpose:
//! the previous conclusion drawn from this same run (nxf 6j6v.dxy0) was wrong, and was closed as a
//! misdiagnosis. So what is written here is a measurement of the declared shape, and it is worth
//! keeping whichever way it comes out — a `next:` that already waits needs a test that says so, or
//! the run advances early tomorrow and nothing reddens.
//!
//! # The two entrances that could advance a flow, and both are driven
//!
//! A set is reconsidered from exactly two places: a REPLY landing on one of its slots
//! (`supervisor_after_reply`) and a TICK on the board (`nxc tick`, what a declared `timeout:`
//! schedules). The live run's finisher started a minute after the fan opened with no reply of the
//! outer set in between, so the tick is the entrance under suspicion and it is driven explicitly —
//! a suite that only replies would measure half the machine.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-09-08T10:00:00Z";

const REVIEWERS: [&str; 3] = ["code-quality", "test-quality", "integrity"];

/// The measured shape: a role step, a CHANNEL step with a `next:` after it, and a role step to
/// land on. The `timeout:` is what puts a clock on the board at all — without one the tick has
/// nothing to fire against and half the question cannot be asked.
const NESTED: &str = "- name: coding\n  members: [coder, review, finisher]\n  timeout: 20m\n  \
                      steps:\n    \
                      - id: build\n      target: coder\n      next: judge\n    \
                      - id: judge\n      target: review\n      next: finish\n    \
                      - id: finish\n      target: finisher\n\
                      - name: review\n  members: [code-quality, test-quality, integrity]\n  \
                      on_complete: summarize\n  \
                      summary_prompt: Fold the three reviews into one verdict.\n";

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
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

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

/// Which roles the engine has started, in start order — the observable this whole item is about.
fn started_roles(tmp: &TempDir) -> Vec<String> {
    trigger_lines(tmp)
        .iter()
        .map(|l| field(l, "role"))
        .collect()
}

fn session_of(tmp: &TempDir, role: &str) -> String {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role} in {:?}", trigger_lines(tmp)))
}

fn slot_for(tmp: &TempDir, parent: &str, handle: &str) -> String {
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

/// The `judge` slot: the nested channel's OWN thread, told apart from a role slot by the channel it
/// lives in — the same discriminator `slot_target` uses.
fn channel_slot(tmp: &TempDir, parent: &str, channel: &str) -> String {
    let store = open_store(tmp);
    let want = format!("decl:{channel}");
    store
        .supervised_children(parent)
        .unwrap()
        .into_iter()
        .find(|t| store.thread_channel(t).as_deref() == Some(want.as_str()))
        .unwrap_or_else(|| panic!("no {want} slot under {parent}"))
}

/// Run `coding` up to the point the finding is about: `build` answered, `judge` open, the three
/// reviewers commissioned and none of them finished. Hands back the board and the `judge` slot.
fn up_to_the_open_review(tmp: &TempDir) -> (String, String) {
    write_roles(
        tmp,
        &[
            "pm",
            "coder",
            "finisher",
            "code-quality",
            "test-quality",
            "integrity",
        ],
        NESTED,
    );
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
    let build = slot_for(tmp, &board, "coder");
    json_of(persona(tmp, "coder", &session_of(tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));
    let judge = channel_slot(tmp, &board, "review");
    assert_eq!(
        started_roles(tmp),
        vec!["pm", "coder", "code-quality", "test-quality", "integrity"],
        "the channel step fanned out to all three reviewers and nothing else has started"
    );
    (board, judge)
}

#[test]
fn the_step_after_a_channel_step_does_not_start_while_that_channel_is_still_working() {
    // The measurement the ticket asks for, in the order it asks for it: is `finish` started while
    // the review fan is open? Asked after each of the three answers, and after a TICK on the board,
    // which is the entrance the live run's timing points at.
    let tmp = workspace();
    let (board, _judge) = up_to_the_open_review(&tmp);

    let threads: Vec<String> = REVIEWERS
        .iter()
        .map(|who| slot_for(&tmp, &channel_slot(&tmp, &board, "review"), who))
        .collect();

    json_of(human(&tmp).args(["--json", "tick", "--thread", &board]));
    assert!(
        !started_roles(&tmp).contains(&"finisher".to_string()),
        "a tick on the board with the review round wide open must not release `next:` — {:?}",
        started_roles(&tmp)
    );

    // One member answers, then two: a set with members of different speeds, which is exactly the
    // shape the ticket asks to be built.
    for (i, (who, thread)) in REVIEWERS.iter().zip(&threads).enumerate() {
        json_of(persona(&tmp, who, &session_of(&tmp, who)).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "my verdict",
        ]));
        json_of(human(&tmp).args(["--json", "tick", "--thread", &board]));
        assert!(
            !started_roles(&tmp).contains(&"finisher".to_string()),
            "with {} of 3 reviewers in, `finish` must still be unstarted — {:?}",
            i + 1,
            started_roles(&tmp)
        );
    }

    // The third answer settled the set, so `#review` FOLDS: its consolidator is what runs now, and
    // the outer flow is still waiting on a delivery that does not exist yet.
    assert!(
        started_roles(&tmp).contains(&"__synth__".to_string()),
        "the settled set spawned the consolidator: {:?}",
        started_roles(&tmp)
    );
    assert!(
        !started_roles(&tmp).contains(&"finisher".to_string()),
        "and `finish` waits for the FOLD, not for the fan: {:?}",
        started_roles(&tmp)
    );
}

#[test]
fn the_channel_steps_own_delivery_is_what_releases_the_next_step() {
    // The other half of the same claim: once the fold lands, `next:` really does fire — so the
    // waiting above is a wait and not a stall.
    let tmp = workspace();
    let (board, judge) = up_to_the_open_review(&tmp);
    let review_thread = channel_slot(&tmp, &board, "review");
    for who in REVIEWERS {
        let thread = slot_for(&tmp, &review_thread, who);
        json_of(persona(&tmp, who, &session_of(&tmp, who)).args([
            "--json",
            "reply",
            "--thread",
            &thread,
            "my verdict",
        ]));
    }
    assert_eq!(
        judge, review_thread,
        "the `judge` slot IS the channel thread"
    );

    let synth = session_of(&tmp, "__synth__");
    json_of(persona(&tmp, "__synth__", &synth).args([
        "--json",
        "reply",
        "--thread",
        &judge,
        "one verdict: ship it",
    ]));
    assert_eq!(
        started_roles(&tmp)
            .into_iter()
            .filter(|r| r == "finisher")
            .count(),
        1,
        "the fold discharged the channel step, so `next:` fired — once: {:?}",
        started_roles(&tmp)
    );
}

#[test]
fn a_channel_step_reads_as_open_until_it_has_delivered() {
    // The state the finisher reported reading — the review branch as `answered` while its children
    // were still working — asked of the record directly. A channel step thread expects the engine's
    // own supervisor identity, and if the supervisor's own opening message counted as that
    // identity's reply the slot would read `complete` from the instant it was minted, which is what
    // `member_set_is_settled` reads to decide the outer set.
    let tmp = workspace();
    let (board, judge) = up_to_the_open_review(&tmp);
    let store = open_store(&tmp);
    let q = store.thread_quorum(&judge, NOW).unwrap().unwrap();
    assert!(
        !q.complete,
        "the channel step is not answered while its own round is running: {q:?}"
    );
    assert!(
        !q.outstanding.is_empty(),
        "…and it still names who owes it an answer: {q:?}"
    );
    let board_q = store.thread_quorum(&board, NOW).unwrap().unwrap();
    assert!(
        !board_q.complete,
        "…so the operation is not finished either: {board_q:?}"
    );
}

#[test]
fn the_review_branch_reads_open_and_an_early_answer_is_no_orphan() {
    // **What ORPHANED meant on a reviewer child, answered by measurement** (the ticket's third
    // question). The finisher of the live run read the review branch as `answered` with two members
    // still working and the third `ORPHANED`, and both readings came from the same false
    // `complete`: `ThreadState::Orphaned` asks, among other things, whether this thread's PARENT
    // was asked and answered — so a parent that read discharged from birth made every child that
    // had already replied look like a dead end. It was a SYMPTOM of the same defect, not a second
    // one, and this is what says so.
    let tmp = workspace();
    let (board, judge) = up_to_the_open_review(&tmp);
    let early = slot_for(&tmp, &judge, "code-quality");
    json_of(
        persona(&tmp, "code-quality", &session_of(&tmp, "code-quality"))
            .args(["--json", "reply", "--thread", &early, "grade A"]),
    );

    let op = json_of(human(&tmp).args(["--json", "status", "--thread", &board]))["operations"][0]
        .clone();
    let state_of = |id: &str| -> String {
        op["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["thread_id"] == id)
            .unwrap_or_else(|| panic!("{id} is not in the operation: {op}"))["state"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        state_of(&judge),
        "open",
        "the review branch is running, and the view has to say so: {op}"
    );
    assert_eq!(
        state_of(&early),
        "answered",
        "the member that answered is answered — not a dead end nobody picked up: {op}"
    );
    assert!(
        op["live"].as_bool().unwrap(),
        "and the operation is still going: {op}"
    );
}
