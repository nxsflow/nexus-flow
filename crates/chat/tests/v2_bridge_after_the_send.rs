//! The bridge between the two engines, part 2 (6j6v.y6v7; epic 6j6v.zjyv §10, §12.9): reply, withdraw, resume
//! and status on a run of the new engine, through the `surface` functions the CLI and `Engine::*` both call,
//! with the v2 lookup first.
//!
//! Each case runs at the seam through `Engine::*`, and the ones §12.9 names once more through the `nxc`
//! binary. A run is stepped by a real `Host` over `ports_v1`, with a scripted backend and tree.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::{ErrorKind, NxfError};
use nexus_chat::facade::{Reader, StatusScope};
use nexus_chat::orchestration::{Caller, ResumeOutcome, ResumeRequest, ResumeScope};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nexus_run_contracts::{EndCause, RunId, RunStore, Timestamp};
use nexus_run_host::bridge::{BridgeCtx, ReplyFlags, RunView, SqliteRunStore};
use nexus_run_host::test_support::scripted_backend::{
    pass, Cue, Scenario, ScriptedBackend, SessionInfo,
};
use nexus_run_host::test_support::scripted_tree::ScriptedTree;
use nexus_run_host::test_support::SimClock;
use nexus_run_host::{Host, Layout, Ports, TreeLedger};
use tempfile::TempDir;

const NOW: &str = "2026-11-13T10:00:00Z";
const CODER: &str =
    "handle: coder\njob_title: Implementer\naddressable: none\nsystem_prompt: You build.\n";
const VERIFIER: &str =
    "handle: verifier\njob_title: Verifier\naddressable: none\nsystem_prompt: You verify.\n";
const REVIEWER: &str = "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You review.\n";
const CODE_QUALITY: &str =
    "handle: code-quality\njob_title: Reviewer\naddressable: none\nsystem_prompt: You judge the code.\n";
const INTEGRITY: &str =
    "handle: integrity\njob_title: Reviewer\naddressable: none\nsystem_prompt: You judge the claims.\n";
/// `coding-v2` in miniature (build -> verify, one pass, so a second verdict reaches the ceiling), a two-step
/// run that ends in a fan-out to a `requester_only` panel, and a v1 `review`.
const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
      next: verify
    - id: verify
      target: verifier
      on_needs_rework: build
      max_passes: 2
      input: []
- name: judged-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
      next: judge
    - id: judge
      target: panel
- name: panel
  members: [code-quality, integrity]
  visibility: requester_only
  on_complete: summarize
  summary_prompt: Consolidate the verdicts.
- name: review
  members: [coder, reviewer]
";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    for (file, body) in [
        ("coder.yaml", CODER),
        ("verifier.yaml", VERIFIER),
        ("reviewer.yaml", REVIEWER),
        ("code-quality.yaml", CODE_QUALITY),
        ("integrity.yaml", INTEGRITY),
        ("channels.yaml", CHANNELS),
    ] {
        std::fs::write(personas.join(file), body).unwrap();
    }
    tmp
}

fn engine(dir: &Path) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

