//! **What the service's host gives a v2 session to know where it is** (6j6v.pn9c; epic 6j6v.zjyv §6.6): a
//! session's identity is its environment, never its directory. `v2_bridge::service_layout` puts `NXC_DB`, the
//! registered root's database, and `NXC_ORIGIN`, the chat origin, beside the allowlist, and the host passes
//! them into every start spec. Driven here through a real `Host` over that layout, read back from the spec the
//! backend was handed — and resolved from the registered root even when the workspace was found from `-v2`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::v2_bridge::{run_sqlite, service_layout};
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, origin_of, setup, Workspace};
use nexus_run_contracts::Timestamp;
use nexus_run_host::bridge::{BridgeCtx, SqliteRunStore};
use nexus_run_host::test_support::scripted_backend::{Cue, ScriptedBackend, SessionInfo};
use nexus_run_host::test_support::scripted_tree::ScriptedTree;
use nexus_run_host::test_support::SimClock;
use nexus_run_host::{Host, Ports, TreeLedger};
use tempfile::TempDir;

const NOW: &str = "2026-11-14T10:00:00Z";
const CODER: &str =
    "handle: coder\njob_title: Implementer\naddressable: none\nsystem_prompt: You build.\n";
const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
";

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

/// The registered root, and `<root>-v2` linked to its `.nxs`.
fn dirs() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    setup(&root, &chat_config()).unwrap();
    let personas = root.join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(personas.join("coder.yaml"), CODER).unwrap();
    std::fs::write(personas.join("channels.yaml"), CHANNELS).unwrap();
    let v2 = root.with_file_name("ws-v2");
    std::fs::create_dir_all(&v2).unwrap();
    std::os::unix::fs::symlink(root.join(".nxs"), v2.join(".nxs")).unwrap();
    (tmp, root, v2)
}

fn env_of(spec: &serde_json::Value, name: &str) -> Option<String> {
    spec["env"]
        .as_array()?
        .iter()
        .find(|v| v["name"] == name)
        .and_then(|v| v["value"].as_str())
        .map(str::to_owned)
}

/// Every start spec the backend was handed, under `<agent-logs>/v2/`.
fn specs(agent_logs: &Path) -> Vec<serde_json::Value> {
    let mut found = Vec::new();
    let mut stack = vec![agent_logs.join("v2")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "spec.json") {
                found.push(serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap());
            }
        }
    }
    found
}

#[test]
fn every_v2_session_is_given_the_registered_roots_database_and_the_chat_origin() {
    let (_tmp, root, v2) = dirs();
    let at_root = Workspace::resolve(None, &root).unwrap();
    // Found from the clone, the layout is still the registered root's.
    let from_v2 = Workspace::resolve(None, &v2).unwrap();
    let layout = service_layout(&from_v2).unwrap();
    assert_eq!(layout.root, root);
    assert_eq!(layout.v2_dir(), v2);
    let db = at_root.db_path_str().unwrap();
    let origin = origin_of(&at_root).to_owned();

    // The owner commissions, and a real host over that layout starts the build slot.
    let eng = Engine::open_with(
        None,
        &root,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    eng.send_to(
        Caller {
            session: None,
            actor: Some("owner"),
            now: Some(NOW),
        },
        SendToRequest {
            to: "coding-v2",
            body: "build it",
            refs: SendToRefs::declared(nexus_chat::model::Refs {
                nxf_ids: vec!["6j6v.pn9c".to_owned()],
                ..nexus_chat::model::Refs::default()
            }),
            machine: None,
        },
    )
    .unwrap();
    // A named sidecar: the scripted backend never runs it (memory v2-host-tests-name-a-sidecar).
    let ports = Arc::new(
        nexus_chat::ports_v1::V1Ports::for_root(&layout.root, &db, &origin)
            .unwrap()
            .with_sidecar(root.join("sidecar.mjs")),
    );
    let store = Arc::new(
        SqliteRunStore::open(&run_sqlite(&db), nexus_run_host::bridge::HOST_BUSY_TIMEOUT).unwrap(),
    );
    let clock = Arc::new(SimClock::at(Timestamp::parse(NOW).unwrap().unix_millis()));
    let backend = Arc::new(ScriptedBackend::new(
        &layout.agent_logs,
        &root.join("timeline.jsonl"),
        &layout.v2_dir(),
        Box::new(silent),
    ));
    let tree = Arc::new(ScriptedTree::new(TreeLedger::new(
        &layout.agent_logs,
        &origin,
    )));
    let mut host = Host::new(
        store,
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
                &run_sqlite(&db),
                nexus_run_host::bridge::BRIDGE_BUSY_TIMEOUT,
            )
            .unwrap(),
        ),
        messages: ports.clone(),
        declarations: ports,
        layout: layout.clone(),
        workspace_id: origin.clone(),
    };
    let ceiling = std::time::Instant::now() + std::time::Duration::from_secs(600);
    while specs(&layout.agent_logs).is_empty() {
        assert!(
            std::time::Instant::now() < ceiling,
            "no session was started"
        );
        let now = nexus_run_host::ports::Clock::now(&*clock);
        host.tick(now);
        backend.advance(&bridge, now);
        if tree.busy() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            clock.advance(1_000);
        }
    }

    for spec in specs(&layout.agent_logs) {
        assert_eq!(env_of(&spec, "NXC_DB"), Some(db.clone()), "{spec}");
        assert_eq!(env_of(&spec, "NXC_ORIGIN"), Some(origin.clone()), "{spec}");
        assert_eq!(spec["cwd"].as_str().map(PathBuf::from), Some(v2.clone()));
    }
}
