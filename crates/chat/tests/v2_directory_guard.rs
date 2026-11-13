//! **The old engine starts nothing in `<registered root>-v2`** (6j6v.pn9c; epic 6j6v.zjyv §6.6, §10, §12.9, DoD
//! 12; round 6 CQ #13, TQ #12, IR #1). The new engine's runs work in their own clone beside the registered
//! root, and its `.nxs` link resolves the same workspace — so every old verb would work from there, and the
//! ones that start a session would start it THERE, two coders in one tree.
//!
//! The guard sits at the SINK: the one place the old engine hands a session to its worker, below the CLI and
//! below `Engine::*`. It recognises exactly `<root>-v2` whose `.nxs` resolves where the root's does, and names
//! the root to run the verb from. `send`, `reply` and `withdraw` on the old engine are also refused BEFORE
//! anything is written (§12.9); every other verb that reaches a spawn is refused at the sink. Another
//! checkout linked to the same workspace keeps its verbs, and the new engine's verbs and every read work from
//! the v2 directory.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::Refs;
use nexus_chat::orchestration::{Caller, ResumeRequest, ResumeScope};
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::v2_bridge::root_of_v2_dir;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-11-13T10:00:00Z";

/// Three personas, a v1 channel with a one-minute window, a v1 channel that hands its working copy from one step to the next, and the new engine's
/// channel — as the REGISTERED root declares them.
const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
- name: pair
  members: [coder, reviewer]
  working_tree: exclusive
  steps:
    - id: build
      target: coder
      next: check
    - id: check
      target: reviewer
- name: timed
  members: [reviewer]
  timeout: 1m
";

/// The clone's own copy, from an older `origin/main`: it does not know `coding-v2` yet. A send typed in the
/// v2 directory compiles from the registered root's declarations, never from these (b8tj note 5).
const CLONE_CHANNELS: &str = "\
- name: pair
  members: [coder, reviewer]
  steps:
    - id: build
      target: coder
";

/// A workspace as the service sees it: the registered root, its v2 directory, and another checkout linked
/// to the same `.nxs`.
struct Dirs {
    _tmp: TempDir,
    base: PathBuf,
    root: PathBuf,
    v2: PathBuf,
    other: PathBuf,
}

fn personas(dir: &Path, channels: &str) {
    let folder = dir.join(".nxs-personas");
    std::fs::create_dir_all(&folder).unwrap();
    for handle in ["coder", "reviewer", "pm"] {
        std::fs::write(
            folder.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(folder.join("channels.yaml"), channels).unwrap();
}

fn linked(dir: &Path, root: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::os::unix::fs::symlink(root.join(".nxs"), dir.join(".nxs")).unwrap();
}

fn dirs() -> Dirs {
    let tmp = TempDir::new().unwrap();
    // The physical path: a process's cwd is, and the guard compares what the caller resolved.
    let base = tmp.path().canonicalize().unwrap();
    let root = base.join("ws");
    std::fs::create_dir_all(&root).unwrap();
    setup(&root, &chat_config()).expect("seed chat workspace");
    personas(&root, CHANNELS);
    let v2 = base.join("ws-v2");
    linked(&v2, &root);
    personas(&v2, CLONE_CHANNELS);
    let other = base.join("other-checkout");
    linked(&other, &root);
    personas(&other, CHANNELS);
    Dirs {
        _tmp: tmp,
        base,
        root,
        v2,
        other,
    }
}

impl Dirs {
    fn dry_log(&self) -> PathBuf {
        self.base.join("dry.log")
    }

    fn engine(&self, dir: &Path) -> Engine {
        Engine::open_with(
            None,
            dir,
            EngineConfig {
                worker: WorkerConfig::Dry {
                    log: Some(self.dry_log()),
                },
                timer: TimerConfig::Disabled,
                ..EngineConfig::default()
            },
        )
        .expect("open engine")
    }

    /// Every write to the chat store is an op: the count is what "before anything is written" is measured by.
    fn ops(&self) -> i64 {
        rusqlite::Connection::open(self.root.join(".nxs").join("db.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM ops", [], |row| row.get(0))
            .unwrap()
    }

    /// The sessions the old engine handed its worker, in order: `(role, session)`.
    fn started(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(self.dry_log())
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("trigger role="))
            .map(|line| (field(line, "role"), field(line, "session")))
            .collect()
    }

    fn nxc(&self, dir: &Path) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(dir)
            .env_remove("NXC_SESSION")
            .env_remove("NXC_DB")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", NOW)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", self.dry_log())
            .env("NXC_TIMER", "dry")
            .env("NXC_TIMER_LOG", self.base.join("timer.log"));
        c
    }

    fn as_session(&self, dir: &Path, handle: &str, session: &str) -> Command {
        let mut c = self.nxc(dir);
        c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
        c
    }

    /// The owner at the terminal opens a chat with `reviewer` from the ROOT: one v1 thread and one session.
    fn a_v1_chat(&self) -> (String, String) {
        let receipt = json_of(self.nxc(&self.root).args([
            "--json",
            "send",
            "--to",
            "reviewer",
            "look at this",
        ]));
        let thread = receipt["thread_id"].as_str().unwrap().to_owned();
        let session = receipt["session"].as_str().unwrap().to_owned();
        (thread, session)
    }

    /// What the refusal must say: the directory, and the root to run it from.
    fn assert_names_the_root(&self, text: &str) {
        assert!(
            text.contains(&self.root.display().to_string())
                && text.contains(&self.v2.display().to_string()),
            "the refusal names the v2 directory and the root to run it from: {text}"
        );
    }
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

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

/// A run of `nxc` that failed, with what it said on both streams.
fn failed(cmd: &mut Command) -> String {
    let out = cmd.assert().failure();
    let out = out.get_output();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Everything a run of `nxc` said, whatever its exit.
fn said(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn the_owner() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("carsten"),
        now: Some(NOW),
    }
}

fn send_req(to: &str) -> SendToRequest<'_> {
    SendToRequest {
        to,
        body: "build it",
        refs: SendToRefs::declared(Refs {
            nxf_ids: vec!["6j6v.pn9c".to_owned()],
            ..Refs::default()
        }),
        machine: None,
    }
}

