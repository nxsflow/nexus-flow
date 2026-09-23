//! **A step continues its own session, and the closing report is BUILT from the run's passes**
//! (nxf 6j6v.y1t9 — parts 3 and 4 of nxf 6j6v.553s).
//!
//! The first half of 553s (`6j6v.nhgw`) made the cycle declarable. It left the two things this file
//! holds, and both were MEASURED on the shipped build before anything was designed:
//!
//! * **The back edge started a FRESH session** (`resume=-` in the dry log) and handed it a notice
//!   saying *"What you handed over was reviewed"* and *"Your claim on the working copy is STILL
//!   YOURS"* — to a session that handed over nothing, holds nothing, and never learns what the
//!   original task was, because a back edge replaces the request with the findings. That is nxf
//!   6j6v.kffm's shape: a claim in the text the code does not make true.
//! * **The report was a flat list.** Six `sender: body` lines with nothing saying which step, which
//!   pass, or which answer carried the verdict that turned the run around — so that
//!   `local/coder: lock order fixed` SUPERSEDES `local/coder: first cut is in` was left for the
//!   reader to guess.
//!
//! **`resume:` is DECLARATIVE** (owner, 2026-08-31): *"aber ich wuerde es deklarativ machen. Hier
//! geht es ja um den `on_needs_rework` Pfad. Dort koennte der Standard sein `resume: true` und wenn
//! einfach nur der naechste Schritt definiert wird, der zufaelligerweise an die gleiche Persona
//! geht, waere `resume: false`, aber wir koennen bei jedem Schritt `resume` auch festlegen."*
//!
//! **Two harnesses, and the second is here because the first cannot answer its question.** The
//! routing tests drive the real `nxc` binary against the dry worker — nothing builds a thread or an
//! expectation by hand. The `still_writing` module goes through the library seam with a worker that
//! answers *"is this session still running"*, which the dry one always denies: a CONTINUED session
//! is one that already announced its end, so whether the liveness gate of nxf 6j6v.10yb still holds
//! it is precisely the question no CLI-driven test in this repo can see.

#[path = "common/mod.rs"]
mod common;

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-31T10:00:00Z";

/// The owner's own example, unchanged from `a_declared_cycle_sends_the_work_back.rs` — `build`
/// declares no `resume:`, so the DEFAULT decides, and the default follows the EDGE.
const CYCLE: &str = "- name: coding\n  members: [coder, review, finisher]\n  steps:\n    \
                     - id: build\n      target: coder\n      next: check\n    \
                     - id: check\n      target: review\n      on_needs_rework: build\n      \
                     max_passes: 3\n      next: ship\n    \
                     - id: ship\n      target: finisher\n";

/// The same run, folded instead of passed through — the shipped shape for a closing report.
const CYCLE_FOLDED: &str = "- name: coding\n  members: [coder, review, finisher]\n  \
                            on_complete: summarize\n  \
                            summary_prompt: Write the closing report.\n  steps:\n    \
                            - id: build\n      target: coder\n      next: check\n    \
                            - id: check\n      target: review\n      on_needs_rework: build\n      \
                            max_passes: 3\n      next: ship\n    \
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

fn started_roles(tmp: &TempDir) -> Vec<String> {
    trigger_lines(tmp)
        .iter()
        .map(|l| field(l, "role"))
        .collect()
}

/// **Every session the engine has put `role` in motion on, in start order** — the observable this
/// whole first half turns on. Two entries that are EQUAL are one session continued; two that differ
/// are two sessions.
///
/// The INTERNAL id and not `resume=`, deliberately: the dry worker answers `Accepted` and never
/// hands back a runtime session, so nothing is ever bound and `resolve_real` is `None` for every
/// session it starts — `resume=` therefore reads `-` here whatever the engine decided. The internal
/// id is in any case the identity that matters to nxc (the session map, the thread position, the
/// depth budget, the liveness question); the real one is only what the SDK needs to rehydrate a
/// transcript, and it travels beside it wherever it is known.
fn sessions_of(tmp: &TempDir, role: &str) -> Vec<String> {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .collect()
}

fn newest_session_of(tmp: &TempDir, role: &str) -> String {
    sessions_of(tmp, role)
        .pop()
        .unwrap_or_else(|| panic!("no trigger for {role}"))
}

/// Has this session announced an end that still stands? The one fact `reopen_session` moves, read
/// straight off the store rather than inferred from behaviour.
fn ended(tmp: &TempDir, session: &str) -> bool {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
        .session_ended_at(session)
        .expect("the session row is readable")
        .is_some()
}