fn chat_store(dir: &Path) -> ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Every write to the chat store is an op: the count is what "before anything is written" is measured by.
fn chat_ops(dir: &Path) -> i64 {
    rusqlite::Connection::open(dir.join(".nxs").join("db.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM ops", [], |row| row.get(0))
        .unwrap()
}

fn run_sqlite(dir: &Path) -> PathBuf {
    dir.join(".nxs").join("run.sqlite")
}

fn db_path(dir: &Path) -> String {
    dir.join(".nxs")
        .join("db.sqlite")
        .to_str()
        .unwrap()
        .to_owned()
}

/// A session the old engine started for `handle`, as its session map records it.
fn persona_session(dir: &Path, handle: &str) -> String {
    let session = format!("s-{handle}");
    chat_store(dir)
        .create_pending_session(&session, handle)
        .unwrap();
    session
}

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

fn escalating(info: &SessionInfo) -> Cue {
    Cue::Reply {
        text: format!("{} cannot", info.member),
        flags: ReplyFlags {
            escalate: true,
            needs_rework: false,
        },
    }
}

/// The verifier sends the work back every time: with one pass declared, the second verdict is the ceiling.
fn to_the_ceiling(info: &SessionInfo) -> Cue {
    if info.step == "verify" {
        Cue::Reply {
            text: "not yet".to_owned(),
            flags: ReplyFlags {
                escalate: false,
                needs_rework: true,
            },
        }
    } else {
        pass()
    }
}

/// The build session ends on the usage limit, which resets an hour on.
fn on_the_usage_limit(info: &SessionInfo) -> Cue {
    if info.step == "build" {
        Cue::End(EndCause::UsageLimit {
            resets_at: Some(Timestamp::from_unix_millis(
                Timestamp::parse(NOW).unwrap().unix_millis() + 3_600_000,
            )),
        })
    } else {
        pass()
    }
}

/// One commission on the new engine, sent by the owner at the terminal, and a real `Host` that steps it.
struct V2Run {
    ws: TempDir,
    eng: Engine,
    host: Host,
    backend: Arc<ScriptedBackend>,
    bridge: BridgeCtx,
    store: Arc<SqliteRunStore>,
    clock: Arc<SimClock>,
    tree: Arc<ScriptedTree>,
    run: RunId,
    root: String,
}

impl V2Run {
    fn commissioned(channel: &str, scenario: Scenario) -> V2Run {
        V2Run::commissioned_in(workspace(), channel, scenario)
    }

    fn commissioned_in(ws: TempDir, channel: &str, scenario: Scenario) -> V2Run {
        let dir = ws.path().to_owned();
        let eng = engine(&dir);
        let receipt = eng
            .send_to(
                the_owner(),
                SendToRequest {
                    to: channel,
                    body: "build it",
                    refs: SendToRefs::declared(nexus_chat::model::Refs {
                        nxf_ids: vec!["6j6v.y6v7".to_owned()],
                        ..nexus_chat::model::Refs::default()
                    }),
                    machine: None,
                },
            )
            .expect("the owner commissions");
        let db = db_path(&dir);
        let ports =
            Arc::new(nexus_chat::ports_v1::V1Ports::for_root(&dir, &db, &eng.origin()).unwrap());
        let layout = Layout {
            root: dir.clone(),
            agent_logs: dir.join(".nxs").join("agent-logs"),
            session_env: Vec::new(),
        };
        let store = Arc::new(
            SqliteRunStore::open(&run_sqlite(&dir), nexus_run_host::bridge::HOST_BUSY_TIMEOUT)
                .unwrap(),
        );
        let clock = Arc::new(SimClock::at(Timestamp::parse(NOW).unwrap().unix_millis()));
        let backend = Arc::new(ScriptedBackend::new(
            &layout.agent_logs,
            &dir.join("timeline.jsonl"),
            &layout.v2_dir(),
            scenario,
        ));
        let tree = Arc::new(ScriptedTree::new(TreeLedger::new(
            &layout.agent_logs,
            &eng.origin(),
        )));
        let host = Host::new(
            store.clone(),
            Ports {
                messages: ports.clone(),
                declarations: ports.clone(),
                sidecar: ports.clone(),
                layout: layout.clone(),
            },
            backend.clone(),
            tree.clone(),
            clock.clone(),
        );
        let bridge = BridgeCtx {
            store: Arc::new(
                SqliteRunStore::open(
                    &run_sqlite(&dir),
                    nexus_run_host::bridge::BRIDGE_BUSY_TIMEOUT,
                )
                .unwrap(),
            ),
            messages: ports.clone(),
            declarations: ports,
            layout,
            workspace_id: eng.origin(),
        };
        let run = store.open_runs().unwrap()[0].run.clone();
        V2Run {
            ws,
            eng,
            host,
            backend,
            bridge,
            store,
            clock,
            tree,
            run,
            root: receipt.thread_id,
        }
    }

    fn dir(&self) -> &Path {
        self.ws.path()
    }

    fn now(&self) -> Timestamp {
        nexus_run_host::ports::Clock::now(&*self.clock)
    }

    /// The instant as the CLI and `Engine::*` are handed it.
    fn now_text(&self) -> String {
        self.now().to_string()
    }

    /// One tick, then the sessions, then the clock: unless the tree is working, because that is real
    /// time on a worker thread and a simulated window must not close around it.
    fn tick(&mut self) {
        let now = self.now();
        self.host.tick(now);
        self.backend.advance(&self.bridge, now);
        if self.tree.busy() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            self.clock.advance(1_000);
        }
    }

    fn view(&self) -> RunView {
        nexus_run_host::bridge::view(&self.bridge.store, &self.run)
            .unwrap()
            .unwrap()
            .state
            .view()
    }

    /// Ticks until `done`. The 400 ticks count only those the run could use: a tick the tree's worker
    /// thread is still working through is real time, and how much of it there is the machine's to say
    /// (a loaded 3-core runner left the run `Prepared` through 400 uncounted ticks). Only a hung tree
    /// ends on the wall-clock ceiling.
    fn until(&mut self, what: &str, done: impl Fn(&V2Run) -> bool) {
        let ceiling = std::time::Instant::now() + std::time::Duration::from_secs(600);
        let mut counted = 0;
        while counted < 400 && std::time::Instant::now() < ceiling {
            if done(self) {
                return;
            }
            let working = self.tree.busy();
            self.tick();
            if !working {
                counted += 1;
            }
        }
        assert!(
            done(self),
            "never reached {what}: {:?}\n{}",
            self.view(),
            self.where_it_stands()
        );
    }

    /// For a failure message: the run's log and the sessions' timeline, so a red names where the run
    /// stopped and not only the view it stopped in.
    fn where_it_stands(&self) -> String {
        let events: Vec<String> = self
            .store
            .load(&self.run, 0)
            .unwrap()
            .iter()
            .map(|event| format!("{:?}", event.body))
            .collect();
        let timeline: Vec<String> = self
            .backend
            .timeline()
            .iter()
            .map(|entry| format!("{} {}", entry.what, entry.session_key))
            .collect();
        format!(
            "tree busy: {}\nevents:\n  {}\ntimeline:\n  {}",
            self.tree.busy(),
            events.join("\n  "),
            timeline.join("\n  ")
        )
    }

    fn until_view(&mut self, what: &str, matches: impl Fn(&RunView) -> bool) {
        self.until(what, |run| matches(&run.view()));
    }

    /// The build slot's session key, once the host has started it.
    fn build_key(&mut self) -> String {
        let key = format!("{}/build/v1/coder/a1", self.run);
        let probe = key.clone();
        self.until("the build session", move |run| {
            run.store
                .get(&format!("session/{probe}"))
                .unwrap()
                .is_some()
        });
        key
    }

    fn thread_of(&self, key: &str) -> String {
        String::from_utf8(self.store.get(&format!("session/{key}")).unwrap().unwrap()).unwrap()
    }

    /// What a verb put into the run's inbox and the run has not taken up yet — the inputs from the CLI's
    /// side, not the host's own.
    fn inputs(&self) -> Vec<String> {
        self.store
            .waiting_inputs(&self.run)
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .filter(|input| input.origin == nexus_run_contracts::Origin::Cli)
            .map(|input| input.body.kind())
            .collect()
    }

    fn owner(&self) -> Caller<'static> {
        the_owner()
    }

    fn reply(
        &self,
        caller: Caller<'_>,
        thread: &str,
        body: &str,
        accept: bool,
    ) -> nexus_chat::error::Result<nexus_chat::orchestration::ReplyReceipt> {
        self.reply_with(
            caller,
            ReplyThreadRequest {
                thread,
                body,
                escalate: false,
                needs_rework: false,
                accept,
                machine: None,
            },
        )
    }

    /// A reply with every field of the request the caller's to set.
    fn reply_with(
        &self,
        caller: Caller<'_>,
        req: ReplyThreadRequest<'_>,
    ) -> nexus_chat::error::Result<nexus_chat::orchestration::ReplyReceipt> {
        let now = self.now_text();
        self.eng.reply_thread(
            Caller {
                now: Some(&now),
                ..caller
            },
            req,
        )
    }

    fn withdraw(
        &self,
        caller: Caller<'_>,
        thread: &str,
    ) -> nexus_chat::error::Result<nexus_chat::orchestration::WithdrawReceipt> {
        let now = self.now_text();
        self.eng.withdraw(
            Caller {
                now: Some(&now),
                ..caller
            },
            thread,
        )
    }

    fn resume(
        &self,
        caller: Caller<'_>,
        thread: &str,
        force: bool,
    ) -> nexus_chat::error::Result<nexus_chat::orchestration::ResumeReceipt> {
        let now = self.now_text();
        self.eng.resume_interrupted(
            Caller {
                now: Some(&now),
                ..caller
            },
            ResumeRequest {
                scope: ResumeScope::Thread(thread),
                force,
            },
        )
    }

    fn status(&self, scope: StatusScope<'_>) -> nexus_chat::facade::StatusReport {
        self.eng
            .status(Reader::Handle("owner"), &self.now_text(), scope)
            .expect("status reads")
    }

    fn status_of_root(&self) -> serde_json::Value {
        let one = [self.root.as_str()];
        serde_json::to_value(self.status(StatusScope::Threads(&one))).unwrap()
    }

    fn ops(&self) -> i64 {
        chat_ops(self.dir())
    }
}