// ---- which directory is a v2 directory ----------------------------------------------------------------------

#[test]
fn exactly_the_root_with_v2_appended_is_a_v2_directory() {
    let d = dirs();
    assert_eq!(root_of_v2_dir(&d.v2), Some(d.root.clone()));
    assert_eq!(root_of_v2_dir(&d.root), None, "the root is the root");
    assert_eq!(
        root_of_v2_dir(&d.other),
        None,
        "another linked checkout is no v2 directory"
    );
    // Near misses, each linked to the same workspace: only `-v2`, and only the whole suffix.
    for name in ["ws-v2x", "ws-v3", "ws_v2", "ws-v2-v2"] {
        let near = d.base.join(name);
        linked(&near, &d.root);
        assert_eq!(root_of_v2_dir(&near), None, "{name} is no v2 directory");
    }
    // Named right, but its `.nxs` resolves elsewhere: a directory of another workspace.
    let stranger = d.base.join("lone");
    std::fs::create_dir_all(&stranger).unwrap();
    setup(&stranger, &chat_config()).unwrap();
    let stranger_v2 = d.base.join("lone-v2");
    std::fs::create_dir_all(&stranger_v2).unwrap();
    setup(&stranger_v2, &chat_config()).unwrap();
    assert_eq!(
        root_of_v2_dir(&stranger_v2),
        None,
        "a sibling with a workspace of its own is not that root's v2 directory"
    );
    // A v2 directory whose root has no `.nxs` at all.
    let orphan = d.base.join("gone-v2");
    linked(&orphan, &d.root);
    assert_eq!(root_of_v2_dir(&orphan), None);
}

// ---- send, reply and withdraw: refused before anything is written ------------------------------------------

#[test]
fn a_v1_send_typed_in_the_v2_directory_is_refused_before_any_write_and_names_the_root() {
    let d = dirs();
    let before = d.ops();
    for to in ["pair", "reviewer"] {
        let err = d
            .engine(&d.v2)
            .send_to(the_owner(), send_req(to))
            .expect_err("a v1 send from the v2 directory is refused");
        d.assert_names_the_root(&err.to_string());
        let text = failed(d.nxc(&d.v2).args(["send", "--to", to, "build it"]));
        d.assert_names_the_root(&text);
    }
    assert_eq!(d.ops(), before, "nothing was written");
    assert!(d.started().is_empty(), "no session was started");
    // Another linked checkout keeps its v1 verbs, at the Engine seam and through the CLI.
    d.engine(&d.other)
        .send_to(the_owner(), send_req("pair"))
        .expect("another checkout sends");
    json_of(
        d.nxc(&d.other)
            .args(["--json", "send", "--to", "reviewer", "build it"]),
    );
    assert_eq!(
        d.started()
            .iter()
            .map(|(r, _)| r.as_str())
            .collect::<Vec<_>>(),
        ["coder", "reviewer"]
    );
}

