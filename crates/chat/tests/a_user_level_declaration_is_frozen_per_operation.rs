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
use nexus_chat::precondition::PreconditionOutcome;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-10-04T10:00:00Z";

/// Records every start and every hurdle it is asked to run; every hurdle passes.
#[derive(Default)]
struct RecordingWorker(Mutex<Vec<TriggerRequest>>, Mutex<Vec<String>>);

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.0.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn run_precondition(&self, command: &str) -> PreconditionOutcome {
        self.1.lock().unwrap().push(command.to_string());
        PreconditionOutcome::Ran {
            status: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
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

/// Both personas AND the channel's hurdle carry the version, so an edit changes a prompt and a
/// piece of executable text from the user-level folder at once (review of PR #17, Code Quality #6).
fn declare_version(user: &std::path::Path, version: &str) {
    for handle in ["coder", "finisher"] {
        std::fs::write(
            user.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: {version}\n"),
        )
        .unwrap();
    }
    std::fs::write(
        user.join("channels.yaml"),
        format!(
            "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n  preconditions:\n    \
             - name: gate\n      run: hurdle-{version}\n"
        ),
    )
    .unwrap();
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
    // The pinning `nxs_test_support::PinHome` gives a subprocess, applied to THIS process: the
    // home, the XDG directories and the instance together (nxf 6j6v.9bjv). Process-wide, which is
    // safe because this binary has exactly one test.
    for (key, value) in nxs_test_support::pinned_home_env() {
        std::env::set_var(key, value);
    }
    // A build reads a user-level folder only under a named development instance.
    std::env::set_var("NXS_SERVICE_INSTANCE", "nexus-flow-test");
    let user = nexus_chat::definitions::user_declarations_dir().expect("a named instance's folder");
    std::fs::create_dir_all(&user).unwrap();
    declare_version(&user, "VERSION-ONE");

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
    // …and so does the HURDLE before that step: executable text from the user-level folder is
    // frozen with the rest.
    let asked = worker.1.lock().unwrap().clone();
    assert!(
        !asked.iter().any(|c| c == "hurdle-VERSION-TWO"),
        "the edited hurdle reached the running operation: {asked:?}"
    );
    assert!(
        asked.len() >= 2,
        "the hurdle was asked before each step: {asked:?}"
    );
    assert!(asked.iter().all(|c| c == "hurdle-VERSION-ONE"), "{asked:?}");

    // The next operation takes the edit.
    open(&engine);
    let coders = worker.prompts_of("coder");
    assert_eq!(coders.len(), 2);
    assert!(coders[1].contains("VERSION-TWO"), "{coders:?}");
    assert_eq!(
        worker.1.lock().unwrap().last().map(String::as_str),
        Some("hurdle-VERSION-TWO"),
        "the next operation asks the edited hurdle"
    );
}
