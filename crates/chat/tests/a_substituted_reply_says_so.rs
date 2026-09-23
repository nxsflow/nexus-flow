//! **A reply the RUNTIME wrote is distinguishable from one the agent wrote** — nxf 6j6v.kffm, DoD
//! point 2, and the twin of `died_session_is_visible.rs` one step further on.
//!
//! When a spawned session ends still owing its thread an answer, the sidecar's teardown posts one in
//! its name so the thread does not go quiet (nxf 6j6v.7e9d). Until this item, the ONLY thing
//! separating that from an answer the agent actually wrote was a message body beginning `sidecar:` —
//! prose, in the place `nxc guide limits-and-safety` tells an app to branch on a field.
//!
//! That is not a hypothetical. It is what let nxf 6j6v.kffm's whole chain run green: a role that
//! declared no `tools:` could not execute the `nxc reply` its own system prompt ordered, the teardown
//! answered for it, and the acceptance run's first smoke reported a healthy round in which the
//! session had never reached the model. Only reading the answer's TEXT gave it away.
//!
//! **`escalated` does not cover it, in either direction**, which is why this is a second field and
//! not a wider reading of the first:
//!
//! | | escalated | substituted |
//! |---|---|---|
//! | the agent said "I cannot" | yes | no |
//! | the session DIED and the teardown said so | yes | yes |
//! | the session ended quietly without answering | no | yes |
//! | the agent answered | no | no |
//!
//! The third row is the one this file exists for: nothing went wrong, so marking it as an escalation
//! would wrongly tell a supervisor to stop — and yet nobody answered.
//!
//! Everything here drives the REAL `nxc` binary with the argv `agent-sidecar/src/main.mjs`'s
//! `postFailureReply` actually builds, because `--if-unanswered` is that teardown's door and is
//! deliberately not on the app seam at all (`surface::settle_if_unanswered`).

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

const NOW: &str = "2026-08-30T10:00:00Z";

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
    serde_json::from_str(
        String::from_utf8_lossy(&out.get_output().stdout)
            .into_owned()
            .trim(),
    )
    .expect("valid json")
}

fn text_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

/// The trap's own role file: NO `tools:` key. That is the default, and what every role written
/// before the role runtime looks like — the state whose auto-approval list was empty, so its own
/// ordered `nxc reply` was refused and this teardown ran in its place.
fn summon(tmp: &TempDir, handle: &str) -> String {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
    )
    .unwrap();
    json_of(
        human(tmp)
            .arg("--json")
            .args(["send", "--to", handle, "<the task>"]),
    )["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string()
}

/// Verbatim the call `postFailureReply` makes for a session that ended without ever answering and
/// without anything having gone wrong — the row of the table this file exists for.
fn teardown_after_a_quiet_ending(tmp: &TempDir, handle: &str, session: &str, thread: &str) {
    persona(tmp, handle, session)
        .arg("--json")
        .args([
            "reply",
            "--thread",
            thread,
            "--if-unanswered",
            "sidecar: this session ended without ever posting a reply",
        ])
        .assert()
        .success();
}

/// …and the one for a session that DIED, which carries `--escalate` as well (nxf 6j6v.mqad).
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