fn the_owner() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("owner"),
        now: Some(NOW),
    }
}

fn as_session<'a>(handle: &'a str, session: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: Some(handle),
        now: None,
    }
}

fn assert_refused_naming(refused: &NxfError, words: &[&str]) {
    for word in words {
        assert!(refused.msg.contains(word), "`{word}` in: {refused}");
    }
}

// ---- reply on a step thread ------------------------------------------------------------------

#[test]
fn the_slots_own_session_replies_in_its_step_thread_and_the_run_hears_it() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let receipt = run
        .reply(as_session("coder", &key), &step, "built", false)
        .expect("the slot answers");
    assert!(receipt.posted);
    let message = receipt.message_id.expect("posted");
    assert_eq!(run.inputs(), ["reply"]);
    let posted = chat_store(run.dir()).messages_in_thread(&step).unwrap();
    assert!(
        posted
            .iter()
            .any(|m| m.message_id == message && m.body == "built"),
        "{posted:?}"
    );
    run.until("the verify step", |run| {
        run.store
            .get(&format!("session/{}/verify/v1/verifier/a1", run.run))
            .unwrap()
            .is_some()
    });
}

#[test]
fn anyone_but_the_slots_session_is_refused_in_a_step_thread_before_anything_is_written() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let persona = persona_session(run.dir(), "coder");
    let before = run.ops();
    let qualified = format!("{}/coder", run.eng.origin());
    for caller in [
        run.owner(),
        as_session("coder", &persona),
        Caller {
            session: None,
            actor: Some(&qualified),
            now: None,
        },
        as_session("coder", "a-key-nobody-has"),
    ] {
        let refused = run
            .reply(caller, &step, "me too", false)
            .expect_err("not the slot");
        assert_eq!(refused.kind, ErrorKind::Forbidden, "{caller:?}: {refused}");
        assert_refused_naming(&refused, &["coder", "nothing was written"]);
    }
    let accept = run
        .reply(as_session("coder", &key), &step, "fine", true)
        .expect_err("--accept is the root's");
    assert_refused_naming(&accept, &["--accept", "ROOT"]);
    assert_eq!(run.ops(), before, "nothing posted");
    assert!(run.inputs().is_empty(), "nothing enqueued");
}

// ---- reply on the root -----------------------------------------------------------------------

#[test]
fn the_requester_answers_an_escalation_on_the_root_and_the_run_hears_it() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    run.backend.set_scenario(Box::new(|_| pass()));
    let stranger = run
        .reply(
            Caller {
                session: None,
                actor: Some("someone-else"),
                now: None,
            },
            &run.root.clone(),
            "I say go",
            false,
        )
        .expect_err("a person who did not commission the run");
    assert_refused_naming(&stranger, &["only", "owner", "nothing was written"]);
    assert!(run.inputs().is_empty());
    let qualified = format!("{}/owner", run.eng.origin());
    let receipt = run
        .reply(
            Caller {
                session: None,
                actor: Some(&qualified),
                now: None,
            },
            &run.root.clone(),
            "go on with the fixture repo",
            false,
        )
        .expect("the requester, handed qualified, is the requester");
    assert!(receipt.posted);
    assert_eq!(run.inputs(), ["answer"]);
    let guidance = receipt
        .await_
        .expect("the root's reader is told how to follow");
    assert_eq!(
        serde_json::to_value(&guidance).unwrap()["poll"],
        serde_json::json!(["nxc", "status", "--thread", run.root, "--json"])
    );
}

