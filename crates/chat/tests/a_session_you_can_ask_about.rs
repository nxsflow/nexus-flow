//! **The session seam gets its reader** (nxf 6j6v.h383).
//!
//! `nxc session bind` writes which runtime session a trigger became. `nxc session ended` writes
//! that its process is over — and that second fact is the one a `working_tree: exclusive` channel's
//! advance has rested on since nxf 6j6v.10yb: *a step ends when its session ends*. Two writers, no
//! reader, on a fact the engine itself branches on.
//!
//! What that cost was measured rather than argued. Over six hours of continuous operation in a
//! foreign project (nxc 0.66.0, eight tickets, ~80 threads), the coordinator of that run answered
//! "is this thread thinking, or is it dead?" with
//!
//! ```text
//! ps -p $(cat .nxs/agent-logs/<internal>.pid)
//! ```
//!
//! — reaching past the tool chain into `nxc`'s own directory, for a file whose format is promised
//! nowhere. Its own report: *"the pid check saved me from reading the interim state as a failure
//! yet again."*
//!
//! Two questions hang on it, and both actually came up in that run:
//!
//! 1. **"Is this still alive, or is it dead?"** — at every silent thread.
//! 2. **"Is the previous session really over before I touch this?"** — before every step.
//!
//! One verb answers both, through a scope: name a session, or name a thread.
//!
//! Everything here drives the LIBRARY HANDLE, because that is the seam the fact lives on and the
//! only place a worker's own answer about a live session can be supplied (`engine-seam-test-rule`);
//! `the_worker_stands_at_the_workspace_root.rs` and the CLI cases at the bottom cover the other
//! side.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::orchestration::{Caller, SessionScope, SessionState};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-25T07:49:53Z";

/// A worker that records its triggers and answers the one read the engine makes of it — the shape a
/// real host worker has, since it owns the processes and is the only thing that can say.
#[derive(Default)]
struct LiveSessionWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl LiveSessionWorker {
    fn only_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        let mut it = seen.iter().filter(|r| r.role.handle == handle);
        let first = it
            .next()
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone();
        assert!(it.next().is_none(), "expected exactly one trigger");
        first
    }
}

impl Worker for LiveSessionWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }

    /// And it says so (nxf 6j6v.t41e) — the second half a host worker that owns processes owes,
    /// without which its `false` above is indistinguishable from a worker that never looked.
    fn answers_liveness(&self) -> bool {
        true
    }
}

/// A worker that answers no liveness question at all — [`Worker::session_is_running`]'s DEFAULT,
/// which is what every host that has not implemented the method gets.
#[derive(Default)]
struct SilentWorker;

impl Worker for SilentWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Ok(TriggerOutcome::Accepted)
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

const CODING: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";

fn team(worker: Arc<dyn Worker>) -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("coder"), role("finisher")], vec![channel(CODING)])
        .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine)
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// Open the round and hand back (the coder's internal session, its slot thread).
fn open_round(engine: &Engine, worker: &LiveSessionWorker) -> (String, String) {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let coder = worker.only_for("coder");
    let slot = coder
        .reply_thread
        .clone()
        .expect("a commissioned step is told which thread it owes an answer on");
    (coder.internal_session.clone(), slot)
}

// ---- the three states ------------------------------------------------------------------------

#[test]
fn a_live_session_reads_running_and_an_announced_one_reads_ended_with_its_instant() {
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, _slot) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());

    let alive = engine
        .session_state(SessionScope::Session(&coder))
        .expect("a minted session can be asked about");
    assert_eq!(alive.sessions.len(), 1, "{alive:?}");
    assert_eq!(alive.sessions[0].session, coder);
    assert_eq!(alive.sessions[0].role, "coder");
    assert_eq!(
        alive.sessions[0].state,
        SessionState::Running,
        "a live process behind the session is what `running` means: {alive:?}"
    );
    assert_eq!(
        alive.sessions[0].ended, None,
        "nothing has announced an end: {alive:?}"
    );

    // The session says so itself — and the announcement, not the worker, is what decides. The
    // process is deliberately left in the `running` set: the party that announces its own end is by
    // definition still alive while it speaks, which is why `session_map.ended` exists at all.
    engine
        .session_ended(caller("coder"), &coder)
        .expect("the announcement is accepted");

    let over = engine.session_state(SessionScope::Session(&coder)).unwrap();
    assert_eq!(
        over.sessions[0].state,
        SessionState::Ended,
        "the session's own word outranks a liveness read taken while it speaks: {over:?}"
    );
    assert_eq!(
        over.sessions[0].ended.as_deref(),
        Some(NOW),
        "and it says WHEN, which is the whole of what `ended` adds over a boolean: {over:?}"
    );
}