fn dry_log(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default()
}

/// The NEWEST slot under `parent` that expects `handle` — a cycle opens a second slot for the same
/// role, and every pass gets a fresh one.
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

/// Open the board and get to the point where the review step is waiting for its verdict.
fn up_to_the_first_review(tmp: &TempDir, channels: &str) -> (String, String) {
    write_roles(tmp, &["pm", "coder", "review", "finisher"], channels);
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
    json_of(
        persona(tmp, "coder", &newest_session_of(tmp, "coder")).args([
            "--json",
            "reply",
            "--thread",
            &build,
            "first cut is in: added the cache",
        ]),
    );
    (pm, board)
}

/// The verdict that sends the run back over the declared edge.
fn send_it_back(tmp: &TempDir, board: &str) {
    let check = newest_slot_for(tmp, board, "review");
    json_of(
        persona(tmp, "review", &newest_session_of(tmp, "review")).args([
            "--json",
            "reply",
            "--thread",
            &check,
            "--needs-rework",
            "the lock order is wrong in two places",
        ]),
    );
}

// ---- part 3: a step continues its own session --------------------------------------------------

#[test]
fn a_back_edge_continues_the_session_that_did_the_step_it_returns_to() {
    // Owner at nxf 6j6v.553s: "Kommt das Review Ergebnis zurueck, wird die Coder-Sitzung mit dem
    // Ergebnis fortgefuehrt." The notice the second turn opens with says "What you handed over was
    // reviewed" and "Your claim on the working copy is STILL YOURS" — sentences that are only true
    // of the session that did hand something over. This is what makes them true.
    let tmp = workspace();
    let (_pm, board) = up_to_the_first_review(&tmp, CYCLE);
    send_it_back(&tmp, &board);

    let coders = sessions_of(&tmp, "coder");
    assert_eq!(
        coders.len(),
        2,
        "the back edge ran, so `build` was commissioned twice: {coders:?}"
    );
    assert_eq!(
        coders[0], coders[1],
        "…and the second turn is the SAME session continued, not a fresh one that has to \
         rediscover its own work: {coders:?}"
    );
    // The slot is fresh either way — `max_passes` counts slots, not sessions.
    //
    // Read off the ADDRESSEE BLOCK the coordinator now composes (nxf 6j6v.dq59) rather than the
    // fixed `Reply via …` sentence it replaced: same claim, same place (the wake the worker was
    // handed), said in the wording that ships.
    let first = newest_slot_for(&tmp, &board, "coder");
    assert!(
        dry_log(&tmp).contains(&format!("nxc reply --thread {first}")),
        "…and it answers on a FRESH slot, which is what the pass counter counts"
    );
}

#[test]
fn a_step_may_declare_that_it_does_not_want_its_session_continued() {
    // `resume: false` is the author saying "start this one clean", and it beats the edge's default.
    // Without it the field would be a suggestion.
    let tmp = workspace();
    let no_resume = CYCLE.replace(
        "- id: build\n      target: coder\n      next: check\n",
        "- id: build\n      target: coder\n      next: check\n      resume: false\n",
    );
    assert_ne!(no_resume, CYCLE, "the fixture must actually declare it");
    let (_pm, board) = up_to_the_first_review(&tmp, &no_resume);
    send_it_back(&tmp, &board);

    let coders = sessions_of(&tmp, "coder");
    assert_eq!(coders.len(), 2, "the back edge still ran: {coders:?}");
    assert_ne!(
        coders[0], coders[1],
        "…but the step asked for a clean start, so the second turn is its own session: {coders:?}"
    );
}

#[test]
fn a_plain_next_onto_the_same_target_starts_fresh_unless_the_step_asks_otherwise() {
    // Owner, 2026-08-31: "wenn einfach nur der naechste Schritt definiert wird, der
    // zufaelligerweise an die gleiche Persona geht, waere `resume: false`". Two steps may share a
    // target — `id` is what tells them apart — and sharing one is not by itself a reason to carry a
    // session across. So the DEFAULT follows the edge, and the author overrides it per step.
    let twice = "- name: coding\n  members: [coder]\n  steps:\n    \
                 - id: build\n      target: coder\n      next: polish\n    \
                 - id: polish\n      target: coder\n";
    for (channels, same, why) in [
        (
            twice.to_string(),
            false,
            "the default over `next` is a clean start",
        ),
        (
            twice.replace(
                "- id: polish\n      target: coder\n",
                "- id: polish\n      target: coder\n      resume: true\n",
            ),
            true,
            "`resume: true` carries the session across a `next` edge too",
        ),
    ] {
        let tmp = workspace();
        write_roles(&tmp, &["pm", "coder"], &channels);
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
        json_of(
            persona(&tmp, "coder", &newest_session_of(&tmp, "coder"))
                .args(["--json", "reply", "--thread", &build, "built"]),
        );

        let coders = sessions_of(&tmp, "coder");
        assert_eq!(coders.len(), 2, "both steps ran: {coders:?}");
        assert_eq!(coders[0] == coders[1], same, "{why} — {coders:?}");
    }
}

