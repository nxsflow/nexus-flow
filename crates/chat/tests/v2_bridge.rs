//! The bridge between the two engines, part 1 (6j6v.b8tj; epic 6j6v.zjyv §10, §12.9): a channel declared
//! `engine: v2` is commissioned through `run_host::commission`, below both callers of `send_to`, and the old
//! engine is never entered for it; no persona and no v2 session commissions one.
//!
//! Each case runs at the `surface` seam through `Engine::send_to`, which is `surface::send_to` and nothing else,
//! and the ones the ticket names once more through the `nxc` binary.

mod common;

use std::path::{Path, PathBuf};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::Refs;
use nexus_chat::orchestration::Caller;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToReceipt, SendToRefs, SendToRequest, TargetKind};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nexus_run_contracts::RunStore;
use tempfile::TempDir;

const NOW: &str = "2026-11-12T10:00:00Z";
const CODER: &str = "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n";
const REVIEWER: &str = "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You review.\n";
/// One channel on each engine: `coding-v2` as the repository declares it in miniature, and a v1 `review`.
const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
- name: review
  members: [coder, reviewer]
";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(personas.join("coder.yaml"), CODER).unwrap();
    std::fs::write(personas.join("reviewer.yaml"), REVIEWER).unwrap();
    std::fs::write(personas.join("channels.yaml"), CHANNELS).unwrap();
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

fn run_sqlite(dir: &Path) -> PathBuf {
    dir.join(".nxs").join("run.sqlite")
}

fn the_owner() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("owner"),
        now: Some(NOW),
    }
}

fn about_the_ticket() -> SendToRefs {
    SendToRefs::declared(Refs {
        nxf_ids: vec!["6j6v.b8tj".to_owned()],
        ..Refs::default()
    })
}

fn send(engine: &Engine, caller: Caller<'_>, to: &str) -> nexus_chat::error::Result<SendToReceipt> {
    engine.send_to(
        caller,
        SendToRequest {
            to,
            body: "build it",
            refs: about_the_ticket(),
            machine: None,
        },
    )
}

#[test]
fn a_send_from_the_terminal_to_a_v2_channel_commissions_a_run_and_answers_with_its_root_at_once() {
    let ws = workspace();
    let receipt =
        send(&engine(ws.path()), the_owner(), "coding-v2").expect("the owner commissions");

    assert_eq!(receipt.target, TargetKind::Channel);
    assert_eq!(receipt.to, "coding-v2");
    assert_eq!(
        receipt.message_id, None,
        "the root message is the host's to post, so the receipt names none"
    );
    let root = receipt.thread_id.clone();
    assert!(root.starts_with("m-"), "a chat thread id: {root}");

    let store = nexus_run_host::bridge::open_store(&run_sqlite(ws.path()))
        .expect("run.sqlite opens")
        .expect("the commission created run.sqlite");
    assert!(
        store.get(&format!("thread/{root}")).unwrap().is_some(),
        "the root thread maps to the run"
    );
    assert_eq!(store.open_runs().unwrap().len(), 1, "one run, commissioned");

    let chat = chat_store(ws.path());
    assert!(
        chat.messages_in_thread(&root).unwrap().is_empty(),
        "nothing is posted before the log can answer for it, and the old engine never posts it"
    );
    assert!(
        chat.threads_opened_by("__channel__").unwrap().is_empty(),
        "channel_open was never entered"
    );
}