#[test]
fn a_session_killed_without_announcing_reads_unknown_rather_than_ended() {
    // The distinction the ADVANCE gate does not make and a human needs: `a_member_session_is_still_
    // writing` asks for `running` and folds everything else into "not writing", which is right for
    // a gate that only has to decide whether to hold. A person deciding whether to intervene is
    // deciding between exactly the two halves it merged.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, _slot) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());
    assert_eq!(
        engine
            .session_state(SessionScope::Session(&coder))
            .unwrap()
            .sessions[0]
            .state,
        SessionState::Running,
        "premise: it was alive"
    );

    // Killed hard: the process is gone and nothing was ever announced.
    worker.running.lock().unwrap().remove(&coder);

    let gone = engine.session_state(SessionScope::Session(&coder)).unwrap();
    assert_eq!(
        gone.sessions[0].state,
        SessionState::Unknown,
        "nothing announced an end and no process answers — reporting `ended` here would state a \
         fact nobody established: {gone:?}"
    );
    assert_eq!(gone.sessions[0].ended, None, "{gone:?}");
}

#[test]
fn a_host_whose_worker_cannot_answer_reads_unknown_for_everything_unended() {
    // `Worker::session_is_running` has a DEFAULT of `false` — that is what makes the method safe to
    // add to a trait every host implements. The honest consequence at this reader is `unknown` for
    // every unended session, not `ended`: nobody asked a runtime that cannot be asked.
    let (_tmp, engine) = team(Arc::new(SilentWorker));
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let store = common::open_store(_tmp.path());
    let minted: Vec<String> = store
        .sessions_in_thread(
            &engine
                .status(NOW, nexus_chat::facade::StatusScope::Workspace)
                .unwrap()
                .operations[0]
                .threads
                .iter()
                .find(|t| t.session.is_some())
                .expect("a commissioned step has a session")
                .thread_id
                .clone(),
        )
        .unwrap()
        .into_iter()
        .map(|r| r.internal_id)
        .collect();
    assert_eq!(minted.len(), 1, "{minted:?}");

    let report = engine
        .session_state(SessionScope::Session(&minted[0]))
        .unwrap();
    assert_eq!(
        report.sessions[0].state,
        SessionState::Unknown,
        "{report:?}"
    );
}

// ---- and whether the answer was ever asked for (nxf 6j6v.t41e) --------------------------------

#[test]
fn a_host_can_ask_whether_its_own_worker_answers_the_liveness_question_at_all() {
    // The question `session_is_running` cannot answer about itself. Reported from app-foundations
    // (41j0.4yjx): a host that drives its own sessions has to know whether the `false` it is about
    // to act on is a fact or a silence, BEFORE any session exists — its own warning fires at
    // startup — so the question is about the WORKER and takes no session.
    let (_tmp_live, live) = team(Arc::new(LiveSessionWorker::default()));
    assert!(
        live.worker_answers_liveness(),
        "this worker overrides `session_is_running`, and the answer travels through the bound \
         wrapper the engine puts around every host-supplied worker"
    );

    let (_tmp_silent, silent) = team(Arc::new(SilentWorker));
    assert!(
        !silent.worker_answers_liveness(),
        "and one that rests on the default admits it, rather than being read as `nothing is \
         running` — which is what every park and reclaim this crate makes would act on"
    );
}

#[test]
fn the_two_unknowns_that_mean_opposite_things_are_told_apart_on_the_report() {
    // `Unknown` has always carried two cases at once — a session killed hard, and a runtime nobody
    // can ask — and its own doc says so. Until this the reader could not tell which it had just
    // reported. It is the same distinction the gate side needs and the reason the item exists.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, _slot) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());
    assert_eq!(
        engine
            .session_state(SessionScope::Session(&coder))
            .unwrap()
            .sessions[0]
            .state,
        SessionState::Running,
        "premise: it was alive"
    );
    worker.running.lock().unwrap().remove(&coder);

    let killed = engine.session_state(SessionScope::Session(&coder)).unwrap();
    assert_eq!(
        killed.sessions[0].state,
        SessionState::Unknown,
        "{killed:?}"
    );
    assert!(
        killed.worker_answers_liveness,
        "a worker that looks at processes said `not running`, so this `unknown` is a HARD KILL: \
         {killed:?}"
    );

    let (_tmp_silent, silent) = team(Arc::new(SilentWorker));
    silent
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let unasked = silent
        .session_state(SessionScope::Thread(
            &silent
                .status(NOW, nexus_chat::facade::StatusScope::Workspace)
                .unwrap()
                .operations[0]
                .threads
                .iter()
                .find(|t| t.session.is_some())
                .expect("a commissioned step has a session")
                .thread_id
                .clone(),
        ))
        .unwrap();
    assert_eq!(
        unasked.sessions[0].state,
        SessionState::Unknown,
        "{unasked:?}"
    );
    assert!(
        !unasked.worker_answers_liveness,
        "and here the identical word means nobody was ever asked: {unasked:?}"
    );
}