// ---- part 4: the report is built from the passes ------------------------------------------------

/// Run the whole cycle to its end — build, verdict, rework, approval, ship.
fn a_whole_run(tmp: &TempDir, channels: &str) -> String {
    let (_pm, board) = up_to_the_first_review(tmp, channels);
    send_it_back(tmp, &board);
    let rework = newest_slot_for(tmp, &board, "coder");
    json_of(
        persona(tmp, "coder", &newest_session_of(tmp, "coder")).args([
            "--json",
            "reply",
            "--thread",
            &rework,
            "lock order fixed",
        ]),
    );
    let check2 = newest_slot_for(tmp, &board, "review");
    json_of(
        persona(tmp, "review", &newest_session_of(tmp, "review"))
            .args(["--json", "reply", "--thread", &check2, "good now"]),
    );
    let ship = newest_slot_for(tmp, &board, "finisher");
    json_of(
        persona(tmp, "finisher", &newest_session_of(tmp, "finisher")).args([
            "--json",
            "reply",
            "--thread",
            &ship,
            "shipped as PR #7",
        ]),
    );
    board
}

/// The one trigger message a role was handed — the dry log's `msg=` runs to the end of the record,
/// so a body is cut out of the whole text rather than off one physical line.
fn message_to(tmp: &TempDir, role: &str) -> String {
    let log = dry_log(tmp);
    let mut found = String::new();
    for chunk in log.split("trigger role=") {
        if chunk.starts_with(role) {
            if let Some(at) = chunk.find(" msg=") {
                found = chunk[at + 5..].to_string();
            }
        }
    }
    assert!(!found.is_empty(), "no trigger for {role} in {log}");
    found
}

#[test]
fn a_folded_report_is_handed_the_run_as_its_passes() {
    // Part 4 of the draft: "Der Abschlussbericht wird gebaut, nicht durchgereicht. Er fasst das
    // erste `reply` ... und das zweite ... zusammen." The fold is the seam that builds it, and what
    // it needs is the STRUCTURE the flat list threw away — which step each answer served, which
    // pass of it, and which answer carried the verdict that turned the run around.
    let tmp = workspace();
    a_whole_run(&tmp, CYCLE_FOLDED);
    let synth = message_to(&tmp, "__synth__");

    assert!(
        synth.contains("step=\"build\" pass=\"1\""),
        "the first cut is named as pass one of `build`: {synth}"
    );
    assert!(
        synth.contains("step=\"build\" pass=\"2\""),
        "…and the mitigation as pass two of the SAME step, which is what a flat list could not \
         say: {synth}"
    );
    assert!(
        synth.contains("step=\"check\" pass=\"1\" needs_rework=\"true\""),
        "…and the verdict that sent it back is marked as one, not left as ordinary prose the \
         reader has to recognise: {synth}"
    );
    assert!(
        !synth.contains("step=\"check\" pass=\"2\" needs_rework=\"true\""),
        "…while the approval carries no verdict: {synth}"
    );
    assert!(
        synth.contains("SUPERSEDES"),
        "…and the one thing nobody can derive is said out loud: a later pass of a step supersedes \
         the earlier one: {synth}"
    );
}

#[test]
fn a_passed_through_report_names_the_passes_too() {
    // The two output forms are ONE seam with a declared difference (nxf 6j6v.e9qj), and this is
    // exactly the place they must not drift: a requester reading raw answers needs the order of the
    // run at least as much as a synthesizer does.
    let tmp = workspace();
    a_whole_run(&tmp, CYCLE);
    let pm = message_to(&tmp, "pm");
    assert!(
        pm.contains("step=\"build\" pass=\"2\"") && pm.contains("needs_rework=\"true\""),
        "the pass-through carries the same structure the fold does: {pm}"
    );
    assert!(
        pm.contains("SUPERSEDES"),
        "…including the sentence about what a repeated step means: {pm}"
    );
}