fn thread_row(report: &Value, thread: &str) -> Value {
    report["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|op| op["threads"].as_array().expect("threads").iter())
        .find(|t| t["thread_id"] == thread)
        .cloned()
        .unwrap_or_else(|| panic!("thread {thread} is in the report: {report}"))
}

fn status(tmp: &TempDir, thread: &str) -> Value {
    let report = json_of(
        human(tmp)
            .arg("--json")
            .args(["status", "--thread", thread]),
    );
    thread_row(&report, thread)
}

#[test]
fn a_round_the_runtime_answered_for_the_agent_says_so_as_a_field() {
    // The row nothing could see before: the session ended, nothing crashed, and NOBODY ANSWERED.
    // Every other field reads exactly like a finished round, and deliberately so — the expectation
    // really is discharged and the operation really is back with its human. This is the third fact.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_quiet_ending(&tmp, "pm", "s-pm", &thread);

    let t = status(&tmp, &thread);
    assert_eq!(
        t["substituted"], true,
        "the substitution is a FIELD an app branches on, which is the whole DoD point: {t}"
    );
    assert_eq!(
        t["escalated"], false,
        "and it is NOT an escalation — nothing went wrong, and marking it would tell a supervisor \
         to stop: {t}"
    );
    assert_eq!(t["state"], "answered", "{t}");
    assert_eq!(t["awaiting_human"], true, "{t}");
}

#[test]
fn an_answer_the_agent_wrote_itself_is_not_marked_substituted() {
    // The other side of the field: it has to be quiet on the ordinary path, or an app that branches
    // on it treats every finished round as a runtime failure.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    persona(&tmp, "pm", "s-pm")
        .arg("--json")
        .args(["reply", "--thread", &thread, "here is the plan"])
        .assert()
        .success();

    let t = status(&tmp, &thread);
    assert_eq!(t["substituted"], false, "{t}");
    assert_eq!(t["state"], "answered", "{t}");
}

#[test]
fn a_live_agent_that_hands_the_task_back_escalates_without_being_a_substitution() {
    // Row one of the table. `--escalate` is what an agent says when it cannot carry the task out,
    // and reading it as "the runtime answered" would put a plumbing fault where a decision belongs.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    persona(&tmp, "pm", "s-pm")
        .arg("--json")
        .args([
            "reply",
            "--thread",
            &thread,
            "--escalate",
            "I need the API key before I can start",
        ])
        .assert()
        .success();

    let t = status(&tmp, &thread);
    assert_eq!(t["escalated"], true, "{t}");
    assert_eq!(t["substituted"], false, "{t}");
}

#[test]
fn a_session_that_died_is_both_escalated_and_substituted() {
    // Row two: the two facts are orthogonal and BOTH travel. `escalated` says a human has to decide;
    // `substituted` says the sentence they are about to read was not written by the agent.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_death(&tmp, "pm", "s-pm", &thread);

    let t = status(&tmp, &thread);
    assert_eq!(t["escalated"], true, "{t}");
    assert_eq!(t["substituted"], true, "{t}");
}

#[test]
fn a_round_nobody_has_answered_yet_is_not_marked_substituted() {
    // "Nothing has come back" is not "the runtime answered" — `state` is what says the round is
    // still open, and this field must not double as a second spelling of it.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");

    let t = status(&tmp, &thread);
    assert_eq!(t["substituted"], false, "{t}");
    assert_eq!(t["state"], "open", "{t}");
}

#[test]
fn a_teardown_that_finds_the_debt_already_settled_marks_nothing() {
    // The teardown runs UNCONDITIONALLY — it cannot see an `nxc reply` the agent ran through its own
    // Bash tool, so the ENGINE's `--if-unanswered` gate is what decides whether anything is written.
    // A call that writes nothing must therefore leave no marker on the agent's own answer, or the
    // ordinary healthy round would report itself as a substitution.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    persona(&tmp, "pm", "s-pm")
        .arg("--json")
        .args(["reply", "--thread", &thread, "here is the plan"])
        .assert()
        .success();
    teardown_after_a_quiet_ending(&tmp, "pm", "s-pm", &thread);

    let t = status(&tmp, &thread);
    assert_eq!(
        t["substituted"], false,
        "the deliberate no-op wrote nothing, so nothing is marked: {t}"
    );
}

#[test]
fn the_terminal_line_says_who_wrote_the_answer() {
    // The same fact where a terminal reader looks. Without it the line read
    // `awaiting you (answered by local/pm)` over a round in which `local/pm` never spoke — the one
    // word in that sentence that was false is the one this turns on.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_quiet_ending(&tmp, "pm", "s-pm", &thread);

    let rendered = text_of(human(&tmp).args(["status", "--thread", &thread]));
    assert!(
        rendered.contains("posted BY THE RUNTIME, not by the agent"),
        "the substitution has to be visible without --json too:\n{rendered}"
    );
}

#[test]
fn an_embedding_host_reads_the_same_fact_off_the_engine_seam() {
    // **The seam the PRODUCTS speak over** — manufakt.io and nexflow.it hold an `Engine`, they do
    // not shell out to `nxc`. A field only the CLI carries is a field they cannot branch on, which
    // is exactly the gap this whole item is about one level down.
    //
    // The WRITE stays on the CLI here on purpose: `--if-unanswered` is the sidecar teardown's door
    // and is deliberately absent from the app seam (`surface::settle_if_unanswered`'s own doc), so
    // a host never posts one. What a host does is READ what came of it.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_quiet_ending(&tmp, "pm", "s-pm", &thread);

    let engine = nexus_chat::engine::Engine::open_with(
        None,
        tmp.path(),
        nexus_chat::engine::EngineConfig {
            timer: nexus_chat::timer::TimerConfig::Dry,
            ..nexus_chat::engine::EngineConfig::default()
        },
    )
    .expect("a host opens the same workspace");
    let report = engine
        .status(
            NOW,
            nexus_chat::facade::StatusScope::Threads(&[thread.as_str()]),
        )
        .expect("the host reads the operation");
    let row = report
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .find(|t| t.thread_id == thread)
        .expect("the thread is in the report");
    assert!(
        row.substituted,
        "the host sees who wrote the answer, without reading a message body: {row:?}"
    );
    assert!(!row.escalated, "{row:?}");
}

#[test]
fn the_marker_is_on_the_message_itself_and_survives_a_reread() {
    // Where the fact actually LIVES: `refs.substituted` on the message, which is what the thread
    // field above is derived from. A consumer reading the conversation — rather than the operation
    // — gets the same answer without a second concept, and a message the agent wrote carries
    // nothing at all, so the bytes of every reply written before this existed are unchanged.
    let tmp = workspace();
    let thread = summon(&tmp, "pm");
    teardown_after_a_quiet_ending(&tmp, "pm", "s-pm", &thread);

    let messages = json_of(human(&tmp).arg("--json").args(["threads", "show", &thread]));
    let rendered = messages.to_string();
    assert!(
        rendered.contains("\"substituted\":true"),
        "the message carries the marker itself: {rendered}"
    );
}

// ---- the mark reaches the CONSOLIDATION, not just the thread ----------------------------------

#[test]
fn a_substituted_member_answer_is_marked_in_what_the_consolidator_is_handed() {
    // **Review of PR #397, Integrity & Robustness · High.** The fix above reaches `nxc status`,
    // which is what a REQUESTER branches on. It did not reach the consolidation — the path where a
    // MODEL reads the members' answers and folds them into one verdict. A quorum member whose
    // session died therefore contributed a `sidecar:` line that the synthesizer saw as an opinion
    // like any other, and this repo's own shipped quorum tells its consolidator to aggregate four
    // verdicts into one merge-or-not answer. A verdict folded out of a member that never spoke is a
    // WRONG answer, not a missing one — the same defect class as the trigger of nxf 6j6v.kffm, one
    // level further down the pipe.
    //
    // Driven end to end through the real write path rather than at the composer (which
    // `channel.rs`'s own unit tests cover): the substitution is made by the exact `nxc reply
    // --if-unanswered` the sidecar teardown runs, and what is read back is the trigger message the
    // engine actually handed the synthesizer — recorded verbatim by the dry worker.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "alice", "bob"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: review\n  members: [alice, bob]\n  on_complete: summarize\n  \
         summary_prompt: Fold their verdicts into one.\n",
    )
    .unwrap();

    let board = json_of(human(&tmp).arg("--json").args([
        "send",
        "--to",
        "review",
        "--no-ref",
        "review the diff",
    ]))["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();

    // alice answers for herself; bob's session ends without answering and the teardown speaks for it
    // — the completing message, so the consolidation runs on this reply.
    let slots = |handle: &str| -> String {
        let report = json_of(
            human(&tmp)
                .arg("--json")
                .args(["status", "--thread", &board]),
        );
        report["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .flat_map(|op| op["threads"].as_array().expect("threads").iter())
            .find(|t| {
                t["expects"].as_array().is_some_and(|e| {
                    e.iter()
                        .any(|h| h == &Value::from(format!("local/{handle}")))
                })
            })
            .map(|t| t["thread_id"].as_str().expect("thread_id").to_string())
            .unwrap_or_else(|| panic!("no member thread for {handle}: {report}"))
    };
    persona(&tmp, "alice", "s-alice")
        .arg("--json")
        .args(["reply", "--thread", &slots("alice"), "Ready to merge? yes"])
        .assert()
        .success();
    teardown_after_a_quiet_ending(&tmp, "bob", "s-bob", &slots("bob"));

    // What the engine handed the synthesizer, verbatim.
    let dry = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    let handed = dry
        .split("trigger role=")
        .find(|entry| entry.starts_with("__synth__"))
        .unwrap_or_else(|| panic!("no synthesizer was triggered; the dry log was:\n{dry}"));
    assert!(
        handed.contains("substituted=\"true\""),
        "the mark reaches the fold: {handed}"
    );
    assert!(
        handed.contains("carries no verdict"),
        "…and so does what it means, because the reader is a model: {handed}"
    );
    assert!(
        handed.contains("Ready to merge? yes"),
        "sanity: alice's real answer is in the same fold: {handed}"
    );
}

#[test]
fn an_embedding_host_sees_the_mark_reach_its_own_consolidator() {
    // **The seam the PRODUCTS speak over, for the consolidation half** (second review round of PR
    // #397, Code Quality). The test above proves the mark reaches the fold through the `nxc` binary;
    // `engine-seam-test-rule` in this project's memory is explicit that CLI coverage does not stand
    // in for it — manufakt.io and nexflow.it hold an `Engine` and install their OWN `Worker`, so
    // what they receive is a `TriggerRequest`, not a dry-log line. That seam has been the heaviest
    // review finding in this repo several times running, and this is the same shape again.
    //
    // The WRITE stays on the CLI, deliberately and unavoidably: `--if-unanswered` is the sidecar
    // teardown's door and is absent from the app seam by design (`surface::settle_if_unanswered`).
    // A host never posts one — it RECEIVES what came of it, which is exactly the split asserted
    // here: the substitution is made by the real teardown call, and the consolidation is driven and
    // observed through `Engine`.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "alice", "bob"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: review\n  members: [alice, bob]\n  on_complete: summarize\n  \
         summary_prompt: Fold their verdicts into one.\n",
    )
    .unwrap();

    // **BOTH HALVES MUST MINT THE SAME IDENTITY, and that is not automatic.**
    // `Engine::origin()` reads the WORKSPACE's own replica prefix and deliberately does not honour
    // `NXC_ORIGIN`, while this file's `human`/`persona` helpers pin the CLI to `local`. Mixed, the
    // teardown posts as `local/bob` while the host replies as `<prefix>/alice`, so the board's
    // `expects` register matches neither and the round never completes — `posted: true`,
    // `completed: None`, nothing spawned, nothing to assert on. Dropping the pin on the CLI side is
    // what makes the two agree; the handles are therefore matched by their bare suffix below rather
    // than spelled out with an origin.
    let cli = |actor: &str, session: Option<&str>| {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env_remove("NXC_ORIGIN")
            .env("NXC_ACTOR", actor)
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
        if let Some(s) = session {
            c.env("NXC_SESSION", s);
        }
        c
    };

    let board = json_of(cli("carsten", None).arg("--json").args([
        "send",
        "--to",
        "review",
        "--no-ref",
        "review the diff",
    ]))["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();
    let slot = |handle: &str| -> String {
        let report = json_of(
            cli("carsten", None)
                .arg("--json")
                .args(["status", "--thread", &board]),
        );
        let suffix = format!("/{handle}");
        report["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .flat_map(|op| op["threads"].as_array().expect("threads").iter())
            .find(|t| {
                t["expects"].as_array().is_some_and(|e| {
                    e.iter()
                        .filter_map(Value::as_str)
                        .any(|h| h.ends_with(&suffix))
                })
            })
            .map(|t| t["thread_id"].as_str().expect("thread_id").to_string())
            .unwrap_or_else(|| panic!("no member thread for {handle}: {report}"))
    };
    // bob's session ends owing an answer and the teardown speaks for it; alice has not answered yet,
    // so the set is not settled and no consolidation has run.
    let bob_slot = slot("bob");
    cli("bob", Some("s-bob"))
        .arg("--json")
        .args([
            "reply",
            "--thread",
            &bob_slot,
            "--if-unanswered",
            "sidecar: this session ended without ever posting a reply",
        ])
        .assert()
        .success();
    let alice_slot = slot("alice");

    // From here on it is the HOST: its own worker, its own handle, its own call.
    let worker = std::sync::Arc::new(Recorder::default());
    let engine = nexus_chat::engine::Engine::open_with(
        None,
        tmp.path(),
        nexus_chat::engine::EngineConfig {
            worker: nexus_chat::worker::WorkerConfig::Custom(worker.clone()),
            timer: nexus_chat::timer::TimerConfig::Dry,
            ..nexus_chat::engine::EngineConfig::default()
        },
    )
    .expect("a host opens the same workspace");
    eprintln!("STEP: engine open");
    let r = engine
        .reply_thread(
            nexus_chat::orchestration::Caller {
                session: None,
                actor: Some("alice"),
                now: Some(NOW),
            },
            nexus_chat::surface::ReplyThreadRequest {
                machine: None,
                thread: &alice_slot,
                body: "Ready to merge? yes",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("alice's answer settles the set and starts the consolidation");
    eprintln!("STEP: replied -> {r:#?}");

    let handed = worker.message_for(nexus_chat::channel::SYNTHESIS_HANDLE);
    assert!(
        handed.contains("substituted=\"true\""),
        "the host's own consolidator is handed the mark: {handed}"
    );
    assert!(
        handed.contains("carries no verdict"),
        "…and what it means, because the reader is a model: {handed}"
    );
    assert!(
        handed.contains("Ready to merge? yes"),
        "sanity: alice's real answer is in the same fold: {handed}"
    );
}

/// Records the trigger requests a host's own `Worker` is handed — the seam an embedding product
/// actually implements.
#[derive(Default)]
struct Recorder {
    seen: std::sync::Mutex<Vec<nexus_chat::worker::TriggerRequest>>,
}

impl Recorder {
    /// ONE guard — see the twin in `an_obligation_comes_with_its_means.rs` for why re-locking inside
    /// `unwrap_or_else` turns a failing lookup into a hung test binary.
    fn message_for(&self, handle: &str) -> String {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .find(|r| r.role.handle == handle)
            .map(|r| r.message.clone())
            .unwrap_or_else(|| {
                let handles: Vec<&str> = seen.iter().map(|r| r.role.handle.as_str()).collect();
                panic!("no trigger for {handle}, only: {handles:?}")
            })
    }
}

impl nexus_chat::worker::Worker for Recorder {
    fn trigger(
        &self,
        req: nexus_chat::worker::TriggerRequest,
    ) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}