// ---- the two scopes --------------------------------------------------------------------------

#[test]
fn a_thread_reports_every_session_that_ran_on_it_ended_ones_included() {
    // The coordinator's question, and the reason this is not `unended_sessions_in_thread` with a
    // public name: that read collects candidates for "somebody is still writing here" and drops an
    // ended session, whereas "is the previous one really over?" is answered BY the ended one.
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (coder, slot) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());
    engine
        .session_ended(caller("coder"), &coder)
        .expect("the announcement is accepted");

    let on_thread = engine.session_state(SessionScope::Thread(&slot)).unwrap();
    assert_eq!(
        on_thread.sessions.len(),
        1,
        "one step, one session standing on its slot: {on_thread:?}"
    );
    assert_eq!(on_thread.sessions[0].session, coder);
    assert_eq!(
        on_thread.sessions[0].state,
        SessionState::Ended,
        "the ended session is the ANSWER here, not a row to be filtered out: {on_thread:?}"
    );
    assert_eq!(on_thread.sessions[0].thread.as_deref(), Some(slot.as_str()));
}

#[test]
fn a_thread_nothing_ran_on_is_an_empty_list_and_an_unminted_session_is_not_found() {
    let worker = Arc::new(LiveSessionWorker::default());
    let (_tmp, engine) = team(worker.clone());
    let (_coder, _slot) = open_round(&engine, &worker);

    let empty = engine
        .session_state(SessionScope::Thread("m-nothing-ever-ran-here"))
        .expect("a thread with no sessions is an answer, not a failure");
    assert!(empty.sessions.is_empty(), "{empty:?}");

    // The session scope is the other way round, and deliberately: `bind` and `ended` both report an
    // unminted id as `not_found`, and a reader that answered "unknown" for a typo would turn a
    // mistake into a plausible-looking state.
    let err = engine
        .session_state(SessionScope::Session("s-never-minted"))
        .expect_err("an id that names no session of ours is not a state");
    assert_eq!(err.kind, ErrorKind::NotFound, "{err:?}");
    assert!(
        err.to_string().contains("s-never-minted"),
        "the refusal names what was asked for, got: {err}"
    );
}

#[test]
fn the_reader_names_the_runtime_session_once_the_runtime_has_bound_one() {
    // The other write on this seam. `bind_runtime_session` records the runtime's own id; before
    // this item nothing could read it back either, and a host reconciling its own session list
    // against ours had no way to line the two up.
    let worker = Arc::new(LiveSessionWorker::default());
    let (tmp, engine) = team(worker.clone());
    let (coder, _slot) = open_round(&engine, &worker);

    let pending = engine.session_state(SessionScope::Session(&coder)).unwrap();
    assert_eq!(
        pending.sessions[0].runtime_session, None,
        "a session is minted before the runtime hands an id back: {pending:?}"
    );

    engine
        .bind_runtime_session(&coder, "sdk-0f3a")
        .expect("the binding is accepted");
    let bound = engine.session_state(SessionScope::Session(&coder)).unwrap();
    assert_eq!(
        bound.sessions[0].runtime_session.as_deref(),
        Some("sdk-0f3a"),
        "{bound:?}"
    );
    // And the same row through the store, so the seam is reading what the write wrote rather than
    // remembering its argument.
    assert_eq!(
        Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap()
            .session_row(&coder)
            .unwrap()
            .expect("the row is there")
            .real_sdk_id
            .as_deref(),
        Some("sdk-0f3a")
    );
}

// ---- and the same answer through the shipped path --------------------------------------------
//
// The seam cases above supply their own worker. `nxc` resolves a REAL `SidecarWorker` and reads a
// REAL pid file, and the two have come apart before: `LazyWorker` forwarded only `trigger`, so
// `Worker::session_is_running`'s default answered for every command-line call there is while the
// seam suite stayed green (nxf 6j6v.10yb's own acceptance found it, with two live Claude sessions
// and eight milliseconds between them). A reader whose whole job is to say whether a process is
// alive has to be tested against one.

