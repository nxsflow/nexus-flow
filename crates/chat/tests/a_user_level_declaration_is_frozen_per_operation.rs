//! **The per-operation freeze holds for the user-level folder** (nxf 6j6v.7k58, nxf 6j6v.n92p).
//!
//! A declaration arriving from `~/.nexusflow/personas/` is executable text from a second place —
//! a persona's prompt, a channel's hurdles. The freeze projects the MERGED catalogue, so an edit to
//! the user-level folder made while an operation runs reaches the next operation and never the
//! running one, exactly as an edit to the workspace's own folder does (`tests/declaration_freeze.rs`).
//!
//! `$HOME` is pinned, and that is process-wide: ONE test in a binary of its own.

use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-10-04T10:00:00Z";

#[derive(Default)]
struct RecordingWorker(Mutex<Vec<TriggerRequest>>);

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.0.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
}

impl RecordingWorker {
    fn prompts_of(&self, handle: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .map(|r| r.role.system_prompt.clone())
            .collect()
    }
}

fn declare_version(user: &std::path::Path, version: &str) {
    for handle in ["coder", "finisher"] {
        std::fs::write(
            user.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: {version}\n"),
        )
        .unwrap();
    }
}

fn open(engine: &Engine) -> String {
    engine
        .send_to(
            Caller {
                session: None,
                actor: Some("pm"),
                now: Some(NOW),
            },
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the user-level channel opens")
        .thread_id
}

#[test]
fn an_edit_to_the_user_level_folder_reaches_the_next_operation_and_not_the_running_one() {
    let home = TempDir::new().unwrap();
    // SAFETY of the mutation: this binary has exactly one test, and nothing else runs in it.
    std::env::set_var("HOME", home.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    std::env::remove_var("XDG_DATA_HOME");
    let user = nexus_chat::definitions::user_declarations_dir().unwrap();
    std::fs::create_dir_all(&user).unwrap();
    declare_version(&user, "VERSION-ONE");
    std::fs::write(
        user.join("channels.yaml"),
        "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n",
    )
    .unwrap();

    // A repository that declares nothing of its own.
    let repo = TempDir::new().unwrap();
    setup(repo.path(), &chat_config()).unwrap();
    let worker = Arc::new(RecordingWorker::default());
    let engine = Engine::open_with(
        None,
        repo.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .unwrap();

    open(&engine);
    let coder = worker
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.role.handle == "coder")
        .cloned()
        .expect("the first step started");
    assert!(coder.role.system_prompt.contains("VERSION-ONE"));

    // The owner edits the user-level folder mid-operation.
    declare_version(&user, "VERSION-TWO");
    engine
        .reply_thread(
            Caller {
                session: Some(&coder.internal_session),
                actor: None,
                now: Some(NOW),
            },
            ReplyThreadRequest {
                machine: None,
                thread: coder.reply_thread.as_deref().unwrap(),
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap();
    let finisher = worker.prompts_of("finisher");
    assert_eq!(finisher.len(), 1, "the flow advanced");
    assert!(
        finisher[0].contains("VERSION-ONE"),
        "the running operation keeps the version it opened with: {finisher:?}"
    );

    // The next operation takes the edit.
    open(&engine);
    let coders = worker.prompts_of("coder");
    assert_eq!(coders.len(), 2);
    assert!(coders[1].contains("VERSION-TWO"), "{coders:?}");
}