#[test]
fn a_channel_that_declares_no_steps_reports_exactly_what_it_always_did() {
    // The shape below is a contract an agent parses, and every channel in every workspace that has
    // no `steps:` must be byte-identical to what it was — which is why the marks are rendered only
    // when they exist, in the build of `substituted="true"` beside them.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "alice", "bob"],
        "- name: review\n  members: [alice, bob]\n",
    );
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "review",
        "look at this",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    for who in ["alice", "bob"] {
        let slot = newest_slot_for(&tmp, &board, who);
        json_of(persona(&tmp, who, &newest_session_of(&tmp, who)).args([
            "--json",
            "reply",
            "--thread",
            &slot,
            "looks fine",
        ]));
    }
    let delivered = message_to(&tmp, "pm");
    assert!(
        !delivered.contains("step=\"") && !delivered.contains("pass=\""),
        "a channel with no steps has no passes to name: {delivered}"
    );
    assert!(
        !delivered.contains("SUPERSEDES"),
        "…and is told nothing about repeated steps it cannot have: {delivered}"
    );
    assert!(
        started_roles(&tmp).contains(&"pm".to_string()),
        "the round did complete"
    );
}

// ---- the liveness gate still holds a CONTINUED session -----------------------------------------

/// **The mode every CLI-driven test above is blind to** (nxf 6j6v.10yb): the dry worker answers
/// "not running" for every session, so nothing that reads liveness ever engages there.
///
/// It matters twice over for a CONTINUED session. `ChatStore::mark_session_ended` is FIRST-WINS, so
/// a session that has once announced its end carries `ended` for good — and
/// `unended_sessions_in_thread`, which is what the gate reads, would then never see it again. A
/// continued session that is genuinely writing would be invisible to the very check that exists to
/// keep two sessions out of one checkout.
mod still_writing {
    use nexus_chat::definitions::Definitions;
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::orchestration::Caller;
    use nexus_chat::role::RoleDecl;
    use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
    use nexus_chat::timer::TimerConfig;
    use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
    use nexus_chat::workspace::{chat_config, setup};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    const NOW: &str = "2026-08-31T10:00:00Z";