#[test]
fn accept_is_refused_on_an_escalation_and_carries_the_run_on_at_a_ceiling() {
    let mut escalated = V2Run::commissioned("coding-v2", Box::new(escalating));
    escalated.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let root = escalated.root.clone();
    let before = escalated.ops();
    let refused = escalated
        .reply(escalated.owner(), &root, "accepted", true)
        .expect_err("an escalation is answered, not accepted");
    assert_refused_naming(&refused, &["--accept", "ceiling"]);
    assert_eq!(escalated.ops(), before);

    let mut ceiling = V2Run::commissioned("coding-v2", Box::new(to_the_ceiling));
    ceiling.until_view("the ceiling", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let root = ceiling.root.clone();
    ceiling
        .reply(ceiling.owner(), &root, "ship it as it is", true)
        .expect("accepted at the ceiling");
    assert_eq!(ceiling.inputs(), ["answer"]);
    ceiling.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
}

#[test]
fn a_plain_root_reply_on_a_held_run_is_refused_naming_status_resume_and_withdraw() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(on_the_usage_limit));
    run.until_view("the hold", |view| matches!(view, RunView::Held { .. }));
    let root = run.root.clone();
    let before = run.ops();
    let refused = run
        .reply(run.owner(), &root, "retry", false)
        .expect_err("a reply never restarts a run");
    assert_refused_naming(
        &refused,
        &[
            &format!("nxc status --thread {root}"),
            &format!("nxc resume --thread {root}"),
            &format!("nxc withdraw --thread {root}"),
            "nothing was written",
        ],
    );
    assert_eq!(run.ops(), before);
    assert!(
        run.inputs().is_empty(),
        "no resume enqueued: {:?}",
        run.inputs()
    );
}

#[test]
fn a_plain_root_reply_on_a_working_run_is_refused_with_nothing_written() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    run.build_key();
    let root = run.root.clone();
    let before = run.ops();
    let refused = run
        .reply(run.owner(), &root, "how is it going?", false)
        .expect_err("nothing to answer");
    assert_refused_naming(
        &refused,
        &["working", "nxc status --thread", "nxc withdraw"],
    );
    assert_eq!(run.ops(), before);
    assert!(run.inputs().is_empty());
}

#[test]
fn a_reply_on_a_finished_run_is_refused_before_anything_is_written() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(|_| pass()));
    let key = run.build_key();
    let step = run.thread_of(&key);
    run.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
    let before = run.ops();
    let root = run.root.clone();
    for (caller, thread) in [
        (run.owner(), root.as_str()),
        (as_session("coder", &key), step.as_str()),
    ] {
        let refused = run
            .reply(caller, thread, "late", false)
            .expect_err("finished");
        assert_refused_naming(&refused, &["start a new commission", "nxc send"]);
    }
    assert_eq!(run.ops(), before);
}

/// A session is a SESSION to the root (§10): a v2 session cannot answer or accept on its own run's root, and a
/// v1 persona session cannot act on a v2 root — whatever handle either presents.
#[test]
fn no_session_answers_accepts_or_withdraws_on_a_v2_root() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let key = format!("{}/build/v1/coder/a1", run.run);
    let persona = persona_session(run.dir(), "reviewer");
    let root = run.root.clone();
    let before = run.ops();
    for (who, caller, words) in [
        (
            "the v2 session",
            as_session("owner", &key),
            &["new engine", "--escalate"][..],
        ),
        (
            "a v1 persona",
            as_session("reviewer", &persona),
            &["only", "reviewer"][..],
        ),
    ] {
        for accept in [false, true] {
            let refused = run
                .reply(caller, &root, "yes", accept)
                .expect_err("not the requester");
            assert_eq!(refused.kind, ErrorKind::Forbidden, "{who}: {refused}");
            assert_refused_naming(&refused, words);
        }
        let refused = run.withdraw(caller, &root).expect_err("not the requester");
        assert_eq!(refused.kind, ErrorKind::Forbidden, "{who}: {refused}");
    }
    assert_eq!(run.ops(), before);
    assert!(run.inputs().is_empty());
}

// ---- withdraw and resume ---------------------------------------------------------------------

#[test]
fn the_requester_withdraws_a_run_on_its_root_and_a_step_thread_is_refused() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let refused = run
        .withdraw(run.owner(), &step)
        .expect_err("the root's verb");
    assert_refused_naming(&refused, &[&run.root.clone(), "root"]);
    let root = run.root.clone();
    let receipt = run.withdraw(run.owner(), &root).expect("withdrawn");
    assert_eq!(receipt.run.as_deref(), Some(run.run.as_str()));
    assert_eq!(receipt.thread_id, root);
    assert_eq!(run.inputs(), ["withdrawn"]);
    run.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
    let again = run.withdraw(run.owner(), &root).expect_err("finished");
    assert_refused_naming(&again, &["start a new commission"]);
}

#[test]
fn resume_says_not_yet_before_the_hold_lifts_and_force_restarts_now() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(on_the_usage_limit));
    run.until_view("the hold", |view| matches!(view, RunView::Held { .. }));
    let root = run.root.clone();
    let receipt = run.resume(run.owner(), &root, false).expect("not yet");
    assert!(
        matches!(&receipt.outcome, ResumeOutcome::NotYet { until: Some(_) }),
        "{receipt:?}"
    );
    assert!(run.inputs().is_empty());
    let key = format!("{}/build/v1/coder/a1", run.run);
    for caller in [
        as_session("coder", &key),
        as_session("reviewer", "s-reviewer"),
    ] {
        run.resume(caller, &root, true)
            .expect_err("the requester's, from the terminal");
    }
    assert!(run.inputs().is_empty());
    let receipt = run.resume(run.owner(), &root, true).expect("forced");
    assert_eq!(receipt.outcome, ResumeOutcome::Resumed);
    assert_eq!(receipt.run.as_deref(), Some(run.run.as_str()));
    assert_eq!(run.inputs(), ["resume"]);
}

// ---- status ----------------------------------------------------------------------------------

fn root_row(status: &serde_json::Value) -> &serde_json::Value {
    &status["operations"][0]
}

