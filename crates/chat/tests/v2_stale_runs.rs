//! A run on the new engine commissioned under another workspace id (6j6v.4b43): sends from before the workspace
//! had its origin left runs in `run.sqlite` under `local`, by `local/carsten` — an identity nobody at this
//! workspace's terminal is. The owner at the terminal withdraws such a run through the plain `nxc withdraw`, in
//! their own name and recorded as the owner's, with no `NXC_ORIGIN`/`NXC_ACTOR` set to the stale requester. A
//! run of THIS workspace stays its requester's to withdraw, and no session withdraws either.
//!
//! At the seam through `Engine::withdraw`, and once through the `nxc` binary.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::orchestration::Caller;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nexus_run_contracts::{EventBody, LivenessPolicy, RunId, Timestamp, WithdrawnBy};
use nexus_run_host::bridge::{self, BridgeCtx, SqliteRunStore};
use nexus_run_host::ports::Refs;
use nexus_run_host::test_support::scripted_backend::{pass, ScriptedBackend};
use nexus_run_host::test_support::scripted_tree::ScriptedTree;
use nexus_run_host::test_support::SimClock;
use nexus_run_host::{Host, Layout, Ports, TreeLedger};
use tempfile::TempDir;

const NOW: &str = "2026-11-13T10:00:00Z";
const STALE: &str = "local";
const STALE_REQUESTER: &str = "local/carsten";
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

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(personas.join("coder.yaml"), CODER).unwrap();
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

fn db_path(dir: &Path) -> String {
    dir.join(".nxs")
        .join("db.sqlite")
        .to_str()
        .unwrap()
        .to_owned()
}

fn layout(dir: &Path) -> Layout {
    Layout {
        root: dir.to_owned(),
        agent_logs: dir.join(".nxs").join("agent-logs"),
        session_env: Vec::new(),
    }
}

fn ports(dir: &Path, origin: &str) -> Arc<nexus_chat::ports_v1::V1Ports> {
    // A named sidecar: the scripted backend never runs it (memory v2-host-tests-name-a-sidecar).
    Arc::new(
        nexus_chat::ports_v1::V1Ports::for_root(dir, &db_path(dir), origin)
            .unwrap()
            .with_sidecar(dir.join("sidecar.mjs")),
    )
}

fn store(dir: &Path) -> Arc<SqliteRunStore> {
    Arc::new(SqliteRunStore::open(&run_sqlite(dir), bridge::BRIDGE_BUSY_TIMEOUT).unwrap())
}

/// A send from before the workspace had its origin: the same `run.sqlite`, under `local`.
fn stale_run(dir: &Path) -> (RunId, String) {
    let ports = ports(dir, STALE);
    let ctx = BridgeCtx {
        store: store(dir),
        messages: ports.clone(),
        declarations: ports,
        layout: layout(dir),
        workspace_id: STALE.to_owned(),
    };
    let receipt = bridge::commission(
        &ctx,
        "coding-v2",
        "a stale commission",
        &Refs::default(),
        STALE_REQUESTER,
        LivenessPolicy::default(),
        Timestamp::parse(NOW).unwrap(),
    )
    .expect("the stale send is accepted");
    (receipt.run, receipt.root_thread)
}

/// This workspace's service: it opens the stale run's root (and claims nothing for it).
fn host(dir: &Path, eng: &Engine) -> Host {
    let ports = ports(dir, &eng.origin());
    let layout = layout(dir);
    Host::new(
        Arc::new(SqliteRunStore::open(&run_sqlite(dir), bridge::HOST_BUSY_TIMEOUT).unwrap()),
        Ports {
            messages: ports.clone(),
            declarations: ports.clone(),
            sidecar: ports,
            layout: layout.clone(),
        },
        Arc::new(ScriptedBackend::new(
            &layout.agent_logs,
            &dir.join("timeline.jsonl"),
            &layout.v2_dir(),
            Box::new(|_| pass()),
        )),
        Arc::new(ScriptedTree::new(TreeLedger::new(
            &layout.agent_logs,
            &eng.origin(),
        ))),
        Arc::new(SimClock::at(Timestamp::parse(NOW).unwrap().unix_millis())),
        &eng.origin(),
    )
}