    /// Keeps every request verbatim, and answers the one read the engine makes of a worker.
    /// ONE lock per lookup — a second taken inside `unwrap_or_else` while the first is alive
    /// deadlocks a `std::sync::Mutex`, and only on a run that is already failing.
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<TriggerRequest>>,
        running: Mutex<std::collections::HashSet<String>>,
    }

    impl Recorder {
        fn newest_for(&self, handle: &str) -> TriggerRequest {
            let seen = self.seen.lock().unwrap();
            match seen.iter().rev().find(|r| r.role.handle == handle) {
                Some(req) => req.clone(),
                None => {
                    let handles: Vec<&str> = seen.iter().map(|r| r.role.handle.as_str()).collect();
                    panic!("no trigger for {handle}, only: {handles:?}")
                }
            }
        }
        fn started(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.role.handle.clone())
                .collect()
        }
        fn exited(&self, session: &str) {
            self.running.lock().unwrap().remove(session);
        }
    }

    impl Worker for Recorder {
        /// **`Started`, not `Accepted`** (independent review of PR #402, Test Quality #4). Both
        /// bundled workers answer `Accepted` and bind out of band, so nothing this crate's CI runs
        /// ever reaches `bind_session` — and the branch where a continued session carries a REAL
        /// runtime id was therefore exercised only by the `#[ignore]`d live test. A remote runtime
        /// answers exactly this way, so it is a shape the engine must handle, not a test fiction.
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.running
                .lock()
                .unwrap()
                .insert(req.internal_session.clone());
            let runtime_session = format!("sdk-{}", req.internal_session);
            self.seen.lock().unwrap().push(req);
            Ok(TriggerOutcome::Started { runtime_session })
        }
        fn session_is_running(&self, internal_session: &str) -> bool {
            self.running.lock().unwrap().contains(internal_session)
        }
    }

    fn team(tmp: &TempDir, channels: &str) -> (Engine, Arc<Recorder>) {
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles: Vec<RoleDecl> = super::PLAIN
            .split(';')
            .map(|r| serde_yaml::from_str(r).expect("role parses"))
            .collect();
        let defs = Definitions::new(
            roles,
            serde_yaml::from_str(channels).expect("channels parse"),
        )
        .expect("catalogue");
        super::common::write_declarations(tmp.path(), &defs);
        let worker = Arc::new(Recorder::default());
        let engine = Engine::open_with(
            None,
            tmp.path(),
            EngineConfig {
                worker: WorkerConfig::Custom(worker.clone()),
                timer: TimerConfig::Dry,
                ..EngineConfig::default()
            },
        )
        .expect("open engine");
        (engine, worker)
    }

    fn caller(actor: &str) -> Caller<'_> {
        Caller {
            session: None,
            actor: Some(actor),
            now: Some(NOW),
        }
    }

    fn answer(engine: &Engine, who: &str, req: &TriggerRequest, body: &str, rework: bool) {
        engine
            .reply_thread(
                caller(who),
                ReplyThreadRequest {
                    machine: None,
                    thread: req
                        .reply_thread
                        .as_deref()
                        .expect("the step owes an answer"),
                    body,
                    escalate: false,
                    needs_rework: rework,
                    accept: false,
                },
            )
            .expect("the reply is posted");
    }

    #[test]
    fn a_step_whose_target_is_a_channel_starts_a_fresh_fan_out_even_when_it_asks_to_continue() {
        // Independent review of PR #402, Test Quality #2, and the finding it turned into. The ask
        // was coverage for `session_to_continue`'s channel early return; the mutation written to
        // answer it showed that branch had NO OBSERVABLE BEHAVIOUR — removing it, and then the role
        // guard behind it as well, left this test green, because `open_flow_step`'s channel branch
        // never reads the answer at all. So the early return went, and the question is now asked
        // only on the role branch that can use it.
        //
        // What is left is this test, and it is named for the PROPERTY rather than for a mechanism
        // (the project's `a-test-name-is-a-claim` rule): a channel step commissions a fresh
        // fan-out, whatever the declaration asks for. That property is now structural — there is no
        // longer a code path that could attach a transcript here — and this pins it against a
        // future change that wires one up.
        //
        // The declaration is reachable without anything exotic: `validate_channels` reports
        // `resume: true` on a channel target ADVISORILY (it needs the role catalogue, so it cannot
        // live in the fail-closed point-of-use check), and nothing refuses to RUN such a file.
        let tmp = TempDir::new().unwrap();
        let channels = "- name: coding\n  members: [coder]\n  steps:\n    \
                        - id: build\n      target: coder\n      next: check\n    \
                        - id: check\n      target: panel\n      resume: true\n\
                        - name: panel\n  members: [review]\n";
        let (engine, worker) = team(&tmp, channels);
        engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed");
        let build = worker.newest_for("coder");
        answer(&engine, "coder", &build, "first cut is in", false);

        assert_eq!(
            worker.started(),
            vec!["coder", "review"],
            "the channel step ran: its own member was fanned out to"
        );
        assert_eq!(
            worker.newest_for("review").resume_real,
            None,
            "a channel has no one session to continue, so `resume: true` there commissions afresh \
             rather than attaching somebody's transcript to a fan-out"
        );
    }

    #[test]
    fn a_requester_whose_session_already_ended_is_live_again_once_the_round_wakes_it() {
        // Independent review of PR #402, Code Quality #1: the back-edge path was regression-tested
        // and the WAKE paths were not — and a requester is the likeliest party to have announced
        // its end long before the round it commissioned comes back, which is exactly when
        // `reopen_session` matters. Without it the woken session would stay `ended` for good and
        // every later liveness question about it would answer "gone" while it was working.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp, super::CYCLE);
        // The requester has to be a SESSION for there to be anything to wake, so it is summoned
        // the way one really is — a person addresses `pm`, and `pm` opens the channel from inside
        // its own session, which is what stamps the return address the completion routes back
        // through.
        engine
            .send_to(
                caller("carsten"),
                SendToRequest {
                    machine: None,
                    to: "pm",
                    body: "ship it",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the persona is addressed");
        let pm = worker.newest_for("pm").internal_session.clone();
        engine
            .send_to(
                Caller {
                    session: Some(&pm),
                    actor: Some("pm"),
                    now: Some(NOW),
                },
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed");

        // The requester goes away while the round works — the ordinary shape, not a contrivance.
        engine
            .session_ended(caller("pm"), &pm)
            .expect("the runtime reports the process gone");
        worker.exited(&pm);
        assert!(
            super::ended(&tmp, &pm),
            "it really did announce an end, or the assertion at the bottom proves nothing"
        );

        // The run goes through to its end, and its consolidation wakes the requester.
        for (who, body) in [
            ("coder", "built"),
            ("review", "good"),
            ("finisher", "shipped"),
        ] {
            let step = worker.newest_for(who);
            answer(&engine, who, &step, body, false);
            engine
                .session_ended(caller(who), &step.internal_session)
                .expect("the runtime reports the process gone");
            worker.exited(&step.internal_session);
        }
        let woken = worker.newest_for("pm");
        assert_eq!(
            woken.internal_session, pm,
            "the completion woke the requester's OWN session rather than minting one"
        );
        assert_eq!(
            woken.resume_real.as_deref(),
            Some(format!("sdk-{pm}").as_str()),
            "…on its own transcript"
        );
        assert!(
            !super::ended(&tmp, &pm),
            "…and it counts as in motion again, so a liveness question about it is answered by the \
             process rather than by a stale end announcement — without that, `reopen_session` \
             would leave every woken requester permanently 'gone'"
        );
    }

    #[test]
    fn a_continued_session_that_is_still_writing_holds_the_flow_exactly_as_a_fresh_one_would() {
        // The property, not the mechanism: after the back edge has continued the coder's session,
        // that session's own answer must not open the next step while its process is still in the
        // checkout. A `resume` that left the session marked ENDED would make it invisible to the
        // one check that enforces this, and the gate would silently stop applying to exactly the
        // sessions this item creates.
        let tmp = TempDir::new().unwrap();
        let exclusive = super::CYCLE.replace(
            "- name: coding\n  members: [coder, review, finisher]\n",
            "- name: coding\n  members: [coder, review, finisher]\n  working_tree: exclusive\n",
        );
        let (engine, worker) = team(&tmp, &exclusive);
        engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed");

        let build = worker.newest_for("coder");
        answer(&engine, "coder", &build, "first cut is in", false);
        engine
            .session_ended(caller("coder"), &build.internal_session)
            .expect("the runtime reports the process gone");
        worker.exited(&build.internal_session);

        let check = worker.newest_for("review");
        answer(&engine, "review", &check, "the lock order is wrong", true);
        engine
            .session_ended(caller("review"), &check.internal_session)
            .expect("the runtime reports the process gone");
        worker.exited(&check.internal_session);

        let again = worker.newest_for("coder");
        assert_eq!(
            again.internal_session, build.internal_session,
            "the back edge continued the coder's own session"
        );
        // **The runtime's own id travels with it** (independent review of PR #402, Test Quality
        // #4). The internal id is what nxc continues; `resume_real` is what the runtime needs to
        // rehydrate the transcript, and handing it over is the difference between continuing a
        // conversation and starting a new one under an old name. It is `None` under a worker that
        // binds out of band, which is why this suite's worker does not.
        assert_eq!(
            again.resume_real.as_deref(),
            Some(format!("sdk-{}", build.internal_session).as_str()),
            "the bound runtime session is handed back for the resume"
        );
        assert_eq!(
            build.resume_real, None,
            "…and the FIRST turn had nothing to resume, which is what makes the line above a claim"
        );
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder"],
            "…and the run is back at `build`"
        );

        // It answers while its process is STILL in the checkout. Nothing may start behind it.
        answer(&engine, "coder", &again, "lock order fixed", false);
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder"],
            "a continued session that is still writing holds the flow just as a fresh one does — \
             the `check` step has NOT been opened into its checkout"
        );

        engine
            .session_ended(caller("coder"), &again.internal_session)
            .expect("the runtime reports the process gone");
        worker.exited(&again.internal_session);
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder", "review"],
            "…and with the checkout free the run goes on"
        );
    }
}