/// `Engine::status` answers for a v2 root as the CLI does, in three states (round 3 TQ #13).
#[test]
fn engine_status_maps_a_v2_root_in_created_needs_decision_and_finished() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    let created = run.status_of_root();
    let op = root_row(&created);
    assert_eq!(op["root"], run.root.as_str(), "{created}");
    assert_eq!(op["open"], 1);
    assert_eq!(op["needs_decision"], false);
    assert_eq!(op["threads"][0]["awaiting_human"], false);
    assert_eq!(op["run"]["state"]["name"], "created", "{created}");

    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let asked = run.status_of_root();
    let op = root_row(&asked);
    assert_eq!(op["open"], 1);
    assert_eq!(op["needs_decision"], true, "{asked}");
    assert_eq!(op["threads"][0]["awaiting_human"], true);
    assert_eq!(op["run"]["state"]["name"], "needs_decision");

    run.withdraw(run.owner(), &run.root.clone()).unwrap();
    run.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
    let finished = run.status_of_root();
    let op = root_row(&finished);
    assert_eq!(op["open"], 0, "{finished}");
    assert_eq!(op["live"], false);
    assert_eq!(op["needs_decision"], false);
    assert_eq!(op["run"]["finished"], true);
}

#[test]
fn a_held_run_is_awaiting_a_human_and_its_status_carries_the_working_copy_and_local() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(on_the_usage_limit));
    run.until_view("the hold", |view| matches!(view, RunView::Held { .. }));
    let status = run.status_of_root();
    let op = root_row(&status);
    assert_eq!(op["open"], 1);
    assert_eq!(op["needs_decision"], false);
    assert_eq!(op["threads"][0]["awaiting_human"], true, "{status}");
    assert!(op["run"]["local"]["owner"].is_object(), "local: {status}");
    assert_eq!(
        op["run"]["local"]["tree"]["path"],
        format!("{}-v2", run.dir().display()),
        "{status}"
    );
}

#[test]
fn a_v2_run_is_listed_once_beside_the_v1_operations() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    run.build_key();
    run.eng
        .send_to(
            the_owner(),
            SendToRequest {
                to: "review",
                body: "look",
                refs: SendToRefs::declared(nexus_chat::model::Refs::default()),
                machine: None,
            },
        )
        .unwrap();
    let report = run.status(StatusScope::Workspace);
    let roots: Vec<&str> = report
        .operations
        .iter()
        .map(|op| op.root.as_str())
        .collect();
    assert_eq!(
        roots.iter().filter(|root| **root == run.root).count(),
        1,
        "once: {roots:?}"
    );
    assert_eq!(roots.len(), 2, "and the v1 round beside it: {roots:?}");
    let ours = report
        .operations
        .iter()
        .find(|op| op.root == run.root)
        .unwrap();
    assert!(ours.run.is_some());
    let one_channel = run.status(StatusScope::Channel("coding-v2"));
    assert_eq!(one_channel.operations.len(), 1);
    // `--all` lists what the old engine does not count as live too — the root thread it finds in the chat
    // store among them — and the run is still there once, as the run.
    let everything = run.status(StatusScope::All(None));
    let ours: Vec<_> = everything
        .operations
        .iter()
        .filter(|op| op.root == run.root)
        .collect();
    assert_eq!(
        ours.len(),
        1,
        "once under --all: {:?}",
        everything.operations
    );
    assert!(ours[0].run.is_some(), "and as the run");
}

/// A write lock on `path`, held from before this returns until the returned sender is dropped or sent to —
/// released by the test when it has what it waited for, never by a clock.
fn hold_a_write_lock(path: PathBuf) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (taken, taken_rx) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        taken.send(()).unwrap();
        let _ = release_rx.recv();
        conn.execute_batch("ROLLBACK").unwrap();
    });
    taken_rx.recv().unwrap();
    (release, holder)
}

// ---- a held run.sqlite lock ------------------------------------------------------------------

/// The bridge waits out a short stall on `run.sqlite` (5 s), so a reply is posted and enqueued once; a lock
/// held past that refuses with the posted message named and the one command to run, and nothing retries
/// silently (round 4 CQ #15, TQ #11).
#[test]
fn a_reply_against_a_held_run_sqlite_lock_is_posted_once_and_enqueued_or_named() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let posts = || {
        chat_store(run.dir())
            .messages_in_thread(&step)
            .unwrap()
            .len()
    };
    let before = posts();

    let lock = |held_for: std::time::Duration| {
        let path = run_sqlite(run.dir());
        let (taken, taken_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let conn = rusqlite::Connection::open(path).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            taken.send(()).unwrap();
            std::thread::sleep(held_for);
            conn.execute_batch("COMMIT").unwrap();
        });
        taken_rx.recv().unwrap();
        holder
    };

    // The one stall that has to END while the reply still waits, so it is timed: 0.8 s inside the bridge's
    // fixed 5 s, a margin of six times the stall. The long holds below are released by the test instead.
    let short = lock(std::time::Duration::from_millis(800));
    run.reply(as_session("coder", &key), &step, "first", false)
        .expect("the stall is waited out");
    short.join().unwrap();
    assert_eq!(posts(), before + 1, "posted once");
    assert_eq!(run.inputs(), ["reply"], "and enqueued once");

    let mut held = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = held.build_key();
    let step = held.thread_of(&key);
    let posts = || {
        chat_store(held.dir())
            .messages_in_thread(&step)
            .unwrap()
            .len()
    };
    let before = posts();
    let (release, holder) = hold_a_write_lock(run_sqlite(held.dir()));
    let started = std::time::Instant::now();
    let refused = held
        .reply(as_session("coder", &key), &step, "second", false)
        .expect_err("the lock outlasts the bridge's timeout");
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(5),
        "waited the bridge's 5 s first: {:?}",
        started.elapsed()
    );
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_refused_naming(
        &refused,
        &[
            "is posted but the run has not heard of it",
            "run the command again",
        ],
    );
    assert_eq!(posts(), before + 1, "posted once, never twice");
    assert!(
        held.inputs().is_empty(),
        "and not enqueued: {:?}",
        held.inputs()
    );
}