mod cli {
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    use nexus_chat::workspace::{chat_config, setup};

    /// A workspace declaring the measured channel, plus a `node` script that simply stays alive.
    fn workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        for handle in ["coder", "finisher"] {
            std::fs::write(
                roles.join(format!("{handle}.yaml")),
                format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
            )
            .unwrap();
        }
        std::fs::write(
            roles.join("channels.yaml"),
            "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n  \
             working_tree: exclusive\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("stub-sidecar.mjs"),
            "setTimeout(() => {}, 30_000);\n",
        )
        .unwrap();
        tmp
    }

    fn nxc(tmp: &TempDir) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", tmp.path().join("stub-sidecar.mjs"))
            .env("NXC_TIMER", "dry")
            .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
        c
    }

    /// The same command line with the RECORDING worker, which spawns nothing and looks at nothing
    /// (nxf 6j6v.t41e).
    fn dry(tmp: &TempDir) -> Command {
        let mut c = nxc(tmp);
        c.env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
        c
    }

    fn stdout_of(cmd: &mut Command) -> String {
        let out = cmd.assert().success();
        String::from_utf8_lossy(&out.get_output().stdout).into_owned()
    }

    fn json_of(cmd: &mut Command) -> Value {
        serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
    }

    fn started_session(tmp: &TempDir) -> String {
        std::fs::read_dir(tmp.path().join(".nxs/agent-logs"))
            .expect("the worker wrote its log directory")
            .filter_map(|e| e.ok())
            .find_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".spec.json"))
                    .map(str::to_string)
            })
            .expect("step one started")
    }

    /// Kill every stub sidecar this workspace started, and **wait until the kernel agrees they are
    /// gone** (nxf 6j6v.jg09).
    ///
    /// The wait is the whole point and it was missing. `kill -9` returns as soon as the signal is
    /// DELIVERED, not once the process is reaped — and what every caller does next is tick, which
    /// asks the liveness gate whether that process still exists. On a fast machine (and in
    /// `--release`, where the gap between the two is smaller) the gate could still find it, and the
    /// tick answered `waiting_for_a_session` instead of `advanced`. That reddened `main` on
    /// 2026-08-26 and went green on a bare re-run, which is exactly what a race looks like from the
    /// outside.
    ///
    /// Polling `pgrep` rather than `kill(pid, 0)` on purpose: `pgrep` is what asks the same
    /// question the gate asks — is there a process matching this script — so the wait ends when the
    /// gate's own answer has changed, not when a neighbouring one has. Bounded, and a breach is
    /// LOUD: a `reap` that silently gave up would put the flake back with an extra second of
    /// latency.
    fn reap(tmp: &TempDir) {
        let pattern = format!("{}", tmp.path().join("stub-sidecar.mjs").display());
        let alive = || {
            std::process::Command::new("pgrep")
                .args(["-f", &pattern])
                .output()
                .map(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        for pid in alive() {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid])
                .status();
        }
        for _ in 0..500 {
            if alive().is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!(
            "a stub sidecar survived `kill -9` for five seconds ({pattern}) — the liveness gate \
             would read it as live, and every assertion after this one would be about the wrong \
             state"
        );
    }

    #[test]
    fn the_verb_reads_a_real_live_process_and_then_its_absence() {
        let tmp = workspace();
        json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
        let coder = started_session(&tmp);

        let alive = json_of(nxc(&tmp).args(["--json", "session", "state", &coder]));
        assert_eq!(
            alive["sessions"][0]["state"], "running",
            "a real `node` holds this session and the pid file its own lock wrote says so: {alive}"
        );
        assert_eq!(alive["sessions"][0]["role"], "coder", "{alive}");
        let human = stdout_of(nxc(&tmp).args(["session", "state", &coder]));
        assert!(
            human.contains(&coder) && human.contains("running"),
            "the terminal line names the session and its state, got: {human}"
        );

        // The hard kill this reader exists for: the process is gone and nothing announced anything.
        reap(&tmp);
        let gone = json_of(nxc(&tmp).args(["--json", "session", "state", &coder]));
        assert_eq!(
            gone["sessions"][0]["state"], "unknown",
            "no announcement and no process — `ended` here would claim a fact nobody stated: {gone}"
        );

        // …and once the end IS announced, the same read says so, with the instant.
        json_of(nxc(&tmp).args(["--json", "session", "ended", &coder]));
        let over = json_of(nxc(&tmp).args(["--json", "session", "state", &coder]));
        assert_eq!(over["sessions"][0]["state"], "ended", "{over}");
        assert!(
            over["sessions"][0]["ended"].is_string(),
            "and it says when: {over}"
        );
    }

    #[test]
    fn the_thread_scope_answers_the_question_a_coordinator_asks_before_a_step() {
        let tmp = workspace();
        json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
        let coder = started_session(&tmp);
        let spec: Value = serde_json::from_str(
            &std::fs::read_to_string(
                tmp.path()
                    .join(".nxs/agent-logs")
                    .join(format!("{coder}.spec.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let slot = spec["replyThread"].as_str().unwrap().to_string();

        let on_thread = json_of(nxc(&tmp).args(["--json", "session", "state", "--thread", &slot]));
        assert_eq!(
            on_thread["sessions"].as_array().map(Vec::len),
            Some(1),
            "{on_thread}"
        );
        assert_eq!(on_thread["sessions"][0]["session"], coder, "{on_thread}");
        assert_eq!(on_thread["sessions"][0]["state"], "running", "{on_thread}");

        // A thread nothing ran on answers, rather than failing — and says so in the terminal too.
        let none = stdout_of(nxc(&tmp).args(["session", "state", "--thread", "m-nothing-here"]));
        assert!(
            none.contains("no session"),
            "an empty answer is still an answer, got: {none}"
        );

        reap(&tmp);
    }

    #[test]
    fn the_shipped_path_reports_that_its_worker_answers_the_liveness_question() {
        // The DELEGATION, on the one path that ships (nxf 6j6v.t41e). `cli.rs`'s `LazyWorker`
        // resolves the real worker per call, and a defaulted method it forgets to forward answers
        // `false` for every command-line call while every seam test stays green — which is exactly
        // how `session_is_running` came to be inert on this path (nxf 6j6v.10yb). So the answer is
        // read back through the real binary, against a real `SidecarWorker`.
        let tmp = workspace();
        json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
        let coder = started_session(&tmp);

        let alive = json_of(nxc(&tmp).args(["--json", "session", "state", &coder]));
        assert_eq!(
            alive["worker_answers_liveness"], true,
            "the sidecar reads the pid file its own lock wrote, and the shipped path forwards that \
             answer rather than the trait default: {alive}"
        );

        // And the terminal line for the case the flag exists to disambiguate: a hard kill reads
        // `unknown` from a worker that DID look.
        reap(&tmp);
        let human = stdout_of(nxc(&tmp).args(["session", "state", &coder]));
        assert!(
            human.contains("unknown"),
            "no announcement and no process: {human}"
        );
        assert!(
            !human.contains("cannot answer"),
            "and it must not be blamed on a worker that answered perfectly well: {human}"
        );
    }

    #[test]
    fn a_worker_that_starts_no_process_says_the_question_was_never_put() {
        // The other direction of the same read, and the one an operator misreads without it: with
        // `NXC_WORKER=dry` nothing is ever spawned, so `unknown` here says nothing about the
        // session and everything about who was asked.
        let tmp = workspace();
        // A PERSONA rather than the channel, so the receipt itself names the session: nothing was
        // spawned, so there is no `.nxs/agent-logs` to read one out of — which is the very state
        // under test.
        let coder = json_of(dry(&tmp).args(["--json", "send", "--no-ref", "--to", "coder", "T5"]))
            ["session"]
            .as_str()
            .expect("the receipt names the session it minted")
            .to_string();

        let seen = json_of(dry(&tmp).args(["--json", "session", "state", &coder]));
        assert_eq!(
            seen["sessions"][0]["state"], "unknown",
            "nothing was spawned, so nothing announced and nothing answers: {seen}"
        );
        assert_eq!(
            seen["worker_answers_liveness"], false,
            "and the report says the silence is the worker's, not the session's: {seen}"
        );

        let human = stdout_of(dry(&tmp).args(["session", "state", &coder]));
        assert!(
            human.contains("cannot answer"),
            "the terminal line says the same thing in words, got: {human}"
        );
    }

    #[test]
    fn naming_neither_a_session_nor_a_thread_is_refused_by_the_parser() {
        let tmp = workspace();
        let refused = nxc(&tmp).args(["session", "state"]).assert().failure();
        let said = String::from_utf8_lossy(&refused.get_output().stderr).into_owned();
        assert!(
            said.contains("--thread") || said.contains("INTERNAL"),
            "the refusal says what is missing, got: {said}"
        );
    }
}