const PLAIN: &str = "handle: pm\nsystem_prompt: You are pm.\ntools: [Bash]\n;\
                     handle: coder\nsystem_prompt: You are coder.\ntools: [Bash]\n;\
                     handle: review\nsystem_prompt: You are review.\ntools: [Bash]\n;\
                     handle: finisher\nsystem_prompt: You are finisher.\ntools: [Bash]\n";

// ---- the live acceptance run -------------------------------------------------------------------

/// **Whether a real session really carries its own work across a back edge**, with real SDK
/// sessions — the one thing no test above can show. Every deterministic test in this file asserts
/// that the engine hands the SAME session id to the worker; whether the model on the other side of
/// that id then remembers what it built is a fact about the runtime, and only a live run has it.
///
/// **Two arms, and the second is the safeguard deliberately turned back.** The first runs the
/// declaration as shipped, where a back edge continues; the second is the same round with
/// `resume: false` on `build`, which is a real author's option and here doubles as the control. The
/// coder invents a code word in its first turn and is asked for it again on the way back — it can
/// only be there if the session is genuinely the same one.
///
/// **The reviewer proves the other default in the same run.** Its step is reached over `next:`, so
/// it would start fresh; the declaration says `resume: true`, and the reviewer is told to send the
/// work back the first time it is asked and to approve the second time. Knowing which time it is is
/// something only a continued session can do — and it is what lets the round end cleanly, so the
/// declared `summarize` fold runs and the report is BUILT from both passes.
///
/// Run it by hand:
///
/// ```console
/// $ cargo build -p nxs && cargo test -p nexus-chat --test a_run_continues_and_reports_itself \
///     -- --ignored --nocapture live_
/// ```
#[test]
#[ignore = "live: spawns real Claude Agent SDK sessions; run by hand"]
fn live_a_continued_session_remembers_its_own_work_and_the_report_is_built_from_both_passes() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/chat")
        .to_path_buf();
    let sidecar = repo.join("agent-sidecar/src/main.mjs");
    assert!(sidecar.is_file(), "live run: no sidecar at {sidecar:?}");
    assert!(
        std::process::Command::new("sh")
            .args(["-c", "command -v claude"])
            .output()
            .is_ok_and(|o| o.status.success()),
        "live run: `claude` is not on PATH — the SDK needs the CLI, authenticated"
    );
    assert!(
        std::env::var_os("ANTHROPIC_API_KEY").is_none(),
        "live run: ANTHROPIC_API_KEY is set; the SDK would prefer it over this machine's own \
         authenticated `claude` session"
    );
    nxs_test_support::assert_multicall_binary_fresh();

    // ARM 1: as declared — the back edge continues, the round ends clean, the fold runs.
    let kept = live_round(&repo, &sidecar, true);
    let (first, second) = (&kept.coder_turns[0], &kept.coder_turns[1]);
    let word = code_word(first);
    assert!(
        second.contains(&word),
        "a CONTINUED session knows what it built: the second turn must repeat the code word its \
         own first turn invented ({word:?}).\nfirst:  {first}\nsecond: {second}"
    );
    assert_eq!(
        kept.coder_turns.len(),
        2,
        "the reviewer sent the work back exactly once and then approved — which it could only \
         decide by remembering that it had already sent it back: {:?}",
        kept.coder_turns
    );
    let report = kept.report.expect("the clean round ran its declared fold");
    assert!(
        report.contains(&word),
        "the BUILT report accounts for what was delivered in the end: {report}"
    );
    assert!(
        report.to_lowercase().contains("lock"),
        "…and for the finding that got it there, which is what a report built from the PASSES has \
         and a delivery of the last answer alone does not: {report}"
    );

    // ARM 2: the safeguard turned back. `resume: false` on `build`, everything else identical.
    let fresh = live_round(&repo, &sidecar, false);
    let second = &fresh.coder_turns[1];
    assert!(
        !second.contains(&code_word(&fresh.coder_turns[0])),
        "with `resume: false` the second turn is a stranger to the first — if it still knew the \
         code word, arm 1 would be proving nothing.\nsecond: {second}"
    );
}