// ---- the read filters ------------------------------------------------------------------------

/// The panel's threads are opened in `panel` (`requester_only`) with the supervisor as opener: a reviewer
/// does not read another reviewer's answer, and the requester reads both (§6.1; design review CQ #21).
#[test]
fn a_reviewer_does_not_read_another_reviewers_answer_and_the_requester_reads_both() {
    let mut run = V2Run::commissioned(
        "judged-v2",
        Box::new(|info: &SessionInfo| Cue::Reply {
            text: format!("{} says yes", info.member),
            flags: ReplyFlags::default(),
        }),
    );
    let quality = format!("{}/judge/v1/code-quality/a1", run.run);
    let integrity = format!("{}/judge/v1/integrity/a1", run.run);
    let (q, i) = (quality.clone(), integrity.clone());
    run.until("both verdicts", move |run| {
        [&q, &i].iter().all(|key| {
            run.store
                .get(&format!("session/{key}"))
                .unwrap()
                .is_some_and(|thread| {
                    let thread = String::from_utf8(thread).unwrap();
                    chat_store(run.dir())
                        .messages_in_thread(&thread)
                        .unwrap()
                        .iter()
                        .any(|m| m.body.ends_with("says yes"))
                })
        })
    });
    let integrity_thread = run.thread_of(&integrity);
    let bodies = |reader: Reader<'_>| -> Vec<String> {
        run.eng
            .thread(&integrity_thread, reader)
            .expect("readable")
            .messages
            .iter()
            .map(|m| m.body.clone())
            .collect()
    };
    let as_quality = bodies(Reader::Handle("code-quality"));
    assert!(
        !as_quality.iter().any(|b| b == "integrity says yes"),
        "another reviewer's answer is hidden: {as_quality:?}"
    );
    let owner = format!("{}/owner", run.eng.origin());
    let as_requester = bodies(Reader::Handle(&owner));
    assert!(
        as_requester.iter().any(|b| b == "integrity says yes"),
        "the requester reads it: {as_requester:?}"
    );
}

// ---- once through the CLI --------------------------------------------------------------------

fn nxc(dir: &Path, now: &str) -> assert_cmd::Command {
    let mut cmd = nxs_test_support::cargo_bin("nxc");
    cmd.current_dir(dir)
        .env_remove("NXC_SESSION")
        .env_remove("NXC_DB")
        .env("NXC_ACTOR", "owner")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", dir.join("dry.log"));
    cmd
}

fn nxc_as(dir: &Path, now: &str, handle: &str, session: &str) -> assert_cmd::Command {
    let mut cmd = nxc(dir, now);
    cmd.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    cmd
}

fn stdout(cmd: &mut assert_cmd::Command) -> String {
    String::from_utf8(cmd.assert().success().get_output().stdout.clone()).unwrap()
}

fn stderr_of_a_refusal(cmd: &mut assert_cmd::Command) -> String {
    String::from_utf8(cmd.assert().failure().get_output().stderr.clone()).unwrap()
}

fn json(cmd: &mut assert_cmd::Command) -> serde_json::Value {
    serde_json::from_str(&stdout(cmd)).expect("one JSON record")
}

#[test]
fn nxc_reply_by_the_slot_with_and_without_the_thread_and_refused_to_anyone_else() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let now = run.now_text();
    let persona = persona_session(run.dir(), "reviewer");
    let before = run.ops();
    let said = stderr_of_a_refusal(
        nxc_as(run.dir(), &now, "reviewer", &persona).args(["reply", "--thread", &step, "me"]),
    );
    assert!(said.contains("coder"), "{said}");
    let said = stderr_of_a_refusal(nxc(run.dir(), &now).args(["reply", "--thread", &step, "me"]));
    assert!(said.contains("nothing was written"), "{said}");
    assert_eq!(run.ops(), before);

    let out =
        stdout(nxc_as(run.dir(), &now, "coder", &key).args(["reply", "built, without an id"]));
    assert!(
        out.contains(&format!("in thread {step}")),
        "the id-free form: {out}"
    );
    assert_eq!(run.inputs(), ["reply"]);
}