#[test]
fn a_reply_on_a_v1_thread_typed_in_the_v2_directory_is_refused_before_any_write() {
    let d = dirs();
    let (thread, _) = d.a_v1_chat();
    let before = (d.ops(), d.started().len());
    let err = d
        .engine(&d.v2)
        .reply_thread(
            the_owner(),
            ReplyThreadRequest {
                thread: &thread,
                body: "and this",
                escalate: false,
                needs_rework: false,
                accept: false,
                machine: None,
            },
        )
        .expect_err("refused at the Engine seam");
    d.assert_names_the_root(&err.to_string());
    let text = failed(
        d.nxc(&d.v2)
            .args(["reply", "--thread", &thread, "and this"]),
    );
    d.assert_names_the_root(&text);
    assert_eq!(
        (d.ops(), d.started().len()),
        before,
        "nothing was written or started"
    );
    // From the root it is an ordinary reply.
    json_of(
        d.nxc(&d.root)
            .args(["--json", "reply", "--thread", &thread, "and this"]),
    );
}

#[test]
fn a_withdraw_of_a_v1_operation_typed_in_the_v2_directory_is_refused_before_any_write() {
    let d = dirs();
    let (thread, _) = d.a_v1_chat();
    let before = d.ops();
    let err = d
        .engine(&d.v2)
        .withdraw(the_owner(), &thread)
        .expect_err("refused at the Engine seam");
    d.assert_names_the_root(&err.to_string());
    let text = failed(d.nxc(&d.v2).args(["withdraw", "--thread", &thread]));
    d.assert_names_the_root(&text);
    assert_eq!(d.ops(), before, "nothing was written");
}

// ---- every other verb that starts a session: refused at the sink -------------------------------------------

#[test]
fn a_resume_typed_in_the_v2_directory_starts_nothing_and_names_the_root() {
    let d = dirs();
    let (thread, session) = d.a_v1_chat();
    // The session stops on a limit; its window has passed.
    d.nxc(&d.root)
        .args([
            "session",
            "interrupted",
            &session,
            "--limit",
            "five_hour",
            "--until",
            "2026-11-13T09:00:00Z",
        ])
        .assert()
        .success();
    let started = d.started().len();
    let err = d
        .engine(&d.v2)
        .resume_interrupted(
            the_owner(),
            ResumeRequest {
                scope: ResumeScope::Thread(&thread),
                force: true,
            },
        )
        .map(|receipt| format!("{receipt:?}"))
        .unwrap_or_else(|e| e.to_string());
    d.assert_names_the_root(&err);
    let text = said(
        d.nxc(&d.v2)
            .args(["resume", "--thread", &thread, "--force"]),
    );
    d.assert_names_the_root(&text);
    assert_eq!(
        d.started().len(),
        started,
        "no session was resumed in the v2 directory"
    );
    // From the root the hold is taken up.
    d.nxc(&d.root)
        .args(["resume", "--thread", &thread, "--force"])
        .assert()
        .success();
    assert_eq!(d.started().len(), started + 1, "the root resumes it");
}

/// A worker that records every start and says every session it started is still running: the copy-handing
/// step then waits for the process to end, which is what `nxc session ended` reports.
#[derive(Default, Debug)]
struct Running {
    started: std::sync::Mutex<Vec<TriggerRequest>>,
}

impl Worker for Running {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.started.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.started
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.internal_session == internal_session)
    }
    fn answers_liveness(&self) -> bool {
        true
    }
}

impl Running {
    fn roles(&self) -> Vec<String> {
        self.started
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }
}

fn running_engine(dir: &Path, worker: &std::sync::Arc<Running>) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