fn withdrawals(dir: &Path, run: &RunId) -> Vec<WithdrawnBy> {
    store(dir)
        .waiting_inputs(run)
        .unwrap()
        .into_iter()
        .filter_map(|input| match input.unwrap().body {
            EventBody::Withdrawn { by, .. } => Some(by),
            _ => None,
        })
        .collect()
}

fn at_the_terminal(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

#[test]
fn the_owner_at_the_terminal_withdraws_a_run_of_another_workspace_as_the_owner() {
    let ws = workspace();
    let eng = engine(ws.path());
    let (run, root) = stale_run(ws.path());
    let receipt = eng
        .withdraw(at_the_terminal("owner"), &root)
        .expect("the owner withdraws a stale run");
    assert_eq!(receipt.run.as_deref(), Some(run.as_str()));
    assert_eq!(withdrawals(ws.path(), &run), [WithdrawnBy::Owner]);
}

/// With its root open, the withdrawal is posted there — in the owner's own name, never the stale requester's —
/// and says whose run it was.
#[test]
fn the_owners_withdrawal_is_posted_on_the_root_in_the_owners_own_name() {
    let ws = workspace();
    let eng = engine(ws.path());
    let (run, root) = stale_run(ws.path());
    let mut host = host(ws.path(), &eng);
    let now = Timestamp::parse(NOW).unwrap();
    for _ in 0..5 {
        host.tick(now);
    }
    let before = chat_store(ws.path())
        .messages_in_thread(&root)
        .unwrap()
        .len();
    assert!(before > 0, "the host opened the root");
    eng.withdraw(at_the_terminal("owner"), &root)
        .expect("the owner withdraws");
    let messages = chat_store(ws.path()).messages_in_thread(&root).unwrap();
    assert_eq!(messages.len(), before + 1);
    let posted = messages.last().unwrap();
    assert_eq!(posted.sender, format!("{}/owner", eng.origin()));
    assert!(posted.body.contains(STALE_REQUESTER), "{}", posted.body);
    assert_eq!(withdrawals(ws.path(), &run), [WithdrawnBy::Owner]);
}

#[test]
fn no_session_withdraws_a_run_of_another_workspace() {
    let ws = workspace();
    let eng = engine(ws.path());
    let (run, root) = stale_run(ws.path());
    let session = "s-coder";
    chat_store(ws.path())
        .create_pending_session(session, "coder")
        .unwrap();
    for caller in [
        Caller {
            session: Some(session),
            actor: Some("coder"),
            now: Some(NOW),
        },
        Caller {
            session: Some("s-unknown"),
            actor: Some("owner"),
            now: Some(NOW),
        },
    ] {
        let refused = eng.withdraw(caller, &root).expect_err("a session");
        assert_eq!(refused.kind, ErrorKind::Forbidden, "{refused}");
    }
    assert!(withdrawals(ws.path(), &run).is_empty());
}

/// The owner's exit is for a run nobody here can act for: on a run of this workspace, somebody who is not its
/// requester is refused as before.
#[test]
fn a_run_of_this_workspace_stays_its_requesters_to_withdraw() {
    let ws = workspace();
    let eng = engine(ws.path());
    let receipt = eng
        .send_to(
            at_the_terminal("owner"),
            SendToRequest {
                to: "coding-v2",
                body: "build it",
                refs: SendToRefs::declared(nexus_chat::model::Refs::default()),
                machine: None,
            },
        )
        .expect("the owner commissions");
    let refused = eng
        .withdraw(at_the_terminal("someone-else"), &receipt.thread_id)
        .expect_err("not the requester");
    assert_eq!(refused.kind, ErrorKind::Forbidden, "{refused}");
    assert!(
        refused.msg.contains("who commissioned this run"),
        "{refused}"
    );
}

#[test]
fn nxc_withdraw_ends_a_run_of_another_workspace_with_no_identity_borrowed() {
    let ws = workspace();
    let (run, root) = stale_run(ws.path());
    nxs_test_support::cargo_bin("nxc")
        .current_dir(ws.path())
        .env_remove("NXC_SESSION")
        .env_remove("NXC_DB")
        .env_remove("NXC_ORIGIN")
        .env("NXC_ACTOR", "owner")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .args(["withdraw", "--thread", &root])
        .assert()
        .success();
    assert_eq!(withdrawals(ws.path(), &run), [WithdrawnBy::Owner]);
}