#[test]
fn nxc_reply_accept_resume_and_withdraw_on_the_root() {
    let mut ceiling = V2Run::commissioned("coding-v2", Box::new(to_the_ceiling));
    ceiling.until_view("the ceiling", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let now = ceiling.now_text();
    let root = ceiling.root.clone();
    let receipt = json(
        nxc(ceiling.dir(), &now)
            .args(["--json", "reply", "--thread", &root, "--accept", "ship it"]),
    );
    assert_eq!(receipt["posted"], true, "{receipt}");
    assert_eq!(ceiling.inputs(), ["answer"]);

    let mut held = V2Run::commissioned("coding-v2", Box::new(on_the_usage_limit));
    held.until_view("the hold", |view| matches!(view, RunView::Held { .. }));
    let now = held.now_text();
    let root = held.root.clone();
    let said =
        stderr_of_a_refusal(nxc(held.dir(), &now).args(["reply", "--thread", &root, "retry"]));
    assert!(
        said.contains(&format!("nxc resume --thread {root}")),
        "{said}"
    );
    let not_yet = stdout(nxc(held.dir(), &now).args(["resume", "--thread", &root]));
    assert!(not_yet.contains("not yet"), "{not_yet}");
    assert!(held.inputs().is_empty());
    let forced =
        json(nxc(held.dir(), &now).args(["--json", "resume", "--thread", &root, "--force"]));
    assert_eq!(forced["outcome"], "resumed", "{forced}");
    assert_eq!(forced["run"], held.run.as_str());
    assert_eq!(held.inputs(), ["resume"]);

    let out = stdout(nxc(held.dir(), &now).args(["withdraw", "--thread", &root]));
    assert!(
        out.contains(&format!("nxc status --thread {root}")),
        "{out}"
    );
    assert_eq!(held.inputs(), ["resume", "withdrawn"]);
}

#[test]
fn nxc_status_json_carries_the_watcher_fields_and_run_prints_contract_d() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let now = run.now_text();
    let root = run.root.clone();
    let status = json(nxc(run.dir(), &now).args(["--json", "status", "--thread", &root]));
    let op = root_row(&status);
    assert_eq!(op["open"], 1, "{status}");
    assert_eq!(op["needs_decision"], true);
    assert_eq!(op["threads"][0]["awaiting_human"], true);
    assert_eq!(op["run"]["run"], run.run.as_str());

    let listing = json(nxc(run.dir(), &now).args(["--json", "status"]));
    assert_eq!(
        listing["operations"].as_array().unwrap().len(),
        1,
        "{listing}"
    );

    let raw = json(nxc(run.dir(), &now).args(["--json", "status", "--run", &root]));
    assert_eq!(raw["run"], run.run.as_str(), "the root's run: {raw}");
    assert!(raw["local"].is_object(), "`local` with --json: {raw}");
    let by_run = json(nxc(run.dir(), &now).args(["--json", "status", "--run", run.run.as_str()]));
    assert_eq!(by_run["run"], run.run.as_str());

    let human = stdout(nxc(run.dir(), &now).args(["status", "--thread", &root]));
    assert!(human.contains("NEEDS DECISION"), "{human}");
    assert!(
        human.contains(&format!("{}-v2", run.dir().display())),
        "the tree: {human}"
    );
    assert!(
        human.contains("no service attends") || human.contains("service"),
        "`local`'s owner: {human}"
    );
}

/// The chat store is where a reply is POSTED, before the run hears of it: a write lock on `db.sqlite` that
/// outlasts its 5 s refuses the reply with the reason, and the run is handed nothing (§12.9).
#[test]
fn a_held_chat_store_lock_refuses_the_reply_with_the_reason_and_enqueues_nothing() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let (release, holder) = hold_a_write_lock(run.dir().join(".nxs").join("db.sqlite"));
    let refused = run
        .reply(as_session("coder", &key), &step, "built", false)
        .expect_err("the chat store does not take the post");
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        refused.msg.contains("locked") || refused.msg.contains("busy"),
        "the reason: {refused}"
    );
    assert!(
        run.inputs().is_empty(),
        "nothing enqueued: {:?}",
        run.inputs()
    );
}

#[test]
fn nxc_refuses_a_late_reply_and_every_session_on_a_v2_root() {
    let mut finished = V2Run::commissioned("coding-v2", Box::new(|_| pass()));
    finished.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
    let now = finished.now_text();
    let root = finished.root.clone();
    let said =
        stderr_of_a_refusal(nxc(finished.dir(), &now).args(["reply", "--thread", &root, "late"]));
    assert!(said.contains("start a new commission"), "{said}");

    let mut asked = V2Run::commissioned("coding-v2", Box::new(escalating));
    asked.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let now = asked.now_text();
    let root = asked.root.clone();
    let key = format!("{}/build/v1/coder/a1", asked.run);
    let persona = persona_session(asked.dir(), "reviewer");
    let before = asked.ops();
    let own_root = stderr_of_a_refusal(
        nxc_as(asked.dir(), &now, "coder", &key)
            .args(["reply", "--thread", &root, "--accept", "ok"]),
    );
    assert!(own_root.contains("new engine"), "{own_root}");
    for args in [
        &["reply", "--thread", root.as_str(), "yes"][..],
        &["withdraw", "--thread", root.as_str()][..],
    ] {
        let said = stderr_of_a_refusal(nxc_as(asked.dir(), &now, "reviewer", &persona).args(args));
        assert!(said.contains("only"), "{args:?}: {said}");
    }
    assert_eq!(asked.ops(), before);
    assert!(asked.inputs().is_empty());
}

/// The receipt of a v2 send names `nxc status --thread <root> --json` as the read, `open: 0` as done and
/// `needs_decision`/`awaiting_human` as stopped (part 1). Here those predicates are held to the payload they
/// name: false while the run works, `stopped` when it asks, `done` once it finished — as
/// `the_done_predicate_holds_against_the_payload_it_names` holds v1's.
#[test]
fn the_v2_receipts_predicates_hold_against_the_status_payload() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    let read = |run: &V2Run| {
        let now = run.now_text();
        json(nxc(run.dir(), &now).args(["status", "--thread", &run.root, "--json"]))
    };
    let done = |status: &serde_json::Value| root_row(status)["open"] == 0;
    let stopped = |status: &serde_json::Value| {
        let op = root_row(status);
        op["needs_decision"] == true || op["threads"][0]["awaiting_human"] == true
    };
    let working = read(&run);
    assert!(!done(&working) && !stopped(&working), "{working}");
    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let asked = read(&run);
    assert!(stopped(&asked) && !done(&asked), "{asked}");
    run.withdraw(run.owner(), &run.root.clone()).unwrap();
    run.until_view("the run finished", |view| {
        matches!(view, RunView::Finished { .. })
    });
    let finished = read(&run);
    assert!(done(&finished) && !stopped(&finished), "{finished}");
}

// ---- the flags a run's threads do not take (review pass 2) -------------------------------------