#[test]
fn a_session_end_reported_in_the_v2_directory_starts_no_next_step_there() {
    let d = dirs();
    let worker = std::sync::Arc::new(Running::default());
    let at_root = running_engine(&d.root, &worker);
    let board = at_root
        .send_to(the_owner(), send_req("pair"))
        .expect("the owner commissions")
        .thread_id;
    let coder = worker.started.lock().unwrap()[0].internal_session.clone();
    let slot = common::open_store(&d.root)
        .supervised_children(&board)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    at_root
        .reply_thread(
            Caller {
                session: Some(&coder),
                actor: None,
                now: Some(NOW),
            },
            ReplyThreadRequest {
                thread: &slot,
                body: "built",
                escalate: false,
                needs_rework: false,
                accept: false,
                machine: None,
            },
        )
        .expect("the coder answers its step");
    assert_eq!(
        worker.roles(),
        ["coder"],
        "the check waits for the coder's end"
    );
    // The end, reported in the v2 directory: the check would start there, and is refused at the sink.
    let reported = running_engine(&d.v2, &worker)
        .session_ended(the_owner(), &coder)
        .map(|receipt| format!("{receipt:?}"))
        .unwrap_or_else(|e| e.to_string());
    d.assert_names_the_root(&reported);
    assert_eq!(
        worker.roles(),
        ["coder"],
        "the end reported from the v2 directory started no reviewer there"
    );
}

#[test]
fn a_tick_typed_in_the_v2_directory_wakes_nobody_there_and_a_machine_read_answers() {
    let d = dirs();
    // The pm, a persona session, commissions a channel with a one-minute window from the root.
    let mut store = common::open_store(&d.root);
    store.create_pending_session("s-pm", "pm").unwrap();
    drop(store);
    d.nxc(&d.root)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();
    let board = json_of(d.as_session(&d.root, "pm", "s-pm").args([
        "--json",
        "send",
        "--to",
        "timed",
        "please review",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = d.started().len();
    // Past the window, the tick would wake the pm: typed in the v2 directory it wakes nobody there.
    let text = said(
        d.nxc(&d.v2)
            .env("NXC_NOW", "2026-11-13T10:05:00Z")
            .args(["--json", "tick", "--thread", &board]),
    );
    d.assert_names_the_root(&text);
    assert_eq!(
        d.started().len(),
        started,
        "nobody was woken in the v2 directory"
    );
    // `machine` is a read and starts nothing: it answers from there like `status`.
    d.nxc(&d.v2)
        .args(["machine", "--to", "reviewer"])
        .assert()
        .success();
    assert_eq!(d.started().len(), started);
}

// ---- what keeps working from the v2 directory ---------------------------------------------------------------

#[test]
fn the_new_engines_send_and_every_read_work_from_the_v2_directory_with_the_roots_declarations() {
    let d = dirs();
    // The clone's own declarations do not know `coding-v2`; the registered root's do.
    let receipt = json_of(d.nxc(&d.v2).args([
        "--json",
        "send",
        "--to",
        "coding-v2",
        "--ref",
        "nxf_ids=6j6v.pn9c",
        "build it",
    ]));
    let root_thread = receipt["thread_id"].as_str().unwrap().to_owned();
    assert!(
        receipt["message_id"].is_null(),
        "a v2 commission: {receipt}"
    );
    // Its status from there names the v2 directory as the tree, not `<v2>-v2`.
    let status = json_of(
        d.nxc(&d.v2)
            .args(["--json", "status", "--thread", &root_thread]),
    );
    let text = status.to_string();
    assert!(
        text.contains(&format!("\"{}\"", d.v2.display())),
        "the tree is the v2 directory: {text}"
    );
    assert!(!text.contains(&format!("{}-v2", d.v2.display())), "{text}");
    d.nxc(&d.v2).args(["list"]).assert().success();
    d.nxc(&d.v2).args(["status"]).assert().success();
    assert!(d.started().is_empty(), "the old engine started nothing");
}

// ---- the sink is the only door -------------------------------------------------------------------------------

/// Every session the old engine starts is handed to its worker in ONE place, and the guard stands there: a
/// second call of `Worker::trigger` would be a door the guard does not watch.
#[test]
fn the_old_engine_hands_a_session_to_its_worker_in_one_place_and_the_guard_stands_there() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = Vec::new();
    for entry in walk(&src) {
        let text = std::fs::read_to_string(&entry).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            if code.contains("worker.trigger(") {
                calls.push(format!("{}:{}", entry.display(), n + 1));
            }
        }
    }
    assert_eq!(
        calls.len(),
        1,
        "one call of the worker's trigger in the old engine: {calls:?}"
    );
    let orchestration = std::fs::read_to_string(src.join("orchestration.rs")).unwrap();
    let body = orchestration
        .split("fn trigger_and_bind(")
        .nth(1)
        .expect("the sink");
    let guard = body
        .find("refuse_the_old_engine_here(")
        .expect("the guard is in the sink");
    let trigger = body.find("worker.trigger(").expect("the trigger");
    assert!(guard < trigger, "the guard stands before the trigger");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}