/// The distinctive token a live coder was told to invent, cut out of its first reply.
#[cfg(test)]
fn code_word(reply: &str) -> String {
    let at = reply
        .find("CODEWORD:")
        .unwrap_or_else(|| panic!("the first coder turn must name its code word: {reply}"));
    reply[at + "CODEWORD:".len()..]
        .split_whitespace()
        .next()
        .unwrap_or_else(|| panic!("an empty code word: {reply}"))
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// What one live arm produced, read off the board rather than remembered.
#[cfg(test)]
struct LiveRound {
    /// Each `build` slot's own reply, in pass order.
    coder_turns: Vec<String>,
    /// The folded report, when the round ended cleanly enough to run its `on_complete`.
    report: Option<String>,
}

/// Run one live round to its end and read what it did off the board.
#[cfg(test)]
fn live_round(repo: &std::path::Path, sidecar: &std::path::Path, continues: bool) -> LiveRound {
    let nxc = std::env::current_exe()
        .expect("the test binary has a path")
        .ancestors()
        .nth(2)
        .expect("the test binary lives in target/<profile>/deps")
        .join(format!("nxc{}", std::env::consts::EXE_SUFFIX));
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Live acceptance coder\nsystem_prompt: |\n  \
         You are a coder in a live acceptance run. Read, write or change NO files and run no\n  \
         command except the one reply you owe. Do not inspect anything.\n  \
         FIRST TURN: invent one distinctive nonsense word of your own and answer with a\n  \
         one-line report of what you would have done, ending with `CODEWORD: <that word>`.\n  \
         A LATER TURN: if you are handed findings about work you did, say in one line what you\n  \
         would change, and end with `CODEWORD: <the very same word you chose before>`. If you\n  \
         genuinely do not know which word you chose, end with `CODEWORD: NO-MEMORY` instead —\n  \
         never invent a second one.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("review.yaml"),
        "handle: review\njob_title: Live acceptance reviewer\nsystem_prompt: |\n  \
         You are reviewing somebody else's work in a live acceptance run. Read, write or change\n  \
         NO files and run no command except the one reply you owe. Do not inspect anything.\n  \
         THE FIRST TIME you are asked in this round: the work does NOT meet the standard,\n  \
         because the lock order is wrong in two places. End your turn in whichever of the ways\n  \
         you were told about carries that verdict back to whoever produced the work, and put\n  \
         that reason in the body.\n  \
         IF YOU HAVE ALREADY SENT THIS ROUND BACK ONCE: it is good now. Approve it plainly with\n  \
         an ordinary reply and nothing else.\n",
    )
    .unwrap();
    // `check` is reached over `next:`, where the default is a fresh session — so `resume: true`
    // there is the author's override, and it is what lets the reviewer know it has been here
    // before. `build`'s `resume:` is what this arm varies.
    let build_resume = match continues {
        true => String::new(),
        false => "      resume: false\n".to_string(),
    };
    std::fs::write(
        roles.join("channels.yaml"),
        format!(
            "- name: coding\n  members: [coder, review]\n  on_complete: summarize\n  \
             summary_prompt: |\n    \
             Write the closing report of this round for the person who asked for the work.\n    \
             Say what was built, what was found, and what was done about it. Keep every code\n    \
             word verbatim.\n  steps:\n    \
             - id: build\n      target: coder\n      next: check\n{build_resume}    \
             - id: check\n      target: review\n      on_needs_rework: build\n      \
             max_passes: 3\n      resume: true\n"
        ),
    )
    .unwrap();

    let live = |args: &[&str]| -> Value {
        let mut c = std::process::Command::new(&nxc);
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env_remove("NXC_NOW")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    repo.join("target/debug").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_WORKER", "sidecar")
            // A real one-shot job would be left in the runner's own login session, due against a
            // `TempDir` that is gone by then. The clock is not what this proves.
            .env("NXC_TIMER", "dry")
            .env("NXC_SIDECAR", sidecar)
            .args(args);
        let out = c.output().expect("nxc runs");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str::<Value>(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "no --json receipt from `nxc {}`: {e}\nstdout: {stdout}\nstderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    };

    let board = live(&["--json", "send", "--to", "coding", "--no-ref", "Build it."])["thread_id"]
        .as_str()
        .expect("the send opened the channel thread")
        .to_string();

    // Up to five real sessions have to run and answer, one after the other. Generously bounded:
    // the failure this guards is a round that stalls, which without a bound is a hang.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1200);
    loop {
        let view = live(&["--json", "threads", "show", &board]);
        if view["complete"] == Value::Bool(true) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the live round never came back up; the board was:\n{view:#}"
        );
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    let store = open_store(&tmp);
    let mut coder_turns = Vec::new();
    for slot in store.supervised_children(&board).unwrap() {
        let is_build = store
            .thread_quorum(&slot, NOW)
            .unwrap()
            .is_some_and(|q| q.expects == ["local/coder".to_string()]);
        if !is_build {
            continue;
        }
        // The coder's own words: the newest message on the slot the SUPERVISOR did not write.
        if let Some(m) = store
            .messages_in_thread(&slot)
            .unwrap()
            .into_iter()
            .rev()
            .find(|m| m.sender == "local/coder")
        {
            coder_turns.push(m.body);
        }
    }
    assert_eq!(
        coder_turns.len(),
        2,
        "the round must have gone back over the edge exactly once: {coder_turns:?}"
    );
    let report = store
        .messages_in_thread(&board)
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.sender == "local/__synth__")
        .map(|m| m.body);
    LiveRound {
        coder_turns,
        report,
    }
}