/// A run's root is answered plainly, or with `--accept` at a ceiling: `--escalate` and `--needs-rework` are
/// a step's words, refused on the root before anything is written — even from the requester, with a
/// question open.
#[test]
fn escalate_and_needs_rework_on_a_v2_root_are_refused_before_anything_is_written() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(escalating));
    run.until_view("the escalation", |view| {
        matches!(view, RunView::NeedsDecision { .. })
    });
    let root = run.root.clone();
    let before = run.ops();
    for (escalate, needs_rework) in [(true, false), (false, true)] {
        let refused = run
            .reply_with(
                run.owner(),
                ReplyThreadRequest {
                    thread: &root,
                    body: "this is not a step",
                    escalate,
                    needs_rework,
                    accept: false,
                    machine: None,
                },
            )
            .expect_err("not a root's words");
        assert_eq!(refused.kind, ErrorKind::Validation, "{refused}");
        assert_eq!(
            refused.msg,
            "a run's root thread is answered plainly, or with `--accept` at a ceiling; \
             `--escalate` and `--needs-rework` are said in a step's thread — nothing was written"
        );
    }
    assert_eq!(run.ops(), before);
    assert!(run.inputs().is_empty(), "{:?}", run.inputs());
}

/// A run on the new engine runs on this machine: `--machine` names nothing on its threads, on a step's as on
/// the root, and is refused before anything is written.
#[test]
fn machine_on_a_v2_step_thread_or_root_is_refused_before_anything_is_written() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let root = run.root.clone();
    let before = run.ops();
    for (caller, thread) in [
        (as_session("coder", key.as_str()), step.as_str()),
        (run.owner(), root.as_str()),
    ] {
        let refused = run
            .reply_with(
                caller,
                ReplyThreadRequest {
                    thread,
                    body: "over there",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                    machine: Some("the-other-laptop"),
                },
            )
            .expect_err("no machine on a run's thread");
        assert_eq!(refused.kind, ErrorKind::Validation, "{thread}: {refused}");
        assert_eq!(
            refused.msg,
            "a run on the new engine runs on this machine; `--machine` names nothing here — \
             nothing was written"
        );
    }
    assert_eq!(run.ops(), before);
    assert!(run.inputs().is_empty(), "{:?}", run.inputs());
}

/// The old sidecar's teardown settle (`--if-unanswered`) has no place in a run on the new engine: refused on
/// a v2 step thread, with nothing written to the chat store and nothing handed to the run.
#[test]
fn if_unanswered_on_a_v2_step_thread_is_refused_and_writes_nothing() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let step = run.thread_of(&key);
    let now = run.now_text();
    let before = run.ops();
    let said = stderr_of_a_refusal(nxc_as(run.dir(), &now, "coder", &key).args([
        "reply",
        "--thread",
        &step,
        "--if-unanswered",
        "the session ended",
    ]));
    assert_eq!(
        said,
        format!(
            "error: thread {step} belongs to a run on the new engine, whose host settles its own \
             sessions; `--if-unanswered` is the old sidecar's — answer with a plain `nxc reply`. \
             Nothing was written\n"
        )
    );
    assert_eq!(run.ops(), before);
    assert!(run.inputs().is_empty(), "{:?}", run.inputs());
}

/// The three lines the CLI prints for a run's resume and withdrawal, pinned whole: a substring test cannot
/// see a run of blanks inside a line (review pass 2, Code Quality #1).
#[test]
fn nxc_resume_and_withdraw_print_their_lines_whole() {
    let mut held = V2Run::commissioned("coding-v2", Box::new(on_the_usage_limit));
    held.until_view("the hold", |view| matches!(view, RunView::Held { .. }));
    let now = held.now_text();
    let root = held.root.clone();
    let run = held.run.to_string();
    let until = json(nxc(held.dir(), &now).args(["--json", "resume", "--thread", &root]))["until"]
        .as_str()
        .expect("not yet names its instant")
        .to_owned();
    assert_eq!(
        stdout(nxc(held.dir(), &now).args(["resume", "--thread", &root])),
        format!(
            "not yet: run {run} is held until {until} and restarts on its own then. `--force` \
             restarts it now\n"
        )
    );
    assert_eq!(
        stdout(nxc(held.dir(), &now).args(["resume", "--thread", &root, "--force"])),
        format!(
            "resumed run {run}: its host restarts it on its next tick — follow it with `nxc \
             status --thread {root}`\n"
        )
    );
    assert_eq!(
        stdout(nxc(held.dir(), &now).args(["withdraw", "--thread", &root])),
        format!(
            "withdrew run {run} on thread {root}: its host stops its sessions and parks any work \
             on its next tick — follow it with `nxc status --thread {root}`\n"
        )
    );
}

/// A session key whose recorded thread no run routes is refused with the reason, never handed on as a thread
/// id an id-free reply would then post into (review pass 2, Code Quality #3 / Integrity #1).
#[test]
fn a_session_whose_thread_routes_nowhere_is_refused_rather_than_given_an_empty_thread() {
    let mut run = V2Run::commissioned("coding-v2", Box::new(silent));
    let key = run.build_key();
    let stray = format!("{}/build/v1/coder/a9", run.run);
    run.store
        .compare_and_set(
            &format!("session/{stray}"),
            None,
            Some(b"m-ROUTESNOWHERE".as_slice()),
        )
        .unwrap();
    let defs = nexus_chat::definitions::Definitions::resolve(run.dir()).unwrap();
    let db = db_path(run.dir());
    let refused = nexus_chat::v2_bridge::session_of(&db, &run.eng.origin(), &defs, Some(&stray))
        .expect_err("a thread that routes nowhere");
    assert!(refused.msg.contains("m-ROUTESNOWHERE"), "{refused}");
    let found = nexus_chat::v2_bridge::session_of(&db, &run.eng.origin(), &defs, Some(&key))
        .unwrap()
        .expect("the build session");
    assert_eq!(found.thread, run.thread_of(&key));
    assert_eq!(
        nexus_chat::v2_bridge::session_of(&db, &run.eng.origin(), &defs, Some("no-such-key"))
            .unwrap(),
        None
    );
}
